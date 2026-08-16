use tauri::State;

use super::dashboard::{
    self, AccountCreditSnapshot, CheckinAccountSnapshot, CheckinBatchSnapshot, DashboardRuntime,
    DashboardSnapshot,
};
use super::update::{self, UpdateCheckSnapshot};

#[tauri::command]
pub(crate) async fn get_dashboard_state(
    state: State<'_, DashboardRuntime>,
) -> Result<DashboardSnapshot, String> {
    Ok(dashboard::reload_accounts(state.inner()).await)
}

#[tauri::command]
pub(crate) async fn reload_accounts(
    state: State<'_, DashboardRuntime>,
) -> Result<DashboardSnapshot, String> {
    Ok(dashboard::reload_accounts(state.inner()).await)
}

#[tauri::command]
pub(crate) async fn refresh_account_credit(
    app: tauri::AppHandle,
    state: State<'_, DashboardRuntime>,
    account_id: String,
) -> Result<AccountCreditSnapshot, String> {
    dashboard::refresh_account_credit(state.inner(), &app, &account_id).await
}

#[tauri::command(rename = "run_checkin_all")]
pub(crate) async fn run_checkin_all_command(
    app: tauri::AppHandle,
    state: State<'_, DashboardRuntime>,
) -> Result<CheckinBatchSnapshot, String> {
    dashboard::run_checkin_all(state.inner(), &app).await
}

#[tauri::command]
pub(crate) async fn run_checkin_account(
    app: tauri::AppHandle,
    state: State<'_, DashboardRuntime>,
    account_id: String,
) -> Result<CheckinAccountSnapshot, String> {
    dashboard::run_checkin_account(state.inner(), &app, &account_id).await
}

#[tauri::command]
pub(crate) async fn reveal_account_identifier(
    state: State<'_, DashboardRuntime>,
    account_id: String,
) -> Result<String, String> {
    dashboard::reveal_account_identifier(state.inner(), &account_id).await
}

#[tauri::command]
pub(crate) async fn check_for_updates(
    app: tauri::AppHandle,
) -> Result<UpdateCheckSnapshot, String> {
    update::check_for_updates(&app.package_info().version.to_string()).await
}
