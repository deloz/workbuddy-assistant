use std::collections::HashSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::Client;

use super::signin::{self, SigninTokens};
use crate::data::workbuddy_signin::{SigninStore, StoredSignin};

/// 本机没有任何可用登录时的统一提示。
pub(crate) const NO_SIGNIN_MESSAGE: &str = "尚未登录，请先完成 WorkBuddy 登录";

/// Access Token 到期前多久开始续期。
const REFRESH_MARGIN_MS: i64 = 60_000;
const PERSONAL_TYPE: &str = "personal";
const ENTERPRISE_TYPE: &str = "enterprise";

/// 一个身份：同一个用户 ID 下的个人或某个企业。
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
    pub needs_signin: bool,
    identity: AccountIdentity,
}

impl AccountDescriptor {
    pub fn is_personal(&self) -> bool {
        self.account_type.eq_ignore_ascii_case(PERSONAL_TYPE)
    }

    pub fn same_identity(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

/// 一个账号（一个用户 ID）的登录凭据。
pub(crate) struct AccountCredential {
    pub uid: String,
    pub access_token: String,
    pub refresh_token: String,
    pub domain: Option<String>,
    pub enterprise_id: Option<String>,
}

pub(crate) struct AuthBundle {
    pub accounts: Vec<AccountDescriptor>,
    /// 每个用户 ID 一条；不可用（已过期且续期失败）的账号不在这里。
    pub credentials: Vec<AccountCredential>,
    pub current_account_id: Option<String>,
}

impl AuthBundle {
    pub fn account(&self, account_id: &str) -> Option<&AccountDescriptor> {
        self.accounts
            .iter()
            .find(|account| account.account_id == account_id)
    }

    pub fn credential_for(&self, uid: &str) -> Option<&AccountCredential> {
        self.credentials
            .iter()
            .find(|credential| credential.uid == uid)
    }
}

/// 读取本机保存的浏览器登录，必要时先续期，再组装成可用的账号清单。
pub(crate) async fn load_bundle(client: &Client, path: &Path) -> Result<AuthBundle, String> {
    let mut store = SigninStore::load(path.to_path_buf());
    if store.is_empty() {
        return Err(NO_SIGNIN_MESSAGE.to_owned());
    }

    let mut signins = store.signins().to_vec();
    let mut renewed = false;
    for signin in &mut signins {
        if !needs_refresh(signin) {
            continue;
        }
        let Ok(tokens) = signin::refresh(
            client,
            &signin.refresh_token,
            signin.domain.as_deref(),
            signin.enterprise_id.as_deref(),
        )
        .await
        else {
            continue;
        };
        apply_tokens(signin, &tokens);
        // 续期成功后顺带刷新身份列表；失败时保留上一次的身份。
        if let Ok(identities) = signin::fetch_identities(
            client,
            &signin.access_token,
            &signin.uid,
            signin.domain.as_deref(),
        )
        .await
            && !identities.is_empty()
        {
            signin.identities = identities;
        }
        renewed = true;
    }
    if renewed {
        store.replace(signins.clone());
        if let Err(error) = store.persist() {
            eprintln!("无法保存登录凭据：{error}");
        }
    }

    build_bundle(&signins)
}

pub(crate) fn build_bundle(signins: &[StoredSignin]) -> Result<AuthBundle, String> {
    let mut ordered: Vec<&StoredSignin> = signins
        .iter()
        .filter(|signin| !signin.uid.trim().is_empty())
        .collect();
    ordered.sort_by_key(|signin| std::cmp::Reverse(signin.added_at_ms));

    let mut accounts = Vec::new();
    let mut credentials = Vec::new();
    let mut seen = HashSet::new();
    let mut current_account_id = None;

    for (index, signin) in ordered.iter().enumerate() {
        let uid = signin.uid.trim().to_owned();
        let usable = credential_usable(signin);
        if usable {
            credentials.push(AccountCredential {
                uid: uid.clone(),
                access_token: signin.access_token.trim().to_owned(),
                refresh_token: signin.refresh_token.trim().to_owned(),
                domain: signin.domain.clone(),
                enterprise_id: signin.enterprise_id.clone(),
            });
        }

        let nickname = default_nickname(&signin.nickname, &uid);
        let is_current = index == 0;
        let login_identity = AccountIdentity {
            uid: uid.clone(),
            account_type: normalized_type(signin.enterprise_id.as_deref()).to_owned(),
            enterprise_id: normalized_enterprise_id(signin.enterprise_id.as_deref()),
        };
        if let Some(account_id) = push_account(
            &mut accounts,
            &mut seen,
            login_identity,
            &nickname,
            is_current,
            !usable,
        ) && is_current
        {
            current_account_id = Some(account_id);
        }

        for identity in &signin.identities {
            if identity.plugin_enabled == Some(false) {
                continue;
            }
            let candidate = AccountIdentity {
                uid: uid.clone(),
                account_type: normalized_type(identity.enterprise_id.as_deref()).to_owned(),
                enterprise_id: normalized_enterprise_id(identity.enterprise_id.as_deref()),
            };
            let nickname = identity
                .nickname
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map_or_else(|| nickname.clone(), str::to_owned);
            push_account(
                &mut accounts,
                &mut seen,
                candidate,
                &nickname,
                false,
                !usable,
            );
        }
    }

    if accounts.is_empty() {
        return Err(NO_SIGNIN_MESSAGE.to_owned());
    }

    Ok(AuthBundle {
        accounts,
        credentials,
        current_account_id,
    })
}

/// 把续期结果合并回登录记录；接口没返回的字段保留原值。
fn apply_tokens(signin: &mut StoredSignin, tokens: &SigninTokens) {
    signin.access_token.clone_from(&tokens.access_token);
    if !tokens.refresh_token.trim().is_empty() {
        signin.refresh_token.clone_from(&tokens.refresh_token);
    }
    if let Some(expires_at) = tokens.expires_at_ms {
        signin.expires_at_ms = Some(expires_at);
    }
    if let Some(refresh_expires_at) = tokens.refresh_expires_at_ms {
        signin.refresh_expires_at_ms = Some(refresh_expires_at);
    }
    if let Some(domain) = tokens.domain.clone() {
        signin.domain = Some(domain);
    }
    if let Some(token_type) = tokens.token_type.clone() {
        signin.token_type = Some(token_type);
    }
}

fn needs_refresh(signin: &StoredSignin) -> bool {
    signin
        .expires_at_ms
        .is_some_and(|expires_at| unix_now_ms() >= expires_at.saturating_sub(REFRESH_MARGIN_MS))
}

fn credential_usable(signin: &StoredSignin) -> bool {
    !signin.access_token.trim().is_empty()
        && signin
            .expires_at_ms
            .is_none_or(|expires_at| expires_at > unix_now_ms())
}

fn push_account(
    accounts: &mut Vec<AccountDescriptor>,
    seen: &mut HashSet<AccountIdentity>,
    identity: AccountIdentity,
    nickname: &str,
    is_current: bool,
    needs_signin: bool,
) -> Option<String> {
    if !seen.insert(identity.clone()) {
        return None;
    }
    let descriptor = descriptor_from(identity, nickname, is_current, needs_signin);
    let account_id = descriptor.account_id.clone();
    accounts.push(descriptor);
    Some(account_id)
}

fn descriptor_from(
    identity: AccountIdentity,
    nickname: &str,
    is_current: bool,
    needs_signin: bool,
) -> AccountDescriptor {
    let display_id = account_display_id(&identity);
    AccountDescriptor {
        account_id: opaque_account_id(&identity),
        masked_display_id: mask_identifier(&display_id),
        display_id,
        uid: identity.uid.clone(),
        account_type: identity.account_type.clone(),
        enterprise_id: identity.enterprise_id.clone(),
        is_current,
        needs_signin,
        nickname: nickname.to_owned(),
        identity,
    }
}

/// 只看企业标识：有企业标识就是企业身份，否则是个人身份（与 `WorkBuddy` 插件一致）。
fn normalized_type(enterprise_id: Option<&str>) -> &'static str {
    if normalized_enterprise_id(enterprise_id).is_some() {
        ENTERPRISE_TYPE
    } else {
        PERSONAL_TYPE
    }
}

fn normalized_enterprise_id(enterprise_id: Option<&str>) -> Option<String> {
    enterprise_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn default_nickname(nickname: &str, uid: &str) -> String {
    let nickname = nickname.trim();
    if nickname.is_empty() {
        uid.to_owned()
    } else {
        nickname.to_owned()
    }
}

fn account_display_id(identity: &AccountIdentity) -> String {
    if identity.account_type == PERSONAL_TYPE {
        return identity.uid.clone();
    }
    identity
        .enterprise_id
        .clone()
        .unwrap_or_else(|| identity.uid.clone())
}

fn opaque_account_id(identity: &AccountIdentity) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
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
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("account-{hash:016x}")
}

pub(crate) fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
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
    use crate::data::workbuddy_signin::StoredIdentity;

