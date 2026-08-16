use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

const AUTH_RELATIVE_PATH: [&str; 5] = [
    "CodeBuddyExtension",
    "Data",
    "Public",
    "auth",
    "workbuddy-desktop.info",
];
const MAX_AUTH_FILE_SIZE: u64 = 1_048_576;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct AccountIdentity {
    uid: String,
    account_type: String,
    enterprise_id: Option<String>,
}

#[derive(Clone)]
pub(crate) struct AccountDescriptor {
    pub account_id: String,
    pub uid: String,
    pub nickname: String,
    pub display_id: String,
    pub masked_display_id: String,
    pub account_type: String,
    pub enterprise_id: Option<String>,
    pub is_current: bool,
    identity: AccountIdentity,
}

impl AccountDescriptor {
    pub fn is_personal(&self) -> bool {
        self.account_type.eq_ignore_ascii_case("personal")
    }

    pub fn same_identity(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

pub(crate) struct AuthCredentials {
    pub uid: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: Option<i64>,
    pub domain: Option<String>,
}

pub(crate) struct AuthBundle {
    pub credentials: AuthCredentials,
    pub accounts: Vec<AccountDescriptor>,
}

impl AuthBundle {
    pub fn account(&self, account_id: &str) -> Option<&AccountDescriptor> {
        self.accounts
            .iter()
            .find(|account| account.account_id == account_id)
    }

    pub fn current_account(&self) -> Option<&AccountDescriptor> {
        self.accounts.iter().find(|account| account.is_current)
    }
}

#[derive(Debug)]
pub enum AuthReadError {
    MissingLocalAppData,
    MissingAccountUid,
    Io(io::Error),
    TooLarge(u64),
    InvalidJson(serde_json::Error),
    MissingAuthObject,
    InvalidCredentials(String),
}

impl fmt::Display for AuthReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingLocalAppData => {
                write!(formatter, "无法确定 WorkBuddy 本机登录信息位置")
            }
            Self::MissingAccountUid => write!(formatter, "WorkBuddy 登录信息缺少当前用户 ID"),
            Self::Io(source) if source.kind() == io::ErrorKind::NotFound => write!(
                formatter,
                "未检测到 WorkBuddy 登录信息，请先打开 WorkBuddy 完成登录"
            ),
            Self::Io(_) => write!(formatter, "无法读取 WorkBuddy 本机登录信息"),
            Self::TooLarge(size) => write!(
                formatter,
                "WorkBuddy 登录信息文件异常（{size} 字节），已停止读取"
            ),
            Self::InvalidJson(_) => write!(formatter, "WorkBuddy 登录信息格式无效"),
            Self::MissingAuthObject => write!(formatter, "WorkBuddy 登录信息缺少认证数据"),
            Self::InvalidCredentials(message) => formatter.write_str(message),
        }
    }
}

