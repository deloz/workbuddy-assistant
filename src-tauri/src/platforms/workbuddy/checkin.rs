use std::str::FromStr;

use chrono::Local;
use reqwest::{Client, StatusCode};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::net;
use super::session::AccountSession;

const STATUS_ENDPOINT: &str =
    "https://copilot.tencent.com/v2/billing/meter/checkin-activity-status";
const CLAIM_ENDPOINT: &str = "https://copilot.tencent.com/v2/billing/meter/daily-checkin";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "直接映射签到接口的字段，前端按同一组布尔值渲染，换成枚举反而多一层转换"
)]
pub struct CheckinSnapshot {
    pub checked_at: String,
    pub state: String,
    pub active: bool,
    pub today_checked_in: bool,
    pub claimed_now: bool,
    pub credit_earned: String,
    pub today_credit: String,
    pub streak_days: u32,
    pub total_credits: String,
    pub is_streak_day: bool,
    pub season: Option<u32>,
    pub start_at: Option<String>,
    pub end_at: Option<String>,
    pub theme_name: Option<String>,
    pub activity_name: Option<String>,
}

#[derive(Deserialize)]
struct ApiEnvelope<T> {
    code: i32,
    data: Option<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct StatusPayload {
    active: Option<bool>,
    today_checked_in: Option<bool>,
    today_credit: Option<Value>,
    streak_days: Option<u32>,
    total_credits: Option<Value>,
    is_streak_day: Option<bool>,
    season: Option<u32>,
    start_time: Option<String>,
    end_time: Option<String>,
    theme_name: Option<String>,
    activity_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct ClaimPayload {
    credit: Option<Value>,
    streak_days: Option<u32>,
    is_streak_day: Option<bool>,
}

#[derive(Clone, Debug)]
struct CheckinStatus {
    active: bool,
    today_checked_in: bool,
    today_credit: Decimal,
    streak_days: u32,
    total_credits: Decimal,
    is_streak_day: bool,
    season: Option<u32>,
    start_at: Option<String>,
    end_at: Option<String>,
    theme_name: Option<String>,
    activity_name: Option<String>,
}

#[derive(Debug)]
struct ClaimResult {
    credit: Decimal,
    streak_days: u32,
    is_streak_day: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CheckinDecision {
    Inactive,
    AlreadyClaimed,
    Claim,
}

pub(crate) async fn auto_claim_daily_checkin(
    client: &Client,
    session: &AccountSession,
) -> Result<CheckinSnapshot, String> {
    let mut status = request_status(client, session).await?;
    match decide(&status) {
        CheckinDecision::Inactive => {
            return Ok(snapshot(status, "inactive", false, Decimal::ZERO));
        }
        CheckinDecision::AlreadyClaimed => {
            return Ok(snapshot(status, "alreadyClaimed", false, Decimal::ZERO));
        }
        CheckinDecision::Claim => {}
    }

    let claim = match request_claim(client, session).await {
        Ok(result) => result,
        Err(claim_error) => {
            if let Ok(current) = request_status(client, session).await
                && current.today_checked_in
            {
                return Ok(snapshot(current, "alreadyClaimed", false, Decimal::ZERO));
            }
            return Err(claim_error);
        }
    };

    if let Ok(mut current) = request_status(client, session).await {
        current.today_checked_in = true;
        current.streak_days = current.streak_days.max(claim.streak_days);
        current.is_streak_day = claim.is_streak_day;
        Ok(snapshot(current, "claimedNow", true, claim.credit))
    } else {
        status.today_checked_in = true;
        status.streak_days = claim.streak_days;
        status.is_streak_day = claim.is_streak_day;
        status.total_credits += claim.credit;
        Ok(snapshot(status, "claimedNow", true, claim.credit))
    }
}

fn decide(status: &CheckinStatus) -> CheckinDecision {
    if !status.active {
        CheckinDecision::Inactive
    } else if status.today_checked_in {
        CheckinDecision::AlreadyClaimed
    } else {
        CheckinDecision::Claim
    }
}

async fn request_status(
    client: &Client,
    session: &AccountSession,
) -> Result<CheckinStatus, String> {
    let body = post_empty_json(client, STATUS_ENDPOINT, session, "查询签到状态").await?;
    parse_status_response(&body)
}

async fn request_claim(client: &Client, session: &AccountSession) -> Result<ClaimResult, String> {
    let body = post_empty_json(client, CLAIM_ENDPOINT, session, "领取签到积分").await?;
    parse_claim_response(&body)
}

async fn post_empty_json(
    client: &Client,
    endpoint: &str,
    session: &AccountSession,
    action: &str,
) -> Result<String, String> {
    let request_builder = session.authorize(
        client
            .post(endpoint)
            .header("Accept", "application/json")
            .json(&serde_json::json!({})),
    );

    // 网络瞬时故障已在 `send_with_retry` 里退避重试，这里只翻译最终错误。
    let response = net::send_with_retry(request_builder)
        .await
        .map_err(|error| {
            if error.is_timeout() {
                format!("{action}超时，请检查网络后重试")
            } else {
                format!("无法连接签到服务以{action}，请检查网络后重试")
            }
        })?;

    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        return Err("签到服务拒绝当前登录凭据，请在 WorkBuddy 重新登录后重试".to_owned());
    }
    if status == StatusCode::FORBIDDEN {
        return Err("当前账号没有参与签到活动的权限".to_owned());
    }
    if !status.is_success() {
        return Err(format!("签到服务暂时不可用（HTTP {status}）"));
    }

    response
        .text()
        .await
        .map_err(|_| "无法读取签到服务响应".to_owned())
}

fn parse_status_response(body: &str) -> Result<CheckinStatus, String> {
    let envelope: ApiEnvelope<StatusPayload> =
        serde_json::from_str(body).map_err(|_| "签到服务返回了无法解析的数据".to_owned())?;
    if envelope.code != 0 {
        return Err(format!("签到服务返回错误（code {}）", envelope.code));
    }

    let payload = envelope
        .data
        .ok_or_else(|| "签到服务响应缺少活动数据".to_owned())?;
    let active = payload
        .active
        .ok_or_else(|| "签到服务响应缺少字段 active".to_owned())?;
    let today_checked_in =
        required_when_active(payload.today_checked_in, active, "today_checked_in", false)?;
    let streak_days = required_when_active(payload.streak_days, active, "streak_days", 0)?;

    Ok(CheckinStatus {
        active,
        today_checked_in,
        today_credit: parse_points(payload.today_credit.as_ref(), "today_credit", active)?,
        streak_days,
        total_credits: parse_points(payload.total_credits.as_ref(), "total_credits", active)?,
        is_streak_day: payload.is_streak_day.unwrap_or(false),
        season: payload.season,
        start_at: non_empty(payload.start_time),
        end_at: non_empty(payload.end_time),
        theme_name: non_empty(payload.theme_name),
        activity_name: non_empty(payload.activity_name),
    })
}

fn parse_claim_response(body: &str) -> Result<ClaimResult, String> {
    let envelope: ApiEnvelope<ClaimPayload> =
        serde_json::from_str(body).map_err(|_| "签到服务返回了无法解析的数据".to_owned())?;
    if envelope.code != 0 {
        return Err(format!("签到服务返回错误（code {}）", envelope.code));
    }

    let payload = envelope
        .data
        .ok_or_else(|| "签到服务响应缺少领取结果".to_owned())?;
    Ok(ClaimResult {
        credit: parse_points(payload.credit.as_ref(), "credit", true)?,
        streak_days: payload
            .streak_days
            .ok_or_else(|| "签到服务响应缺少字段 streak_days".to_owned())?,
        is_streak_day: payload
            .is_streak_day
            .ok_or_else(|| "签到服务响应缺少字段 is_streak_day".to_owned())?,
    })
}

fn required_when_active<T: Copy>(
    value: Option<T>,
    active: bool,
    field: &str,
    inactive_default: T,
) -> Result<T, String> {
    if active {
        value.ok_or_else(|| format!("签到服务响应缺少字段 {field}"))
    } else {
        Ok(value.unwrap_or(inactive_default))
    }
}

fn parse_points(value: Option<&Value>, field: &str, required: bool) -> Result<Decimal, String> {
    let Some(value) = value else {
        return if required {
            Err(format!("签到服务响应缺少字段 {field}"))
        } else {
            Ok(Decimal::ZERO)
        };
    };
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => return Err(format!("签到服务响应字段 {field} 格式无效")),
    };

    Decimal::from_str(text.trim()).map_err(|_| format!("签到服务响应字段 {field} 格式无效"))
}

fn snapshot(
    status: CheckinStatus,
    state: &str,
    claimed_now: bool,
    credit_earned: Decimal,
) -> CheckinSnapshot {
    CheckinSnapshot {
        checked_at: Local::now().to_rfc3339(),
        state: state.to_owned(),
        active: status.active,
        today_checked_in: status.today_checked_in,
        claimed_now,
        credit_earned: format_points(credit_earned),
        today_credit: format_points(status.today_credit),
        streak_days: status.streak_days,
        total_credits: format_points(status.total_credits),
        is_streak_day: status.is_streak_day,
        season: status.season,
        start_at: status.start_at,
        end_at: status.end_at,
        theme_name: status.theme_name,
        activity_name: status.activity_name,
    }
}

fn format_points(points: Decimal) -> String {
    points.normalize().to_string()
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_status_fixture(today_checked_in: bool) -> String {
        format!(
            r#"{{
                "code": 0,
                "data": {{
                    "active": true,
                    "today_checked_in": {today_checked_in},
                    "today_credit": "100.00",
                    "streak_days": 2,
                    "total_credits": 200,
                    "is_streak_day": false,
                    "season": 7,
                    "start_time": "2026-08-13 00:00:00",
                    "end_time": "2026-08-24 11:00:00",
                    "theme_name": "Buddy加油站",
                    "activity_name": "本期：资料库"
                }}
            }}"#
        )
    }

    #[test]
    fn parses_active_status_and_requests_claim_when_needed() {
        let status = parse_status_response(&active_status_fixture(false))
            .expect("active fixture should parse");

        assert!(status.active);
        assert!(!status.today_checked_in);
        assert_eq!(status.today_credit, Decimal::new(100, 0));
        assert_eq!(status.total_credits, Decimal::new(200, 0));
        assert_eq!(status.season, Some(7));
        assert_eq!(status.activity_name.as_deref(), Some("本期：资料库"));
        assert_eq!(decide(&status), CheckinDecision::Claim);
    }

    #[test]
    fn maps_checked_status_without_requesting_another_claim() {
        let status = parse_status_response(&active_status_fixture(true))
            .expect("checked fixture should parse");

        assert_eq!(decide(&status), CheckinDecision::AlreadyClaimed);
        let result = snapshot(status, "alreadyClaimed", false, Decimal::ZERO);
        assert_eq!(result.state, "alreadyClaimed");
        assert_eq!(result.credit_earned, "0");
        assert!(result.today_checked_in);
    }

    #[test]
    fn inactive_status_accepts_missing_activity_counters() {
        let status = parse_status_response(r#"{"code":0,"data":{"active":false}}"#)
            .expect("inactive fixture should parse");

        assert_eq!(decide(&status), CheckinDecision::Inactive);
        assert_eq!(status.today_credit, Decimal::ZERO);
        assert_eq!(status.total_credits, Decimal::ZERO);
    }

    #[test]
    fn parses_claim_result_with_numeric_credit() {
        let result = parse_claim_response(
            r#"{"code":0,"data":{"credit":100,"streak_days":3,"is_streak_day":true}}"#,
        )
        .expect("claim fixture should parse");

        assert_eq!(result.credit, Decimal::new(100, 0));
        assert_eq!(result.streak_days, 3);
        assert!(result.is_streak_day);
    }

    #[test]
    fn parses_fractional_string_credit_without_losing_precision() {
        let result = parse_claim_response(
            r#"{"code":0,"data":{"credit":"0.100000000000000001","streak_days":1,"is_streak_day":false}}"#,
        )
        .expect("precise claim fixture should parse");

        assert_eq!(format_points(result.credit), "0.100000000000000001");
    }

    #[test]
    fn rejects_nonzero_codes_without_echoing_service_messages() {
        let body = r#"{"code":41001,"msg":"secret token and uid details"}"#;
        let status_error = parse_status_response(body).expect_err("status should fail");
        let claim_error = parse_claim_response(body).expect_err("claim should fail");

        assert_eq!(status_error, "签到服务返回错误（code 41001）");
        assert_eq!(claim_error, "签到服务返回错误（code 41001）");
        assert!(!status_error.contains("secret token"));
        assert!(!claim_error.contains("uid details"));
    }

    #[test]
    fn rejects_missing_or_invalid_required_fields_safely() {
        let missing_data =
            parse_status_response(r#"{"code":0}"#).expect_err("missing data should be rejected");
        let missing_field = parse_status_response(r#"{"code":0,"data":{"active":true}}"#)
            .expect_err("missing active fields should be rejected");
        let invalid_credit = parse_claim_response(
            r#"{"code":0,"data":{"credit":{"raw":"secret"},"streak_days":1,"is_streak_day":false}}"#,
        )
        .expect_err("invalid credit should be rejected");

        assert_eq!(missing_data, "签到服务响应缺少活动数据");
        assert!(missing_field.contains("today_checked_in"));
        assert_eq!(invalid_credit, "签到服务响应字段 credit 格式无效");
        assert!(!invalid_credit.contains("secret"));
    }

    #[test]
    fn malformed_json_does_not_echo_response_content() {
        let error = parse_status_response(r#"{"accessToken":"do-not-echo""#)
            .expect_err("malformed fixture should fail");

        assert_eq!(error, "签到服务返回了无法解析的数据");
        assert!(!error.contains("do-not-echo"));
    }
}
