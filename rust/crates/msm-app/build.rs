fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        println!("cargo:rustc-link-lib=dylib=dwmapi");
        println!("cargo:rustc-link-lib=dylib=advapi32");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/MichStartupMaster.ico");
        res.set_manifest(include_str!("app.manifest"));
        res.set("FileDescription", "Mich Startup Master");
        res.set("ProductName", "Mich Startup Master");
        res.set("FileVersion", "3.0.0.0");
        res.set("ProductVersion", "3.0.0.0");
        res.compile().expect("embed windows resources");
    }
}
