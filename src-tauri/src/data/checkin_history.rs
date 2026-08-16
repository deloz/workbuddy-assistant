use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const HISTORY_VERSION: u8 = 1;
const MAX_HISTORY_FILE_BYTES: u64 = 1024 * 1024;
const MAX_HISTORY_PER_ACCOUNT: usize = 90;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckinHistoryEntry {
    pub local_date: String,
    pub checked_at: String,
    pub status: String,
    pub credit_earned: Option<String>,
    pub streak_days: Option<u32>,
    pub total_credits: Option<String>,
    pub activity_name: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct CheckinHistoryOutcome {
    pub status: String,
    pub credit_earned: Option<String>,
    pub streak_days: u32,
    pub total_credits: String,
    pub activity_name: Option<String>,
}

impl CheckinHistoryEntry {
    pub(crate) fn success(
        local_date: String,
        checked_at: String,
        outcome: CheckinHistoryOutcome,
    ) -> Self {
        Self {
            local_date,
            checked_at,
            status: outcome.status,
            credit_earned: outcome.credit_earned,
            streak_days: Some(outcome.streak_days),
            total_credits: Some(outcome.total_credits),
            activity_name: outcome.activity_name,
            error: None,
        }
    }

    pub(crate) fn failure(local_date: String, checked_at: String, error: String) -> Self {
        Self {
            local_date,
            checked_at,
            status: "error".to_owned(),
            credit_earned: None,
            streak_days: None,
            total_credits: None,
            activity_name: None,
            error: Some(error),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredHistoryEntry {
    account_id: String,
    #[serde(flatten)]
    entry: CheckinHistoryEntry,
}

#[derive(Deserialize, Serialize)]
struct HistoryDocument {
    version: u8,
    entries: Vec<StoredHistoryEntry>,
}

#[derive(Default)]
pub struct CheckinHistoryStore {
    path: Option<PathBuf>,
    entries: Vec<StoredHistoryEntry>,
}

impl CheckinHistoryStore {
    pub fn load(path: PathBuf) -> Self {
        let entries = read_entries(&path)
            .or_else(|_| read_entries(&temporary_path(&path)))
            .unwrap_or_default();
        Self {
            path: Some(path),
            entries,
        }
    }

    pub fn entries_for(&self, account_id: &str) -> Vec<CheckinHistoryEntry> {
        let mut entries = self
            .entries
            .iter()
            .filter(|stored| stored.account_id == account_id)
            .map(|stored| stored.entry.clone())
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            right
                .local_date
                .cmp(&left.local_date)
                .then_with(|| right.checked_at.cmp(&left.checked_at))
        });
        entries
    }

    pub fn record(&mut self, account_id: &str, entry: CheckinHistoryEntry) -> io::Result<()> {
        if !self.upsert(account_id, entry) {
            return Ok(());
        }
        self.persist()
    }

    fn upsert(&mut self, account_id: &str, entry: CheckinHistoryEntry) -> bool {
        if let Some(stored) = self.entries.iter_mut().find(|stored| {
            stored.account_id == account_id && stored.entry.local_date == entry.local_date
        }) {
            if status_priority(&entry.status) < status_priority(&stored.entry.status) {
                return false;
            }
            stored.entry = entry;
        } else {
            self.entries.push(StoredHistoryEntry {
                account_id: account_id.to_owned(),
                entry,
            });
        }

        self.trim_account(account_id);
        true
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
                .min_by(|(_, left), (_, right)| {
                    left.entry
                        .local_date
                        .cmp(&right.entry.local_date)
                        .then_with(|| left.entry.checked_at.cmp(&right.entry.checked_at))
                })
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
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&HistoryDocument {
            version: HISTORY_VERSION,
            entries: self.entries.clone(),
        })
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let temporary_path = temporary_path(path);
        fs::write(&temporary_path, bytes)?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(temporary_path, path)
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    path.with_extension("json.tmp")
}

fn read_entries(path: &Path) -> io::Result<Vec<StoredHistoryEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    if fs::metadata(path)?.len() > MAX_HISTORY_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "签到历史文件过大",
        ));
    }
    let document: HistoryDocument = serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if document.version != HISTORY_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "不支持的签到历史版本",
        ));
    }
    Ok(document.entries)
}

