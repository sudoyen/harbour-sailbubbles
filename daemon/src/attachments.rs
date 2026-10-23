use std::fs::{self, File};
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rustpush::{APSConnectionResource, Attachment, MMCSFile};
use sha2::{Digest, Sha256};

use crate::store;

/// Larger attachments wait until the user asks for them.
pub const AUTO_DOWNLOAD_MAX: i64 = 25 * 1024 * 1024;

/// Apple refuses larger iMessage attachments.
pub const SEND_MAX: u64 = 100 * 1024 * 1024;

// Folders the sandboxed app can see itself. The daemon runs outside the
// sandbox, so it sends nothing from elsewhere, whatever the caller asks.
const SENDABLE_DIRS: &[&str] = &["Pictures", "Videos", "Music", "Documents", "Downloads"];

/// Under the app's own data folder, the one place Sailjail lets the
/// sandboxed app read and the daemon (outside the sandbox) write.
pub fn root() -> Result<PathBuf> {
    Ok(home()?.join(".local/share/org.sailbubbles/harbour-sailbubbles/attachments"))
}

pub fn home() -> Result<PathBuf> {
    Ok(PathBuf::from(std::env::var_os("HOME").context("HOME not set")?))
}

/// "<message id>:<index>", as the app names an attachment.
pub fn id(message_id: &str, idx: u32) -> String {
    format!("{message_id}:{idx}")
}

pub fn parse_id(id: &str) -> Option<(&str, u32)> {
    let (message, idx) = id.rsplit_once(':')?;
    Some((message, idx.parse().ok()?))
}

// Names come from the sender; keep them to plain characters so they cannot
// leave the message's folder.
fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .take(80)
        .collect();
    match cleaned.trim_start_matches('.') {
        "" => "attachment".to_string(),
        rest => rest.to_string(),
    }
}

pub fn path_for(root: &Path, message_id: &str, idx: u32, name: &str) -> PathBuf {
    root.join(safe_name(message_id)).join(format!("{idx}-{}", safe_name(name)))
}

pub fn remove(root: &Path, message_id: &str) {
    let dir = root.join(safe_name(message_id));
    if let Err(e) = fs::remove_dir_all(&dir) {
        if e.kind() != io::ErrorKind::NotFound {
            log::error!("removing attachment files failed: {e}");
        }
    }
}

fn is_heic(mime: &str, name: &str) -> bool {
    let name = name.to_lowercase();
    matches!(mime.to_lowercase().as_str(), "image/heic" | "image/heif")
        || name.ends_with(".heic") || name.ends_with(".heif")
}

#[cfg(sailfish)]
fn heic_to_jpeg(input: &Path, output: &Path) -> Result<()> {
    use std::ffi::{c_char, c_int, CStr, CString};
    use std::os::unix::ffi::OsStrExt;

    extern "C" {
        fn obd_heic_to_jpeg(input: *const c_char, output: *const c_char, err: *mut c_char, errlen: usize) -> c_int;
    }

    let input = CString::new(input.as_os_str().as_bytes())?;
    let output = CString::new(output.as_os_str().as_bytes())?;
    let mut err = [0 as c_char; 256];
    if unsafe { obd_heic_to_jpeg(input.as_ptr(), output.as_ptr(), err.as_mut_ptr(), err.len()) } != 0 {
        let msg = unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy();
        anyhow::bail!("converting a HEIC photo failed: {msg}");
    }
    Ok(())
}

#[cfg(not(sailfish))]
fn heic_to_jpeg(_input: &Path, _output: &Path) -> Result<()> {
    anyhow::bail!("this build cannot convert HEIC photos")
}

/// Sailfish cannot show HEIC, which iPhones send by default; such photos are
/// turned into JPEG. Returns the file and type the app should use.
pub fn displayable(path: PathBuf, mime: &str, name: &str) -> (PathBuf, String) {
    if !is_heic(mime, name) {
        return (path, mime.to_string());
    }
    let jpeg = path.with_extension("jpg");
    match heic_to_jpeg(&path, &jpeg) {
        Ok(()) => {
            let _ = fs::remove_file(&path);
            (jpeg, "image/jpeg".to_string())
        }
        Err(e) => {
            log::error!("{e}");
            (path, mime.to_string())
        }
    }
}

/// The file at `path`, resolved, if the app may send it.
pub fn sendable(home: &Path, path: &str) -> Result<PathBuf> {
    let file = fs::canonicalize(path).context("no such file")?;
    let user = home.file_name().map(|n| Path::new("/run/media").join(n));
    let allowed = SENDABLE_DIRS.iter().map(|d| home.join(d)).chain(user)
        .any(|dir| fs::canonicalize(dir).is_ok_and(|dir| file.starts_with(dir)));
    if !allowed {
        anyhow::bail!("only files from Pictures, Videos, Music, Documents, Downloads or a memory card can be sent");
    }
    let meta = fs::metadata(&file)?;
    if !meta.is_file() {
        anyhow::bail!("not a file");
    }
    if meta.len() > SEND_MAX {
        anyhow::bail!("larger than 100 MB");
    }
    Ok(file)
}