    const FAR_FUTURE_MS: i64 = 4_102_444_800_000;

    fn signin(uid: &str, added_at_ms: i64) -> StoredSignin {
        StoredSignin {
            uid: uid.to_owned(),
            nickname: format!("账号 {uid}"),
            account_type: PERSONAL_TYPE.to_owned(),
            enterprise_id: None,
            domain: Some("www.codebuddy.cn".to_owned()),
            token_type: Some("Bearer".to_owned()),
            access_token: format!("access-{uid}"),
            refresh_token: format!("refresh-{uid}"),
            expires_at_ms: Some(FAR_FUTURE_MS),
            refresh_expires_at_ms: None,
            added_at_ms,
            identities: Vec::new(),
        }
    }

    fn identity(account_type: &str, enterprise_id: Option<&str>) -> StoredIdentity {
        StoredIdentity {
            account_type: account_type.to_owned(),
            enterprise_id: enterprise_id.map(str::to_owned),
            nickname: None,
            plugin_enabled: Some(true),
        }
    }

    #[test]
    fn masks_identifiers_without_exposing_short_values() {
        assert_eq!(mask_identifier("15e8641234566945"), "15e864…6945");
        assert_eq!(mask_identifier("short"), "••••");
    }

    #[test]
    fn without_signins_the_bundle_is_a_login_request() {
        assert_eq!(build_bundle(&[]).err(), Some(NO_SIGNIN_MESSAGE.to_owned()));

        let blank = StoredSignin {
            uid: "  ".to_owned(),
            ..signin("ignored", 1)
        };
        assert_eq!(
            build_bundle(&[blank]).err(),
            Some(NO_SIGNIN_MESSAGE.to_owned())
        );
    }

