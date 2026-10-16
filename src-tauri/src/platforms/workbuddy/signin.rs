use std::time::Duration;

use reqwest::header::{ACCEPT, USER_AGENT};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode, Url};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::time::{Instant, sleep};

use super::auth::unix_now_ms;
use super::session::API_BASE_URL;
use crate::data::workbuddy_signin::StoredIdentity;

/// WorkBuddy 客户端身份。登录页的主机由它决定（`WorkBuddy/*` 才返回桌面端那个登录页）。
const SIGNIN_USER_AGENT: &str = "WorkBuddy/5.5.6";
const PLATFORM: &str = "workbuddy";
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
/// 「还在登录中」的应答码：token 与账号各一个。
const TOKEN_PENDING_CODE: i32 = 11217;
const ACCOUNT_PENDING_CODE: i32 = 12151;

pub(crate) const LOGIN_TIMEOUT_MS: i64 = 300_000;

pub(crate) struct SigninStart {
    pub state: String,
    pub url: String,
}

pub(crate) struct SigninTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: Option<i64>,
    pub refresh_expires_at_ms: Option<i64>,
    pub domain: Option<String>,
    pub token_type: Option<String>,
}

pub(crate) struct SigninOutcome {
    pub tokens: SigninTokens,
    pub uid: String,
    pub nickname: String,
    pub account_type: String,
    pub enterprise_id: Option<String>,
    pub identities: Vec<StoredIdentity>,
}

