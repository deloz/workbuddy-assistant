use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Local, NaiveDate, TimeZone};
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, RwLock};

use super::STATE_CHANGED_EVENT;
use super::models::{
    AccountCheckinState, AccountCreditSnapshot, AccountCreditState, AccountView,
    CheckinAccountSnapshot, DashboardSnapshot,
};
use crate::data::checkin_history::{
    CheckinHistoryEntry, CheckinHistoryOutcome, CheckinHistoryStore,
};
use crate::platforms::workbuddy::auth::{AccountDescriptor, AuthBundle};
use crate::platforms::workbuddy::checkin::CheckinSnapshot;
use crate::platforms::workbuddy::credit::CreditSnapshot;

struct AccountCache {
    descriptor: AccountDescriptor,
    credit: AccountCreditState,
    checkin: AccountCheckinState,
}

#[derive(Default)]
struct RuntimeState {
    logged_in: bool,
    login_error: Option<String>,
    current_account_id: Option<String>,
    accounts: Vec<AccountCache>,
    next_auto_checkin_at: Option<String>,
    scheduler_running: bool,
    last_auto_checkin_date: Option<NaiveDate>,
    checkin_history: CheckinHistoryStore,
}

pub struct DashboardRuntime {
    state: RwLock<RuntimeState>,
    pub(super) operation_lock: Mutex<()>,
    exiting: AtomicBool,
}

impl Default for DashboardRuntime {
    fn default() -> Self {
        Self {
            state: RwLock::new(RuntimeState::default()),
            operation_lock: Mutex::new(()),
            exiting: AtomicBool::new(false),
        }
    }
}

impl DashboardRuntime {
    pub fn with_history_path(path: std::path::PathBuf) -> Self {
        Self {
            state: RwLock::new(RuntimeState {
                checkin_history: CheckinHistoryStore::load(path),
                ..RuntimeState::default()
            }),
            operation_lock: Mutex::new(()),
            exiting: AtomicBool::new(false),
        }
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
                    credit: account.credit.clone(),
                    checkin: account.checkin.clone(),
                    checkin_history: state
                        .checkin_history
                        .entries_for(&account.descriptor.account_id),
                })
                .collect(),
            next_auto_checkin_at: state.next_auto_checkin_at.clone(),
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
                }
            })
            .collect();
        state.logged_in = true;
        state.login_error = None;
        state.current_account_id = bundle
            .current_account()
            .map(|account| account.account_id.clone());
    }

    pub(super) async fn clear_login(&self, error: String) {
        let mut state = self.state.write().await;
        state.logged_in = false;
        state.login_error = Some(error);
        state.current_account_id = None;
        state.accounts.clear();
    }

    pub async fn set_scheduler(&self, running: bool, next_at: Option<String>) {
        let mut state = self.state.write().await;
        state.scheduler_running = running;
        state.next_auto_checkin_at = next_at;
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
