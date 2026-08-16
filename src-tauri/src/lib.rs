mod core;
mod data;
mod platforms;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    core::app::run();
}
