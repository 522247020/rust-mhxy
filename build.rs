fn main() {
    println!("cargo:rerun-if-changed=assets/icons/app-icon.ico");

    // Build scripts run on the host, so cfg!(windows) is false during Linux -> Windows
    // cross-compilation. Cargo's target environment is the source of truth here.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icons/app-icon.ico");
        resource
            .compile()
            .expect("failed to embed the Windows application icon");
    }
}
