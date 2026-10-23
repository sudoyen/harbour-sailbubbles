use env_logger::Builder;
use log::LevelFilter;

// rustpush logs message bodies, 2FA codes and push tokens at info and below,
// so only our own crate may log above error, whatever RUST_LOG says.
fn builder() -> Builder {
    let mut b = Builder::new();
    b.filter_level(LevelFilter::Error)
        .filter_module(env!("CARGO_CRATE_NAME"), LevelFilter::Info);
    b
}

pub fn init() {
    builder().init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::{Level, Log, Metadata};

    fn enabled(target: &str, level: Level) -> bool {
        builder()
            .build()
            .enabled(&Metadata::builder().target(target).level(level).build())
    }

    #[test]
    fn own_info_is_logged() {
        assert!(enabled(env!("CARGO_CRATE_NAME"), Level::Info));
        assert!(!enabled(env!("CARGO_CRATE_NAME"), Level::Debug));
    }

    #[test]
    fn dependencies_only_log_errors() {
        for target in ["rustpush", "rustpush::imessage::aps_client", "icloud_auth", "omnisette"] {
            assert!(!enabled(target, Level::Info), "{target}");
            assert!(!enabled(target, Level::Warn), "{target}");
            assert!(enabled(target, Level::Error), "{target}");
        }
    }

    #[test]
    fn rust_log_is_ignored() {
        std::env::set_var("RUST_LOG", "debug");
        assert!(!enabled("rustpush", Level::Debug));
    }
}
