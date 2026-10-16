//! 本地 JSON 存档的公共读写：体积上限、版本校验与原子替换。
//!
//! 签到历史、成长历史、应用设置与登录凭据都按同一套约定落盘：
//! 先写同目录的 `.json.tmp`，删掉旧文件后再改名，避免中途崩溃留下半个文件。
//! 读取时若主文件缺失、损坏或版本不符，历史类存档会拿 `.json.tmp` 兜底，
//! 再失败才回退到默认值。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// 一份 JSON 存档的读取约束：体积上限、版本号与对应的错误文案。
#[derive(Clone, Copy)]
pub(crate) struct JsonFileSpec {
    pub(crate) max_bytes: u64,
    pub(crate) version: u8,
    pub(crate) oversize_message: &'static str,
    pub(crate) version_message: &'static str,
}

/// 带版本号的存档文档，读取时用它校验兼容性。
pub(crate) trait VersionedDocument {
    fn version(&self) -> u8;
}

/// 临时文件名：与目标文件同目录，保证 `rename` 不跨卷。
fn temporary_path(path: &Path) -> PathBuf {
    path.with_extension("json.tmp")
}

/// 原子写入 JSON：序列化 → 写临时文件 → 删除旧文件 → 改名。
pub(crate) fn write_json_atomic<T: Serialize>(path: &Path, document: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(document)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let temporary = temporary_path(path);
    fs::write(&temporary, bytes)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)
}

/// 读取 JSON 文档：先查大小上限，再解析，最后用 [`VersionedDocument`] 校验版本。
///
/// 文件缺失返回 [`io::ErrorKind::NotFound`]，超大、损坏或版本不符返回
/// [`io::ErrorKind::InvalidData`]；回退策略由调用方决定。
pub(crate) fn read_json_document<T>(path: &Path, spec: JsonFileSpec) -> io::Result<T>
where
    T: DeserializeOwned + VersionedDocument,
{
    if !path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "文件不存在"));
    }
    if fs::metadata(path)?.len() > spec.max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            spec.oversize_message,
        ));
    }
    let document: T = serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if document.version() != spec.version {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            spec.version_message,
        ));
    }
    Ok(document)
}

/// 读取会随时间累积的存档：主文件不可用时再读同目录的 `.json.tmp`，
/// 仍然不可用就回退到默认值（空历史），保证启动不被坏文件卡住。
pub(crate) fn load_json_or_default<T>(path: &Path, spec: JsonFileSpec) -> T
where
    T: DeserializeOwned + VersionedDocument + Default,
{
    read_json_document(path, spec)
        .or_else(|_| read_json_document(&temporary_path(path), spec))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde::Deserialize;

    use super::*;

    const FIXTURE_SPEC: JsonFileSpec = JsonFileSpec {
        max_bytes: 4096,
        version: 1,
        oversize_message: "文件过大",
        version_message: "版本不受支持",
    };
    const TINY_SPEC: JsonFileSpec = JsonFileSpec {
        max_bytes: 8,
        ..FIXTURE_SPEC
    };

    #[derive(Debug, Default, Deserialize, PartialEq, Serialize)]
    struct Fixture {
        version: u8,
        value: String,
    }

    impl VersionedDocument for Fixture {
        fn version(&self) -> u8 {
            self.version
        }
    }

    fn fixture(value: &str) -> Fixture {
        Fixture {
            version: 1,
            value: value.to_owned(),
        }
    }

    fn unique_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "workbuddy-file-store-{}-{nonce}",
                std::process::id()
            ))
            .join(name)
    }

    #[test]
    fn atomic_write_round_trips_and_leaves_no_temporary_file() {
        let path = unique_path("document.json");
        let parent = path.parent().unwrap().to_path_buf();

        write_json_atomic(&path, &fixture("first")).unwrap();
        write_json_atomic(&path, &fixture("second")).unwrap();

        let document: Fixture = read_json_document(&path, FIXTURE_SPEC).unwrap();
        assert_eq!(document.value, "second");
        assert!(!temporary_path(&path).exists());
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn missing_file_is_reported_so_callers_can_fall_back() {
        let path = unique_path("missing.json");

        let error = read_json_document::<Fixture>(&path, FIXTURE_SPEC)
            .expect_err("missing file should be an error");

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn oversize_and_unsupported_versions_are_rejected() {
        let path = unique_path("document.json");
        let parent = path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&parent).unwrap();
        fs::write(
            &path,
            br#"{"version":2,"value":"payload that is long enough to exceed the limit"}"#,
        )
        .unwrap();

        let oversize = read_json_document::<Fixture>(&path, TINY_SPEC)
            .expect_err("oversize file should be rejected");
        assert_eq!(oversize.to_string(), "文件过大");

        let mismatch = read_json_document::<Fixture>(&path, FIXTURE_SPEC)
            .expect_err("version mismatch should be rejected");
        assert_eq!(mismatch.to_string(), "版本不受支持");
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn load_recovers_the_document_left_in_the_temporary_file() {
        let path = unique_path("document.json");
        let parent = path.parent().unwrap().to_path_buf();

        write_json_atomic(&temporary_path(&path), &fixture("interrupted")).unwrap();
        fs::write(&path, b"{ not json").unwrap();

        let document: Fixture = load_json_or_default(&path, FIXTURE_SPEC);

        assert_eq!(document.value, "interrupted");
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn load_falls_back_to_the_default_when_nothing_is_readable() {
        let path = unique_path("document.json");

        let document: Fixture = load_json_or_default(&path, FIXTURE_SPEC);

        assert_eq!(document.value, "");
        assert_eq!(document.version, 0);
    }
}
