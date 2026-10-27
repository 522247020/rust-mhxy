fn main() {
    println!("cargo:rerun-if-changed=assets/icons/app-icon.ico");

    // .cargo/config.toml 把 TMP/TEMP 指向相对路径 target/tmp（MSVC 工具链写系统
    // 临时目录会失败）。winresource 调用的 llvm-rc 会在子进程里解析这个相对路径，
    // 目录不存在时报 "Could not compile resource file"，这里提前建好并改成绝对路径。
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let tmp = std::path::Path::new(&manifest_dir)
            .join("target")
            .join("tmp");
        if std::fs::create_dir_all(&tmp).is_ok() {
            std::env::set_var("TMP", &tmp);
            std::env::set_var("TEMP", &tmp);
        }
    }

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
