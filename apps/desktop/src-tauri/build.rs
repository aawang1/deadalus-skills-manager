fn main() {
    // Windows resource compilation must rerun when the application icon changes.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
