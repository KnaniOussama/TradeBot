//! Atomic JSON write (temp file + fsync + rename) and read-or-default helpers.
//! Mirrors `tradebot/storage/atomic.py`.

use crate::error::StorageError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `data` to `path` atomically: serialize to a temp file in the same
/// directory, fsync it, then rename it over `path`. On any failure the temp
/// file is removed and no partial write is left at `path`.
pub fn atomic_write_json<T: Serialize>(path: &Path, data: &T) -> Result<(), StorageError> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir).map_err(|source| StorageError::Io {
        path: dir.to_path_buf(),
        source,
    })?;

    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("data.json");
    let unique = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(
        "{file_name}.{}.{}.{}.tmp",
        std::process::id(),
        nanos,
        unique
    );
    let tmp_path = dir.join(tmp_name);

    let json = serde_json::to_string_pretty(data).map_err(|source| StorageError::Serialize {
        path: path.to_path_buf(),
        source,
    })?;

    let write_result: std::io::Result<()> = (|| {
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(json.as_bytes())?;
        f.flush()?;
        f.sync_all()?;
        Ok(())
    })();

    if let Err(source) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(StorageError::Io {
            path: tmp_path,
            source,
        });
    }

    if let Err(source) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(StorageError::Io {
            path: path.to_path_buf(),
            source,
        });
    }

    Ok(())
}

/// Return the JSON parsed from `path`, or `default` if the file is missing,
/// unreadable, or fails to parse into `T`.
pub fn read_json_or_default<T>(path: &Path, default: T) -> T
where
    T: DeserializeOwned,
{
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return default,
    };
    serde_json::from_str(&content).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "tradebot_storage_atomic_test_{}_{}_{name}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        p
    }

    #[test]
    fn write_creates_file() {
        let dir = tmp_dir("create");
        let p = dir.join("x.json");
        let mut data = HashMap::new();
        data.insert("a", 1);
        atomic_write_json(&p, &data).unwrap();
        assert!(p.exists());
        let read: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(read, json!({"a": 1}));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_overwrites_safely() {
        let dir = tmp_dir("overwrite");
        let p = dir.join("x.json");
        let mut data = HashMap::new();
        data.insert("a", 1);
        atomic_write_json(&p, &data).unwrap();
        data.insert("a", 2);
        atomic_write_json(&p, &data).unwrap();
        let read: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(read, json!({"a": 2}));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_creates_parent_dirs() {
        let dir = tmp_dir("nested");
        let p = dir.join("deep").join("nested").join("x.json");
        atomic_write_json(&p, &vec![1, 2, 3]).unwrap();
        assert!(p.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_leaves_no_temp_file() {
        let dir = tmp_dir("notemp");
        let p = dir.join("x.json");
        atomic_write_json(&p, &vec![1, 2, 3]).unwrap();
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries, vec!["x.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_missing_returns_default() {
        let dir = tmp_dir("missing");
        let out: Vec<i32> = read_json_or_default(&dir.join("missing.json"), Vec::new());
        assert_eq!(out, Vec::<i32>::new());
    }

    #[test]
    fn read_existing_returns_parsed() {
        let dir = tmp_dir("existing");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.json");
        std::fs::write(&p, "[1,2,3]").unwrap();
        let out: Vec<i32> = read_json_or_default(&p, Vec::new());
        assert_eq!(out, vec![1, 2, 3]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_corrupted_returns_default() {
        let dir = tmp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bad.json");
        std::fs::write(&p, "not json").unwrap();
        let default: HashMap<String, String> = HashMap::from([("k".to_string(), "v".to_string())]);
        let out: HashMap<String, String> = read_json_or_default(&p, default.clone());
        assert_eq!(out, default);
        std::fs::remove_dir_all(&dir).ok();
    }
}
