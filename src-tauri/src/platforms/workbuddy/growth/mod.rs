//! 成长中心自动化：把参考实现（workbuddy-auto-signin）的领取流程移植到 Rust 侧。
//!
//! 调用顺序与参考实现一致：Buddy 旅行 → 任务接单/领奖 → 断登补登 → 连登兑换 →
//! 盲盒抽奖 → Buddy 盲盒 → 状态读取。查询先于写入以保证幂等，写操作一律不重试。

mod rewards;
mod streak;
mod tasks;
mod travel;

use std::fmt::Write as _;
use std::str::FromStr;
use std::time::Duration;

use chrono::Local;
use reqwest::{Client, Method, StatusCode};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::Value;
use tokio::time::{Instant, sleep};

use super::session::{API_BASE_URL, AccountSession};
use super::net::RETRY_DELAYS;
use super::signin::random_hex;

/// 成长中心接口前缀。
const GROWTH_PATH: &str = "/v2/activity/growth";
/// 单账号一轮的总时间预算：网络异常时避免一次自动运行被拖穿。
const RUN_BUDGET: Duration = Duration::from_mins(4);
/// 接单接口的批量上限。
const TASK_BATCH_SIZE: usize = 20;

const STEP_OK: &str = "ok";
const STEP_NORMAL: &str = "normal";
const STEP_SKIPPED: &str = "skipped";
const STEP_FAILED: &str = "failed";

/// 只读状态快照，供界面展示当前成长中心概况。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GrowthOverview {
    pub travel_state: Option<String>,
    pub travel_location: Option<String>,
    pub travel_eta: Option<String>,
    pub travel_daily_limit_reached: Option<bool>,
    pub energy: Option<String>,
    pub streak_days: Option<u32>,
    pub makeup_cards: Option<u32>,
    pub lottery_chances: Option<u32>,
    pub redeem_starter: Option<String>,
    pub redeem_advanced: Option<String>,
    pub redeem_legendary: Option<String>,
    pub tasks_available: Option<u32>,
    pub tasks_in_progress: Option<u32>,
    pub tasks_claimable: Option<u32>,
}

/// 一次运行中的单个动作结果。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GrowthStep {
    pub id: &'static str,
    pub label: &'static str,
    /// `ok` 领到了东西，`normal` 属于常态，`skipped` 预算耗尽，`failed` 失败。
    pub status: &'static str,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GrowthSnapshot {
    pub ran_at: String,
    pub trigger: String,
    pub report: String,
    pub overview: GrowthOverview,
    pub steps: Vec<GrowthStep>,
    pub credits_gained: String,
    pub energy: Option<String>,
    pub streak_days: Option<u32>,
    pub needs_attention: bool,
    pub idle: bool,
}

#[derive(Debug)]
/// 一次调用的失败分类；只有硬失败才需要用户关注。
enum CallFailure {
    /// 认证或权限被拒绝，本轮立即停止。
    Auth,
    /// 时间预算耗尽。
    Budget,
    /// 连不上服务端或请求超时。
    Network,
    /// 其它非 2xx 响应，携带状态码与响应体。
    Http { status: StatusCode, body: Value },
}

impl CallFailure {
    fn is_hard(&self) -> bool {
        match self {
            Self::Auth | Self::Budget | Self::Network => true,
            Self::Http { status, .. } => status.as_u16() >= 500,
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Auth => "登录凭据被拒绝".to_owned(),
            Self::Budget => "时间预算耗尽".to_owned(),
            Self::Network => "网络不可达".to_owned(),
            Self::Http { status, body } => {
                let detail = body_message(body);
                if detail.is_empty() {
                    format!("HTTP {status}")
                } else {
                    format!("HTTP {status}（{detail}）")
                }
            }
        }
    }
}

pub(crate) struct GrowthRun<'a> {
    client: &'a Client,
    session: &'a AccountSession,
    deadline: Instant,
    overview: GrowthOverview,
    steps: Vec<GrowthStep>,
    credits_gained: Decimal,
    successes: usize,
    failures: usize,
    hard_failures: usize,
    /// 认证被拒或网络不可达时的停止原因，剩余模块直接跳过。
    halt: Option<String>,
    streak_days: Option<u32>,
    streak_stale: bool,
    energy: Option<String>,
}