impl Error for AuthReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(source) => Some(source),
            Self::InvalidJson(source) => Some(source),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAuthFile {
    account: Option<StoredAccount>,
    auth: Option<StoredAuth>,
    #[serde(default)]
    accounts: Vec<StoredAccount>,
    #[serde(default)]
    all_accounts: Vec<StoredAccount>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAccount {
    uid: Option<String>,
    nickname: Option<String>,
    #[serde(rename = "type")]
    edition: Option<String>,
    account_type: Option<String>,
    enterprise_id: Option<String>,
    plugin_enabled: Option<bool>,
    oneid_account_id: Option<String>,
    uin: Option<String>,
    last_login: Option<bool>,
    is_current_one_id_enterprise: Option<bool>,
    is_current_one_id_personal: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAuth {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    domain: Option<String>,
    expires_at: Option<i64>,
}

pub fn default_auth_path() -> Result<PathBuf, AuthReadError> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or(AuthReadError::MissingLocalAppData)?;

    Ok(AUTH_RELATIVE_PATH
        .iter()
        .fold(local_app_data, |path, segment| path.join(segment)))
}

fn read_stored_file(path: &Path) -> Result<StoredAuthFile, AuthReadError> {
    let metadata = fs::metadata(path).map_err(AuthReadError::Io)?;
    if metadata.len() > MAX_AUTH_FILE_SIZE {
        return Err(AuthReadError::TooLarge(metadata.len()));
    }

    let content = fs::read_to_string(path).map_err(AuthReadError::Io)?;
    serde_json::from_str(&content).map_err(AuthReadError::InvalidJson)
}

pub(crate) fn read_auth_bundle() -> Result<AuthBundle, String> {
    let path = default_auth_path().map_err(|error| error.to_string())?;
    read_auth_bundle_from(&path).map_err(|error| error.to_string())
}

pub(crate) fn read_auth_bundle_from(path: &Path) -> Result<AuthBundle, AuthReadError> {
    let stored = read_stored_file(path)?;
    let auth = stored.auth.ok_or(AuthReadError::MissingAuthObject)?;
    let current = stored.account.ok_or(AuthReadError::MissingAccountUid)?;
    let current_uid = non_empty(current.uid.clone()).ok_or(AuthReadError::MissingAccountUid)?;

    let credentials = AuthCredentials {
        uid: current_uid,
        access_token: auth.access_token,
        refresh_token: auth.refresh_token,
        expires_at_ms: auth.expires_at,
        domain: non_empty(auth.domain),
    };
    validate_credentials(&credentials).map_err(AuthReadError::InvalidCredentials)?;

    let mut switchable = stored.accounts;
    switchable.extend(stored.all_accounts);
    let accounts = build_account_catalog(current, switchable)?;

    Ok(AuthBundle {
        credentials,
        accounts,
    })
}

fn build_account_catalog(
    current: StoredAccount,
    switchable: Vec<StoredAccount>,
) -> Result<Vec<AccountDescriptor>, AuthReadError> {
    let current_match = switchable
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            match_score(&current, candidate).map(|score| (index, score))
        })
        .max_by_key(|(_, score)| *score)
        .map(|(index, _)| index);
    let current = current_match
        .and_then(|index| switchable.get(index))
        .map_or(current.clone(), |candidate| {
            merge_account(&current, candidate)
        });

    let current_descriptor = descriptor_from(current, true)?;
    let mut seen = HashSet::from([current_descriptor.identity.clone()]);
    let mut accounts = vec![current_descriptor];

    for stored in switchable {
        if stored.plugin_enabled != Some(true) {
            continue;
        }
        let descriptor = descriptor_from(stored, false)?;
        if seen.insert(descriptor.identity.clone()) {
            accounts.push(descriptor);
        }
    }

    Ok(accounts)
}

fn match_score(current: &StoredAccount, candidate: &StoredAccount) -> Option<u8> {
    if non_empty(current.uid.clone())? != non_empty(candidate.uid.clone())? {
        return None;
    }

    let current_type = normalized_account_type(current);
    let candidate_type = normalized_account_type(candidate);
    if current_type != candidate_type {
        return None;
    }

    let mut score = 1;
    if same_non_empty(&current.oneid_account_id, &candidate.oneid_account_id) {
        score += 8;
    }
    if same_non_empty(&current.uin, &candidate.uin) {
        score += 4;
    }
    if candidate.last_login == Some(true) {
        score += 2;
    }
    if candidate.is_current_one_id_enterprise == Some(true)
        || candidate.is_current_one_id_personal == Some(true)
    {
        score += 1;
    }
    Some(score)
}

