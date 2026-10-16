use std::cmp::Ordering;
use std::time::Duration;

use reqwest::StatusCode;
use semver::Version;
use serde::{Deserialize, Serialize};

const RELEASES_FEED_URL: &str = "https://github.com/deloz/workbuddy-assistant/releases.atom";
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
struct ReleaseFeed {
    #[serde(default)]
    entry: Vec<ReleaseEntry>,
}

#[derive(Deserialize)]
struct ReleaseEntry {
    #[serde(default)]
    id: Option<String>,
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
        .get(RELEASES_FEED_URL)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "检查更新超时，请稍后重试".to_owned()
            } else {
                "暂时无法连接更新服务，请稍后重试".to_owned()
            }
        })?;

    ensure_success_status(response.status())?;

    let feed = response
        .text()
        .await
        .map_err(|_| "无法读取更新服务返回的数据，请稍后重试".to_owned())?;
    parse_release_feed(&feed, &current)
}

fn ensure_success_status(status: StatusCode) -> Result<(), String> {
    status
        .is_success()
        .then_some(())
        .ok_or_else(|| "更新服务暂时不可用，请稍后重试".to_owned())
}

fn parse_release_feed(feed: &str, current: &Version) -> Result<UpdateCheckSnapshot, String> {
    let feed: ReleaseFeed =
        quick_xml::de::from_str(feed).map_err(|_| "更新服务返回的数据无法识别".to_owned())?;
    let Some(entry) = feed.entry.first() else {
        return Ok(unpublished_snapshot(current));
    };
    let release_id = entry
        .id
        .as_deref()
        .ok_or_else(|| "更新服务返回的数据缺少版本信息".to_owned())?;
    let release_tag = release_tag_from_id(release_id)?;
    compare_release(current, release_tag)
}

fn release_tag_from_id(release_id: &str) -> Result<&str, String> {
    let (prefix, release_tag) = release_id
        .trim()
        .rsplit_once('/')
        .ok_or_else(|| "更新服务返回的数据缺少版本信息".to_owned())?;
    if !prefix.starts_with("tag:github.com,")
        || !prefix.contains(":Repository/")
        || release_tag.is_empty()
    {
        return Err("更新服务返回的数据缺少版本信息".to_owned());
    }
    Ok(release_tag)
}

fn unpublished_snapshot(current: &Version) -> UpdateCheckSnapshot {
    UpdateCheckSnapshot {
        status: UpdateCheckStatus::Unpublished,
        current_version: current.to_string(),
        latest_version: None,
        release_url: None,
    }
}

fn compare_release(current: &Version, release_tag: &str) -> Result<UpdateCheckSnapshot, String> {
    let latest =
        parse_version(release_tag).map_err(|_| "最新版本号格式无效，请稍后重试".to_owned())?;
    let status = match latest.cmp(current) {
        Ordering::Greater => UpdateCheckStatus::UpdateAvailable,
        Ordering::Equal => UpdateCheckStatus::UpToDate,
        Ordering::Less => UpdateCheckStatus::Ahead,
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

    fn release_feed(release_tag: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>tag:github.com,2008:Repository/1335752855/{release_tag}</id>
  </entry>
</feed>"#
        )
    }

    #[test]
    fn atom_feed_reports_newer_release() {
        let snapshot = parse_release_feed(&release_feed("v0.2.0"), &current_version()).unwrap();

        assert_eq!(snapshot.status, UpdateCheckStatus::UpdateAvailable);
        assert_eq!(snapshot.latest_version.as_deref(), Some("0.2.0"));
        assert_eq!(snapshot.release_url.as_deref(), Some(LATEST_RELEASE_URL));
    }

    #[test]
    fn atom_feed_accepts_matching_tag_without_v_prefix() {
        let snapshot = parse_release_feed(&release_feed("0.1.0"), &current_version()).unwrap();

        assert_eq!(snapshot.status, UpdateCheckStatus::UpToDate);
        assert!(snapshot.release_url.is_none());
    }

    #[test]
    fn atom_feed_marks_current_build_ahead_of_older_release() {
        let snapshot = parse_release_feed(&release_feed("v0.0.9"), &current_version()).unwrap();

        assert_eq!(snapshot.status, UpdateCheckStatus::Ahead);
        assert!(snapshot.release_url.is_none());
    }

    #[test]
    fn empty_atom_feed_maps_to_unpublished() {
        let snapshot = parse_release_feed(
            r#"<feed xmlns="http://www.w3.org/2005/Atom"></feed>"#,
            &current_version(),
        )
        .unwrap();

        assert_eq!(snapshot.status, UpdateCheckStatus::Unpublished);
        assert!(snapshot.latest_version.is_none());
        assert!(snapshot.release_url.is_none());
    }

    #[test]
    fn malformed_atom_xml_is_rejected() {
        let error = parse_release_feed("<feed><entry>", &current_version()).unwrap_err();

        assert_eq!(error, "更新服务返回的数据无法识别");
    }

    #[test]
    fn atom_entry_without_id_is_rejected() {
        let error = parse_release_feed(
            r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry /></feed>"#,
            &current_version(),
        )
        .unwrap_err();

        assert_eq!(error, "更新服务返回的数据缺少版本信息");
    }

    #[test]
    fn malformed_atom_entry_id_is_rejected() {
        let error = parse_release_feed(
            r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><id>v0.1.0</id></entry></feed>"#,
            &current_version(),
        )
        .unwrap_err();

        assert_eq!(error, "更新服务返回的数据缺少版本信息");
    }

    #[test]
    fn invalid_release_version_is_rejected() {
        let error = parse_release_feed(&release_feed("latest"), &current_version()).unwrap_err();

        assert_eq!(error, "最新版本号格式无效，请稍后重试");
    }

    #[test]
    fn unsuccessful_feed_response_is_rejected() {
        let error = ensure_success_status(StatusCode::FORBIDDEN).unwrap_err();

        assert_eq!(error, "更新服务暂时不可用，请稍后重试");
    }
}