/// 执行一轮成长中心，返回可展示、可入库的结果。
pub(crate) async fn run_growth(
    client: &Client,
    session: &AccountSession,
    trigger: &str,
) -> GrowthSnapshot {
    let mut run = GrowthRun {
        client,
        session,
        deadline: Instant::now() + RUN_BUDGET,
        overview: GrowthOverview::default(),
        steps: Vec::new(),
        credits_gained: Decimal::ZERO,
        successes: 0,
        failures: 0,
        hard_failures: 0,
        halt: None,
        streak_days: None,
        streak_stale: false,
        energy: None,
    };
    run.execute().await;
    run.into_snapshot(trigger)
}

impl GrowthRun<'_> {
    async fn execute(&mut self) {
        self.run_travel().await;
        if self.halt.is_none() {
            self.run_tasks().await;
        }
        if self.halt.is_none() {
            self.run_makeup().await;
        }
        if self.halt.is_none() {
            self.run_redeem().await;
        }
        if self.halt.is_none() {
            self.run_lottery().await;
        }
        if self.halt.is_none() {
            self.run_buddy_open().await;
        }
        if self.halt.is_none() {
            self.run_status_readout().await;
        }
        if let Some(reason) = self.halt.take() {
            self.note_failed("run", "成长中心", &reason);
        }
    }

    fn budget_left(&self) -> bool {
        Instant::now() < self.deadline
    }

    fn push_step(
        &mut self,
        id: &'static str,
        label: &'static str,
        status: &'static str,
        detail: String,
    ) {
        self.steps.push(GrowthStep {
            id,
            label,
            status,
            detail,
        });
    }

    fn note_ok(&mut self, id: &'static str, label: &'static str, detail: String) {
        self.successes += 1;
        self.push_step(id, label, STEP_OK, detail);
    }

    fn note_normal(&mut self, id: &'static str, label: &'static str, detail: String) {
        self.push_step(id, label, STEP_NORMAL, detail);
    }

    fn note_failure_by_severity(
        &mut self,
        id: &'static str,
        label: &'static str,
        detail: String,
        hard: bool,
    ) {
        self.failures += 1;
        if hard {
            self.hard_failures += 1;
        }
        self.push_step(id, label, STEP_FAILED, detail);
    }

    fn note_failed(&mut self, id: &'static str, label: &'static str, detail: &str) {
        self.note_failure_by_severity(id, label, detail.to_owned(), true);
    }

    fn note_skipped(&mut self, id: &'static str, label: &'static str, detail: &str) {
        self.failures += 1;
        self.hard_failures += 1;
        self.push_step(id, label, STEP_SKIPPED, detail.to_owned());
    }

    /// 认证被拒时记录停止原因，让调用方跳过剩余模块。
    fn handle_auth(&mut self, failure: &CallFailure) -> bool {
        if matches!(failure, CallFailure::Auth) {
            self.halt = Some("登录凭据被拒绝，成长中心已停止，请在设置中重新登录".to_owned());
            true
        } else {
            false
        }
    }

    fn note_call_failure(
        &mut self,
        id: &'static str,
        label: &'static str,
        action: &str,
        failure: &CallFailure,
    ) {
        if self.handle_auth(failure) {
            return;
        }
        let detail = format!("{action}失败：{}", failure.describe());
        let hard = failure.is_hard();
        self.note_failure_by_severity(id, label, detail, hard);
    }

    async fn get_json(&self, path: &str) -> Result<Value, CallFailure> {
        self.send(Method::GET, path, None, true).await
    }

    async fn post_json(&self, path: &str, payload: Value) -> Result<Value, CallFailure> {
        self.send(Method::POST, path, Some(payload), false).await
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        payload: Option<Value>,
        retry: bool,
    ) -> Result<Value, CallFailure> {
        let url = format!("{API_BASE_URL}{GROWTH_PATH}{path}");
        let mut attempt = 0;
        loop {
            if !self.budget_left() {
                return Err(CallFailure::Budget);
            }
            let mut request = self.session.authorize(
                self.client
                    .request(method.clone(), url.as_str())
                    .header("Accept", "application/json"),
            );
            if let Some(payload) = &payload {
                request = request.json(payload);
            }

            let failure = match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                        return Err(CallFailure::Auth);
                    }
                    let body = response.json::<Value>().await.unwrap_or(Value::Null);
                    if status.is_success() {
                        return Ok(body);
                    }
                    CallFailure::Http { status, body }
                }
                Err(_) => CallFailure::Network,
            };

            if retry && failure.is_hard() && attempt < RETRY_DELAYS.len() {
                let delay = RETRY_DELAYS[attempt];
                attempt += 1;
                if !self.budget_left() {
                    return Err(CallFailure::Budget);
                }
                sleep(delay).await;
                continue;
            }
            return Err(failure);
        }
    }

    fn into_snapshot(mut self, trigger: &str) -> GrowthSnapshot {
        self.overview.energy.clone_from(&self.energy);
        self.overview.streak_days = self.streak_days;
        let report = self.build_report();
        GrowthSnapshot {
            ran_at: Local::now().to_rfc3339(),
            trigger: trigger.to_owned(),
            report,
            overview: self.overview,
            steps: self.steps,
            credits_gained: format_decimal(self.credits_gained),
            energy: self.energy,
            streak_days: self.streak_days,
            needs_attention: self.hard_failures > 0,
            idle: self.successes == 0 && self.failures == 0,
        }
    }

    fn build_report(&self) -> String {
        let mut parts: Vec<String> = self.steps.iter().map(|step| step.detail.clone()).collect();
        if parts.is_empty() {
            parts.push(if self.failures > 0 {
                "成长中心各步骤均失败".to_owned()
            } else {
                "成长中心无可领取项".to_owned()
            });
        }

        let mut tail = Vec::new();
        if let Some(energy) = &self.energy {
            tail.push(format!("能量 {energy}"));
        }
        if let Some(days) = self.streak_days {
            tail.push(format!("连签 {days} 天"));
        }
        if self.credits_gained > Decimal::ZERO {
            tail.push(format!(
                "本次 +共 {} 积分",
                format_decimal(self.credits_gained)
            ));
        }

        let mut report = parts.join("；");
        if !tail.is_empty() {
            let _ = write!(report, "（{}）", tail.join("，"));
        }
        report
    }
}

