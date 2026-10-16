use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::data::file_store::{
    JsonFileSpec, VersionedDocument, load_json_or_default, write_json_atomic,
};

const SIGNIN_FILE: JsonFileSpec = JsonFileSpec {
    max_bytes: 1024 * 1024,
    version: 1,
    oversize_message: "登录凭据文件过大",
    version_message: "登录凭据文件版本不受支持",
};

/// 一个账号的浏览器登录结果。凭据以明文 JSON 保存在应用数据目录，仅当前用户可读。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredSignin {
    pub uid: String,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub account_type: String,
    #[serde(default)]
    pub enterprise_id: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub expires_at_ms: Option<i64>,
    #[serde(default)]
    pub refresh_expires_at_ms: Option<i64>,
    pub added_at_ms: i64,
    #[serde(default)]
    pub identities: Vec<StoredIdentity>,
}

/// 账号的某个身份（个人或企业）；企业身份才有 `enterprise_id`。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredIdentity {
    #[serde(default)]
    pub account_type: String,
    #[serde(default)]
    pub enterprise_id: Option<String>,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub plugin_enabled: Option<bool>,
}

#[derive(Default, Deserialize, Serialize)]
struct SigninDocument {
    version: u8,
    #[serde(default)]
    signins: Vec<StoredSignin>,
}

impl VersionedDocument for SigninDocument {
    fn version(&self) -> u8 {
        self.version
    }
}

pub(crate) struct SigninStore {
    path: Option<PathBuf>,
    signins: Vec<StoredSignin>,
}

impl SigninStore {
    pub fn load(path: PathBuf) -> Self {
        let document = load_json_or_default::<SigninDocument>(&path, SIGNIN_FILE);
        Self {
            path: Some(path),
            signins: document.signins,
        }
    }

    pub fn signins(&self) -> &[StoredSignin] {
        &self.signins
    }

    pub fn is_empty(&self) -> bool {
        self.signins.is_empty()
    }

    /// 用整份列表覆盖（刷新令牌后回写）。
    pub fn replace(&mut self, signins: Vec<StoredSignin>) {
        self.signins = signins;
    }

    /// 同一个 uid 只保留一条登录（重新登录即覆盖）。
    pub fn upsert(&mut self, signin: StoredSignin) -> io::Result<()> {
        match self
            .signins
            .iter_mut()
            .find(|stored| stored.uid == signin.uid)
        {
            Some(stored) => *stored = signin,
            None => self.signins.push(signin),
        }
        self.persist()
    }

    pub fn remove(&mut self, uid: &str) -> io::Result<bool> {
        let before = self.signins.len();
        self.signins.retain(|stored| stored.uid != uid);
        if self.signins.len() == before {
            return Ok(false);
        }
        self.persist()?;
        Ok(true)
    }

    pub fn persist(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        write_json_atomic(
            path,
            &SigninDocument {
                version: SIGNIN_FILE.version,
                signins: self.signins.clone(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn fixture_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "workbuddy-assistant-signin-{}-{nonce}-{name}",
            std::process::id()
        ))
    }

    fn signin(uid: &str, added_at_ms: i64) -> StoredSignin {
        StoredSignin {
            uid: uid.to_owned(),
            nickname: format!("账号 {uid}"),
            account_type: "personal".to_owned(),
            enterprise_id: None,
            domain: Some("www.codebuddy.cn".to_owned()),
            token_type: Some("Bearer".to_owned()),
            access_token: format!("access-{uid}"),
            refresh_token: format!("refresh-{uid}"),
            expires_at_ms: Some(4_102_444_800_000),
            refresh_expires_at_ms: None,
            added_at_ms,
            identities: vec![StoredIdentity {
                account_type: "personal".to_owned(),
                enterprise_id: None,
                nickname: None,
                plugin_enabled: Some(true),
            }],
        }
    }

    #[test]
    fn round_trips_multiple_signins() {
        let path = fixture_path("multi.json");
        let mut store = SigninStore::load(path.clone());
        store
            .upsert(signin("user-a", 1))
            .expect("write should work");
        store
            .upsert(signin("user-b", 2))
            .expect("write should work");

        let reloaded = SigninStore::load(path.clone());
        assert_eq!(reloaded.signins().len(), 2);
        assert_eq!(reloaded.signins()[0].uid, "user-a");
        assert_eq!(
            reloaded.signins()[1].access_token,
            "access-user-b".to_owned()
        );

        fs::remove_file(path).expect("fixture should be removable");
    }

    #[test]
    fn upsert_replaces_the_same_uid_and_remove_drops_it() {
        let path = fixture_path("replace.json");
        let mut store = SigninStore::load(path.clone());
        store
            .upsert(signin("user-a", 1))
            .expect("write should work");

        let mut renewed = signin("user-a", 5);
        renewed.access_token = "access-renewed".to_owned();
        store.upsert(renewed).expect("write should work");
        assert_eq!(store.signins().len(), 1);
        assert_eq!(store.signins()[0].added_at_ms, 5);

        assert!(store.remove("user-a").expect("remove should work"));
        assert!(store.is_empty());
        assert!(!store.remove("user-a").expect("remove should work"));
        assert!(SigninStore::load(path.clone()).is_empty());

        fs::remove_file(path).expect("fixture should be removable");
    }

    #[test]
    fn malformed_or_future_files_do_not_block_startup() {
        let path = fixture_path("broken.json");
        fs::write(&path, b"{ not json").expect("fixture should be writable");
        assert!(SigninStore::load(path.clone()).is_empty());

        fs::write(&path, br#"{"version":9,"signins":[]}"#).expect("fixture should be writable");
        assert!(SigninStore::load(path.clone()).is_empty());

        fs::remove_file(path).expect("fixture should be removable");
    }
}
