fn main() {
    // The catalog is built into the exe (see main.rs), so rebuild when it changes.
    println!("cargo:rerun-if-changed=../catalog");
    tauri_build::build()
}
