// Test the UAC contract even on non-Windows CI hosts. Windows packaging
// also compiles the actual resource through winres in build.rs.
#[test]
fn manager_gui_requests_windows_administrator_consent() {
    let manifest = include_str!("../app.manifest");
    let build = include_str!("../build.rs");
    assert!(manifest.contains("urn:schemas-microsoft-com:asm.v1"));
    assert!(manifest.contains("level=\"requireAdministrator\""));
    assert!(manifest.contains("uiAccess=\"false\""));
    assert!(build.contains("resource.set_manifest_file(\"app.manifest\")"));
    assert!(build.contains("#[cfg(windows)]"));
}

#[test]
fn gui_uac_policy_does_not_change_the_cli_worker() {
    let core_manifest = include_str!("../../evolve-core/Cargo.toml");
    let cli = include_str!("../../evolve-core/src/bin/evolve-cli.rs");
    assert!(core_manifest.contains("name = \"evolve-core\""));
    assert!(!cli.contains("requireAdministrator"));
    assert!(!cli.contains("ShellExecuteW"));
}

#[test]
fn installer_requests_uac_before_launching_privileged_gui() {
    let setup = include_str!("../../../installer/EvolveModManager.iss");
    assert!(setup.contains("PrivilegesRequired=admin"));
    assert!(setup.contains(r"DefaultDirName={autopf}\EvolveModManager"));
    // Never install a requireAdministrator EXE to a user-writable directory.
    assert!(!setup.contains(r"DefaultDirName={localappdata}\EvolveModManager"));
    assert!(setup.contains("UsePreviousAppDir=no"));
    assert!(setup.contains(r#"Filename: "{app}\evolve-mod-manager.exe""#));
}

#[test]
fn isolated_worker_and_overlay_do_not_request_administrator_by_manifest() {
    for manifest in [
        include_str!("../../evolve-privileged-worker/app.manifest"),
        include_str!("../../evolve-teleport-overlay/app.manifest"),
    ] {
        assert!(manifest.contains("level=\"asInvoker\""));
        assert!(manifest.contains("uiAccess=\"false\""));
        assert!(!manifest.contains("requireAdministrator"));
    }
    let worker = include_str!("../../evolve-privileged-worker/Cargo.toml");
    let overlay = include_str!("../../evolve-teleport-overlay/Cargo.toml");
    assert!(worker.contains("name = \"evolve-privileged-worker\""));
    assert!(overlay.contains("name = \"evolve-teleport-overlay\""));
    let workspace = include_str!("../../../Cargo.toml");
    assert!(workspace.contains("\"crates/evolve-privileged-worker\""));
    assert!(workspace.contains("\"crates/evolve-teleport-overlay\""));
}
