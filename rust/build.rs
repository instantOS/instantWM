fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if cfg!(feature = "xinerama") {
        println!("cargo:rustc-cfg=feature=\"xinerama\"");
        println!("cargo:rustc-link-lib=Xinerama");
    }

    println!("cargo:rustc-link-lib=X11");
    println!("cargo:rustc-link-lib=Xft");
    println!("cargo:rustc-link-lib=Xrender");
    println!("cargo:rustc-link-lib=fontconfig");
    println!("cargo:rustc-link-lib=freetype");
}
