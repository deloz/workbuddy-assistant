use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Local, NaiveDate, TimeZone};
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, RwLock};

use super::STATE_CHANGED_EVENT;
use super::models::{
    AccountCheckinState, AccountCreditSnapshot, AccountCreditState, AccountGrowthSnapshot,
    AccountGrowthState, AccountView, AppSettingsView, CheckinAccountSnapshot, DashboardSnapshot,
};
use crate::data::checkin_history::{
    CheckinHistoryEntry, CheckinHistoryOutcome, CheckinHistoryStore,
};
use crate::data::growth_history::{GrowthHistoryEntry, GrowthHistoryStore};
use crate::data::settings::{AppSettings, SettingsStore};
use crate::platforms::workbuddy::auth::{AccountDescriptor, AuthBundle};
use crate::platforms::workbuddy::checkin::CheckinSnapshot;
use crate::platforms::workbuddy::credit::CreditSnapshot;
use crate::platforms::workbuddy::growth::GrowthSnapshot;

/// 等待用户在浏览器里完成的一次登录。
#[derive(Clone)]
pub(crate) struct PendingSignin {
    pub state: String,
    pub url: String,
    pub started_at_ms: i64,
}

impl PendingSignin {
    fn is_fresh(&self, now_ms: i64) -> bool {
        now_ms.saturating_sub(self.started_at_ms)
            < crate::platforms::workbuddy::signin::LOGIN_TIMEOUT_MS
    }
}

struct AccountCache {
    descriptor: AccountDescriptor,
    credit: AccountCreditState,
    checkin: AccountCheckinState,
    growth: AccountGrowthState,
}

#[derive(Default)]
struct RuntimeState {
    logged_in: bool,
    login_error: Option<String>,
    current_account_id: Option<String>,
    accounts: Vec<AccountCache>,
    next_auto_checkin_at: Option<String>,
    next_growth_run_at: Option<String>,
    last_batch_at: Option<String>,
    scheduler_running: bool,
    last_auto_checkin_date: Option<NaiveDate>,
    checkin_history: CheckinHistoryStore,
    growth_history: GrowthHistoryStore,
}

pub struct DashboardRuntime {
    state: RwLock<RuntimeState>,
    pub(super) operation_lock: Mutex<()>,
    signin_path: PathBuf,
    settings: Mutex<SettingsStore>,
    pending_signin: Mutex<Option<PendingSignin>>,
    exiting: AtomicBool,
}

impl Default for DashboardRuntime {
    fn default() -> Self {
        Self::with_paths(
            PathBuf::new(),
            PathBuf::new(),
            PathBuf::new(),
            PathBuf::new(),
        )
    }
}

impl DashboardRuntime {
    pub fn with_paths(
        history_path: PathBuf,
        growth_history_path: PathBuf,
        settings_path: PathBuf,
        signin_path: PathBuf,
    ) -> Self {
        Self {
            state: RwLock::new(RuntimeState {
                checkin_history: CheckinHistoryStore::load(history_path),
                growth_history: GrowthHistoryStore::load(growth_history_path),
                ..RuntimeState::default()
            }),
            operation_lock: Mutex::new(()),
            signin_path,
            settings: Mutex::new(SettingsStore::load(settings_path)),
            pending_signin: Mutex::new(None),
            exiting: AtomicBool::new(false),
        }
    }

    pub(crate) fn signin_path(&self) -> &Path {
        &self.signin_path
    }

    pub(crate) async fn app_settings(&self) -> AppSettings {
        self.settings.lock().await.settings().clone()
    }

    pub(crate) async fn settings_view(&self) -> AppSettingsView {
        AppSettingsView {
            growth_automation_enabled: self.app_settings().await.growth_automation_enabled,
        }
    }

    pub(crate) async fn set_growth_automation_enabled(
        &self,
        enabled: bool,
    ) -> Result<AppSettingsView, String> {
        let mut settings = self.settings.lock().await;
        settings
            .set_growth_automation_enabled(enabled)
            .map_err(|_| "无法保存设置，请检查磁盘权限后重试".to_owned())?;
        Ok(AppSettingsView {
            growth_automation_enabled: enabled,
        })
    }

    /// 尚未超时的待完成登录；超时即视为没有。
    pub(crate) async fn pending_signin(&self) -> Option<PendingSignin> {
        let mut pending = self.pending_signin.lock().await;
        match pending.as_ref() {
            Some(signin) if signin.is_fresh(crate::platforms::workbuddy::auth::unix_now_ms()) => {
                Some(signin.clone())
            }
            Some(_) => {
                *pending = None;
                None
            }
            None => None,
        }
    }