/// 抽奖、连登兑换等接口要求的防重放 token；服务端只做幂等去重。
fn client_token() -> String {
    format!("u-{}", random_hex(32))
}

fn format_decimal(value: Decimal) -> String {
    value.normalize().to_string()
}

/// 兼容 `data`/`result` 包裹的响应里查找字段。
fn dig<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let map = value.as_object()?;
    if let Some(found) = map.get(key).filter(|found| !found.is_null()) {
        return Some(found);
    }
    for wrapper in ["data", "result", "resp", "response"] {
        if let Some(inner) = map.get(wrapper)
            && let Some(found) = dig(inner, key)
        {
            return Some(found);
        }
    }
    None
}

/// 业务码：缺省视为成功，只有显式的非零码才算失败。
fn business_code(body: &Value) -> Option<i64> {
    body.get("code").and_then(Value::as_i64)
}

fn api_success(body: &Value) -> bool {
    business_code(body).is_none_or(|code| code == 0)
}

fn business_detail(body: &Value) -> String {
    let detail = body_message(body);
    if !detail.is_empty() {
        return detail;
    }
    match business_code(body) {
        Some(code) => format!("服务端返回错误（code {code}）"),
        None => "服务端返回了无法识别的响应".to_owned(),
    }
}

/// 服务端给的说明文案；截断后再展示，避免把整段响应塞进界面。
fn body_message(body: &Value) -> String {
    for key in ["msg", "error", "message"] {
        if let Some(text) = body.get(key).and_then(Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                return text.chars().take(120).collect();
            }
        }
    }
    String::new()
}

