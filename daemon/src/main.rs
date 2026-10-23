mod anisette;
mod api;
mod attachments;
mod db;
mod health;
mod logging;
mod login;
mod migrate;
mod notify;
mod prompt;
mod random;
mod run;
mod signin;
mod state;
mod store;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

const KEY_LEN: usize = 32;
use keystore::software::{SoftwareEncryptor, SoftwareKeystore, SoftwareKeystoreState};
use keystore::init_keystore;

const KEYSTORE_FILE: &str = "keystore.plist";
// The keystore key, readable only by the user; home encryption guards it while
// the phone is off. Sailfish Secrets would lock it after every reboot until a
// lock-code dialog runs, which a background service cannot wait for.
const KEY_FILE: &str = "keystore.key";

/// Present while the app asked to sign in again; survives the restart that takes.
pub const SIGNIN_MARKER: &str = "signin-requested";

/// Starts the daemon afresh; returns only if that failed.
pub fn restart() -> std::io::Error {
    use std::os::unix::process::CommandExt;
    match std::env::current_exe() {
        Ok(exe) => std::process::Command::new(exe).exec(),
        Err(e) => e,
    }
}

fn state_dir(home: &Path) -> PathBuf {
    home.join(".local/share/sailbubbles")
}

fn keystore_key(dir: &Path) -> Result<[u8; KEY_LEN]> {
    let path = dir.join(KEY_FILE);
    match std::fs::read(&path) {
        Ok(data) => data.try_into().map_err(|_| anyhow!("{KEY_FILE} is damaged")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if dir.join(KEYSTORE_FILE).exists() {
                bail!("{KEY_FILE} is missing, so the saved keys cannot be opened; sign in again after moving \
                       {KEYSTORE_FILE} away");
            }
            let key = random::bytes()?;
            store::write_private(&path, &key)?;
            log::info!("keystore key saved");
            Ok(key)
        }
        Err(e) => Err(e).context("reading the keystore key"),
    }
}

fn init_encrypted_keystore(dir: &Path, key: [u8; KEY_LEN]) -> Result<()> {
    let path = dir.join(KEYSTORE_FILE);
    let state: SoftwareKeystoreState = match std::fs::read(&path) {
        Ok(data) => plist::from_bytes(&data).context("reading keystore")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
        Err(e) => return Err(e).context("reading keystore"),
    };
    init_keystore(SoftwareKeystore {
        state: state.into(),
        update_state: Box::new(move |state| {
            let mut buf = Vec::new();
            let saved = plist::to_writer_binary(&mut buf, state)
                .map_err(std::io::Error::other)
                .and_then(|_| store::write_private(&path, &buf));
            if let Err(e) = saved {
                log::error!("saving keystore failed: {e}");
            }
        }),
        encryptor: SoftwareEncryptor(key),
    });
    Ok(())
}

fn main() -> Result<()> {
    logging::init();
    let home = attachments::home()?;
    let dir = state_dir(&home);
    migrate::from_openbubbles(&home, &dir, &attachments::root()?).context("moving the old install")?;
    store::ensure_private_dir(&dir)?;
    // from a terminal: SAILBUBBLES_SIGNIN=1 sailbubbles-daemon
    let resign = std::env::var_os("SAILBUBBLES_SIGNIN").is_some_and(|v| v == "1");
    init_encrypted_keystore(&dir, keystore_key(&dir)?)?;
    log::info!("keystore ready");

    tokio::runtime::Runtime::new()?.block_on(async {
        let marker = dir.join(SIGNIN_MARKER);
        if resign || marker.exists() || state::load(&dir).context("reading state")?.is_none() {
            // from a terminal it asks there, as a service through the app
            if std::fs::File::open("/dev/tty").is_ok() {
                login::login(&dir, &mut prompt::Terminal).await?;
            } else {
                signin::serve(&dir).await?;
            }
            let _ = std::fs::remove_file(&marker);
        }
        run::run(&dir).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("obd-key-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        store::ensure_private_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn a_new_key_is_made_and_kept() {
        let dir = scratch("new");
        let key = keystore_key(&dir).unwrap();
        assert_eq!(keystore_key(&dir).unwrap(), key);
        let mode = std::fs::metadata(dir.join(KEY_FILE)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn saved_keys_without_their_key_are_reported() {
        let dir = scratch("lost");
        std::fs::write(dir.join(KEYSTORE_FILE), b"x").unwrap();
        assert!(keystore_key(&dir).is_err());
        assert!(!dir.join(KEY_FILE).exists(), "no new key that could never open them");
    }

    #[test]
    fn a_damaged_key_file_is_reported() {
        let dir = scratch("damaged");
        std::fs::write(dir.join(KEY_FILE), b"short").unwrap();
        assert!(keystore_key(&dir).is_err());
    }
}