#[derive(Deserialize)]
struct Envelope<T> {
    code: Option<i32>,
    msg: Option<String>,
    data: Option<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateData {
    #[serde(default)]
    state: String,
    #[serde(default)]
    auth_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenData {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    expires_at: Option<i64>,
    #[serde(default)]
    refresh_expires_in: Option<i64>,
    #[serde(default)]
    refresh_expires_at: Option<i64>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountData {
    #[serde(default)]
    uid: String,
    #[serde(default)]
    nickname: Option<String>,
    #[serde(default)]
    phone_number: Option<String>,
    #[serde(default, rename = "type")]
    account_type: String,
    #[serde(default)]
    enterprise_id: Option<String>,
}

#[derive(Deserialize)]
struct AccountsData {
    #[serde(default)]
    accounts: Vec<AccountEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountEntry {
    #[serde(default)]
    uid: String,
    #[serde(default)]
    nickname: Option<String>,
    #[serde(default, rename = "type")]
    account_type: String,
    #[serde(default)]
    enterprise_id: Option<String>,
    #[serde(default)]
    plugin_enabled: Option<bool>,
}

/// 取一次登录会话：返回插件登录接口给出的 state 与浏览器登录页地址。
pub(crate) async fn begin(client: &Client) -> Result<SigninStart, String> {
    let url = endpoint("/v2/plugin/auth/state", &[("platform", PLATFORM)])?;
    let response = request(client, Method::POST, url)
        .header("X-No-Authorization", "true")
        .header("X-No-User-Id", "true")
        .header("X-No-Enterprise-Id", "true")
        .header("X-No-Department-Info", "true")
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|_| "无法连接 WorkBuddy 登录服务，请检查网络后重试".to_owned())?;

    let (status, envelope) = read_envelope::<StateData>(response).await?;
    ensure_success(status, &envelope)?;
    let data = envelope
        .data
        .ok_or_else(|| "WorkBuddy 未返回登录入口".to_owned())?;
    if data.state.trim().is_empty() || data.auth_url.trim().is_empty() {
        return Err("WorkBuddy 未返回登录入口".to_owned());
    }

    Ok(SigninStart {
        state: data.state.clone(),
        url: login_url(&data.auth_url)?,
    })
}

/// 等用户在浏览器完成登录：先取凭据，再取账号与身份列表。
pub(crate) async fn await_login(client: &Client, state: &str) -> Result<SigninOutcome, String> {
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    let token_url = endpoint("/v2/plugin/auth/token", &[("state", state)])?;
    let tokens = poll(
        client,
        token_url,
        &[],
        TOKEN_PENDING_CODE,
        deadline,
        |data: &TokenData| !data.access_token.trim().is_empty(),
    )
    .await?
    .into_tokens();

    let domain = tokens.domain.clone();
    let account_url = endpoint("/v2/plugin/login/account", &[("state", state)])?;
    let headers = vec![
        ("Authorization", format!("Bearer {}", tokens.access_token)),
        ("X-Domain", domain_or_host(domain.as_deref())),
        ("X-No-User-Id", "true".to_owned()),
        ("X-No-Enterprise-Id", "true".to_owned()),
    ];
    let account = poll(
        client,
        account_url,
        &headers,
        ACCOUNT_PENDING_CODE,
        deadline,
        |data: &AccountData| !data.uid.trim().is_empty(),
    )
    .await?;

    let identities = fetch_identities(
        client,
        &tokens.access_token,
        &account.uid,
        domain.as_deref(),
    )
    .await
    .unwrap_or_default();
    let uid = account.uid.trim().to_owned();
    let nickname = text(account.nickname.as_deref())
        .or_else(|| text(account.phone_number.as_deref()))
        .unwrap_or_else(|| uid.clone());

    Ok(SigninOutcome {
        tokens,
        enterprise_id: resolve_enterprise_id(
            &account.account_type,
            account.enterprise_id.as_deref(),
            &identities,
        ),
        nickname,
        account_type: account.account_type.trim().to_ascii_lowercase(),
        uid,
        identities,
    })
}

/// 用刷新令牌续期；失败时调用方决定是否继续使用旧凭据。
pub(crate) async fn refresh(
    client: &Client,
    refresh_token: &str,
    domain: Option<&str>,
    enterprise_id: Option<&str>,
) -> Result<SigninTokens, String> {
    if refresh_token.trim().is_empty() {
        return Err("当前登录缺少刷新令牌，请重新登录".to_owned());
    }

    let mut request = request(
        client,
        Method::POST,
        endpoint("/v2/plugin/auth/token/refresh", &[])?,
    )
    .header("X-Refresh-Token", refresh_token)
    .header("X-Auth-Refresh-Source", "plugin")
    .header("X-Domain", domain_or_host(domain));
    if let Some(enterprise_id) = text(enterprise_id) {
        request = request
            .header("X-Enterprise-Id", enterprise_id.clone())
            .header("X-Tenant-Id", enterprise_id);
    }

    let response = request
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|_| "无法连接 WorkBuddy 登录服务，请检查网络后重试".to_owned())?;
    let (status, envelope) = read_envelope::<TokenData>(response).await?;
    ensure_success(status, &envelope)?;
    let data = envelope
        .data
        .ok_or_else(|| "WorkBuddy 未返回刷新后的登录凭据".to_owned())?;
    if data.access_token.trim().is_empty() {
        return Err("WorkBuddy 未返回刷新后的 Access Token".to_owned());
    }

    Ok(data.into_tokens())
}

/// 该账号在 WorkBuddy 上的全部身份；接口不可用时返回空列表，由调用方退化为单身份。
pub(crate) async fn fetch_identities(
    client: &Client,
    access_token: &str,
    uid: &str,
    domain: Option<&str>,
) -> Result<Vec<StoredIdentity>, String> {
    let response = request(client, Method::GET, endpoint("/v2/plugin/accounts", &[])?)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("X-Domain", domain_or_host(domain))
        .send()
        .await
        .map_err(|_| "无法连接 WorkBuddy 账号服务，请检查网络后重试".to_owned())?;
    let (status, envelope) = read_envelope::<AccountsData>(response).await?;
    ensure_success(status, &envelope)?;
    let data = envelope
        .data
        .ok_or_else(|| "WorkBuddy 未返回账号列表".to_owned())?;
    let uid = uid.trim();

    Ok(data
        .accounts
        .into_iter()
        .filter(|entry| entry.uid.trim() == uid)
        .map(|entry| StoredIdentity {
            account_type: entry.account_type.trim().to_ascii_lowercase(),
            enterprise_id: text(entry.enterprise_id.as_deref()),
            nickname: text(entry.nickname.as_deref()),
            plugin_enabled: entry.plugin_enabled,
        })
        .collect())
}

fn endpoint(path: &str, query: &[(&str, &str)]) -> Result<Url, String> {
    let mut url = Url::parse(API_BASE_URL).map_err(|_| "WorkBuddy 服务地址无效".to_owned())?;
    url.set_path(path);
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }
    Ok(url)
}

fn request(client: &Client, method: Method, url: Url) -> RequestBuilder {
    client
        .request(method, url)
        .header(USER_AGENT, SIGNIN_USER_AGENT)
        .header(ACCEPT, "application/json")
}

/// 登录页地址：与桌面端一致，补上版本与本次浏览器会话标识。
fn login_url(auth_url: &str) -> Result<String, String> {
    let mut url = Url::parse(auth_url).map_err(|_| "WorkBuddy 返回的登录地址无效".to_owned())?;
    if url.scheme() != "https" {
        return Err("WorkBuddy 返回的登录地址不是 https".to_owned());
    }
    url.query_pairs_mut()
        .append_pair("version", env!("CARGO_PKG_VERSION"))
        .append_pair("loginSessionId", &random_hex(32));
    Ok(url.into())
}

enum PollOutcome<T> {
    Ready(T),
    Pending,
    Fatal(String),
}

async fn poll<T, F>(
    client: &Client,
    url: Url,
    headers: &[(&str, String)],
    retry_code: i32,
    deadline: Instant,
    valid: F,
) -> Result<T, String>
where
    T: DeserializeOwned,
    F: Fn(&T) -> bool,
{
    loop {
        if Instant::now() >= deadline {
            return Err("登录等待超时（5 分钟），请重新发起登录".to_owned());
        }
        match poll_once(client, url.clone(), headers, retry_code, &valid).await {
            PollOutcome::Ready(data) => return Ok(data),
            PollOutcome::Pending => sleep(POLL_INTERVAL).await,
            PollOutcome::Fatal(message) => return Err(message),
        }
    }
}

async fn poll_once<T, F>(
    client: &Client,
    url: Url,
    headers: &[(&str, String)],
    retry_code: i32,
    valid: &F,
) -> PollOutcome<T>
where
    T: DeserializeOwned,
    F: Fn(&T) -> bool,
{
    let mut request = request(client, Method::GET, url);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) if error.is_timeout() => return PollOutcome::Pending,
        Err(_) => {
            return PollOutcome::Fatal("无法连接 WorkBuddy 登录服务，请检查网络后重试".to_owned());
        }
    };

    let status = response.status();
    let Ok(text) = response.text().await else {
        return PollOutcome::Pending;
    };
    let Ok(envelope) = serde_json::from_str::<Envelope<T>>(&text) else {
        return if status == StatusCode::OK {
            PollOutcome::Pending
        } else {
            PollOutcome::Fatal(format!(
                "WorkBuddy 登录服务返回了无法解析的数据（HTTP {status}）"
            ))
        };
    };

    if !status.is_success() {
        if matches!(
            status,
            StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
        ) || envelope.code == Some(retry_code)
        {
            return PollOutcome::Pending;
        }
        return PollOutcome::Fatal(format!(
            "WorkBuddy 登录失败（HTTP {status}{}）",
            code_suffix(&envelope)
        ));
    }
    if let Some(code) = envelope.code.filter(|code| *code != 0) {
        if code == retry_code {
            return PollOutcome::Pending;
        }
        return PollOutcome::Fatal(format!(
            "WorkBuddy 登录失败（{}）",
            message_of(&envelope.msg, code)
        ));
    }

    match envelope.data {
        Some(data) if valid(&data) => PollOutcome::Ready(data),
        _ => PollOutcome::Pending,
    }
}

