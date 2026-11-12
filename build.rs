// Exe icon (resource id 1, also used for the tray) from the .ico build.ps1 generates, plus the app manifest
// (asInvoker, Windows 10+, per-monitor DPI v2).
fn main() {
    let ico = std::path::Path::new("build/T3Presence.ico");
    println!("cargo:rerun-if-changed=build/T3Presence.ico");
    println!("cargo:rerun-if-changed=src/app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut r = winresource::WindowsResource::new();
        if ico.exists() { r.set_icon_with_id(ico.to_str().unwrap(), "1"); } else { println!("cargo:warning=build/T3Presence.ico missing: run build.ps1 for the icon"); }
        r.set_manifest(include_str!("src/app.manifest"));
        r.set("FileDescription", "T3 Presence").set("ProductName", "T3 Presence");
        if let Err(e) = r.compile() { println!("cargo:warning=icon not embedded: {e}"); }
    }
}
