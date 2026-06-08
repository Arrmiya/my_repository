#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Desktop entry point — delegates to lib.rs
    app_lib::run();
}