// The app scales a picked picture to Apple's 570x570 PNG in its own cache,
// the only place it can write to that is not one of the user's folders.
const APP_CACHE: &str = ".cache/org.sailbubbles/harbour-sailbubbles";
const GROUP_PHOTO_MAX: u64 = 5 * 1024 * 1024;

/// The group photo PNG at `path`, resolved, if it is one the app made.
pub fn group_photo_source(home: &Path, path: &str) -> Result<PathBuf> {
    let file = fs::canonicalize(path).context("no such file")?;
    let cache = fs::canonicalize(home.join(APP_CACHE)).context("no app cache folder")?;
    let meta = fs::metadata(&file)?;
    if !file.starts_with(cache) || !meta.is_file() || meta.len() > GROUP_PHOTO_MAX {
        anyhow::bail!("not a group photo made by the app");
    }
    Ok(file)
}

/// Where a group photo of the chat is kept; the version in the name makes a
/// new photo a new file, so the app does not show a cached old one.
pub fn group_photo_path(root: &Path, chat_id: &str, version: u64) -> PathBuf {
    let hash: String = Sha256::digest(chat_id.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect();
    root.join("_groups").join(format!("{hash}-{version}.png"))
}

/// Removes the chat's group photos other than `keep`.
pub fn remove_group_photos(root: &Path, chat_id: &str, keep: Option<&Path>) {
    let current = group_photo_path(root, chat_id, 0);
    let (Some(dir), Some(name)) = (current.parent(), current.file_name()) else { return };
    let prefix = name.to_string_lossy().trim_end_matches("0.png").to_string();
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with(&prefix) && Some(path.as_path()) != keep {
            let _ = fs::remove_file(path);
        }
    }
}

/// Uploads a group photo to Apple's attachment storage.
pub async fn upload_group_photo(conn: &APSConnectionResource, path: &Path) -> Result<MMCSFile> {
    let mut file = File::open(path)?;
    let prepared = MMCSFile::prepare_put(&mut file).await?;
    file.seek(SeekFrom::Start(0))?;
    Ok(MMCSFile::new(conn, &prepared, file, |_, _| {}).await?)
}

/// Fetches a group photo to `dest`, through a temporary file.
pub async fn download_group_photo(conn: &APSConnectionResource, photo: &MMCSFile, dest: &Path) -> Result<()> {
    store::ensure_private_dir(dest.parent().context("no folder")?)?;
    let tmp = dest.with_extension("part");
    photo.get_attachment(conn, File::create(&tmp)?, |_, _| {}).await?;
    fs::rename(&tmp, dest)?;
    Ok(())
}

/// Links `file` to `dest`, which is instant and takes no space; copies when
/// they are on different file systems, as with a memory card.
pub async fn keep(file: &Path, dest: &Path) -> io::Result<()> {
    if fs::hard_link(file, dest).is_ok() {
        return Ok(());
    }
    tokio::fs::copy(file, dest).await.map(|_| ())
}

/// Apple's type identifier for a MIME type, which iPhones go by.
pub fn uti(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "public.jpeg",
        "image/png" => "public.png",
        "image/gif" => "com.compuserve.gif",
        "image/heic" => "public.heic",
        "image/heif" => "public.heif",
        "image/webp" => "org.webmproject.webp",
        "image/bmp" => "com.microsoft.bmp",
        "image/tiff" => "public.tiff",
        "video/mp4" => "public.mpeg-4",
        "video/quicktime" => "com.apple.quicktime-movie",
        "video/3gpp" => "public.3gpp",
        "video/webm" => "org.webmproject.webm",
        "audio/mp4" => "public.mpeg-4-audio",
        "audio/x-m4a" => "com.apple.m4a-audio",
        "audio/mpeg" => "public.mp3",
        "audio/aac" => "public.aac-audio",
        "application/pdf" => "com.adobe.pdf",
        "text/plain" => "public.plain-text",
        "text/vcard" => "public.vcard",
        _ => "public.data",
    }
}

/// Uploads the file to Apple's attachment storage.
pub async fn upload(conn: &APSConnectionResource, path: &Path, name: &str, mime: &str) -> Result<Attachment> {
    let mut file = File::open(path)?;
    let prepared = MMCSFile::prepare_put(&mut file).await?;
    file.seek(SeekFrom::Start(0))?;
    Ok(Attachment::new_mmcs(conn, &prepared, file, mime, uti(mime), name, |_, _| {}).await?)
}

