//! Direct Launch service preflight.
//! This draft keeps writes disabled until the actual Windows game/client runtime
//! is verified end to end with the user's installed service implementation.
use std::path::Path;

pub fn require_ready(_game: &Path, _client: &Path) -> Result<(), String> {
    Err(
        "Direct Launch is not yet validated: the independent Stage 2 profile/Steam compatibility service lifecycle and Stage2Trace initialization must pass Windows gameplay and original-file recovery tests. No game PAKs were changed. Select Client Launch in Settings for now."
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unverified_direct_runtime_never_allows_game_start() {
        let tmp = tempfile::tempdir().unwrap();
        let err = require_ready(tmp.path(), tmp.path()).unwrap_err();
        assert!(err.contains("not yet validated"));
        assert!(err.contains("No game PAKs were changed"));
    }
}
