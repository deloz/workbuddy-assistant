//! 一键自检：离线检查本机登录凭据，再探一次服务端连通性。
//!
//! 参考 workbuddy-auto-signin 的 `doctor`：不领取、不续期、不写文件，
//! 报告只出现昵称与脱敏标识，便于用户自助排错。

use chrono::Local;

use super::models::{DiagnosticItem, DiagnosticReport};
use super::runtime::DashboardRuntime;
use crate::data::workbuddy_signin::{SigninStore, StoredSignin};
use crate::platforms::workbuddy::auth::{self, mask_identifier};
use crate::platforms::workbuddy::{session, signin};

const STATUS_OK: &str = "ok";
const STATUS_WARN: &str = "warn";
const STATUS_FAIL: &str = "fail";

pub(crate) async fn run(runtime: &DashboardRuntime) -> DiagnosticReport {
    let signins = SigninStore::inspect(runtime.signin_path());
    let mut items = vec![check_file(&signins)];
    items.extend(check_accounts(&signins.unwrap_or_default()));
    items.push(check_reachability().await);

    let healthy = !items.iter().any(|item| item.status == STATUS_FAIL);
    DiagnosticReport {
        ran_at: Local::now().to_rfc3339(),
        healthy,
        items,
    }
}

/// 凭据文件是否可读；尚未登录只是提醒，损坏或版本不符才需要用户处理。
fn check_file(signins: &Result<Vec<StoredSignin>, std::io::Error>) -> DiagnosticItem {
    let label = "登录凭据文件".to_owned();
    match signins {
        Ok(list) if list.is_empty() => item(label, STATUS_WARN, "还没有登录记录，请先添加账号"),
        Ok(list) => item(
            label,
            STATUS_OK,
            &format!("可读，共 {} 条登录记录", list.len()),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            item(label, STATUS_WARN, "还没有登录凭据，添加账号后会自动生成")
        }
        Err(_) => item(label, STATUS_FAIL, "凭据文件损坏或版本不受支持，请重新登录"),
    }
}

/// 逐个用户检查 access token 与有效期，判定口径与自动续期完全一致。
fn check_accounts(signins: &[StoredSignin]) -> Vec<DiagnosticItem> {
    signins
        .iter()
        .map(|signin| {
            let label = account_label(signin);
            if signin.access_token.trim().is_empty() {
                return item(label, STATUS_FAIL, "缺少访问凭据，请重新登录");
            }
            if !auth::credential_usable(signin) {
                let detail = if signin.refresh_token.trim().is_empty() {
                    "访问凭据已过期，请重新登录"
                } else {
                    "访问凭据已过期，运行任务时会自动续期；续期失败需重新登录"
                };
                return item(label, STATUS_FAIL, detail);
            }
            if auth::needs_refresh(signin) {
                return item(label, STATUS_WARN, "即将到期，运行任务时会自动续期");
            }
            item(label, STATUS_OK, "凭据可用")
        })
        .collect()
}

/// 无鉴权探一次登录入口接口：拿得到即说明网络与服务端可达。
async fn check_reachability() -> DiagnosticItem {
    let label = "WorkBuddy 服务连通性".to_owned();
    let probe = async {
        let client = session::network_client()?;
        signin::begin(&client).await.map(|_| ())
    }
    .await;
    match probe {
        Ok(()) => item(label, STATUS_OK, "登录服务可达"),
        Err(detail) => item(label, STATUS_FAIL, detail.as_str()),
    }
}

fn account_label(signin: &StoredSignin) -> String {
    let nickname = signin.nickname.trim();
    let name = if nickname.is_empty() {
        signin.uid.trim()
    } else {
        nickname
    };
    format!("{name}（{}）", mask_identifier(&signin.uid))
}

fn item(label: String, status: &'static str, detail: &str) -> DiagnosticItem {
    DiagnosticItem {
        label,
        status,
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAR_FUTURE_MS: i64 = 4_102_444_800_000;

    fn signin(uid: &str) -> StoredSignin {
        StoredSignin {
            uid: uid.to_owned(),
            nickname: String::new(),
            account_type: "personal".to_owned(),
            enterprise_id: None,
            domain: None,
            token_type: None,
            access_token: format!("secret-access-{uid}"),
            refresh_token: format!("secret-refresh-{uid}"),
            expires_at_ms: Some(FAR_FUTURE_MS),
            refresh_expires_at_ms: None,
            added_at_ms: 1,
            identities: Vec::new(),
        }
    }

    fn io_error(kind: std::io::ErrorKind) -> Result<Vec<StoredSignin>, std::io::Error> {
        Err(std::io::Error::new(kind, "boom"))
    }

    #[test]
    fn missing_credential_file_asks_for_a_signin() {
        let item = check_file(&io_error(std::io::ErrorKind::NotFound));

        assert_eq!(item.status, STATUS_WARN);
        assert!(item.detail.contains("添加账号"));

        let damaged = check_file(&io_error(std::io::ErrorKind::InvalidData));
        assert_eq!(damaged.status, STATUS_FAIL);
        assert!(damaged.detail.contains("重新登录"));

        let empty = check_file(&Ok(Vec::new()));
        assert_eq!(empty.status, STATUS_WARN);
    }

    #[test]
    fn flags_missing_near_expiry_and_expired_credentials() {
        let mut blank = signin("15e8640000000001");
        blank.access_token = "  ".to_owned();
        let mut near = signin("15e8640000000002");
        near.expires_at_ms = Some(auth::unix_now_ms() + 30_000);
        let mut expired = signin("15e8640000000003");
        expired.expires_at_ms = Some(auth::unix_now_ms() - 1);
        expired.refresh_token = String::new();

        let items = check_accounts(&[blank, near, expired]);

        assert_eq!(items[0].status, STATUS_FAIL);
        assert!(items[0].detail.contains("缺少访问凭据"));
        assert_eq!(items[1].status, STATUS_WARN);
        assert!(items[1].detail.contains("自动续期"));
        assert_eq!(items[2].status, STATUS_FAIL);
        assert!(items[2].detail.contains("请重新登录"));
    }

    #[test]
    fn report_never_carries_credentials() {
        let mut usable = signin("15e8641234566945");
        usable.nickname = "小明".to_owned();
        let mut expired = signin("15e8649999888877");
        expired.expires_at_ms = Some(auth::unix_now_ms() - 1);

        let items = check_accounts(&[usable, expired]);
        let json = serde_json::to_string(&items).expect("items should serialize");

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].status, STATUS_OK);
        assert_eq!(items[1].status, STATUS_FAIL);
        assert!(items[0].label.starts_with("小明"));
        assert!(!json.contains("secret-access"), "报告不能带出访问凭据");
        assert!(!json.contains("secret-refresh"), "报告不能带出刷新凭据");
        assert!(!json.contains("15e8641234566945"), "报告只允许脱敏标识");
    }
}
