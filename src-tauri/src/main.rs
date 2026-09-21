#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    llm_quota_overlay::app::run();
}