fn text_of(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn u32_of(value: Option<&Value>) -> Option<u32> {
    match value? {
        Value::Number(number) => number.as_u64().and_then(|value| u32::try_from(value).ok()),
        Value::String(text) => text.trim().parse::<u32>().ok(),
        _ => None,
    }
}

fn bool_of(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(value) => Some(*value),
        _ => None,
    }
}

fn decimal_of(value: Option<&Value>) -> Option<Decimal> {
    let text = match value? {
        Value::String(text) => text.trim().to_owned(),
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    Decimal::from_str(&text).ok()
}

fn number_of(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// 用服务端时间戳算「还有多久回来」；任一缺失或非法时返回空串。
#[allow(
    clippy::cast_possible_truncation,
    reason = "已排除非有限值与非正数，取整只用于展示分钟数，不会溢出 i64"
)]
fn travel_eta(arrive_at: Option<&Value>, server_now: Option<&Value>) -> String {
    let (Some(arrive_at), Some(server_now)) = (number_of(arrive_at), number_of(server_now)) else {
        return String::new();
    };
    let left = arrive_at - server_now;
    if !left.is_finite() {
        return String::new();
    }
    if left <= 0.0 {
        return "，已到达待领取".to_owned();
    }
    let minutes = (left / 60.0).round() as i64;
    if minutes < 60 {
        return format!("，约 {} 分钟后回", minutes.max(1));
    }
    format!("，约 {:.1} 小时后回", left / 3600.0)
}

