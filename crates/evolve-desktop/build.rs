fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-env-changed=EVOLVE_TEST_NO_MANIFEST");
        // Cargo test executables must remain runnable by an ordinary account;
        // the released GUI still embeds its normal elevation manifest.
        if std::env::var_os("EVOLVE_TEST_NO_MANIFEST").is_some() {
            return;
        }
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("../../assets/hunt.ico");
        // The private Qt manager requires UAC for protected game-file and
        // Windows Firewall workflows. Only this GUI executable embeds the
        // manifest: evolve-cli remains suitable for non-elevated workers.
        resource.set_manifest_file("app.manifest");
        resource.set("ProductName", "Evolve Mod Manager");
        resource.set("FileDescription", "Evolve Mod Manager");
        resource
            .compile()
            .expect("Application icon resource failed");
    }
}
