fn main() {
    println!("cargo:rerun-if-changed=src/macos/long_stream.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/macos/long_stream.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .flag("-mmacosx-version-min=11.0")
            .compile("snappin_long_stream");
        // ScreenCaptureKit is loaded at runtime so macOS 11 can use the legacy path.
        for framework in ["Foundation", "CoreMedia", "CoreVideo", "CoreGraphics"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
}
