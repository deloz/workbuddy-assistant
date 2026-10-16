//! 成长中心执行历史：每个账号按时间保留最近若干条，文件损坏时回退为空。

use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::data::file_store::{
    JsonFileSpec, VersionedDocument, load_json_or_default, write_json_atomic,
};

const HISTORY_FILE: JsonFileSpec = JsonFileSpec {
    max_bytes: 1024 * 1024,
    version: 1,
    oversize_message: "成长中心历史文件过大",
    version_message: "不支持的成长中心历史版本",
};
const MAX_HISTORY_PER_ACCOUNT: usize = 90;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthHistoryEntry {
    pub local_date: String,
    pub ran_at: String,
    pub trigger: String,
    pub status: String,
    pub report: String,
    pub credits_gained: String,
    pub energy: Option<String>,
    pub needs_attention: bool,
    pub error: Option<String>,
}

/// 是否值得落盘：领到东西、出过问题或非轮询触发都记录，纯空跑的轮询跳过。
pub(crate) fn should_record(trigger: &str, credits_gained: &str, needs_attention: bool) -> bool {
    trigger != "poll" || needs_attention || credits_gained.trim() != "0"
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredGrowthEntry {
    account_id: String,
    #[serde(flatten)]
    entry: GrowthHistoryEntry,
}

#[derive(Default, Deserialize, Serialize)]
struct GrowthHistoryDocument {
    version: u8,
    entries: Vec<StoredGrowthEntry>,
}

impl VersionedDocument for GrowthHistoryDocument {
    fn version(&self) -> u8 {
        self.version
    }
}

#[derive(Default)]
pub struct GrowthHistoryStore {
    path: Option<PathBuf>,
    entries: Vec<StoredGrowthEntry>,
}

impl GrowthHistoryStore {
    pub fn load(path: PathBuf) -> Self {
        let document = load_json_or_default::<GrowthHistoryDocument>(&path, HISTORY_FILE);
        Self {
            path: Some(path),
            entries: document.entries,
        }
    }

    pub fn entries_for(&self, account_id: &str) -> Vec<GrowthHistoryEntry> {
        let mut entries = self
            .entries
            .iter()
            .filter(|stored| stored.account_id == account_id)
            .map(|stored| stored.entry.clone())
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            right
                .ran_at
                .cmp(&left.ran_at)
                .then_with(|| right.local_date.cmp(&left.local_date))
        });
        entries
    }

    pub fn record(&mut self, account_id: &str, entry: GrowthHistoryEntry) -> io::Result<()> {
        self.entries.push(StoredGrowthEntry {
            account_id: account_id.to_owned(),
            entry,
        });
        self.trim_account(account_id);
        self.persist()
    }

    fn trim_account(&mut self, account_id: &str) {
        let count = self
            .entries
            .iter()
            .filter(|stored| stored.account_id == account_id)
            .count();
        for _ in MAX_HISTORY_PER_ACCOUNT..count {
            let oldest = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, stored)| stored.account_id == account_id)
                .min_by(|(_, left), (_, right)| left.entry.ran_at.cmp(&right.entry.ran_at))
                .map(|(index, _)| index);
            if let Some(index) = oldest {
                self.entries.remove(index);
            }
        }
    }

    fn persist(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        write_json_atomic(
            path,
            &GrowthHistoryDocument {
                version: HISTORY_FILE.version,
                entries: self.entries.clone(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn entry(ran_at: &str, credits: &str) -> GrowthHistoryEntry {
        GrowthHistoryEntry {
            local_date: ran_at[..10].to_owned(),
            ran_at: ran_at.to_owned(),
            trigger: "manual".to_owned(),
            status: "ok".to_owned(),
            report: "领旅行礼物 +10 积分".to_owned(),
            credits_gained: credits.to_owned(),
            energy: Some("5".to_owned()),
            needs_attention: false,
            error: None,
        }
    }

    fn unique_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("workbuddy-growth-{}-{nonce}", std::process::id()))
            .join(name)
    }

    #[test]
    fn poll_runs_are_recorded_only_when_they_matter() {
        assert!(should_record("manual", "0", false));
        assert!(should_record("daily", "0", false));
        assert!(!should_record("poll", "0", false));
        assert!(should_record("poll", "0", true));
        assert!(should_record("poll", "10", false));
    }

    #[test]
    fn history_is_isolated_and_trimmed_per_account() {
        let mut store = GrowthHistoryStore::default();

        for offset in 0..=MAX_HISTORY_PER_ACCOUNT {
            let ran_at = format!("2026-08-{:02}T09:00:00+08:00", (offset % 28) + 1);
            store.record("account-a", entry(&ran_at, "10")).unwrap();
        }
        store
            .record("account-b", entry("2026-08-16T09:00:00+08:00", "5"))
            .unwrap();

        assert_eq!(
            store.entries_for("account-a").len(),
            MAX_HISTORY_PER_ACCOUNT
        );
        assert_eq!(store.entries_for("account-b").len(), 1);
    }

    #[test]
    fn newest_entries_come_first() {
        let mut store = GrowthHistoryStore::default();

        store
            .record("account-a", entry("2026-08-16T09:00:00+08:00", "10"))
            .unwrap();
        store
            .record("account-a", entry("2026-08-17T09:00:00+08:00", "20"))
            .unwrap();

        let history = store.entries_for("account-a");
        assert_eq!(history[0].credits_gained, "20");
        assert_eq!(history[1].credits_gained, "10");
    }

    #[test]
    fn history_round_trips_through_json_file() {
        let path = unique_path("growth-history.json");
        let parent = path.parent().unwrap().to_path_buf();
        let mut store = GrowthHistoryStore::load(path.clone());

        store
            .record("account-a", entry("2026-08-16T09:00:00+08:00", "10"))
            .unwrap();

        let loaded = GrowthHistoryStore::load(path);
        assert_eq!(loaded.entries_for("account-a").len(), 1);
        assert_eq!(
            loaded.entries_for("account-a")[0].energy.as_deref(),
            Some("5")
        );
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn malformed_history_does_not_block_startup() {
        let path = unique_path("growth-history.json");
        let parent = path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        fs::write(&path, br#"{"entries":"not-valid""#).unwrap();

        let loaded = GrowthHistoryStore::load(path);

        assert!(loaded.entries_for("account-a").is_empty());
        fs::remove_dir_all(parent).unwrap();
    }
}
