use chrono::Local;
use tauri::AppHandle;

use super::models::{
    AccountCreditSnapshot, CheckinAccountSnapshot, CheckinBatchSnapshot, DashboardSnapshot,
};
use super::runtime::DashboardRuntime;
use crate::platforms::workbuddy::auth::{self, AccountDescriptor, AuthBundle};
use crate::platforms::workbuddy::checkin;
use crate::platforms::workbuddy::credit;
use crate::platforms::workbuddy::session::{network_client, session_for_account};

pub(crate) async fn reload_accounts(runtime: &DashboardRuntime) -> DashboardSnapshot {
    match auth::read_auth_bundle() {
        Ok(bundle) => runtime.install_bundle(&bundle).await,
        Err(error) => runtime.clear_login(error.to_string()).await,
    }
    runtime.snapshot().await
}

pub(crate) async fn refresh_account_credit(
    runtime: &DashboardRuntime,
    app: &AppHandle,
    account_id: &str,
) -> Result<AccountCreditSnapshot, String> {
    let _operation = runtime.operation_lock.lock().await;
    let bundle = match auth::read_auth_bundle() {
        Ok(bundle) => bundle,
        Err(error) => {
            let message = error.to_string();
            runtime.clear_login(message.clone()).await;
            runtime.emit(app).await;
            return Err(message);
        }
    };
    runtime.install_bundle(&bundle).await;
    let target = bundle
        .account(account_id)
        .cloned()
        .ok_or_else(|| "未找到该账号，请重新检测登录信息".to_owned())?;
    runtime.set_credit_loading(account_id).await;
    runtime.emit(app).await;

    let result = async {
        let client = network_client()?;
        let session = session_for_account(&client, &bundle, &target).await?;
        credit::fetch_credit_usage(&client, &session).await
    }
    .await;
    let snapshot = runtime.set_credit_result(account_id, result).await;
    runtime.emit(app).await;
    Ok(snapshot)
}

pub(crate) async fn run_checkin_all(
    runtime: &DashboardRuntime,
    app: &AppHandle,
) -> Result<CheckinBatchSnapshot, String> {
    let _operation = runtime.operation_lock.lock().await;
    run_checkin_all_locked(runtime, app).await
}

async fn run_checkin_all_locked(
    runtime: &DashboardRuntime,
    app: &AppHandle,
) -> Result<CheckinBatchSnapshot, String> {
    let bundle = match auth::read_auth_bundle() {
        Ok(bundle) => bundle,
        Err(error) => {
            let message = error.to_string();
            runtime.clear_login(message.clone()).await;
            runtime.emit(app).await;
            return Err(message);
        }
    };
    runtime.install_bundle(&bundle).await;
    let client = network_client()?;
    let targets = runtime.account_descriptors().await;
    let mut results = Vec::with_capacity(targets.len());

    for target in targets {
        results.push(run_checkin_target(runtime, app, &client, &bundle, &target).await);
    }

    Ok(CheckinBatchSnapshot {
        completed_at: Local::now().to_rfc3339(),
        accounts: results,
    })
}

pub(crate) async fn run_checkin_account(
    runtime: &DashboardRuntime,
    app: &AppHandle,
    account_id: &str,
) -> Result<CheckinAccountSnapshot, String> {
    let _operation = runtime.operation_lock.lock().await;
    let bundle = match auth::read_auth_bundle() {
        Ok(bundle) => bundle,
        Err(error) => {
            let message = error.to_string();
            runtime.clear_login(message.clone()).await;
            runtime.emit(app).await;
            return Err(message);
        }
    };
    runtime.install_bundle(&bundle).await;
    let target = bundle
        .account(account_id)
        .cloned()
        .ok_or_else(|| "未找到该账号，请重新检测登录信息".to_owned())?;
    let client = network_client()?;

    Ok(run_checkin_target(runtime, app, &client, &bundle, &target).await)
}

async fn run_checkin_target(
    runtime: &DashboardRuntime,
    app: &AppHandle,
    client: &reqwest::Client,
    bundle: &AuthBundle,
    target: &AccountDescriptor,
) -> CheckinAccountSnapshot {
    runtime.set_checkin_loading(&target.account_id).await;
    runtime.emit(app).await;
    let result = async {
        let session = session_for_account(client, bundle, target).await?;
        checkin::auto_claim_daily_checkin(client, &session).await
    }
    .await;
    let snapshot = runtime.set_checkin_result(&target.account_id, result).await;
    runtime.emit(app).await;
    snapshot
}

pub(crate) async fn reveal_account_identifier(
    runtime: &DashboardRuntime,
    account_id: &str,
) -> Result<String, String> {
    let bundle = auth::read_auth_bundle().map_err(|error| error.to_string())?;
    runtime.install_bundle(&bundle).await;
    bundle
        .account(account_id)
        .map(|account| account.display_id.clone())
        .ok_or_else(|| "未找到该账号，请重新检测登录信息".to_owned())
}
