//! Toki Hub on a computer: everything is in lib.rs (which a phone starts from too).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    toki_hub_lib::run()
}
