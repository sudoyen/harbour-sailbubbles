use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::db::Db;
use crate::store;

// Where the daemon kept its data before the project was renamed to SailBubbles.
const OLD_STATE: &str = ".local/share/harbour-openbubbles";
const OLD_FILES: &str = ".local/share/org.openbubbles/harbour-openbubbles/attachments";

/// Moves the saved login and the received files of an install from before the
/// rename into `state` and `files`; does nothing on a new install or once moved.
pub fn from_openbubbles(home: &Path, state: &Path, files: &Path) -> Result<()> {
    let (old_state, old_files) = (home.join(OLD_STATE), home.join(OLD_FILES));
    if old_state.is_dir() && !state.exists() {
        fs::rename(&old_state, state)?;
        log::info!("moved the saved login to its new folder");
    }
    if old_files.is_dir() && !files.exists() {
        if let Some(parent) = files.parent() {
            store::ensure_private_dir(parent)?;
        }
        fs::rename(&old_files, files)?;
        let db = state.join("messages.db");
        if db.exists() {
            Db::open(&db)?.move_files(&old_files.to_string_lossy(), &files.to_string_lossy())?;
        }
        log::info!("moved received files to their new folder");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Message, Status};

    #[test]
    fn an_old_install_moves_with_its_paths() {
        let home = std::env::temp_dir().join(format!("obd-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let (old_state, old_files) = (home.join(OLD_STATE), home.join(OLD_FILES));
        fs::create_dir_all(&old_state).unwrap();
        fs::create_dir_all(old_files.join("A")).unwrap();
        fs::write(old_state.join("keystore.key"), b"key").unwrap();
        fs::write(old_files.join("A/0-p.jpg"), b"jpg").unwrap();
        {
            let db = Db::open(&old_state.join("messages.db")).unwrap();
            db.add_message(&Message { id: "A", chat_id: "c", sender: "tel:+1", text: "\u{fffc}", ts: 1,
                                      status: Status::Received, reply_to: None }, None, None).unwrap();
            db.add_attachment("A", 0, "p.jpg", "image/jpeg", 3, &[]).unwrap();
            db.set_attachment_file("A", 0, &old_files.join("A/0-p.jpg").to_string_lossy(), "image/jpeg").unwrap();
        }

        let (state, files) = (home.join("new/state"), home.join("new/app/attachments"));
        fs::create_dir_all(home.join("new")).unwrap();
        from_openbubbles(&home, &state, &files).unwrap();
        assert_eq!(fs::read(state.join("keystore.key")).unwrap(), b"key");
        let path = Db::open(&state.join("messages.db")).unwrap().attachments("A").unwrap()[0].path.clone().unwrap();
        assert_eq!(path, files.join("A/0-p.jpg").to_string_lossy());
        assert_eq!(fs::read(&path).unwrap(), b"jpg");
        assert!(!old_state.exists() && !old_files.exists());

        from_openbubbles(&home, &state, &files).unwrap();
        fs::remove_dir_all(&home).unwrap();
    }
}