/// Fetches the attachment described by `info` (a stored rustpush Attachment)
/// to `dest`, through a temporary file so a broken download leaves nothing.
pub async fn download(conn: &APSConnectionResource, info: &[u8], dest: &Path) -> Result<()> {
    let attachment: Attachment = plist::from_bytes(info)?;
    let dir = dest.parent().context("attachment path has no folder")?;
    store::ensure_private_dir(dir)?;
    let tmp = dest.with_extension("part");
    attachment.get_attachment(conn, File::create(&tmp)?, |_, _| {}).await?;
    fs::rename(&tmp, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_photos_get_a_file_per_version() {
        let root = Path::new("/r");
        let a = group_photo_path(root, "c1", 3);
        assert!(a.starts_with("/r/_groups") && a.to_string_lossy().ends_with("-3.png"));
        assert_ne!(group_photo_path(root, "c2", 3), a);
    }

    #[test]
    fn old_group_photos_are_removed() {
        let root = std::env::temp_dir().join(format!("obd-gp-{}", std::process::id()));
        let (old, new, other) = (group_photo_path(&root, "c", 1), group_photo_path(&root, "c", 2),
                                 group_photo_path(&root, "d", 1));
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        for p in [&old, &new, &other] {
            fs::write(p, b"x").unwrap();
        }
        remove_group_photos(&root, "c", Some(&new));
        assert!(!old.exists() && new.exists() && other.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn group_photos_come_only_from_the_app_cache() {
        let home = std::env::temp_dir().join(format!("obd-home-{}", std::process::id()));
        let cache = home.join(APP_CACHE);
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(home.join("Pictures")).unwrap();
        fs::write(cache.join("g.png"), b"png").unwrap();
        fs::write(home.join("Pictures/p.png"), b"png").unwrap();
        assert!(group_photo_source(&home, &cache.join("g.png").to_string_lossy()).is_ok());
        assert!(group_photo_source(&home, &home.join("Pictures/p.png").to_string_lossy()).is_err());
        assert!(group_photo_source(&home, &cache.join("../../../Pictures/p.png").to_string_lossy()).is_err());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn names_stay_inside_their_folder() {
        let root = Path::new("/r");
        assert_eq!(path_for(root, "ABC-1", 0, "IMG_0001.HEIC"), Path::new("/r/ABC-1/0-IMG_0001.HEIC"));
        assert_eq!(path_for(root, "../x", 2, "../../etc/passwd"), Path::new("/r/_x/2-_.._etc_passwd"));
        assert_eq!(path_for(root, "A", 1, ""), Path::new("/r/A/1-attachment"));
    }

    #[test]
    fn heic_is_recognised_by_type_or_name() {
        assert!(is_heic("image/heic", "x"));
        assert!(is_heic("application/octet-stream", "IMG_1.HEIC"));
        assert!(!is_heic("image/jpeg", "a.jpg"));
    }

    #[test]
    fn other_files_are_shown_as_they_are() {
        let (p, m) = displayable(PathBuf::from("/r/a.png"), "image/png", "a.png");
        assert_eq!((p, m.as_str()), (PathBuf::from("/r/a.png"), "image/png"));
    }

    #[test]
    fn only_the_apps_own_folders_are_sendable() {
        let home = std::env::temp_dir().join(format!("obd-send-{}", std::process::id()));
        fs::create_dir_all(home.join("Pictures/Camera")).unwrap();
        fs::create_dir_all(home.join(".local")).unwrap();
        fs::write(home.join("Pictures/Camera/a.jpg"), b"x").unwrap();
        fs::write(home.join(".local/key"), b"x").unwrap();
        std::os::unix::fs::symlink(home.join(".local/key"), home.join("Pictures/link")).unwrap();
        let p = |rel: &str| home.join(rel).to_string_lossy().into_owned();
        assert!(sendable(&home, &p("Pictures/Camera/a.jpg")).is_ok());
        assert!(sendable(&home, &p(".local/key")).is_err());
        assert!(sendable(&home, &p("Pictures/../.local/key")).is_err());
        assert!(sendable(&home, &p("Pictures/link")).is_err(), "symlinks are followed before checking");
        assert!(sendable(&home, &p("Pictures/Camera")).is_err(), "a folder");
        assert!(sendable(&home, &p("Pictures/missing.jpg")).is_err());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn utis_fall_back_to_data() {
        assert_eq!(uti("image/jpeg"), "public.jpeg");
        assert_eq!(uti("video/mp4"), "public.mpeg-4");
        assert_eq!(uti("application/x-whatever"), "public.data");
    }

    #[test]
    fn ids_round_trip() {
        assert_eq!(parse_id(&id("ABC-1", 3)), Some(("ABC-1", 3)));
        assert_eq!(parse_id("nope"), None);
    }
}
