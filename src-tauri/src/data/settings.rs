//! 应用设置：成长中心自动化开关与开机启动；文件损坏时回落到默认值。

use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::data::file_store::{
    JsonFileSpec, VersionedDocument, read_json_document, write_json_atomic,
};

const SETTINGS_FILE: JsonFileSpec = JsonFileSpec {
    max_bytes: 64 * 1024,
    version: 1,
    oversize_message: "设置文件过大",
    version_message: "不支持的设置版本",
};

fn default_growth_automation_enabled() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default = "default_growth_automation_enabled")]
    pub growth_automation_enabled: bool,
    /// 开机启动的用户意图；`None` 表示从未在本机应用过（新装或老版本升级），按默认启用处理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_at_startup: Option<bool>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            growth_automation_enabled: default_growth_automation_enabled(),
            launch_at_startup: None,
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
struct SettingsDocument {
    version: u8,
    #[serde(flatten)]
    settings: AppSettings,
}

impl VersionedDocument for SettingsDocument {
    fn version(&self) -> u8 {
        self.version
    }
}

pub struct SettingsStore {
    path: Option<PathBuf>,
    settings: AppSettings,
}

impl SettingsStore {
    pub fn load(path: PathBuf) -> Self {
        let document: SettingsDocument =
            read_json_document(&path, SETTINGS_FILE).unwrap_or_default();
        Self {
            path: Some(path),
            settings: document.settings,
        }
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn set_growth_automation_enabled(&mut self, enabled: bool) -> io::Result<AppSettings> {
        self.settings.growth_automation_enabled = enabled;
        self.persist()?;
        Ok(self.settings.clone())
    }

    /// 用户显式设置过的开机启动意图：`None` 表示还没在本机应用过。
    pub fn launch_at_startup_intent(&self) -> Option<bool> {
        self.settings.launch_at_startup
    }

    pub fn set_launch_at_startup(&mut self, enabled: bool) -> io::Result<AppSettings> {
        self.settings.launch_at_startup = Some(enabled);
        self.persist()?;
        Ok(self.settings.clone())
    }

    fn persist(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        write_json_atomic(
            path,
            &SettingsDocument {
                version: SETTINGS_FILE.version,
                settings: self.settings.clone(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn unique_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("workbuddy-settings-{}-{nonce}", std::process::id()))
            .join(name)
    }

    #[test]
    fn missing_file_keeps_growth_automation_on_by_default() {
        let path = unique_path("settings.json");

        let store = SettingsStore::load(path);

        assert!(store.settings().growth_automation_enabled);
        assert_eq!(store.launch_at_startup_intent(), None);
    }

    #[test]
    fn malformed_file_falls_back_to_defaults() {
        let path = unique_path("settings.json");
        let parent = path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        fs::write(&path, br#"{"version":1,"growthAutomationEnabled":"yes""#).unwrap();

        let store = SettingsStore::load(path);

        assert!(store.settings().growth_automation_enabled);
        assert_eq!(store.launch_at_startup_intent(), None);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn unsupported_version_falls_back_to_defaults() {
        let path = unique_path("settings.json");
        let parent = path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        fs::write(&path, br#"{"version":99,"growthAutomationEnabled":false}"#).unwrap();

        let store = SettingsStore::load(path);

        assert!(store.settings().growth_automation_enabled);
        assert_eq!(store.launch_at_startup_intent(), None);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn toggling_round_trips_through_json_file() {
        let path = unique_path("settings.json");
        let parent = path.parent().unwrap().to_path_buf();
        let mut store = SettingsStore::load(path.clone());

        let updated = store.set_growth_automation_enabled(false).unwrap();

        assert!(!updated.growth_automation_enabled);
        let reloaded = SettingsStore::load(path);
        assert!(!reloaded.settings().growth_automation_enabled);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn settings_written_by_older_versions_leave_launch_intent_unset() {
        let path = unique_path("settings.json");
        let parent = path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        fs::write(&path, br#"{"version":1,"growthAutomationEnabled":false}"#).unwrap();

        let store = SettingsStore::load(path);

        assert!(!store.settings().growth_automation_enabled);
        assert_eq!(store.launch_at_startup_intent(), None);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn launch_intent_round_trips_through_json_file() {
        let path = unique_path("settings.json");
        let parent = path.parent().unwrap().to_path_buf();
        let mut store = SettingsStore::load(path.clone());

        store.set_launch_at_startup(false).unwrap();
        assert_eq!(
            SettingsStore::load(path.clone()).launch_at_startup_intent(),
            Some(false)
        );

        store.set_launch_at_startup(true).unwrap();
        assert_eq!(
            SettingsStore::load(path).launch_at_startup_intent(),
            Some(true)
        );
        fs::remove_dir_all(parent).unwrap();
    }
}
