// Embeds the app icon and version info (Explorer, Task Manager, the taskbar)
// into the Windows executable. Needs `windres` on PATH (part of mingw-w64).
fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=packaging/icons/app.ico");
        let version = env!("CARGO_PKG_VERSION");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("packaging/icons/app.ico");
        res.set("ProductName", "Metamug API Tester");
        res.set("FileDescription", "Metamug API Tester");
        res.set("CompanyName", "Metamug");
        res.set("LegalCopyright", "Copyright Metamug");
        res.set("ProductVersion", version);
        res.set("FileVersion", version);
        res.compile().expect("failed to embed Windows resources (is windres on PATH?)");
    }
}
