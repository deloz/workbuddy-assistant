use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use super::models::{DashboardSnapshot, SigninStartView};
use super::operations::reload_accounts;
use super::runtime::{DashboardRuntime, PendingSignin};
use crate::data::workbuddy_signin::{SigninStore, StoredSignin};
use crate::platforms::workbuddy::auth;
use crate::platforms::workbuddy::session::network_client;
use crate::platforms::workbuddy::signin;

const NO_PENDING_MESSAGE: &str = "没有正在进行的登录，请先点击浏览器登录";
const SAVE_FAILED_MESSAGE: &str = "无法保存登录凭据，请检查磁盘权限后重试";

/// 发起一次浏览器登录：取登录地址、记为待完成会话，并打开系统浏览器。
pub(crate) async fn begin(
    runtime: &DashboardRuntime,
    app: &AppHandle,
) -> Result<SigninStartView, String> {
    if let Some(pending) = runtime.pending_signin().await {
        return Ok(start_view(&pending));
    }

    let client = network_client()?;
    let start = signin::begin(&client).await?;
    let pending = PendingSignin {
        state: start.state,
        url: start.url,
        started_at_ms: auth::unix_now_ms(),
    };
    runtime.set_pending_signin(pending.clone()).await;
    open_in_browser(app, &pending.url)?;
    Ok(start_view(&pending))
}

/// 重新打开尚未完成的登录页面。
pub(crate) async fn open_pending(
    runtime: &DashboardRuntime,
    app: &AppHandle,
) -> Result<(), String> {
    let pending = runtime
        .pending_signin()
        .await
        .ok_or_else(|| NO_PENDING_MESSAGE.to_owned())?;
    open_in_browser(app, &pending.url)
}

/// 等待用户在浏览器里完成登录；成功后保存凭据并装载账号。
pub(crate) async fn complete(
    runtime: &DashboardRuntime,
    app: &AppHandle,
) -> Result<DashboardSnapshot, String> {
    let _operation = runtime.operation_lock.lock().await;
    let pending = runtime
        .pending_signin()
        .await
        .ok_or_else(|| NO_PENDING_MESSAGE.to_owned())?;

    let client = network_client()?;
    let outcome = signin::await_login(&client, &pending.state).await?;
    let stored = StoredSignin {
        uid: outcome.uid,
        nickname: outcome.nickname,
        account_type: outcome.account_type,
        enterprise_id: outcome.enterprise_id,
        domain: outcome.tokens.domain,
        token_type: outcome.tokens.token_type,
        access_token: outcome.tokens.access_token,
        refresh_token: outcome.tokens.refresh_token,
        expires_at_ms: outcome.tokens.expires_at_ms,
        refresh_expires_at_ms: outcome.tokens.refresh_expires_at_ms,
        added_at_ms: auth::unix_now_ms(),
        identities: outcome.identities,
    };

    let mut store = SigninStore::load(runtime.signin_path().to_path_buf());
    store
        .upsert(stored)
        .map_err(|_| SAVE_FAILED_MESSAGE.to_owned())?;
    runtime.clear_pending_signin().await;

    let snapshot = reload_accounts(runtime).await;
    runtime.emit(app).await;
    Ok(snapshot)
}

/// 移除一个账号的登录，返回移除后的快照。
pub(crate) async fn sign_out(
    runtime: &DashboardRuntime,
    app: &AppHandle,
    account_id: &str,
) -> Result<DashboardSnapshot, String> {
    let _operation = runtime.operation_lock.lock().await;
    let mut store = SigninStore::load(runtime.signin_path().to_path_buf());
    let uid = auth::build_bundle(store.signins())
        .ok()
        .and_then(|bundle| {
            bundle
                .account(account_id)
                .map(|account| account.uid.clone())
        })
        .ok_or_else(|| "未找到该账号，请重新检测登录信息".to_owned())?;

    store
        .remove(&uid)
        .map_err(|_| SAVE_FAILED_MESSAGE.to_owned())?;

    let snapshot = reload_accounts(runtime).await;
    runtime.emit(app).await;
    Ok(snapshot)
}

fn start_view(pending: &PendingSignin) -> SigninStartView {
    SigninStartView {
        url: pending.url.clone(),
        expires_at_ms: pending
            .started_at_ms
            .saturating_add(signin::LOGIN_TIMEOUT_MS),
    }
}

fn open_in_browser(app: &AppHandle, url: &str) -> Result<(), String> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| "无法打开系统浏览器，请重试".to_owned())
}
