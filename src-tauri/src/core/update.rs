use std::time::Duration;

use reqwest::StatusCode;
use semver::Version;
use serde::{Deserialize, Serialize};

const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/deloz/workbuddy-assistant/releases/latest";
pub const LATEST_RELEASE_URL: &str = "https://github.com/deloz/workbuddy-assistant/releases/latest";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateCheckStatus {
    UpdateAvailable,
    UpToDate,
    Ahead,
    Unpublished,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckSnapshot {
    pub status: UpdateCheckStatus,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub release_url: Option<String>,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

pub async fn check_for_updates(current_version: &str) -> Result<UpdateCheckSnapshot, String> {
    let current = parse_version(current_version)
        .map_err(|_| "当前应用版本格式无效，无法检查更新".to_owned())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("workbuddy-assistant/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "无法初始化更新检查".to_owned())?;
    let response = client
        .get(LATEST_RELEASE_API)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "检查更新超时，请稍后重试".to_owned()
            } else {
                "暂时无法连接更新服务，请稍后重试".to_owned()
            }
        })?;

    if let Some(snapshot) = snapshot_for_empty_release(response.status(), &current) {
        return Ok(snapshot);
    }
    if response.status() == StatusCode::FORBIDDEN {
        return Err("检查更新过于频繁，请稍后再试".to_owned());
    }
    if !response.status().is_success() {
        return Err("更新服务暂时不可用，请稍后重试".to_owned());
    }

    let release: GitHubRelease = response
        .json()
        .await
        .map_err(|_| "更新服务返回的数据无法识别".to_owned())?;
    compare_release(&current, &release.tag_name)
}

fn snapshot_for_empty_release(
    status: StatusCode,
    current: &Version,
) -> Option<UpdateCheckSnapshot> {
    (status == StatusCode::NOT_FOUND).then(|| UpdateCheckSnapshot {
        status: UpdateCheckStatus::Unpublished,
        current_version: current.to_string(),
        latest_version: None,
        release_url: None,
    })
}

fn compare_release(current: &Version, release_tag: &str) -> Result<UpdateCheckSnapshot, String> {
    let latest =
        parse_version(release_tag).map_err(|_| "最新版本号格式无效，请稍后重试".to_owned())?;
    let status = if latest > *current {
        UpdateCheckStatus::UpdateAvailable
    } else if latest == *current {
        UpdateCheckStatus::UpToDate
    } else {
        UpdateCheckStatus::Ahead
    };
    let release_url =
        (status == UpdateCheckStatus::UpdateAvailable).then(|| LATEST_RELEASE_URL.to_owned());

    Ok(UpdateCheckSnapshot {
        status,
        current_version: current.to_string(),
        latest_version: Some(latest.to_string()),
        release_url,
    })
}

fn parse_version(value: &str) -> Result<Version, semver::Error> {
    Version::parse(value.trim().trim_start_matches(['v', 'V']))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current_version() -> Version {
        Version::parse("0.1.0").unwrap()
    }

    #[test]
    fn newer_release_is_available() {
        let snapshot = compare_release(&current_version(), "v0.2.0").unwrap();

        assert_eq!(snapshot.status, UpdateCheckStatus::UpdateAvailable);
        assert_eq!(snapshot.latest_version.as_deref(), Some("0.2.0"));
        assert_eq!(snapshot.release_url.as_deref(), Some(LATEST_RELEASE_URL));
    }

    #[test]
    fn matching_release_is_up_to_date() {
        let snapshot = compare_release(&current_version(), "0.1.0").unwrap();

        assert_eq!(snapshot.status, UpdateCheckStatus::UpToDate);
        assert!(snapshot.release_url.is_none());
    }

    #[test]
    fn older_release_marks_current_build_ahead() {
        let snapshot = compare_release(&current_version(), "v0.0.9").unwrap();

        assert_eq!(snapshot.status, UpdateCheckStatus::Ahead);
        assert!(snapshot.release_url.is_none());
    }

    #[test]
    fn invalid_release_tag_is_rejected() {
        let error = compare_release(&current_version(), "latest").unwrap_err();

        assert_eq!(error, "最新版本号格式无效，请稍后重试");
    }

    #[test]
    fn github_not_found_maps_to_unpublished() {
        let snapshot = snapshot_for_empty_release(StatusCode::NOT_FOUND, &current_version())
            .expect("404 should have a snapshot");

        assert_eq!(snapshot.status, UpdateCheckStatus::Unpublished);
        assert!(snapshot.latest_version.is_none());
        assert!(snapshot.release_url.is_none());
    }

    #[test]
    fn non_not_found_status_is_not_treated_as_unpublished() {
        assert!(snapshot_for_empty_release(StatusCode::FORBIDDEN, &current_version()).is_none());
    }
}
