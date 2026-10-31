fn main() {
    #[cfg(windows)]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_manifest_file("app.manifest");
        resource.set("ProductName", "Evolve Mod Manager");
        resource.set("FileDescription", "External teleport overlay");
        resource.compile().expect("Windows resource failed");
    }
}
