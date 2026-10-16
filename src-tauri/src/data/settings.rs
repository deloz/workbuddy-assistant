//! 应用设置：目前只有成长中心自动化开关；文件损坏时回落到默认值。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const SETTINGS_VERSION: u8 = 1;
const MAX_SETTINGS_FILE_BYTES: u64 = 64 * 1024;

fn default_growth_automation_enabled() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default = "default_growth_automation_enabled")]
    pub growth_automation_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            growth_automation_enabled: default_growth_automation_enabled(),
        }
    }
}

#[derive(Deserialize, Serialize)]
struct SettingsDocument {
    version: u8,
    #[serde(flatten)]
    settings: AppSettings,
}

pub struct SettingsStore {
    path: Option<PathBuf>,
    settings: AppSettings,
}

impl SettingsStore {
    pub fn load(path: PathBuf) -> Self {
        let settings = read_settings(&path).unwrap_or_default();
        Self {
            path: Some(path),
            settings,
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

    fn persist(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&SettingsDocument {
            version: SETTINGS_VERSION,
            settings: self.settings.clone(),
        })
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let temporary = temporary_path(path);
        fs::write(&temporary, bytes)?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(temporary, path)
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    path.with_extension("json.tmp")
}

fn read_settings(path: &Path) -> io::Result<AppSettings> {
    if !path.exists() {
        return Ok(AppSettings::default());
    }
    if fs::metadata(path)?.len() > MAX_SETTINGS_FILE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "设置文件过大"));
    }
    let document: SettingsDocument = serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if document.version != SETTINGS_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "不支持的设置版本",
        ));
    }
    Ok(document.settings)
}

#[cfg(test)]
mod tests {
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
    }

    #[test]
    fn malformed_file_falls_back_to_defaults() {
        let path = unique_path("settings.json");
        let parent = path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        fs::write(&path, br#"{"version":1,"growthAutomationEnabled":"yes""#).unwrap();

        let store = SettingsStore::load(path);

        assert!(store.settings().growth_automation_enabled);
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
}