    pub(crate) async fn set_pending_signin(&self, signin: PendingSignin) {
        *self.pending_signin.lock().await = Some(signin);
    }

    pub(crate) async fn clear_pending_signin(&self) {
        *self.pending_signin.lock().await = None;
    }

    pub async fn snapshot(&self) -> DashboardSnapshot {
        let state = self.state.read().await;
        DashboardSnapshot {
            logged_in: state.logged_in,
            login_error: state.login_error.clone(),
            current_account_id: state.current_account_id.clone(),
            accounts: state
                .accounts
                .iter()
                .map(|account| AccountView {
                    account_id: account.descriptor.account_id.clone(),
                    nickname: account.descriptor.nickname.clone(),
                    masked_account_id: account.descriptor.masked_display_id.clone(),
                    account_type: account.descriptor.account_type.clone(),
                    is_current: account.descriptor.is_current,
                    needs_signin: account.descriptor.needs_signin,
                    credit: account.credit.clone(),
                    checkin: account.checkin.clone(),
                    growth: account.growth.clone(),
                    checkin_history: state
                        .checkin_history
                        .entries_for(&account.descriptor.account_id),
                    growth_history: state
                        .growth_history
                        .entries_for(&account.descriptor.account_id),
                })
                .collect(),
            next_auto_checkin_at: state.next_auto_checkin_at.clone(),
            next_growth_run_at: state.next_growth_run_at.clone(),
            scheduler_running: state.scheduler_running,
        }
    }

    pub fn request_exit(&self) {
        self.exiting.store(true, Ordering::Release);
    }

    pub fn is_exiting(&self) -> bool {
        self.exiting.load(Ordering::Acquire)
    }

    pub(super) async fn install_bundle(&self, bundle: &AuthBundle) {
        let mut state = self.state.write().await;
        let old_accounts = std::mem::take(&mut state.accounts);
        state.accounts = bundle
            .accounts
            .iter()
            .map(|descriptor| {
                let cached = old_accounts
                    .iter()
                    .find(|old| old.descriptor.same_identity(descriptor));
                AccountCache {
                    descriptor: descriptor.clone(),
                    credit: cached.map_or_else(
                        || AccountCreditState::idle(&descriptor.account_id),
                        |old| old.credit.clone(),
                    ),
                    checkin: cached.map_or_else(
                        || AccountCheckinState::idle(&descriptor.account_id),
                        |old| old.checkin.clone(),
                    ),
                    growth: cached.map_or_else(
                        || AccountGrowthState::idle(&descriptor.account_id),
                        |old| old.growth.clone(),
                    ),
                }
            })
            .collect();
        state.logged_in = true;
        state.login_error = None;
        state.current_account_id = bundle.current_account_id.clone();
    }

    pub(super) async fn clear_login(&self, error: String) {
        let mut state = self.state.write().await;
        state.logged_in = false;
        state.login_error = Some(error);
        state.current_account_id = None;
        state.accounts.clear();
    }

    /// 返回计划时间是否变化；调度循环据此决定要不要广播快照。
    pub async fn set_scheduler(&self, running: bool, next_at: Option<String>) -> bool {
        let mut state = self.state.write().await;
        let changed = state.scheduler_running != running || state.next_auto_checkin_at != next_at;
        state.scheduler_running = running;
        state.next_auto_checkin_at = next_at;
        changed
    }

    /// 同上：只有计划时间真的变了才需要通知前端重绘。
    pub async fn set_next_growth_run(&self, next_at: Option<String>) -> bool {
        let mut state = self.state.write().await;
        let changed = state.next_growth_run_at != next_at;
        state.next_growth_run_at = next_at;
        changed
    }

    /// 最近一次自动批次（签到 + 成长）的时间；轮询以它为基准计时。
    pub async fn last_batch_at(&self) -> Option<String> {
        self.state.read().await.last_batch_at.clone()
    }

    pub async fn is_logged_in(&self) -> bool {
        self.state.read().await.logged_in
    }

    pub async fn mark_batch_ran(&self, at: String) {
        self.state.write().await.last_batch_at = Some(at);
    }

    pub async fn mark_auto_date_if_needed(&self, date: NaiveDate) -> bool {
        let mut state = self.state.write().await;
        if !should_run_automatic(state.last_auto_checkin_date, date) {
            return false;
        }
        state.last_auto_checkin_date = Some(date);
        true
    }

