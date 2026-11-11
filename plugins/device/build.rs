// No commands for the web page: Toki Hub's Rust code calls the Android side itself (src/lib.rs).
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).android_path("android").build();
}
