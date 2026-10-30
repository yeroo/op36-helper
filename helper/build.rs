use embed_manifest::{embed_manifest, manifest::DpiAwareness, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest(new_manifest("Op36Helper").dpi_awareness(DpiAwareness::PerMonitorV2Only))
            .expect("embed manifest");
        // Version resource (Explorer's Details tab, the installer's AppVersion) and the exe icon.
        winresource::WindowsResource::new()
            .set_icon("assets/icon_on.ico")
            .set("ProductName", "OP36 Helper")
            .set("FileDescription", "Ergohaven OmegaPoint36 layout helper")
            .set("LegalCopyright", "MIT License, Boris Kudriashov")
            .compile()
            .expect("version resource");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon_on.ico");
}
