fn main() {
    println!("cargo:rerun-if-changed=assets/fpscraft.ico");

    if std::env::var_os("CARGO_CFG_TARGET_OS").as_deref() == Some(std::ffi::OsStr::new("windows")) {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/fpscraft.ico");
        if let Err(err) = res.compile() {
            println!("cargo:warning=failed to embed the application icon: {err}");
        }
    }
}
