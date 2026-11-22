fn main() {
    if std::env::var("TARGET").is_ok_and(|target| target.ends_with("windows-msvc")) {
        // GPUI's nested message/tool layouts exceed the default 1 MiB UI stack.
        println!("cargo:rustc-link-arg=/STACK:16777216");
        // Embed the app icon so Explorer, shortcuts, and the taskbar show it.
        println!("cargo:rerun-if-changed=windows-icon.rc");
        println!("cargo:rerun-if-changed=../../packaging/icon.ico");
        embed_resource::compile("windows-icon.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed the Windows icon resource");
    }
}