    #[test]
    fn merges_the_login_identity_with_the_enumerated_ones() {
        let mut stored = signin("shared-user", 10);
        stored.account_type = "ultimate".to_owned();
        stored.enterprise_id = Some("enterprise-a".to_owned());
        stored.identities = vec![
            identity("ultimate", Some("enterprise-a")),
            identity("personal", None),
            identity("enterprise", Some("enterprise-b")),
        ];

        let bundle = build_bundle(&[stored]).expect("bundle should build");

        assert_eq!(bundle.accounts.len(), 3);
        assert!(bundle.accounts[0].is_current);
        assert_eq!(
            bundle.accounts[0].enterprise_id.as_deref(),
            Some("enterprise-a")
        );
        assert_eq!(bundle.accounts[0].display_id, "enterprise-a");
        assert!(bundle.accounts[1].is_personal());
        assert_eq!(bundle.accounts[1].display_id, "shared-user");
        assert_eq!(
            bundle.accounts[2].enterprise_id.as_deref(),
            Some("enterprise-b")
        );
        assert_eq!(
            bundle.current_account_id.as_deref(),
            Some(bundle.accounts[0].account_id.as_str())
        );
        for account in &bundle.accounts {
            assert!(!account.needs_signin);
        }
        assert_eq!(bundle.credentials.len(), 1);
        assert_eq!(bundle.credentials[0].uid, "shared-user");
    }

