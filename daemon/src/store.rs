use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    // create() leaves an existing dir's mode alone
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

// Written to a temp file and renamed, so a crash never leaves half a key file.
pub fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let tmp = path.with_file_name(format!(".{}.tmp", name.to_string_lossy()));
    // a stale temp file would keep its old mode
    let _ = fs::remove_file(&tmp);
    let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("obd-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn new_dir_is_private() {
        let dir = scratch("newdir").join("a/b");
        ensure_private_dir(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn existing_dir_is_tightened() {
        let dir = scratch("olddir");
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn file_is_private_and_replaced() {
        let dir = scratch("file");
        ensure_private_dir(&dir).unwrap();
        let path = dir.join("state.plist");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        write_private(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(mode(&path), 0o600);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn stale_temp_file_is_not_reused() {
        let dir = scratch("stale");
        ensure_private_dir(&dir).unwrap();
        let tmp = dir.join(".state.plist.tmp");
        fs::write(&tmp, b"junk").unwrap();
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o666)).unwrap();

        let path = dir.join("state.plist");
        write_private(&path, b"new").unwrap();
        assert_eq!(mode(&path), 0o600);
        assert!(!tmp.exists());
    }
}