    pub async fn auto_date_is_pending(&self, date: NaiveDate) -> bool {
        self.state.read().await.last_auto_checkin_date != Some(date)
    }

    pub(super) async fn set_credit_loading(&self, account_id: &str) {
        let mut state = self.state.write().await;
        if let Some(account) = state
            .accounts
            .iter_mut()
            .find(|account| account.descriptor.account_id == account_id)
        {
            account.credit.status = "loading".to_owned();
            account.credit.error = None;
        }
    }

    pub(super) async fn set_credit_result(
        &self,
        account_id: &str,
        result: Result<CreditSnapshot, String>,
    ) -> AccountCreditSnapshot {
        let mut state = self.state.write().await;
        let Some(account) = state
            .accounts
            .iter_mut()
            .find(|account| account.descriptor.account_id == account_id)
        else {
            return AccountCreditSnapshot::error(account_id, "未找到该账号，请重新检测登录信息");
        };

        let updated_at = Some(Local::now().to_rfc3339());
        match result {
            Ok(snapshot) => {
                account.credit.status = "ready".to_owned();
                account.credit.data = Some(snapshot);
                account.credit.error = None;
                account.credit.updated_at = updated_at;
            }
            Err(error) => {
                account.credit.status = "error".to_owned();
                account.credit.error = Some(error);
                account.credit.updated_at = updated_at;
            }
        }
        AccountCreditSnapshot::from_state(&account.credit)
    }

    pub(super) async fn set_checkin_loading(&self, account_id: &str) {
        let mut state = self.state.write().await;
        if let Some(account) = state
            .accounts
            .iter_mut()
            .find(|account| account.descriptor.account_id == account_id)
        {
            account.checkin.status = "loading".to_owned();
            account.checkin.error = None;
        }
    }

    pub(super) async fn set_checkin_result(
        &self,
        account_id: &str,
        result: Result<CheckinSnapshot, String>,
    ) -> CheckinAccountSnapshot {
        let mut state = self.state.write().await;
        let Some(account) = state
            .accounts
            .iter_mut()
            .find(|account| account.descriptor.account_id == account_id)
        else {
            return CheckinAccountSnapshot::error(account_id, "未找到该账号，请重新检测登录信息");
        };

        let checked_at = Local::now();
        let updated_at = checked_at.to_rfc3339();
        let local_date = checked_at.date_naive().to_string();
        let history_entry = match &result {
            Ok(snapshot) => CheckinHistoryEntry::success(
                local_date,
                updated_at.clone(),
                CheckinHistoryOutcome {
                    status: snapshot.state.clone(),
                    credit_earned: snapshot.claimed_now.then(|| snapshot.credit_earned.clone()),
                    streak_days: snapshot.streak_days,
                    total_credits: snapshot.total_credits.clone(),
                    activity_name: snapshot.activity_name.clone(),
                },
            ),
            Err(error) => {
                CheckinHistoryEntry::failure(local_date, updated_at.clone(), error.clone())
            }
        };
        let snapshot = match result {
            Ok(snapshot) => {
                account.checkin.status = snapshot.state.clone();
                account.checkin.data = Some(snapshot);
                account.checkin.error = None;
                account.checkin.updated_at = Some(updated_at);
                CheckinAccountSnapshot::from_state(&account.checkin)
            }
            Err(error) => {
                account.checkin.status = "error".to_owned();
                account.checkin.error = Some(error);
                account.checkin.updated_at = Some(updated_at);
                CheckinAccountSnapshot::from_state(&account.checkin)
            }
        };
        if let Err(error) = state.checkin_history.record(account_id, history_entry) {
            eprintln!("无法保存签到历史：{error}");
        }
        snapshot
    }

    pub(super) async fn set_growth_running(&self, account_id: &str) {
        let mut state = self.state.write().await;
        if let Some(account) = state
            .accounts
            .iter_mut()
            .find(|account| account.descriptor.account_id == account_id)
        {
            account.growth.status = "running".to_owned();
            account.growth.error = None;
        }
    }