/// Buddy 盲盒一次能开的数量：受服务端上限约束。
fn buddy_open_count(affordable: u32, max_open: Option<u32>) -> Option<u32> {
    if affordable == 0 {
        return None;
    }
    Some(affordable.min(max_open.filter(|value| *value > 0).unwrap_or(1)))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// 只读联调：需要真实凭据时用 `WORKBUDDY_LIVE_TOKEN` + `WORKBUDDY_LIVE_UID` 打开。
    /// 只发 GET，不触发任何领取，因此不会消耗奖励；默认跳过。
    #[test]
    fn live_readout_parses_the_real_service_when_opted_in() {
        let (Ok(token), Ok(uid)) = (
            std::env::var("WORKBUDDY_LIVE_TOKEN"),
            std::env::var("WORKBUDDY_LIVE_UID"),
        ) else {
            return;
        };
        let session = AccountSession::for_test(uid, token, None);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("client should build");
        let run = GrowthRun {
            client: &client,
            session: &session,
            deadline: Instant::now() + RUN_BUDGET,
            overview: GrowthOverview::default(),
            steps: Vec::new(),
            credits_gained: Decimal::ZERO,
            successes: 0,
            failures: 0,
            hard_failures: 0,
            halt: None,
            streak_days: None,
            streak_stale: false,
            energy: None,
        };

        tauri::async_runtime::block_on(async {
            let travel = run.get_json("/buddy/travel/status").await;
            let streak = run.get_json("/streak").await;
            let heatmap = run.get_json("/heatmap").await;
            let tasks = run.get_json("/tasks").await;
            let quota = run.get_json("/buddy/quota").await;
            let energy = run.get_json("/energy").await;
            let redeem = run.get_json("/redeem/summary").await;
            let lottery = run.get_json("/lottery/chances").await;

            let travel = travel.expect("travel status should be readable");
            assert!(text_of(dig(&travel, "state")).is_some());
            let streak = streak.expect("streak should be readable");
            assert!(
                u32_of(dig(&streak, "makeup_cards").and_then(|cards| dig(cards, "balance")))
                    .is_some()
            );
            assert!(
                streak::makeup_candidates(&streak, &heatmap.expect("heatmap should be readable"))
                    .is_ok()
            );
            assert!(dig(&tasks.expect("tasks should be readable"), "tasks").is_some());
            assert!(u32_of(dig(&quota.expect("quota should be readable"), "affordable")).is_some());
            assert!(text_of(dig(&energy.expect("energy should be readable"), "balance")).is_some());
            assert!(
                dig(
                    &redeem.expect("redeem should be readable"),
                    "starter_status"
                )
                .is_some()
            );
            assert!(
                u32_of(dig(
                    &lottery.expect("lottery should be readable"),
                    "balance"
                ))
                .is_some()
            );
        });
    }

    /// 全流程联调：除了凭据，还要显式设置 `WORKBUDDY_LIVE_WRITE=1`。
    /// 这会真实领取奖励，仅供接口改版时人工核对，默认绝不执行。
    #[test]
    fn live_growth_run_is_opt_in_and_prints_the_report() {
        let (Ok(token), Ok(uid)) = (
            std::env::var("WORKBUDDY_LIVE_TOKEN"),
            std::env::var("WORKBUDDY_LIVE_UID"),
        ) else {
            return;
        };
        if std::env::var("WORKBUDDY_LIVE_WRITE").ok().as_deref() != Some("1") {
            return;
        }
        let session = AccountSession::for_test(uid, token, None);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("client should build");
        let snapshot = tauri::async_runtime::block_on(run_growth(&client, &session, "manual"));

        println!("report={}", snapshot.report);
        println!("credits_gained={}", snapshot.credits_gained);
        println!("needs_attention={}", snapshot.needs_attention);
        for step in &snapshot.steps {
            println!("  [{}] {}: {}", step.status, step.label, step.detail);
        }
    }

    #[test]
    fn dig_reads_nested_envelopes() {
        let body = json!({"code": 0, "data": {"result": {"state": "idle"}}});

        assert_eq!(text_of(dig(&body, "state")).as_deref(), Some("idle"));
        assert_eq!(business_code(&body), Some(0));
    }

    #[test]
    fn dig_skips_null_fields() {
        let body = json!({"state": null, "data": {"state": "traveling"}});

        assert_eq!(text_of(dig(&body, "state")).as_deref(), Some("traveling"));
    }

    #[test]
    fn business_code_defaults_to_success_only_when_absent() {
        assert!(api_success(&json!({"data": {}})));
        assert!(api_success(&json!({"code": 0})));
        assert!(!api_success(&json!({"code": 41001})));
        assert_eq!(
            business_detail(&json!({"code": 41001})),
            "服务端返回错误（code 41001）"
        );
    }

    #[test]
    fn body_message_prefers_msg_and_truncates() {
        assert_eq!(
            body_message(&json!({"msg": "boom", "error": "network down"})),
            "boom"
        );
        assert_eq!(
            body_message(&json!({"error": "network down"})),
            "network down"
        );
        assert_eq!(
            body_message(&json!({"msg": "x".repeat(300)}))
                .chars()
                .count(),
            120
        );
        assert_eq!(body_message(&json!({})), "");
    }

    #[test]
    fn travel_eta_handles_missing_and_boundary_values() {
        assert_eq!(travel_eta(None, Some(&json!(100))), "");
        assert_eq!(travel_eta(Some(&json!(100)), None), "");
        assert_eq!(
            travel_eta(Some(&json!(100)), Some(&json!(100))),
            "，已到达待领取"
        );
        assert_eq!(
            travel_eta(Some(&json!(1_600)), Some(&json!(1_000))),
            "，约 10 分钟后回"
        );
        assert_eq!(
            travel_eta(Some(&json!(13_600)), Some(&json!(1_000))),
            "，约 3.5 小时后回"
        );
    }

    #[test]
    fn buddy_open_count_respects_server_limits() {
        assert_eq!(buddy_open_count(0, Some(5)), None);
        assert_eq!(buddy_open_count(3, Some(2)), Some(2));
        assert_eq!(buddy_open_count(3, None), Some(1));
        assert_eq!(buddy_open_count(3, Some(0)), Some(1));
    }

    #[test]
    fn numeric_helpers_accept_strings_and_reject_garbage() {
        assert_eq!(u32_of(Some(&json!("12"))), Some(12));
        assert_eq!(u32_of(Some(&json!("-1"))), None);
        assert_eq!(u32_of(Some(&json!({}))), None);
        assert_eq!(
            decimal_of(Some(&json!("100.5"))).map(format_decimal),
            Some("100.5".to_owned())
        );
        assert_eq!(decimal_of(Some(&json!([1]))), None);
        assert_eq!(bool_of(Some(&json!(true))), Some(true));
        assert_eq!(bool_of(Some(&json!("true"))), None);
    }
}
