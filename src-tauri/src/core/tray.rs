use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager};

use super::dashboard::{self, DashboardRuntime, OPEN_CHECKIN_EVENT};

const TRAY_ID: &str = "main-tray";

pub(crate) fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let open = MenuItemBuilder::with_id("open", "打开").build(app)?;
    let checkin = MenuItemBuilder::with_id("checkin", "立即签到").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&open, &checkin, &quit])
        .build()?;
    let icon = app.default_window_icon().cloned().ok_or("未找到应用图标")?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .menu(&menu)
        .tooltip("WorkBuddy助手")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "checkin" => open_checkin_and_run(app.clone()),
            "quit" => {
                app.state::<DashboardRuntime>().request_exit();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn open_checkin_and_run(app: AppHandle) {
    show_main_window(&app);
    let _ = app.emit(OPEN_CHECKIN_EVENT, ());
    spawn_checkin(app);
}

fn spawn_checkin(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let runtime = app.state::<DashboardRuntime>();
        let _ = dashboard::run_checkin_all(runtime.inner(), &app).await;
    });
}
