use tauri::State;

use super::dashboard::{
    self, AccountCreditSnapshot, AccountGrowthSnapshot, AppSettingsView, CheckinAccountSnapshot,
    CheckinBatchSnapshot, DashboardRuntime, DashboardSnapshot, GrowthBatchSnapshot,
    SigninStartView,
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
pub(crate) async fn begin_workbuddy_signin(
    app: tauri::AppHandle,
    state: State<'_, DashboardRuntime>,
) -> Result<SigninStartView, String> {
    dashboard::begin_signin(state.inner(), &app).await
}

#[tauri::command]
pub(crate) async fn open_workbuddy_signin_page(
    app: tauri::AppHandle,
    state: State<'_, DashboardRuntime>,
) -> Result<(), String> {
    dashboard::open_signin_page(state.inner(), &app).await
}

#[tauri::command]
pub(crate) async fn complete_workbuddy_signin(
    app: tauri::AppHandle,
    state: State<'_, DashboardRuntime>,
) -> Result<DashboardSnapshot, String> {
    dashboard::complete_signin(state.inner(), &app).await
}

#[tauri::command]
pub(crate) async fn sign_out_account(
    app: tauri::AppHandle,
    state: State<'_, DashboardRuntime>,
    account_id: String,
) -> Result<DashboardSnapshot, String> {
    dashboard::sign_out_account(state.inner(), &app, &account_id).await
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

#[tauri::command(rename = "run_growth_all")]
pub(crate) async fn run_growth_all_command(
    app: tauri::AppHandle,
    state: State<'_, DashboardRuntime>,
) -> Result<GrowthBatchSnapshot, String> {
    dashboard::run_growth_all(state.inner(), &app, "manual").await
}

#[tauri::command]
pub(crate) async fn run_growth_account(
    app: tauri::AppHandle,
    state: State<'_, DashboardRuntime>,
    account_id: String,
) -> Result<AccountGrowthSnapshot, String> {
    dashboard::run_growth_account(state.inner(), &app, &account_id).await
}

#[tauri::command]
pub(crate) async fn get_app_settings(
    state: State<'_, DashboardRuntime>,
) -> Result<AppSettingsView, String> {
    Ok(state.settings_view().await)
}

#[tauri::command]
pub(crate) async fn set_growth_automation_enabled(
    state: State<'_, DashboardRuntime>,
    enabled: bool,
) -> Result<AppSettingsView, String> {
    state.set_growth_automation_enabled(enabled).await
}

#[tauri::command]
pub(crate) async fn set_launch_at_startup(
    state: State<'_, DashboardRuntime>,
    enabled: bool,
) -> Result<AppSettingsView, String> {
    state.set_launch_at_startup(enabled).await
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
