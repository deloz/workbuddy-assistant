mod models;
mod operations;
mod runtime;

pub(crate) use models::{
    AccountCreditSnapshot, CheckinAccountSnapshot, CheckinBatchSnapshot, DashboardSnapshot,
};
pub(crate) use operations::{
    refresh_account_credit, reload_accounts, reveal_account_identifier, run_checkin_account,
    run_checkin_all,
};
pub(crate) use runtime::{DashboardRuntime, local_today, next_local_midnight};

pub(crate) const STATE_CHANGED_EVENT: &str = "dashboard://state-changed";
pub(crate) const OPEN_CHECKIN_EVENT: &str = "dashboard://open-checkin";