fn same_non_empty(left: &Option<String>, right: &Option<String>) -> bool {
    match (non_empty(left.clone()), non_empty(right.clone())) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn merge_account(current: &StoredAccount, candidate: &StoredAccount) -> StoredAccount {
    StoredAccount {
        uid: current.uid.clone().or_else(|| candidate.uid.clone()),
        nickname: non_empty(current.nickname.clone()).or_else(|| candidate.nickname.clone()),
        edition: non_empty(current.edition.clone()).or_else(|| candidate.edition.clone()),
        account_type: non_empty(current.account_type.clone())
            .or_else(|| candidate.account_type.clone()),
        enterprise_id: non_empty(current.enterprise_id.clone())
            .or_else(|| candidate.enterprise_id.clone()),
        plugin_enabled: current.plugin_enabled.or(candidate.plugin_enabled),
        oneid_account_id: current
            .oneid_account_id
            .clone()
            .or_else(|| candidate.oneid_account_id.clone()),
        uin: current.uin.clone().or_else(|| candidate.uin.clone()),
        last_login: current.last_login.or(candidate.last_login),
        is_current_one_id_enterprise: current
            .is_current_one_id_enterprise
            .or(candidate.is_current_one_id_enterprise),
        is_current_one_id_personal: current
            .is_current_one_id_personal
            .or(candidate.is_current_one_id_personal),
    }
}

fn descriptor_from(
    stored: StoredAccount,
    is_current: bool,
) -> Result<AccountDescriptor, AuthReadError> {
    let account_type = normalized_account_type(&stored);
    let uid = non_empty(stored.uid.clone()).ok_or(AuthReadError::MissingAccountUid)?;
    let enterprise_id = non_empty(stored.enterprise_id.clone());
    let display_id = account_display_id(&stored, &account_type, &uid, enterprise_id.as_deref());
    let identity = AccountIdentity {
        uid: uid.clone(),
        account_type: account_type.clone(),
        enterprise_id: enterprise_id.clone(),
    };
    let account_id = opaque_account_id(&identity);

    Ok(AccountDescriptor {
        account_id,
        nickname: non_empty(stored.nickname).unwrap_or_else(|| "未命名账号".to_owned()),
        masked_display_id: mask_identifier(&display_id),
        display_id,
        uid,
        account_type,
        enterprise_id,
        is_current,
        identity,
    })
}

fn account_display_id(
    account: &StoredAccount,
    account_type: &str,
    uid: &str,
    enterprise_id: Option<&str>,
) -> String {
    if account_type.eq_ignore_ascii_case("personal") {
        return non_empty(account.uin.clone()).unwrap_or_else(|| uid.to_owned());
    }

    non_empty(account.oneid_account_id.clone())
        .or_else(|| enterprise_id.map(str::to_owned))
        .unwrap_or_else(|| uid.to_owned())
}

fn normalized_account_type(account: &StoredAccount) -> String {
    non_empty(account.edition.clone())
        .or_else(|| non_empty(account.account_type.clone()))
        .unwrap_or_else(|| "personal".to_owned())
        .to_ascii_lowercase()
}

fn opaque_account_id(identity: &AccountIdentity) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in identity
        .uid
        .bytes()
        .chain([0])
        .chain(identity.account_type.bytes())
        .chain([0])
        .chain(
            identity
                .enterprise_id
                .as_deref()
                .unwrap_or_default()
                .bytes(),
        )
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("account-{hash:016x}")
}

pub(crate) fn validate_credentials(credentials: &AuthCredentials) -> Result<(), String> {
    if credentials.access_token.trim().is_empty() {
        return Err("当前账号没有可用的 Access Token，请先在 WorkBuddy 登录".to_owned());
    }
    if credentials
        .expires_at_ms
        .is_some_and(|expires_at| expires_at <= unix_now_ms())
    {
        return Err("Access Token 已过期，请在 WorkBuddy 重新登录后重试".to_owned());
    }
    Ok(())
}

pub(crate) fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