    #[test]
    fn keeps_one_entry_per_identity_and_hides_disabled_ones() {
        let mut stored = signin("shared-user", 10);
        stored.identities = vec![
            identity("enterprise", Some("enterprise-a")),
            identity("ultimate", Some("enterprise-a")),
            identity("personal", None),
            StoredIdentity {
                plugin_enabled: Some(false),
                ..identity("enterprise", Some("enterprise-disabled"))
            },
        ];

        let bundle = build_bundle(&[stored]).expect("bundle should build");

        assert_eq!(bundle.accounts.len(), 2);
        assert!(bundle.accounts[0].is_personal());
        assert!(bundle.accounts[0].is_current);
        assert_eq!(
            bundle.accounts[1].enterprise_id.as_deref(),
            Some("enterprise-a")
        );
        assert!(
            bundle
                .accounts
                .iter()
                .all(|account| account.enterprise_id.as_deref() != Some("enterprise-disabled"))
        );
    }

    #[test]
    fn multiple_signins_share_one_credential_each_and_current_is_the_newest() {
        let mut older = signin("user-old", 1);
        older.identities = vec![identity("personal", None)];
        let newer = signin("user-new", 5);

        let bundle = build_bundle(&[older, newer]).expect("bundle should build");

        assert_eq!(bundle.accounts.len(), 2);
        assert_eq!(bundle.accounts[0].uid, "user-new");
        assert!(bundle.accounts[0].is_current);
        assert!(!bundle.accounts[1].is_current);
        assert_eq!(bundle.credentials.len(), 2);
        assert!(bundle.credential_for("user-old").is_some());
        assert!(bundle.credential_for("user-new").is_some());
        assert!(bundle.credential_for("missing").is_none());
    }

    #[test]
    fn expired_signins_keep_their_accounts_but_ask_for_a_new_login() {
        let mut expired = signin("user-expired", 2);
        expired.expires_at_ms = Some(unix_now_ms() - 1);

        let bundle = build_bundle(&[expired]).expect("bundle should build");

        assert_eq!(bundle.accounts.len(), 1);
        assert!(bundle.accounts[0].needs_signin);
        assert!(bundle.credentials.is_empty());
    }

    #[test]
    fn apply_tokens_keeps_previous_values_when_the_response_omits_them() {
        let mut stored = signin("user-a", 1);
        apply_tokens(
            &mut stored,
            &SigninTokens {
                access_token: "new-access".to_owned(),
                refresh_token: String::new(),
                expires_at_ms: None,
                refresh_expires_at_ms: Some(4_102_444_800_001),
                domain: None,
                token_type: None,
            },
        );

        assert_eq!(stored.access_token, "new-access");
        assert_eq!(stored.refresh_token, "refresh-user-a");
        assert_eq!(stored.expires_at_ms, Some(FAR_FUTURE_MS));
        assert_eq!(stored.domain.as_deref(), Some("www.codebuddy.cn"));
        assert_eq!(stored.token_type.as_deref(), Some("Bearer"));
        assert_eq!(stored.refresh_expires_at_ms, Some(4_102_444_800_001));
    }

    #[test]
    fn only_signins_near_expiry_are_renewed() {
        let mut fresh = signin("user-fresh", 1);
        fresh.expires_at_ms = Some(unix_now_ms() + 10 * 60_000);
        assert!(!needs_refresh(&fresh));

        let mut almost = signin("user-almost", 1);
        almost.expires_at_ms = Some(unix_now_ms() + 30_000);
        assert!(needs_refresh(&almost));

        let mut unknown = signin("user-unknown", 1);
        unknown.expires_at_ms = None;
        assert!(!needs_refresh(&unknown));
    }
}
