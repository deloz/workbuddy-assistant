use tauri::{Manager, WindowEvent};

use super::app_commands;
use super::dashboard::DashboardRuntime;
use super::{scheduler, tray};

pub(crate) fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let local_data_dir = app.path().app_local_data_dir()?;
            let history_path = local_data_dir.join("checkin-history.json");
            let growth_history_path = local_data_dir.join("growth-history.json");
            let settings_path = local_data_dir.join("settings.json");
            let signin_path = local_data_dir.join("workbuddy-signin.json");
            app.manage(DashboardRuntime::with_paths(
                history_path,
                growth_history_path,
                settings_path,
                signin_path,
            ));
            tray::setup(app)?;
            scheduler::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let runtime = window.app_handle().state::<DashboardRuntime>();
                if !runtime.is_exiting() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_commands::get_dashboard_state,
            app_commands::reload_accounts,
            app_commands::begin_workbuddy_signin,
            app_commands::open_workbuddy_signin_page,
            app_commands::complete_workbuddy_signin,
            app_commands::sign_out_account,
            app_commands::refresh_account_credit,
            app_commands::run_checkin_all_command,
            app_commands::run_checkin_account,
            app_commands::run_growth_all_command,
            app_commands::run_growth_account,
            app_commands::get_app_settings,
            app_commands::set_growth_automation_enabled,
            app_commands::reveal_account_identifier,
            app_commands::check_for_updates
        ])
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!())
        .expect("启动 WorkBuddy助手失败");
}
