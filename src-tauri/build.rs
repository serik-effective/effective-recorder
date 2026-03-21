fn main() {
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=CoreGraphics");

    if let Ok(ffmpeg_dir) = std::env::var("FFMPEG_DIR") {
        println!("cargo:rustc-link-search=native={}/lib", ffmpeg_dir);

        // Static FFmpeg: link x264 and platform-specific system libs
        if std::env::var("FFMPEG_STATIC").is_ok() {
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

            #[cfg(target_os = "linux")]
            {
                println!("cargo:rustc-link-lib=z");
                println!("cargo:rustc-link-lib=m");
            }

            #[cfg(target_os = "windows")]
            {
                println!("cargo:rustc-link-lib=bcrypt");
                println!("cargo:rustc-link-lib=secur32");
            }
        }
    }

    tauri_build::build()
}
