mod diagnostics;
mod models;
mod operations;
mod runtime;
mod signin;

pub(crate) use models::{
    AccountCreditSnapshot, AccountGrowthSnapshot, AppSettingsView, CheckinAccountSnapshot,
    CheckinBatchSnapshot, DashboardSnapshot, DiagnosticReport, GrowthBatchSnapshot, SigninStartView,
};
pub(crate) use diagnostics::run as run_diagnostics;
pub(crate) use operations::{
    refresh_account_credit, reload_accounts, reveal_account_identifier, run_checkin_account,
    run_checkin_all, run_growth_account, run_growth_all,
};
pub(crate) use runtime::{DashboardRuntime, local_today, next_local_midnight};
pub(crate) use signin::{
    begin as begin_signin, complete as complete_signin, open_pending as open_signin_page,
    sign_out as sign_out_account,
};

pub(crate) const STATE_CHANGED_EVENT: &str = "dashboard://state-changed";
pub(crate) const OPEN_CHECKIN_EVENT: &str = "dashboard://open-checkin";