    pub(super) async fn set_growth_result(
        &self,
        account_id: &str,
        trigger: &str,
        result: Result<GrowthSnapshot, String>,
    ) -> AccountGrowthSnapshot {
        let mut state = self.state.write().await;
        let Some(account) = state
            .accounts
            .iter_mut()
            .find(|account| account.descriptor.account_id == account_id)
        else {
            return AccountGrowthSnapshot::error(account_id, "未找到该账号，请重新检测登录信息");
        };

        let updated_at = Local::now().to_rfc3339();
        let (snapshot, history) = match result {
            Ok(snapshot) => {
                let history = crate::data::growth_history::should_record(
                    &snapshot.trigger,
                    &snapshot.credits_gained,
                    snapshot.needs_attention,
                )
                .then(|| GrowthHistoryEntry {
                    local_date: Local::now().date_naive().to_string(),
                    ran_at: snapshot.ran_at.clone(),
                    trigger: snapshot.trigger.clone(),
                    status: growth_state_label(&snapshot),
                    report: snapshot.report.clone(),
                    credits_gained: snapshot.credits_gained.clone(),
                    energy: snapshot.energy.clone(),
                    needs_attention: snapshot.needs_attention,
                    error: None,
                });
                account.growth.status = "ready".to_owned();
                account.growth.data = Some(snapshot);
                account.growth.error = None;
                account.growth.updated_at = Some(updated_at);
                (AccountGrowthSnapshot::from_state(&account.growth), history)
            }
            Err(error) => {
                let history = GrowthHistoryEntry {
                    local_date: Local::now().date_naive().to_string(),
                    ran_at: updated_at.clone(),
                    trigger: trigger.to_owned(),
                    status: "error".to_owned(),
                    report: error.clone(),
                    credits_gained: "0".to_owned(),
                    energy: None,
                    needs_attention: true,
                    error: Some(error.clone()),
                };
                account.growth.status = "error".to_owned();
                account.growth.error = Some(error);
                account.growth.updated_at = Some(updated_at);
                (
                    AccountGrowthSnapshot::from_state(&account.growth),
                    Some(history),
                )
            }
        };

        if let Some(history) = history
            && let Err(error) = state.growth_history.record(account_id, history)
        {
            eprintln!("无法保存成长中心历史：{error}");
        }
        snapshot
    }

    pub(super) async fn account_descriptors(&self) -> Vec<AccountDescriptor> {
        self.state
            .read()
            .await
            .accounts
            .iter()
            .map(|account| account.descriptor.clone())
            .collect()
    }

    pub async fn emit(&self, app: &AppHandle) {
        let snapshot = self.snapshot().await;
        let _ = app.emit(STATE_CHANGED_EVENT, snapshot);
    }
}

pub(crate) fn local_today() -> NaiveDate {
    Local::now().date_naive()
}

/// 历史条目里的状态标签：优先反映「有东西可领」与「需要关注」。
fn growth_state_label(snapshot: &GrowthSnapshot) -> String {
    if snapshot.needs_attention {
        "needsAttention".to_owned()
    } else if snapshot.credits_gained.trim() != "0" {
        "claimed".to_owned()
    } else if snapshot.idle {
        "idle".to_owned()
    } else {
        "checked".to_owned()
    }
}

fn should_run_automatic(last_run: Option<NaiveDate>, today: NaiveDate) -> bool {
    last_run != Some(today)
}

pub(crate) fn next_local_midnight(now: chrono::DateTime<Local>) -> chrono::DateTime<Local> {
    let tomorrow = now
        .date_naive()
        .succ_opt()
        .expect("chrono date should have a successor");
    let midnight = tomorrow
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be valid");
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .unwrap_or_else(|| now + chrono::Duration::days(1))
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};

    use super::*;

    #[test]
    fn automatic_gate_runs_on_startup_and_deduplicates_same_day() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 15).expect("date should be valid");

        assert!(should_run_automatic(None, today));
        assert!(!should_run_automatic(Some(today), today));
    }

    #[test]
    fn automatic_gate_catches_up_after_sleep_crosses_date() {
        let yesterday = NaiveDate::from_ymd_opt(2026, 8, 14).expect("date should be valid");
        let today = NaiveDate::from_ymd_opt(2026, 8, 15).expect("date should be valid");

        assert!(should_run_automatic(Some(yesterday), today));
    }

    #[test]
    fn manual_check_does_not_change_next_day_gate() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 15).expect("date should be valid");
        let tomorrow = today.succ_opt().expect("date should have successor");

        assert!(should_run_automatic(Some(today), tomorrow));
    }

    #[test]
    fn next_run_is_local_midnight_on_following_date() {
        let now = Local
            .with_ymd_and_hms(2026, 8, 15, 23, 59, 45)
            .single()
            .expect("local time should be valid");
        let next = next_local_midnight(now);

        assert_eq!(
            next.date_naive(),
            NaiveDate::from_ymd_opt(2026, 8, 16).unwrap()
        );
        assert_eq!((next.hour(), next.minute(), next.second()), (0, 0, 0));
        assert_eq!(next.year(), 2026);
    }
}
