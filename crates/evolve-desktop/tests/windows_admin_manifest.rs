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
