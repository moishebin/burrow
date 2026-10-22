//! Embeds the app icon in burrow.exe (Windows); sets the libcef.so search path (Linux).

fn main() {
    println!("cargo::rerun-if-changed=resources/burrow.rc");
    println!("cargo::rerun-if-changed=resources/burrow.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("resources/burrow.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
    // On Linux, find libcef.so next to the executable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo::rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    }
}
