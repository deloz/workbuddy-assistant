use std::collections::HashSet;
use std::str::FromStr;

use chrono::{Duration, Local};
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::session::AccountSession;

const CREDIT_ENDPOINT: &str = "https://copilot.tencent.com/billing/meter/get-user-resource";
const ENTERPRISE_CREDIT_ENDPOINT: &str =
    "https://copilot.tencent.com/v2/billing/meter/get-enterprise-user-usage";
const PRODUCT_CODE: &str = "p_tcaca";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditSnapshot {
    pub queried_at: String,
    pub total_credits: String,
    pub used_credits: String,
    pub remaining_credits: String,
    pub resources: Vec<CreditResource>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditResource {
    pub package_name: String,
    pub status: String,
    pub total_credits: String,
    pub used_credits: String,
    pub remaining_credits: String,
    pub expires_at: Option<String>,
    pub refresh_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct CreditRequest {
    page_number: u32,
    page_size: u32,
    product_code: &'static str,
    status: [i32; 2],
    package_end_time_range_begin: String,
    package_end_time_range_end: String,
}

#[derive(Deserialize)]
struct CreditApiEnvelope {
    code: i32,
    #[serde(default)]
    data: Option<CreditApiData>,
}

#[derive(Deserialize)]
struct CreditApiData {
    #[serde(rename = "Response")]
    response: Option<CreditApiResponse>,
}

#[derive(Deserialize)]
struct CreditApiResponse {
    #[serde(rename = "Data")]
    data: Option<CreditApiDataBody>,
}

#[derive(Deserialize)]
struct CreditApiDataBody {
    #[serde(rename = "Accounts")]
    accounts: Option<Vec<CreditApiResource>>,
}

#[derive(Deserialize)]
struct EnterpriseEnvelope {
    #[serde(default)]
    code: Option<i32>,
    data: Option<Value>,
}

#[derive(Deserialize)]
struct CreditApiResource {
    #[serde(rename = "ResourceId")]
    resource_id: Option<Value>,
    #[serde(rename = "PackageName")]
    package_name: Option<String>,
    #[serde(rename = "Status")]
    status: Option<i32>,
    #[serde(rename = "CycleCapacitySizePrecise")]
    cycle_capacity_size: Option<Value>,
    #[serde(rename = "CycleCapacityRemainPrecise")]
    cycle_capacity_remain: Option<Value>,
    #[serde(rename = "DeductionEndTime")]
    deduction_end_time: Option<Value>,
    #[serde(rename = "ExpiredTime")]
    expired_time: Option<Value>,
    #[serde(rename = "CycleEndTime")]
    cycle_end_time: Option<Value>,
}

pub(crate) async fn fetch_credit_usage(
    client: &reqwest::Client,
    session: &AccountSession,
) -> Result<CreditSnapshot, String> {
    if session.is_enterprise() {
        return fetch_enterprise_credit(client, session).await;
    }

    let now = Local::now();
    let request = CreditRequest {
        page_number: 1,
        page_size: 100,
        product_code: PRODUCT_CODE,
        status: [0, 3],
        package_end_time_range_begin: format_api_time(now),
        package_end_time_range_end: format_api_time(now + Duration::days(365 * 101)),
    };
    let request_builder = session.authorize(client.post(CREDIT_ENDPOINT).json(&request));

    let response = request_builder
        .send()
        .await
        .map_err(|_| "无法连接额度服务，请检查网络后重试".to_owned())?;

    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        return Err("额度服务拒绝当前登录凭据，请在 WorkBuddy 重新登录后重试".to_owned());
    }
    if status == StatusCode::FORBIDDEN {
        return Err("当前账号没有读取额度的权限".to_owned());
    }
    if !status.is_success() {
        return Err(format!("额度服务暂时不可用（HTTP {status}）"));
    }

    let body = response
        .text()
        .await
        .map_err(|_| "无法读取额度服务响应".to_owned())?;
    parse_credit_response(&body, now.to_rfc3339())
}

async fn fetch_enterprise_credit(
    client: &reqwest::Client,
    session: &AccountSession,
) -> Result<CreditSnapshot, String> {
    let response = session
        .authorize(
            client
                .post(ENTERPRISE_CREDIT_ENDPOINT)
                .json(&serde_json::json!({})),
        )
        .send()
        .await
        .map_err(|_| "无法连接企业额度服务，请检查网络后重试".to_owned())?;

    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        return Err("企业额度服务拒绝当前登录凭据，请在 WorkBuddy 重新登录".to_owned());
    }
    if status == StatusCode::FORBIDDEN {
        return Err("当前账号没有读取企业额度的权限".to_owned());
    }
    if !status.is_success() {
        return Err(format!("企业额度服务暂时不可用（HTTP {status}）"));
    }

    let body = response
        .text()
        .await
        .map_err(|_| "无法读取企业额度服务响应".to_owned())?;
    parse_enterprise_response(&body, Local::now().to_rfc3339())
}

