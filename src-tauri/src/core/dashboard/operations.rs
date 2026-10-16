use chrono::Local;
use tauri::AppHandle;

use super::models::{
    AccountCreditSnapshot, AccountGrowthSnapshot, CheckinAccountSnapshot, CheckinBatchSnapshot,
    DashboardSnapshot, GrowthBatchSnapshot,
};
use super::runtime::DashboardRuntime;
use crate::platforms::workbuddy::auth::{self, AccountDescriptor, AuthBundle};
use crate::platforms::workbuddy::checkin;
use crate::platforms::workbuddy::credit;
use crate::platforms::workbuddy::growth;
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

/// 全部账号依次运行成长中心；`trigger` 区分手动、每日与轮询，用于历史记录。
pub(crate) async fn run_growth_all(
    runtime: &DashboardRuntime,
    app: &AppHandle,
    trigger: &str,
) -> Result<GrowthBatchSnapshot, String> {
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
    // 成长中心按用户维度记账：同一 uid 的个人/企业身份只运行一次，
    // 否则会重复领取同一份奖励，历史里也会出现重复记录。
    let targets = unique_by_uid(runtime.account_descriptors().await);
    let mut results = Vec::with_capacity(targets.len());

    for target in targets {
        results.push(run_growth_target(runtime, app, &client, &bundle, &target, trigger).await);
    }

    Ok(GrowthBatchSnapshot {
        completed_at: Local::now().to_rfc3339(),
        accounts: results,
    })
}

pub(crate) async fn run_growth_account(
    runtime: &DashboardRuntime,
    app: &AppHandle,
    account_id: &str,
) -> Result<AccountGrowthSnapshot, String> {
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

    Ok(run_growth_target(runtime, app, &client, &bundle, &target, "manual").await)
}

async fn run_growth_target(
    runtime: &DashboardRuntime,
    app: &AppHandle,
    client: &reqwest::Client,
    bundle: &AuthBundle,
    target: &AccountDescriptor,
    trigger: &str,
) -> AccountGrowthSnapshot {
    runtime.set_growth_running(&target.account_id).await;
    runtime.emit(app).await;
    let result = async {
        let session = session_for_account(client, bundle, target).await?;
        // 成长中心只按个人身份记账，带企业头的请求会被服务端拒绝。
        let session = session.personal_context();
        Ok::<_, String>(growth::run_growth(client, &session, trigger).await)
    }
    .await;
    let snapshot = runtime
        .set_growth_result(&target.account_id, trigger, result)
        .await;
    runtime.emit(app).await;
    snapshot
}

/// 同一用户 ID 的多个身份只保留一个，避免成长中心重复领取。
/// 优先保留个人身份：成长中心按个人记账，状态也应显示在个人账号下。
fn unique_by_uid(targets: Vec<AccountDescriptor>) -> Vec<AccountDescriptor> {
    let mut selected: Vec<AccountDescriptor> = Vec::new();
    for target in targets {
        match selected.iter_mut().find(|kept| kept.uid == target.uid) {
            Some(kept) if kept.is_personal() => {}
            Some(kept) => {
                if target.is_personal() {
                    *kept = target;
                }
            }
            None => selected.push(target),
        }
    }
    selected
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

#[cfg(test)]
mod tests {
    use super::super::runtime::DashboardRuntime;
    use super::unique_by_uid;
    use crate::data::workbuddy_signin::{StoredIdentity, StoredSignin};
    use crate::platforms::workbuddy::auth;

    fn signin(uid: &str, account_type: &str, enterprise_id: Option<&str>) -> StoredSignin {
        StoredSignin {
            uid: uid.to_owned(),
            nickname: format!("账号 {uid}"),
            account_type: account_type.to_owned(),
            enterprise_id: enterprise_id.map(str::to_owned),
            domain: Some("example.invalid".to_owned()),
            token_type: Some("Bearer".to_owned()),
            access_token: "test-access-token".to_owned(),
            refresh_token: "test-refresh-token".to_owned(),
            expires_at_ms: Some(4_102_444_800_000),
            refresh_expires_at_ms: None,
            added_at_ms: 10,
            identities: Vec::new(),
        }
    }

    #[test]
    fn growth_targets_keep_one_identity_per_user_preferring_personal() {
        // 同一个用户同时有企业（当前登录）与个人身份，另有第二个用户。
        let mut stored = signin("user-a", "ultimate", Some("enterprise-1"));
        stored.identities = vec![StoredIdentity {
            account_type: "personal".to_owned(),
            enterprise_id: None,
            nickname: Some("个人账号".to_owned()),
            plugin_enabled: Some(true),
        }];
        let other = signin("user-b", "personal", None);
        let bundle = auth::build_bundle(&[stored, other]).expect("bundle should build");

        let targets = unique_by_uid(bundle.accounts);

        assert_eq!(targets.len(), 2);
        let user_a = targets
            .iter()
            .find(|target| target.uid == "user-a")
            .expect("user-a should be kept");
        assert!(user_a.is_personal());
        assert!(targets.iter().any(|target| target.uid == "user-b"));
    }

    #[test]
    fn growth_targets_keep_enterprise_only_accounts() {
        let bundle = auth::build_bundle(&[signin("user-a", "enterprise", Some("enterprise-1"))])
            .expect("bundle should build");

        let targets = unique_by_uid(bundle.accounts);

        assert_eq!(targets.len(), 1);
        assert!(!targets[0].is_personal());
    }

    #[test]
    fn runtime_defaults_are_safe_without_paths() {
        let runtime = DashboardRuntime::default();
        let snapshot = tauri::async_runtime::block_on(runtime.snapshot());

        assert!(!snapshot.logged_in);
        assert!(snapshot.accounts.is_empty());
        assert_eq!(snapshot.next_growth_run_at, None);
        assert!(tauri::async_runtime::block_on(runtime.settings_view()).growth_automation_enabled);
    }
}
