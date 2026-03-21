fn main() {
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=CoreGraphics");

    // Static FFmpeg: link x264 and system frameworks that FFmpeg needs
    if let Ok(ffmpeg_dir) = std::env::var("FFMPEG_DIR") {
        println!("cargo:rustc-link-search=native={}/lib", ffmpeg_dir);
        println!("cargo:rustc-link-lib=static=x264");

        #[cfg(target_os = "macos")]
        {
            println!("cargo:rustc-link-lib=framework=VideoToolbox");
            println!("cargo:rustc-link-lib=framework=CoreMedia");
            println!("cargo:rustc-link-lib=framework=CoreVideo");
            println!("cargo:rustc-link-lib=framework=CoreServices");
            println!("cargo:rustc-link-lib=framework=CoreFoundation");
            println!("cargo:rustc-link-lib=z");
            println!("cargo:rustc-link-lib=iconv");
        }
    }

    tauri_build::build()
}
