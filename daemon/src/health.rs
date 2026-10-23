use rustpush::ResourceState;

/// What the app shows about the connection to Apple.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Health {
    pub push: &'static str,
    pub registration: &'static str,
    /// When rustpush next renews the iMessage registration (ms); the Mac
    /// relay has to be reachable then.
    pub renew_at_ms: i64,
}

pub fn push_state(s: &ResourceState) -> &'static str {
    match s {
        ResourceState::Generated => "connected",
        ResourceState::Generating => "connecting",
        ResourceState::Failed(_) | ResourceState::Closed => "offline",
    }
}

pub fn registration_state(s: &ResourceState) -> &'static str {
    match s {
        ResourceState::Generated => "ok",
        ResourceState::Generating => "renewing",
        ResourceState::Failed(f) if f.retry_wait.is_some() => "retrying",
        // rustpush gives up only when Apple wants a new sign-in
        ResourceState::Failed(_) => "signin",
        ResourceState::Closed => "stopped",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustpush::{PushError, ResourceFailure};
    use std::sync::Arc;

    fn failed(retry_wait: Option<u64>) -> ResourceState {
        ResourceState::Failed(ResourceFailure { retry_wait, error: Arc::new(PushError::DeviceNotFound) })
    }

    #[test]
    fn states_map_to_words() {
        assert_eq!(push_state(&ResourceState::Generated), "connected");
        assert_eq!(push_state(&failed(Some(5))), "offline");
        assert_eq!(registration_state(&ResourceState::Generated), "ok");
        assert_eq!(registration_state(&failed(Some(300))), "retrying");
        assert_eq!(registration_state(&failed(None)), "signin");
    }
}
