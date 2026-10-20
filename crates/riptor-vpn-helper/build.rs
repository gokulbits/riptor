fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // UAC shows this name once the helper is signed; unsigned it shows the file name.
    let mut res = tauri_winres::WindowsResource::new();
    res.set_icon("../../src-tauri/icons/icon.ico")
        .set("FileDescription", "Riptor VPN Service")
        .set("ProductName", "Riptor")
        .set("InternalName", "Riptor VPN Service")
        .set("OriginalFilename", "Riptor VPN Service.exe");
    res.compile()
        .expect("embedding the Windows version resource");
}