fn status_priority(status: &str) -> u8 {
    match status {
        "claimedNow" => 3,
        "alreadyClaimed" => 2,
        "inactive" => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use chrono::{Duration, NaiveDate};

    use super::*;

    fn outcome(state: &str, claimed_now: bool, credit: &str) -> CheckinHistoryOutcome {
        CheckinHistoryOutcome {
            status: state.to_owned(),
            credit_earned: claimed_now.then(|| credit.to_owned()),
            streak_days: 3,
            total_credits: "300".to_owned(),
            activity_name: Some("本期：资料库".to_owned()),
        }
    }

    fn entry(
        date: &str,
        time: &str,
        result: &Result<CheckinHistoryOutcome, String>,
    ) -> CheckinHistoryEntry {
        match result {
            Ok(outcome) => {
                CheckinHistoryEntry::success(date.to_owned(), time.to_owned(), outcome.clone())
            }
            Err(error) => {
                CheckinHistoryEntry::failure(date.to_owned(), time.to_owned(), error.clone())
            }
        }
    }

    fn unique_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("workbuddy-history-{}-{nonce}", std::process::id()))
            .join(name)
    }

    #[test]
    fn successful_claim_is_not_overwritten_by_later_status_checks() {
        let mut store = CheckinHistoryStore::default();
        let claimed = Ok(outcome("claimedNow", true, "100"));
        let already = Ok(outcome("alreadyClaimed", false, "0"));

        store
            .record(
                "account-a",
                entry("2026-08-16", "2026-08-16T09:00:00+08:00", &claimed),
            )
            .unwrap();
        store
            .record(
                "account-a",
                entry("2026-08-16", "2026-08-16T10:00:00+08:00", &already),
            )
            .unwrap();

        let history = store.entries_for("account-a");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, "claimedNow");
        assert_eq!(history[0].credit_earned.as_deref(), Some("100"));
    }

    #[test]
    fn success_replaces_an_error_for_the_same_day() {
        let mut store = CheckinHistoryStore::default();
        let failure = Err("网络错误".to_owned());
        let success = Ok(outcome("alreadyClaimed", false, "0"));

        store
            .record(
                "account-a",
                entry("2026-08-16", "2026-08-16T08:00:00+08:00", &failure),
            )
            .unwrap();
        store
            .record(
                "account-a",
                entry("2026-08-16", "2026-08-16T09:00:00+08:00", &success),
            )
            .unwrap();

        let history = store.entries_for("account-a");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, "alreadyClaimed");
        assert!(history[0].error.is_none());
    }

    #[test]
    fn history_is_isolated_and_trimmed_per_account() {
        let mut store = CheckinHistoryStore::default();
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let result = Ok(outcome("alreadyClaimed", false, "0"));

        for offset in 0..=MAX_HISTORY_PER_ACCOUNT {
            let date = start + Duration::days(offset as i64);
            store
                .record(
                    "account-a",
                    entry(
                        &date.to_string(),
                        &format!("{date}T09:00:00+08:00"),
                        &result,
                    ),
                )
                .unwrap();
        }
        store
            .record(
                "account-b",
                entry("2026-01-01", "2026-01-01T09:00:00+08:00", &result),
            )
            .unwrap();

        let account_a = store.entries_for("account-a");
        assert_eq!(account_a.len(), MAX_HISTORY_PER_ACCOUNT);
        assert_eq!(account_a.last().unwrap().local_date, "2026-01-02");
        assert_eq!(store.entries_for("account-b").len(), 1);
    }

    #[test]
    fn history_round_trips_through_json_file() {
        let path = unique_path("checkin-history.json");
        let parent = path.parent().unwrap().to_path_buf();
        let result = Ok(outcome("claimedNow", true, "100"));
        let mut store = CheckinHistoryStore::load(path.clone());

        store
            .record(
                "account-a",
                entry("2026-08-16", "2026-08-16T09:00:00+08:00", &result),
            )
            .unwrap();

        let loaded = CheckinHistoryStore::load(path);
        assert_eq!(loaded.entries_for("account-a").len(), 1);
        assert_eq!(loaded.entries_for("account-a")[0].status, "claimedNow");
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn malformed_history_does_not_block_startup() {
        let path = unique_path("checkin-history.json");
        let parent = path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        fs::write(&path, br#"{"accountId":"not-valid""#).unwrap();

        let loaded = CheckinHistoryStore::load(path);

        assert!(loaded.entries_for("account-a").is_empty());
        fs::remove_dir_all(parent).unwrap();
    }
}
