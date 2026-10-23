use std::io;
use std::path::Path;

use rustpush::{APSState, IDSUser, RelayConfig};
use serde::{Deserialize, Serialize};

use crate::store;

const STATE_FILE: &str = "state.plist";

// Private keys are not in here: APSState and IDSUser only name keys in the
// encrypted keystore, and the identity blob is sealed with a keystore key.
#[derive(Serialize, Deserialize, Clone)]
pub struct Saved {
    pub relay: RelayConfig,
    pub push: APSState,
    pub users: Vec<IDSUser>,
    pub identity: Vec<u8>,
}

pub fn load(dir: &Path) -> io::Result<Option<Saved>> {
    match std::fs::read(dir.join(STATE_FILE)) {
        Ok(data) => plist::from_bytes(&data).map(Some).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn save(dir: &Path, saved: &Saved) -> io::Result<()> {
    let mut buf = Vec::new();
    plist::to_writer_binary(&mut buf, saved).map_err(io::Error::other)?;
    store::write_private(&dir.join(STATE_FILE), &buf)
}
