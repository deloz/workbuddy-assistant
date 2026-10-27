use chrono::Local;
use serde::Serialize;

use crate::data::checkin_history::CheckinHistoryEntry;
use crate::data::growth_history::GrowthHistoryEntry;
use crate::platforms::workbuddy::checkin::CheckinSnapshot;
use crate::platforms::workbuddy::credit::CreditSnapshot;
use crate::platforms::workbuddy::growth::GrowthSnapshot;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCreditState {
    pub account_id: String,
    pub status: String,
    pub data: Option<CreditSnapshot>,
    pub error: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCheckinState {
    pub account_id: String,
    pub status: String,
    pub data: Option<CheckinSnapshot>,
    pub error: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountGrowthState {
    pub account_id: String,
    pub status: String,
    pub data: Option<GrowthSnapshot>,
    pub error: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub account_id: String,
    pub nickname: String,
    pub masked_account_id: String,
    pub account_type: String,
    pub is_current: bool,
    pub needs_signin: bool,
    pub credit: AccountCreditState,
    pub checkin: AccountCheckinState,
    pub growth: AccountGrowthState,
    pub checkin_history: Vec<CheckinHistoryEntry>,
    pub growth_history: Vec<GrowthHistoryEntry>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SigninStartView {
    pub url: String,
    pub expires_at_ms: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSnapshot {
    pub logged_in: bool,
    pub login_error: Option<String>,
    pub current_account_id: Option<String>,
    pub accounts: Vec<AccountView>,
    pub next_auto_checkin_at: Option<String>,
    pub next_growth_run_at: Option<String>,
    pub scheduler_running: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCreditSnapshot {
    pub account_id: String,
    pub status: String,
    pub data: Option<CreditSnapshot>,
    pub error: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckinAccountSnapshot {
    pub account_id: String,
    pub status: String,
    pub data: Option<CheckinSnapshot>,
    pub error: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckinBatchSnapshot {
    pub completed_at: String,
    pub accounts: Vec<CheckinAccountSnapshot>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountGrowthSnapshot {
    pub account_id: String,
    pub status: String,
    pub data: Option<GrowthSnapshot>,
    pub error: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthBatchSnapshot {
    pub completed_at: String,
    pub accounts: Vec<AccountGrowthSnapshot>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsView {
    pub growth_automation_enabled: bool,
    /// 开机启动的当前实际状态：任务管理器里禁用过也算关闭。
    pub launch_at_startup: bool,
}

/// 诊断报告里的一条检查结果。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticItem {
    pub label: String,
    /// `ok` 正常、`warn` 可继续使用但有隐患、`fail` 需要用户处理。
    pub status: &'static str,
    pub detail: String,
}

/// 一次自检的完整结果；只含脱敏标识，不含任何凭据内容。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReport {
    pub ran_at: String,
    pub healthy: bool,
    pub items: Vec<DiagnosticItem>,
}

impl AccountCreditState {
    pub(super) fn idle(account_id: &str) -> Self {
        Self {
            account_id: account_id.to_owned(),
            status: "idle".to_owned(),
            data: None,
            error: None,
            updated_at: None,
        }
    }
}

impl AccountCheckinState {
    pub(super) fn idle(account_id: &str) -> Self {
        Self {
            account_id: account_id.to_owned(),
            status: "waiting".to_owned(),
            data: None,
            error: None,
            updated_at: None,
        }
    }
}

impl AccountGrowthState {
    pub(super) fn idle(account_id: &str) -> Self {
        Self {
            account_id: account_id.to_owned(),
            status: "idle".to_owned(),
            data: None,
            error: None,
            updated_at: None,
        }
    }
}

impl AccountCreditSnapshot {
    pub(super) fn from_state(state: &AccountCreditState) -> Self {
        Self {
            account_id: state.account_id.clone(),
            status: state.status.clone(),
            data: state.data.clone(),
            error: state.error.clone(),
            updated_at: state.updated_at.clone(),
        }
    }

    pub(super) fn error(account_id: &str, message: &str) -> Self {
        Self {
            account_id: account_id.to_owned(),
            status: "error".to_owned(),
            data: None,
            error: Some(message.to_owned()),
            updated_at: Some(Local::now().to_rfc3339()),
        }
    }
}

impl CheckinAccountSnapshot {
    pub(super) fn from_state(state: &AccountCheckinState) -> Self {
        Self {
            account_id: state.account_id.clone(),
            status: state.status.clone(),
            data: state.data.clone(),
            error: state.error.clone(),
            updated_at: state.updated_at.clone(),
        }
    }

    pub(super) fn error(account_id: &str, message: &str) -> Self {
        Self {
            account_id: account_id.to_owned(),
            status: "error".to_owned(),
            data: None,
            error: Some(message.to_owned()),
            updated_at: Some(Local::now().to_rfc3339()),
        }
    }
}

impl AccountGrowthSnapshot {
    pub(super) fn from_state(state: &AccountGrowthState) -> Self {
        Self {
            account_id: state.account_id.clone(),
            status: state.status.clone(),
            data: state.data.clone(),
            error: state.error.clone(),
            updated_at: state.updated_at.clone(),
        }
    }

    pub(super) fn error(account_id: &str, message: &str) -> Self {
        Self {
            account_id: account_id.to_owned(),
            status: "error".to_owned(),
            data: None,
            error: Some(message.to_owned()),
            updated_at: Some(Local::now().to_rfc3339()),
        }
    }
}
