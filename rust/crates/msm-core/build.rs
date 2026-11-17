fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        // ConvertSidToStringSidW lives in advapi32.
        println!("cargo:rustc-link-lib=dylib=advapi32");
    }
}