async fn read_envelope<T: DeserializeOwned>(
    response: Response,
) -> Result<(StatusCode, Envelope<T>), String> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|_| "无法读取 WorkBuddy 响应".to_owned())?;
    let envelope = serde_json::from_str::<Envelope<T>>(&text)
        .map_err(|_| format!("WorkBuddy 登录服务返回了无法解析的数据（HTTP {status}）"))?;
    Ok((status, envelope))
}

fn ensure_success<T>(status: StatusCode, envelope: &Envelope<T>) -> Result<(), String> {
    if !status.is_success() {
        return Err(format!(
            "WorkBuddy 登录服务暂时不可用（HTTP {status}{}）",
            code_suffix(envelope)
        ));
    }
    if let Some(code) = envelope.code.filter(|code| *code != 0) {
        return Err(format!(
            "WorkBuddy 登录失败（{}）",
            message_of(&envelope.msg, code)
        ));
    }
    Ok(())
}

fn code_suffix<T>(envelope: &Envelope<T>) -> String {
    match envelope.code.filter(|code| *code != 0) {
        Some(code) => format!("，code {code}"),
        None => String::new(),
    }
}

fn message_of(msg: &Option<String>, code: i32) -> String {
    text(msg.as_deref()).unwrap_or_else(|| format!("code {code}"))
}

/// X-Domain 取凭据带回的域名，没有就用服务主机名（与插件一致）。
fn domain_or_host(domain: Option<&str>) -> String {
    text(domain)
        .or_else(|| {
            Url::parse(API_BASE_URL)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
        })
        .unwrap_or_default()
}

/// 登录结果没给企业标识时，按插件规则在该账号的身份列表里找同类型的唯一一条。
fn resolve_enterprise_id(
    account_type: &str,
    enterprise_id: Option<&str>,
    identities: &[StoredIdentity],
) -> Option<String> {
    if let Some(enterprise_id) = text(enterprise_id) {
        return Some(enterprise_id);
    }
    let kind = account_type.trim().to_ascii_lowercase();
    if kind.is_empty() || kind == "personal" {
        return None;
    }
    let mut matches = identities.iter().filter(|identity| {
        identity.account_type.trim().to_ascii_lowercase() == kind
            && identity.enterprise_id.is_some()
    });
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    first.enterprise_id.clone()
}

