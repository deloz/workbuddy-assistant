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
    match load_bundle(runtime).await {
        Ok(bundle) => runtime.install_bundle(&bundle).await,
        Err(message) => runtime.clear_login(message).await,
    }
    runtime.snapshot().await
}

/// 读取本机保存的浏览器登录，并在此过程中完成必要的续期。
async fn load_bundle(runtime: &DashboardRuntime) -> Result<AuthBundle, String> {
    let client = network_client()?;
    auth::load_bundle(&client, runtime.signin_path()).await
}

pub(crate) async fn refresh_account_credit(
    runtime: &DashboardRuntime,
    app: &AppHandle,
    account_id: &str,
) -> Result<AccountCreditSnapshot, String> {
    let _operation = runtime.operation_lock.lock().await;
    let client = network_client()?;
    let bundle = match auth::load_bundle(&client, runtime.signin_path()).await {
        Ok(bundle) => bundle,
        Err(message) => {
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
    let client = network_client()?;
    let bundle = match auth::load_bundle(&client, runtime.signin_path()).await {
        Ok(bundle) => bundle,
        Err(message) => {
            runtime.clear_login(message.clone()).await;
            runtime.emit(app).await;
            return Err(message);
        }
    };
    runtime.install_bundle(&bundle).await;
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
    let client = network_client()?;
    let bundle = match auth::load_bundle(&client, runtime.signin_path()).await {
        Ok(bundle) => bundle,
        Err(message) => {
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
    let client = network_client()?;
    let bundle = auth::load_bundle(&client, runtime.signin_path()).await?;
    runtime.install_bundle(&bundle).await;
    bundle
        .account(account_id)
        .map(|account| account.display_id.clone())
        .ok_or_else(|| "未找到该账号，请重新检测登录信息".to_owned())
}
