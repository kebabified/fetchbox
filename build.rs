// Embeds the icon and version info into both programs (needs windres from MinGW-w64, or rc.exe with MSVC).
fn main() {
    println!("cargo:rerun-if-changed=fetchbox.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("fetchbox.ico");
        res.set("ProductName", "Fetchbox");
        res.set("FileDescription", "Fetchbox");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon not embedded: {e}");
        }
    }
}