fn parse_enterprise_response(body: &str, queried_at: String) -> Result<CreditSnapshot, String> {
    let envelope: EnterpriseEnvelope =
        serde_json::from_str(body).map_err(|_| "企业额度服务返回了无法解析的数据".to_owned())?;
    if envelope.code.is_some_and(|code| code != 0) {
        return Err(format!(
            "企业额度服务返回错误（code {}）",
            envelope.code.unwrap_or_default()
        ));
    }

    let usage = envelope
        .data
        .as_ref()
        .and_then(|data| data.get("data").or(Some(data)));
    let limit = usage
        .and_then(|data| data.get("limitNum"))
        .map(|value| parse_amount(Some(value), "limitNum"))
        .transpose()?
        .unwrap_or(Decimal::ZERO);
    let mut used = usage
        .and_then(|data| data.get("credit"))
        .map(|value| parse_amount(Some(value), "credit"))
        .transpose()?
        .unwrap_or(Decimal::ZERO);
    if used < Decimal::ZERO {
        used = Decimal::ZERO;
    }
    let refresh_at = usage
        .and_then(|data| data.get("cycleResetTime"))
        .and_then(|value| optional_text(Some(value)));

    let (total, remaining) = if limit == Decimal::NEGATIVE_ONE {
        ("不限".to_owned(), "不限".to_owned())
    } else {
        let remaining = (limit - used).max(Decimal::ZERO);
        (format_amount(limit), format_amount(remaining))
    };
    let used = format_amount(used);

    Ok(CreditSnapshot {
        queried_at,
        total_credits: total.clone(),
        used_credits: used.clone(),
        remaining_credits: remaining.clone(),
        resources: vec![CreditResource {
            package_name: "企业额度".to_owned(),
            status: "valid".to_owned(),
            total_credits: total,
            used_credits: used,
            remaining_credits: remaining,
            expires_at: None,
            refresh_at,
        }],
    })
}

fn parse_credit_response(body: &str, queried_at: String) -> Result<CreditSnapshot, String> {
    let envelope: CreditApiEnvelope =
        serde_json::from_str(body).map_err(|_| "额度服务返回了无法解析的数据".to_owned())?;
    if envelope.code != 0 {
        return Err(format!("额度服务返回错误（code {}）", envelope.code));
    }

    let accounts = envelope
        .data
        .and_then(|data| data.response)
        .and_then(|response| response.data)
        .and_then(|data| data.accounts)
        .ok_or_else(|| "额度服务响应缺少资源列表".to_owned())?;

    let mut total_credits = Decimal::ZERO;
    let mut used_credits = Decimal::ZERO;
    let mut remaining_credits = Decimal::ZERO;
    let mut resources = Vec::with_capacity(accounts.len());
    let mut seen_resource_ids = HashSet::new();

    for account in accounts {
        if let Some(resource_id) = resource_id_key(account.resource_id.as_ref())
            && !seen_resource_ids.insert(resource_id)
        {
            continue;
        }

        let total = parse_amount(
            account.cycle_capacity_size.as_ref(),
            "CycleCapacitySizePrecise",
        )?;
        let remaining = parse_amount(
            account.cycle_capacity_remain.as_ref(),
            "CycleCapacityRemainPrecise",
        )?;
        let mut used = total - remaining;
        if used < Decimal::ZERO {
            used = Decimal::ZERO;
        }

        total_credits += total;
        used_credits += used;
        remaining_credits += remaining;
        resources.push(CreditResource {
            package_name: account
                .package_name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "未知套餐".to_owned()),
            status: status_label(account.status),
            total_credits: format_amount(total),
            used_credits: format_amount(used),
            remaining_credits: format_amount(remaining),
            expires_at: first_text(&[
                account.deduction_end_time.as_ref(),
                account.expired_time.as_ref(),
                account.cycle_end_time.as_ref(),
            ]),
            refresh_at: optional_text(account.cycle_end_time.as_ref()),
        });
    }

    Ok(CreditSnapshot {
        queried_at,
        total_credits: format_amount(total_credits),
        used_credits: format_amount(used_credits),
        remaining_credits: format_amount(remaining_credits),
        resources,
    })
}

