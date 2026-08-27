// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let _ = dbox_lib::environment::fix_gui_path();
    dbox_lib::run()
}
