fn main() {
    // the deb and rpm ship libgrokj2k in /usr/lib/imfwizard, not beside the binary
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN/../lib/imfwizard");
    }
    postkit_ffmpeg_link_search::emit_ffmpeg_link_search();
    tauri_build::build()
}