fn parse_amount(value: Option<&Value>, field: &str) -> Result<Decimal, String> {
    let value = value.ok_or_else(|| format!("额度服务响应缺少字段 {field}"))?;
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => return Err(format!("额度字段 {field} 格式无效")),
    };

    Decimal::from_str(text.trim()).map_err(|_| format!("额度字段 {field} 格式无效"))
}

fn format_amount(amount: Decimal) -> String {
    amount.normalize().to_string()
}

fn resource_id_key(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn status_label(status: Option<i32>) -> String {
    match status {
        Some(0) => "valid".to_owned(),
        Some(3) => "usedUp".to_owned(),
        _ => "other".to_owned(),
    }
}

fn optional_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn first_text(values: &[Option<&Value>]) -> Option<String> {
    values.iter().find_map(|value| optional_text(*value))
}

fn format_api_time(time: chrono::DateTime<Local>) -> String {
    time.format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn success_fixture() -> &'static str {
        r#"{
            "code": 0,
            "data": {
                "Response": {
                    "Data": {
                        "Accounts": [
                            {
                                "PackageName": "月度额度",
                                "Status": 0,
                                "CycleCapacitySizePrecise": "2.50",
                                "CycleCapacityRemainPrecise": "1.25",
                                "DeductionEndTime": "2026-09-01 00:00:00",
                                "CycleEndTime": "2026-09-02 00:00:00"
                            },
                            {
                                "PackageName": "活动额度",
                                "Status": 3,
                                "CycleCapacitySizePrecise": 3,
                                "CycleCapacityRemainPrecise": 0,
                                "ExpiredTime": "2026-08-20 00:00:00"
                            }
                        ]
                    }
                }
            }
        }"#
    }

    #[test]
    fn parses_and_aggregates_precise_resource_amounts() {
        let snapshot = parse_credit_response(success_fixture(), "now".to_owned())
            .expect("fixture should parse");

        assert_eq!(snapshot.queried_at, "now");
        assert_eq!(snapshot.total_credits, "5.5");
        assert_eq!(snapshot.used_credits, "4.25");
        assert_eq!(snapshot.remaining_credits, "1.25");
        assert_eq!(snapshot.resources.len(), 2);
        assert_eq!(snapshot.resources[0].status, "valid");
        assert_eq!(snapshot.resources[1].status, "usedUp");
        assert_eq!(
            snapshot.resources[0].expires_at.as_deref(),
            Some("2026-09-01 00:00:00")
        );
    }

    #[test]
    fn empty_resource_list_returns_zero_values() {
        let body = r#"{"code":0,"data":{"Response":{"Data":{"Accounts":[]}}}}"#;
        let snapshot = parse_credit_response(body, "now".to_owned()).expect("fixture should parse");

        assert_eq!(snapshot.total_credits, "0");
        assert_eq!(snapshot.used_credits, "0");
        assert_eq!(snapshot.remaining_credits, "0");
        assert!(snapshot.resources.is_empty());
    }

    #[test]
    fn rejects_nonzero_api_code_without_echoing_response() {
        let body = r#"{"code":14018,"msg":"secret response details"}"#;
        let error = parse_credit_response(body, "now".to_owned()).expect_err("fixture should fail");

        assert_eq!(error, "额度服务返回错误（code 14018）");
        assert!(!error.contains("secret response details"));
    }

    #[test]
    fn rejects_missing_resource_fields() {
        let body = r#"{"code":0,"data":{"Response":{"Data":{"Accounts":[{"Status":0}]}}}}"#;
        let error = parse_credit_response(body, "now".to_owned()).expect_err("fixture should fail");

        assert!(error.contains("CycleCapacitySizePrecise"));
        assert!(!error.contains("accessToken"));
    }

    #[test]
    fn maps_unknown_status_and_uses_package_fallback() {
        let body = r#"{
            "code": 0,
            "data": {"Response": {"Data": {"Accounts": [{
                "Status": 9,
                "CycleCapacitySizePrecise": "1",
                "CycleCapacityRemainPrecise": "2"
            }]}}}
        }"#;
        let snapshot = parse_credit_response(body, "now".to_owned()).expect("fixture should parse");

        assert_eq!(snapshot.resources[0].status, "other");
        assert_eq!(snapshot.resources[0].package_name, "未知套餐");
        assert_eq!(snapshot.resources[0].used_credits, "0");
    }

    #[test]
    fn deduplicates_resources_by_resource_id() {
        let body = r#"{
            "code": 0,
            "data": {"Response": {"Data": {"Accounts": [
                {"ResourceId":"same","PackageName":"月度额度","Status":0,"CycleCapacitySizePrecise":"2","CycleCapacityRemainPrecise":"1"},
                {"ResourceId":"same","PackageName":"月度额度","Status":0,"CycleCapacitySizePrecise":"2","CycleCapacityRemainPrecise":"1"},
                {"ResourceId":"other","PackageName":"活动额度","Status":3,"CycleCapacitySizePrecise":"3","CycleCapacityRemainPrecise":"0"}
            ]}}}
        }"#;
        let snapshot = parse_credit_response(body, "now".to_owned()).expect("fixture should parse");

        assert_eq!(snapshot.resources.len(), 2);
        assert_eq!(snapshot.total_credits, "5");
        assert_eq!(snapshot.used_credits, "4");
        assert_eq!(snapshot.remaining_credits, "1");
    }

    #[test]
    fn rejects_null_amounts() {
        let body = r#"{"code":0,"data":{"Response":{"Data":{"Accounts":[{"CycleCapacitySizePrecise":null,"CycleCapacityRemainPrecise":"0"}]}}}}"#;
        let error = parse_credit_response(body, "now".to_owned()).expect_err("fixture should fail");

        assert!(error.contains("CycleCapacitySizePrecise"));
    }

    #[test]
    fn parses_enterprise_usage_into_single_resource() {
        let body = r#"{"code":0,"data":{"data":{"limitNum":1000,"credit":250,"cycleResetTime":"2026-09-01 00:00:00"}}}"#;

        let snapshot =
            parse_enterprise_response(body, "now".to_owned()).expect("fixture should parse");

        assert_eq!(snapshot.total_credits, "1000");
        assert_eq!(snapshot.used_credits, "250");
        assert_eq!(snapshot.remaining_credits, "750");
        assert_eq!(snapshot.resources.len(), 1);
        assert_eq!(snapshot.resources[0].package_name, "企业额度");
        assert_eq!(
            snapshot.resources[0].refresh_at.as_deref(),
            Some("2026-09-01 00:00:00")
        );
    }

    #[test]
    fn maps_unlimited_enterprise_usage_without_losing_used_amount() {
        let body = r#"{"code":0,"data":{"limitNum":-1,"credit":"12.5"}}"#;

        let snapshot =
            parse_enterprise_response(body, "now".to_owned()).expect("fixture should parse");

        assert_eq!(snapshot.total_credits, "不限");
        assert_eq!(snapshot.used_credits, "12.5");
        assert_eq!(snapshot.remaining_credits, "不限");
    }

    #[test]
    fn enterprise_errors_do_not_echo_service_messages() {
        let body = r#"{"code":10000,"message":"secret service details"}"#;

        let error =
            parse_enterprise_response(body, "now".to_owned()).expect_err("fixture should fail");

        assert_eq!(error, "企业额度服务返回错误（code 10000）");
        assert!(!error.contains("secret service details"));
    }
}
