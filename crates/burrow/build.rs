//! Embeds the app icon and version info in burrow.exe (Windows); sets the libcef.so search path (Linux).

fn main() {
    println!("cargo::rerun-if-changed=resources/burrow.rc");
    println!("cargo::rerun-if-changed=resources/burrow.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // burrow.rc has VER_* placeholders; fill them in from Cargo.toml's version.
        let var = |k: &str| std::env::var(k).unwrap();
        let dir = std::path::PathBuf::from(var("CARGO_MANIFEST_DIR")).join("resources");
        let rc = std::fs::read_to_string(dir.join("burrow.rc"))
            .unwrap()
            .replace("burrow.ico", &dir.join("burrow.ico").to_string_lossy().replace('\\', "/"))
            .replace("VER_MAJOR", &var("CARGO_PKG_VERSION_MAJOR"))
            .replace("VER_MINOR", &var("CARGO_PKG_VERSION_MINOR"))
            .replace("VER_PATCH", &var("CARGO_PKG_VERSION_PATCH"))
            .replace("VER_STRING", &format!("\"{}\"", var("CARGO_PKG_VERSION")));
        let out = std::path::PathBuf::from(var("OUT_DIR")).join("burrow.rc");
        std::fs::write(&out, rc).unwrap();
        embed_resource::compile(&out, embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
    // On Linux, find libcef.so next to the executable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo::rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    }
}
