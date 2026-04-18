#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    token_burger::logger::init();
    token_burger::run();
}
