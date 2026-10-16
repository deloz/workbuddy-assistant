use std::time::Duration;

use reqwest::header::{AUTHORIZATION, HeaderValue};
use reqwest::{Client, Request, RequestBuilder, StatusCode, Url};
use serde::Deserialize;

use super::auth::{AccountCredential, AccountDescriptor, AuthBundle};

pub(crate) const API_BASE_URL: &str = "https://copilot.tencent.com";
const REQUEST_TIMEOUT_SECONDS: u64 = 15;
/// 该账号没有可用凭据（已过期且续期失败）时的提示。
const EXPIRED_SIGNIN_MESSAGE: &str = "该账号的登录已过期，请在设置中重新登录";

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

    /// 成长中心只按个人身份记账：带上企业头的请求会被服务端直接 403。
    pub fn personal_context(&self) -> Self {
        Self {
            uid: self.uid.clone(),
            access_token: self.access_token.clone(),
            domain: self.domain.clone(),
            enterprise_id: None,
        }
    }

    /// 只读联调用的构造入口，让测试能在不读取磁盘凭据的情况下复用请求封装。
    #[cfg(test)]
    pub(crate) fn for_test(uid: String, access_token: String, domain: Option<String>) -> Self {
        Self {
            uid,
            access_token,
            domain,
            enterprise_id: None,
        }
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
    let credentials = bundle
        .credential_for(&target.uid)
        .ok_or_else(|| EXPIRED_SIGNIN_MESSAGE.to_owned())?;
    if credentials.enterprise_id == target.enterprise_id {
        return Ok(current_session(credentials, target));
    }

    let request = build_switch_request(client, API_BASE_URL, credentials, target)?;
    let response = client.execute(request).await.map_err(|error| {
        if error.is_timeout() {
            "切换账号超时，请检查网络后重试".to_owned()
        } else {
            "无法连接账号服务，请检查网络后重试".to_owned()
        }
    })?;

    let status = response.status();
    let envelope = response.json::<SwitchEnvelope>().await.ok();
    // 浏览器登录拿到的令牌不一定被切换接口接受；插件本身也是“同一令牌 + 企业头”。
    // 被拒绝时按插件的方式直接带上该身份的企业头，而不是报错。
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return Ok(current_session(credentials, target));
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
        domain: temporary.domain.or_else(|| credentials.domain.clone()),
        enterprise_id: target.enterprise_id.clone(),
    })
}

fn current_session(credentials: &AccountCredential, account: &AccountDescriptor) -> AccountSession {
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
    current: &AccountCredential,
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
        .or(current.enterprise_id.as_ref())
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
            .map_err(|()| "账号服务地址无效".to_owned())?;
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
    use super::super::auth;
    use super::*;
    use crate::data::workbuddy_signin::{StoredIdentity, StoredSignin};

    const FAR_FUTURE_MS: i64 = 4_102_444_800_000;

    fn signin(uid: &str, nickname: &str, added_at_ms: i64) -> StoredSignin {
        StoredSignin {
            uid: uid.to_owned(),
            nickname: nickname.to_owned(),
            account_type: "personal".to_owned(),
            enterprise_id: None,
            domain: Some("example.invalid".to_owned()),
            token_type: Some("Bearer".to_owned()),
            access_token: "source-access-secret".to_owned(),
            refresh_token: "source-refresh-secret".to_owned(),
            expires_at_ms: Some(FAR_FUTURE_MS),
            refresh_expires_at_ms: None,
            added_at_ms,
            identities: Vec::new(),
        }
    }

    fn enterprise_identity(enterprise_id: &str, nickname: &str) -> StoredIdentity {
        StoredIdentity {
            account_type: "enterprise".to_owned(),
            enterprise_id: Some(enterprise_id.to_owned()),
            nickname: Some(nickname.to_owned()),
            plugin_enabled: Some(true),
        }
    }

    fn fixture_bundle() -> AuthBundle {
        let mut stored = signin("current-user", "当前账号", 10);
        stored.identities = vec![enterprise_identity("enterprise-42", "企业账号")];
        auth::build_bundle(&[stored]).expect("bundle should build")
    }

    fn enterprise_account(bundle: &AuthBundle) -> &AccountDescriptor {
        bundle
            .accounts
            .iter()
            .find(|account| !account.is_personal())
            .expect("enterprise account should exist")
    }

    fn current_account(bundle: &AuthBundle) -> &AccountDescriptor {
        bundle
            .accounts
            .iter()
            .find(|account| account.is_current)
            .expect("current account should exist")
    }

    #[test]
    fn builds_personal_switch_url_without_enterprise_headers() {
        let bundle = fixture_bundle();
        let current = current_account(&bundle);
        let credentials = bundle
            .credential_for(&current.uid)
            .expect("credentials should exist");
        let request = build_switch_request(
            &Client::new(),
            "https://example.invalid",
            credentials,
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
    }

    #[test]
    fn builds_enterprise_switch_url_and_required_headers() {
        let bundle = fixture_bundle();
        let enterprise = enterprise_account(&bundle);
        let credentials = bundle
            .credential_for(&enterprise.uid)
            .expect("credentials should exist");
        let request = build_switch_request(
            &Client::new(),
            "https://example.invalid/base/",
            credentials,
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
    }

    #[test]
    fn switching_from_enterprise_to_personal_keeps_current_context_headers() {
        let mut stored = signin("shared-user", "企业当前", 10);
        stored.account_type = "ultimate".to_owned();
        stored.enterprise_id = Some("enterprise-current".to_owned());
        stored.identities = vec![StoredIdentity {
            account_type: "personal".to_owned(),
            enterprise_id: None,
            nickname: Some("个人账号".to_owned()),
            plugin_enabled: Some(true),
        }];
        let bundle = auth::build_bundle(&[stored]).expect("bundle should build");
        let current = current_account(&bundle);
        let personal = bundle
            .accounts
            .iter()
            .find(|account| account.is_personal())
            .expect("personal account should exist");
        let credentials = bundle
            .credential_for(&personal.uid)
            .expect("credentials should exist");

        let request = build_switch_request(
            &Client::new(),
            "https://example.invalid",
            credentials,
            personal,
        )
        .expect("request should build");

        assert!(!current.is_personal());
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
    }

    #[test]
    fn switch_errors_do_not_echo_credentials() {
        let bundle = fixture_bundle();
        let target = enterprise_account(&bundle);
        let credentials = AccountCredential {
            uid: "current-user".to_owned(),
            access_token: "source-access-secret".to_owned(),
            refresh_token: String::new(),
            domain: Some("example.invalid".to_owned()),
            enterprise_id: None,
        };

        let error = build_switch_request(
            &Client::new(),
            "https://example.invalid",
            &credentials,
            target,
        )
        .expect_err("missing refresh token should fail");

        assert!(error.contains("Refresh Token"));
        assert!(!error.contains("source-access-secret"));
    }

    #[test]
    fn accounts_without_credentials_ask_for_a_new_login() {
        let mut expired = signin("current-user", "当前账号", 10);
        expired.expires_at_ms = Some(auth::unix_now_ms() - 1);
        let bundle = auth::build_bundle(&[expired]).expect("bundle should build");
        let account = current_account(&bundle);

        let error =
            tauri::async_runtime::block_on(session_for_account(&Client::new(), &bundle, account))
                .err();

        assert_eq!(error.as_deref(), Some(EXPIRED_SIGNIN_MESSAGE));
    }
}
