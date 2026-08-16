use std::time::Duration;

use reqwest::header::{AUTHORIZATION, HeaderValue};
use reqwest::{Client, Request, RequestBuilder, StatusCode, Url};
use serde::Deserialize;

use super::auth::{AccountDescriptor, AuthBundle, AuthCredentials};

const API_BASE_URL: &str = "https://copilot.tencent.com";
const REQUEST_TIMEOUT_SECONDS: u64 = 15;

pub(crate) struct AccountSession {
    uid: String,
    access_token: String,
    domain: Option<String>,
    enterprise_id: Option<String>,
}

impl AccountSession {
    pub fn is_enterprise(&self) -> bool {
        self.enterprise_id.is_some()
    }

    pub fn authorize(&self, mut request: RequestBuilder) -> RequestBuilder {
        request = request
            .bearer_auth(&self.access_token)
            .header("X-User-Id", &self.uid);
        if let Some(domain) = &self.domain {
            request = request.header("X-Domain", domain);
        }
        if let Some(enterprise_id) = &self.enterprise_id {
            request = request
                .header("X-Enterprise-Id", enterprise_id)
                .header("X-Tenant-Id", enterprise_id);
        }
        request
    }
}

#[derive(Deserialize)]
struct SwitchEnvelope {
    #[serde(default)]
    code: Option<i32>,
    data: Option<TemporaryAuth>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TemporaryAuth {
    #[serde(default)]
    access_token: String,
    domain: Option<String>,
}

pub(crate) fn network_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .user_agent(concat!("workbuddy-assistant/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "无法初始化网络客户端".to_owned())
}

pub(crate) async fn session_for_account(
    client: &Client,
    bundle: &AuthBundle,
    target: &AccountDescriptor,
) -> Result<AccountSession, String> {
    let current = bundle
        .current_account()
        .ok_or_else(|| "WorkBuddy 登录信息缺少当前账号".to_owned())?;
    if current.same_identity(target) {
        return Ok(current_session(&bundle.credentials, target));
    }

    let request = build_switch_request(client, API_BASE_URL, &bundle.credentials, current, target)?;
    let response = client.execute(request).await.map_err(|error| {
        if error.is_timeout() {
            "切换账号超时，请检查网络后重试".to_owned()
        } else {
            "无法连接账号服务，请检查网络后重试".to_owned()
        }
    })?;

    let status = response.status();
    let envelope = response.json::<SwitchEnvelope>().await.ok();
    if status == StatusCode::UNAUTHORIZED {
        return Err("账号服务拒绝当前登录凭据，请在 WorkBuddy 重新登录".to_owned());
    }
    if status == StatusCode::FORBIDDEN {
        return Err("当前登录无法切换到该账号".to_owned());
    }
    if !status.is_success() {
        if let Some(code) = envelope.as_ref().and_then(|body| body.code) {
            return Err(format!("账号服务返回错误（HTTP {status}，code {code}）"));
        }
        return Err(format!("账号服务暂时不可用（HTTP {status}）"));
    }
    let envelope = envelope.ok_or_else(|| "账号服务返回了无法解析的数据".to_owned())?;
    if envelope.code.is_some_and(|code| code != 0) {
        return Err(format!(
            "账号服务返回错误（code {}）",
            envelope.code.unwrap_or_default()
        ));
    }
    let temporary = envelope
        .data
        .ok_or_else(|| "账号服务响应缺少临时会话".to_owned())?;
    if temporary.access_token.trim().is_empty() {
        return Err("账号服务响应缺少临时 Access Token".to_owned());
    }

    Ok(AccountSession {
        uid: target.uid.clone(),
        access_token: temporary.access_token,
        domain: temporary
            .domain
            .or_else(|| bundle.credentials.domain.clone()),
        enterprise_id: target.enterprise_id.clone(),
    })
}

fn current_session(credentials: &AuthCredentials, account: &AccountDescriptor) -> AccountSession {
    AccountSession {
        uid: credentials.uid.clone(),
        access_token: credentials.access_token.clone(),
        domain: credentials.domain.clone(),
        enterprise_id: account.enterprise_id.clone(),
    }
}

fn build_switch_request(
    client: &Client,
    base_url: &str,
    current: &AuthCredentials,
    current_account: &AccountDescriptor,
    target: &AccountDescriptor,
) -> Result<Request, String> {
    if current.refresh_token.trim().is_empty() {
        return Err("当前登录缺少 Refresh Token，无法切换账号".to_owned());
    }

    let endpoint = switch_endpoint(base_url, target)?;
    let mut request = client
        .post(endpoint)
        .header("Accept", "application/json")
        .header("X-User-Id", &current.uid)
        .header("X-Refresh-Token", &current.refresh_token)
        .header(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", current.access_token))
                .map_err(|_| "当前登录凭据格式无效".to_owned())?,
        )
        .json(&serde_json::json!({}));

    if let Some(domain) = &current.domain {
        request = request.header("X-Domain", domain);
    }
    if let Some(enterprise_id) = target
        .enterprise_id
        .as_ref()
        .or(current_account.enterprise_id.as_ref())
    {
        request = request
            .header("X-Enterprise-Id", enterprise_id)
            .header("X-Tenant-Id", enterprise_id);
    }

    request
        .build()
        .map_err(|_| "无法创建账号切换请求".to_owned())
}

fn switch_endpoint(base_url: &str, target: &AccountDescriptor) -> Result<Url, String> {
    let mut endpoint = Url::parse(base_url).map_err(|_| "账号服务地址无效".to_owned())?;
    {
        let mut segments = endpoint
            .path_segments_mut()
            .map_err(|_| "账号服务地址无效".to_owned())?;
        segments.pop_if_empty();
        segments.extend(["v2", "plugin", "login", "enterprise"]);
        if !target.is_personal() {
            let enterprise_id = target
                .enterprise_id
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "企业账号缺少企业标识，无法切换".to_owned())?;
            segments.push(enterprise_id);
        }
    }
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::super::auth;
    use super::*;

    fn fixture_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "workbuddy-assistant-session-{}-{nonce}-{name}",
            std::process::id()
        ))
    }

    fn fixture_bundle() -> (PathBuf, AuthBundle) {
        let path = fixture_path("accounts.info");
        let fixture = r#"{
            "account":{"uid":"current-user","nickname":"当前账号","type":"personal"},
            "auth":{"accessToken":"source-access-secret","refreshToken":"source-refresh-secret","domain":"example.invalid","expiresAt":4102444800000},
            "accounts":[
                {"uid":"current-user","nickname":"当前账号","type":"personal","pluginEnabled":true},
                {"uid":"enterprise-user","nickname":"企业账号","type":"ultimate","enterpriseId":"enterprise-42","pluginEnabled":true}
            ]
        }"#;
        fs::write(&path, fixture).expect("fixture should be writable");
        let bundle = auth::read_auth_bundle_from(&path).expect("fixture should parse");
        (path, bundle)
    }

    #[test]
    fn builds_personal_switch_url_without_enterprise_headers() {
        let (path, bundle) = fixture_bundle();
        let current = bundle
            .current_account()
            .expect("current account should exist");
        let request = build_switch_request(
            &Client::new(),
            "https://example.invalid",
            &bundle.credentials,
            current,
            current,
        )
        .expect("request should build");

        assert_eq!(
            request.url().as_str(),
            "https://example.invalid/v2/plugin/login/enterprise"
        );
        assert!(request.headers().get("X-Enterprise-Id").is_none());
        assert!(request.headers().get("X-Tenant-Id").is_none());
        assert_eq!(
            request
                .headers()
                .get("X-User-Id")
                .and_then(|value| value.to_str().ok()),
            Some("current-user")
        );
        assert_eq!(
            request
                .headers()
                .get("X-Refresh-Token")
                .and_then(|value| value.to_str().ok()),
            Some("source-refresh-secret")
        );

        fs::remove_file(path).expect("fixture should be removable");
    }

    #[test]
    fn builds_enterprise_switch_url_and_required_headers() {
        let (path, bundle) = fixture_bundle();
        let enterprise = bundle
            .accounts
            .iter()
            .find(|account| !account.is_personal())
            .expect("enterprise account should exist");
        let request = build_switch_request(
            &Client::new(),
            "https://example.invalid/base/",
            &bundle.credentials,
            bundle
                .current_account()
                .expect("current account should exist"),
            enterprise,
        )
        .expect("request should build");

        assert_eq!(
            request.url().as_str(),
            "https://example.invalid/base/v2/plugin/login/enterprise/enterprise-42"
        );
        for header in ["X-Enterprise-Id", "X-Tenant-Id"] {
            assert_eq!(
                request
                    .headers()
                    .get(header)
                    .and_then(|value| value.to_str().ok()),
                Some("enterprise-42")
            );
        }
        assert_eq!(
            request
                .headers()
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer source-access-secret")
        );

        fs::remove_file(path).expect("fixture should be removable");
    }

    #[test]
    fn switching_from_enterprise_to_personal_keeps_current_context_headers() {
        let path = fixture_path("enterprise-current.info");
        let fixture = r#"{
            "account":{"uid":"shared-user","nickname":"企业当前","type":"ultimate","oneidAccountId":"enterprise-one"},
            "auth":{"accessToken":"source-access","refreshToken":"source-refresh","expiresAt":4102444800000},
            "accounts":[
                {"uid":"shared-user","nickname":"企业当前","type":"ultimate","enterpriseId":"enterprise-current","oneidAccountId":"enterprise-one","pluginEnabled":true},
                {"uid":"shared-user","nickname":"个人账号","type":"personal","pluginEnabled":true}
            ]
        }"#;
        fs::write(&path, fixture).expect("fixture should be writable");
        let bundle = auth::read_auth_bundle_from(&path).expect("fixture should parse");
        let current = bundle
            .current_account()
            .expect("current account should exist");
        let personal = bundle
            .accounts
            .iter()
            .find(|account| account.is_personal())
            .expect("personal account should exist");

        let request = build_switch_request(
            &Client::new(),
            "https://example.invalid",
            &bundle.credentials,
            current,
            personal,
        )
        .expect("request should build");

        assert_eq!(
            request.url().as_str(),
            "https://example.invalid/v2/plugin/login/enterprise"
        );
        for header in ["X-Enterprise-Id", "X-Tenant-Id"] {
            assert_eq!(
                request
                    .headers()
                    .get(header)
                    .and_then(|value| value.to_str().ok()),
                Some("enterprise-current")
            );
        }
        fs::remove_file(path).expect("fixture should be removable");
    }

    #[test]
    fn switch_errors_do_not_echo_credentials() {
        let (path, bundle) = fixture_bundle();
        let current = bundle
            .current_account()
            .expect("current account should exist")
            .clone();
        let target = bundle.accounts.last().expect("target should exist").clone();
        let mut credentials = bundle.credentials;
        credentials.refresh_token.clear();

        let error = build_switch_request(
            &Client::new(),
            "https://example.invalid",
            &credentials,
            &current,
            &target,
        )
        .expect_err("missing refresh token should fail");

        assert!(error.contains("Refresh Token"));
        assert!(!error.contains("source-access-secret"));
        fs::remove_file(path).expect("fixture should be removable");
    }

    #[test]
    #[ignore = "requires a current local WorkBuddy login and live official services"]
    fn live_all_accounts_support_credit_and_idempotent_checkin() {
        tauri::async_runtime::block_on(async {
            let bundle = auth::read_auth_bundle().expect("local WorkBuddy login should be valid");
            let client = network_client().expect("network client should initialize");
            assert!(
                !bundle.accounts.is_empty(),
                "at least one account is required"
            );

            for account in &bundle.accounts {
                let session = session_for_account(&client, &bundle, account)
                    .await
                    .expect("account session should resolve");
                let credit =
                    crate::platforms::workbuddy::credit::fetch_credit_usage(&client, &session)
                        .await
                        .expect("account credit should load");
                assert!(!credit.queried_at.is_empty());

                let checkin = crate::platforms::workbuddy::checkin::auto_claim_daily_checkin(
                    &client, &session,
                )
                .await
                .expect("account check-in should complete");
                assert!(
                    matches!(
                        checkin.state.as_str(),
                        "inactive" | "alreadyClaimed" | "claimedNow"
                    ),
                    "check-in state should be recognized"
                );
            }
        });
    }
}