pub(crate) fn mask_identifier(value: &str) -> String {
    let characters: Vec<char> = value.chars().collect();
    if characters.len() <= 8 {
        return "••••".to_owned();
    }

    let prefix: String = characters.iter().take(6).collect();
    let suffix: String = characters.iter().rev().take(4).rev().collect();
    format!("{prefix}…{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "workbuddy-assistant-auth-{}-{nonce}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn parses_current_personal_enterprise_and_ignores_disabled_accounts() {
        let path = fixture_path("catalog.info");
        let fixture = r#"{
            "account": {"uid":"shared-user","nickname":"企业当前","type":"ultimate","oneidAccountId":"enterprise-one"},
            "auth": {"accessToken":"secret-access","refreshToken":"secret-refresh","expiresAt":4102444800000},
            "accounts": [
                {"uid":"shared-user","uin":"personal-uin","nickname":"企业当前","type":"ultimate","enterpriseId":"enterprise-a","oneidAccountId":"enterprise-one","pluginEnabled":true},
                {"uid":"shared-user","uin":"personal-uin","nickname":"个人身份","type":"personal","pluginEnabled":true},
                {"uid":"disabled-user","nickname":"已禁用","type":"personal","pluginEnabled":false}
            ],
            "allAccounts": [
                {"uid":"shared-user","nickname":"企业重复","type":"ultimate","enterpriseId":"enterprise-a","pluginEnabled":true}
            ]
        }"#;
        fs::write(&path, fixture).expect("fixture should be writable");

        let bundle = read_auth_bundle_from(&path).expect("fixture should parse");

        assert_eq!(bundle.accounts.len(), 2);
        assert!(bundle.accounts[0].is_current);
        assert_eq!(
            bundle.accounts[0].enterprise_id.as_deref(),
            Some("enterprise-a")
        );
        assert!(!bundle.accounts[0].is_personal());
        assert!(bundle.accounts[1].is_personal());
        assert_ne!(bundle.accounts[0].account_id, bundle.accounts[1].account_id);
        assert_eq!(bundle.accounts[0].display_id, "enterprise-one");
        assert_eq!(bundle.accounts[1].display_id, "personal-uin");
        assert_ne!(bundle.accounts[0].display_id, bundle.accounts[1].display_id);
        assert!(
            bundle
                .accounts
                .iter()
                .all(|account| account.uid != "disabled-user")
        );
        assert_eq!(bundle.credentials.refresh_token, "secret-refresh");

        fs::remove_file(path).expect("fixture should be removable");
    }

    #[test]
    fn same_uid_enterprise_identities_remain_distinct() {
        let current = StoredAccount {
            uid: Some("same-uid".to_owned()),
            nickname: Some("企业甲".to_owned()),
            edition: Some("ultimate".to_owned()),
            enterprise_id: Some("enterprise-a".to_owned()),
            ..StoredAccount::default()
        };
        let switchable = vec![
            StoredAccount {
                uid: Some("same-uid".to_owned()),
                nickname: Some("企业甲".to_owned()),
                edition: Some("ultimate".to_owned()),
                enterprise_id: Some("enterprise-a".to_owned()),
                plugin_enabled: Some(true),
                ..StoredAccount::default()
            },
            StoredAccount {
                uid: Some("same-uid".to_owned()),
                nickname: Some("企业乙".to_owned()),
                edition: Some("ultimate".to_owned()),
                enterprise_id: Some("enterprise-b".to_owned()),
                plugin_enabled: Some(true),
                ..StoredAccount::default()
            },
        ];

        let accounts = build_account_catalog(current, switchable).expect("catalog should build");

        assert_eq!(accounts.len(), 2);
        assert!(!accounts[0].same_identity(&accounts[1]));
        assert_ne!(accounts[0].account_id, accounts[1].account_id);
    }

    #[test]
    fn invalid_json_error_never_echoes_contents() {
        let path = fixture_path("invalid.info");
        fs::write(&path, r#"{"accessToken":"do-not-echo""#).expect("fixture should be writable");

        let error = match read_auth_bundle_from(&path) {
            Ok(_) => panic!("fixture should fail"),
            Err(error) => error,
        };

        assert_eq!(error.to_string(), "WorkBuddy 登录信息格式无效");
        assert!(!error.to_string().contains("do-not-echo"));
        fs::remove_file(path).expect("fixture should be removable");
    }

    #[test]
    fn rejects_oversized_missing_and_expired_credentials() {
        let path = fixture_path("too-large.info");
        fs::write(&path, vec![b' '; MAX_AUTH_FILE_SIZE as usize + 1])
            .expect("fixture should be writable");
        assert!(matches!(
            read_auth_bundle_from(&path),
            Err(AuthReadError::TooLarge(_))
        ));
        fs::remove_file(path).expect("fixture should be removable");

        let missing = AuthCredentials {
            uid: "user-123".to_owned(),
            access_token: " ".to_owned(),
            refresh_token: String::new(),
            expires_at_ms: None,
            domain: None,
        };
        let missing_error = validate_credentials(&missing).expect_err("token should be required");
        assert!(missing_error.contains("Access Token"));
        assert!(!missing_error.contains("user-123"));

        let expired = AuthCredentials {
            uid: "user-123".to_owned(),
            access_token: "secret-access".to_owned(),
            refresh_token: "secret-refresh".to_owned(),
            expires_at_ms: Some(unix_now_ms() - 1),
            domain: None,
        };
        let expired_error = validate_credentials(&expired).expect_err("token should be expired");
        assert!(expired_error.contains("过期"));
        assert!(!expired_error.contains("secret-access"));
    }

    #[test]
    fn masks_identifiers_without_exposing_short_values() {
        assert_eq!(mask_identifier("15e8641234566945"), "15e864…6945");
        assert_eq!(mask_identifier("short"), "••••");
    }
}
