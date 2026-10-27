//! Runtime handshake required before a manager-owned Evolve launch.
//! The client does far more than set SteamAppId or call CreateProcessW.
//! Until a separately managed service host implements the full contract,
//! never swap PAKs or start a game that cannot initialize its profile.
use std::path::Path;

/// Preflight must remain fail-closed until the independent host supports:
/// 1. Local TLS/HTTP services with the original client profile/identity state.
/// 2. Steam compatibility provider/relay lifecycle.
/// 3. A session-specific environment for Stage2Trace.
/// 4. Suspended creation, verified Stage2Trace load, and resume.
/// 5. Lifetime ownership and stop/recovery on game exit.
/// Merely observing open localhost ports or DLL files is NOT proof of readiness.
pub fn require_ready(_game: &Path) -> Result<(), String> {
    Err(
        "Direct Launch cannot start yet: the manager has not initialized the Modded Evolve Steam compatibility and profile services, nor loaded Stage2Trace.dll. The game would not work with process arguments alone. No game files were swapped or modified. Select Client Launch in Settings until independent runtime service hosting is implemented and verified."
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_runtime_is_not_reported_ready_without_compat_and_profile_services() {
        let root = tempfile::tempdir().unwrap();
        let error = require_ready(root.path()).unwrap_err();
        assert!(error.contains("Steam compatibility"));
        assert!(error.contains("profile services"));
        assert!(error.contains("No game files were swapped"));
    }
}
