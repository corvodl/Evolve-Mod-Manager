fn main() {
    #[cfg(windows)]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("../../assets/hunt.ico");
        resource.set("ProductName", "Evolve Mod Manager");
        resource.set("FileDescription", "Evolve Mod Manager");
        resource.compile().expect("Application icon resource failed");
    }
}