impl TokenData {
    fn into_tokens(self) -> SigninTokens {
        let now = unix_now_ms();
        SigninTokens {
            access_token: self.access_token.trim().to_owned(),
            refresh_token: self.refresh_token.trim().to_owned(),
            expires_at_ms: expiry(self.expires_at, self.expires_in, now),
            refresh_expires_at_ms: expiry(self.refresh_expires_at, self.refresh_expires_in, now),
            domain: text(self.domain.as_deref()),
            token_type: text(self.token_type.as_deref()),
        }
    }
}

/// 到期时间优先取毫秒时间戳，否则按「还有多少秒」换算。
fn expiry(at: Option<i64>, in_seconds: Option<i64>, now: i64) -> Option<i64> {
    match at {
        Some(value) if value > 0 => Some(value),
        _ => match in_seconds {
            Some(value) if value > 0 => Some(now + value.saturating_mul(1000)),
            _ => None,
        },
    }
}

fn text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// 32 位十六进制会话标识。不引入随机数依赖：`RandomState` 的哈希密钥来自操作系统的随机源。
pub(crate) fn random_hex(length: usize) -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let mut value = String::with_capacity(length);
    while value.len() < length {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_i64(unix_now_ms());
        value.push_str(&format!("{:016x}", hasher.finish()));
    }
    value.truncate(length);
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_url_keeps_the_page_and_adds_the_session_parameters() {
        let url = login_url(
            "https://www.workbuddy.cn/login?platform=workbuddy&state=f1eb5893-9e28-48f8-9161-6461f98d136c",
        )
        .expect("login url should build");

        assert!(url.starts_with("https://www.workbuddy.cn/login?platform=workbuddy"));
        assert!(url.contains("version="));
        assert!(url.contains("loginSessionId="));
        let parsed = Url::parse(&url).expect("login url should parse");
        let session = parsed
            .query_pairs()
            .find(|(key, _)| key == "loginSessionId")
            .map(|(_, value)| value.to_string())
            .expect("loginSessionId should be present");
        assert_eq!(session.len(), 32);
        assert!(
            session
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        );
    }

    #[test]
    fn login_url_rejects_non_https_and_garbage() {
        assert!(login_url("http://www.workbuddy.cn/login").is_err());
        assert!(login_url("not a url").is_err());
    }

    #[test]
    fn generated_sessions_differ() {
        assert_ne!(random_hex(32), random_hex(32));
    }

    #[test]
    fn tokens_use_milliseconds_else_seconds() {
        let data = TokenData {
            access_token: " access ".to_owned(),
            refresh_token: String::new(),
            expires_in: Some(3_600),
            expires_at: Some(1_700_000_000_000),
            refresh_expires_in: Some(60),
            refresh_expires_at: None,
            domain: Some(" www.codebuddy.cn ".to_owned()),
            token_type: Some("Bearer".to_owned()),
        };
        let tokens = data.into_tokens();

        assert_eq!(tokens.access_token, "access");
        assert_eq!(tokens.refresh_token, "");
        assert_eq!(tokens.expires_at_ms, Some(1_700_000_000_000));
        assert_eq!(tokens.domain.as_deref(), Some("www.codebuddy.cn"));
        assert!(tokens.refresh_expires_at_ms.is_some_and(|value| value > 0));
    }

    #[test]
    fn endpoint_keeps_the_query_encoded() {
        let url = endpoint("/v2/plugin/auth/token", &[("state", "a b&c")])
            .expect("endpoint should build");
        assert_eq!(
            url.as_str(),
            "https://copilot.tencent.com/v2/plugin/auth/token?state=a+b%26c"
        );
    }

    #[test]
    fn enterprise_id_is_only_taken_from_a_unique_match() {
        let identities = vec![
            identity("personal", None),
            identity("enterprise", Some("enterprise-42")),
        ];
        assert_eq!(
            resolve_enterprise_id("enterprise", None, &identities).as_deref(),
            Some("enterprise-42")
        );
        assert_eq!(resolve_enterprise_id("personal", None, &identities), None);
        assert_eq!(
            resolve_enterprise_id("enterprise", Some("enterprise-7"), &identities).as_deref(),
            Some("enterprise-7")
        );

        let ambiguous = vec![
            identity("enterprise", Some("enterprise-a")),
            identity("enterprise", Some("enterprise-b")),
        ];
        assert_eq!(resolve_enterprise_id("enterprise", None, &ambiguous), None);
    }

    fn identity(account_type: &str, enterprise_id: Option<&str>) -> StoredIdentity {
        StoredIdentity {
            account_type: account_type.to_owned(),
            enterprise_id: enterprise_id.map(str::to_owned),
            nickname: None,
            plugin_enabled: Some(true),
        }
    }
}
