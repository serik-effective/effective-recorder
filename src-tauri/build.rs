fn main() {
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=CoreGraphics");

    // When FFMPEG_DIR is set (macOS/Linux static builds), add lib search path
    if let Ok(ffmpeg_dir) = std::env::var("FFMPEG_DIR") {
        println!("cargo:rustc-link-search=native={}/lib", ffmpeg_dir);
    }

    // Static FFmpeg: link x264 and platform-specific system libs
    if std::env::var("FFMPEG_STATIC").is_ok() {
        // On Windows with vcpkg, x264 is found automatically via vcpkg-rs
        // On macOS/Linux, we need to link it explicitly
        #[cfg(not(target_os = "windows"))]
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
            // x264
            println!("cargo:rustc-link-lib=static=libx264");
            // Windows system libs required by static FFmpeg
            println!("cargo:rustc-link-lib=bcrypt");
            println!("cargo:rustc-link-lib=secur32");
            // Media Foundation (mfenc, mf_utils)
            println!("cargo:rustc-link-lib=mfplat");
            println!("cargo:rustc-link-lib=mfuuid");
            println!("cargo:rustc-link-lib=mf");
            // DirectShow (dshow)
            println!("cargo:rustc-link-lib=strmiids");
            println!("cargo:rustc-link-lib=ole32");
            println!("cargo:rustc-link-lib=oleaut32");
            println!("cargo:rustc-link-lib=uuid");
            // Network/WinSock (for rtmp/network protocols)
            println!("cargo:rustc-link-lib=ws2_32");
            // Other system libs
            println!("cargo:rustc-link-lib=user32");
            println!("cargo:rustc-link-lib=gdi32");
            println!("cargo:rustc-link-lib=vfw32");
            println!("cargo:rustc-link-lib=shlwapi");
            println!("cargo:rustc-link-lib=advapi32");
        }
    }

    tauri_build::build()
}
