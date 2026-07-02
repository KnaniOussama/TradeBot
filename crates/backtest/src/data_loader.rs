//! Port of `tradebot/backtest/data_loader.py`: loads an OHLCV CSV (columns
//! `timestamp,open,high,low,close,volume`, UTC ISO timestamps) into candles,
//! sorted ascending by timestamp.
//!
//! The Python loader uses `pandas.read_csv` + `pd.to_datetime(..., utc=True)`,
//! which is lenient about timestamp formats (offset suffix, `Z`, naive
//! implicitly treated as UTC). This port accepts the same range of formats
//! the fixtures and tests use: RFC3339 with an explicit offset/`Z`, or a
//! naive `YYYY-MM-DDTHH:MM:SS`/`YYYY-MM-DD HH:MM:SS` timestamp assumed to
//! already be UTC.

use std::path::Path;

use chrono::{DateTime, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use tradebot_storage::Candle;

const REQUIRED_COLS: [&str; 6] = ["timestamp", "open", "high", "low", "close", "volume"];

/// Errors raised while loading a backtest OHLCV CSV. Mirrors
/// `DataLoaderError` in data_loader.py.
#[derive(Debug, Clone, thiserror::Error)]
pub enum DataLoaderError {
    #[error("missing required columns: {0:?}")]
    MissingColumns(Vec<String>),
    #[error("CSV has no data rows")]
    Empty,
    #[error("failed to read CSV: {0}")]
    Io(String),
    #[error("failed to parse CSV: {0}")]
    Parse(String),
}

fn parse_timestamp(s: &str) -> Result<DateTime<Utc>, DataLoaderError> {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Fall back to naive formats, assumed UTC (mirrors pandas' utc=True
    // localizing naive timestamps rather than rejecting them).
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
        }
    }
    Err(DataLoaderError::Parse(format!(
        "unrecognized timestamp: {s}"
    )))
}

fn parse_decimal(field: &str, name: &str) -> Result<Decimal, DataLoaderError> {
    field
        .trim()
        .parse::<Decimal>()
        .map_err(|e| DataLoaderError::Parse(format!("bad {name} value {field:?}: {e}")))
}

fn parse_rows(text: &str) -> Result<Vec<Candle>, DataLoaderError> {
    let mut lines = text.lines();
    let header_line = lines.next().ok_or(DataLoaderError::Empty)?;
    let headers: Vec<&str> = header_line.split(',').map(|s| s.trim()).collect();

    let mut missing: Vec<String> = REQUIRED_COLS
        .iter()
        .filter(|c| !headers.contains(c))
        .map(|c| c.to_string())
        .collect();
    if !missing.is_empty() {
        missing.sort();
        return Err(DataLoaderError::MissingColumns(missing));
    }

    let idx = |name: &str| {
        headers
            .iter()
            .position(|h| *h == name)
            .expect("checked above")
    };
    let (ti, oi, hi, li, ci, vi) = (
        idx("timestamp"),
        idx("open"),
        idx("high"),
        idx("low"),
        idx("close"),
        idx("volume"),
    );

    let mut candles = Vec::new();
    for line in lines {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        let needed = [ti, oi, hi, li, ci, vi].into_iter().max().unwrap_or(0);
        if fields.len() <= needed {
            return Err(DataLoaderError::Parse(format!(
                "row has too few columns: {line:?}"
            )));
        }
        candles.push(Candle {
            timestamp: parse_timestamp(fields[ti])?,
            open: parse_decimal(fields[oi], "open")?,
            high: parse_decimal(fields[hi], "high")?,
            low: parse_decimal(fields[li], "low")?,
            close: parse_decimal(fields[ci], "close")?,
            volume: parse_decimal(fields[vi], "volume")?,
        });
    }

    if candles.is_empty() {
        return Err(DataLoaderError::Empty);
    }
    candles.sort_by_key(|c| c.timestamp);
    Ok(candles)
}

/// Loads and validates an OHLCV CSV file, returning candles sorted ascending
/// by timestamp. Mirrors `load_csv_path` in data_loader.py.
pub fn load_csv_path(path: &Path) -> Result<Vec<Candle>, DataLoaderError> {
    let content = std::fs::read_to_string(path).map_err(|e| DataLoaderError::Io(e.to_string()))?;
    parse_rows(&content)
}

/// Loads and validates an OHLCV CSV from raw bytes. Mirrors `load_csv_bytes`
/// in data_loader.py.
pub fn load_csv_bytes(content: &[u8]) -> Result<Vec<Candle>, DataLoaderError> {
    let text = String::from_utf8_lossy(content);
    parse_rows(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "timestamp,open,high,low,close,volume\n\
        2026-05-01T00:00:00+00:00,100,101,99,100.5,1000\n\
        2026-05-01T00:01:00+00:00,100.5,102,100,101.5,1100\n";

    #[test]
    fn load_csv_path_reads_rows() {
        let dir = std::env::temp_dir();
        let path = dir.join("tradebot_backtest_test_load.csv");
        std::fs::write(&path, CSV).unwrap();
        let candles = load_csv_path(&path).unwrap();
        assert_eq!(candles.len(), 2);
        assert_eq!(candles.last().unwrap().close, Decimal::new(1015, 1));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_csv_bytes_roundtrips() {
        let candles = load_csv_bytes(CSV.as_bytes()).unwrap();
        assert_eq!(candles.len(), 2);
    }

    #[test]
    fn missing_columns_raises() {
        let bad = b"timestamp,close\n2026-05-01T00:00:00+00:00,100\n";
        let err = load_csv_bytes(bad).unwrap_err();
        assert!(matches!(err, DataLoaderError::MissingColumns(_)));
    }

    #[test]
    fn empty_csv_raises() {
        let err = load_csv_bytes(b"timestamp,open,high,low,close,volume\n").unwrap_err();
        assert!(matches!(err, DataLoaderError::Empty));
    }

    #[test]
    fn unsorted_timestamps_get_sorted() {
        let csv = "timestamp,open,high,low,close,volume\n\
            2026-05-01T00:01:00+00:00,100.5,102,100,101.5,1100\n\
            2026-05-01T00:00:00+00:00,100,101,99,100.5,1000\n";
        let candles = load_csv_bytes(csv.as_bytes()).unwrap();
        assert_eq!(candles[0].close, Decimal::new(1005, 1));
        assert_eq!(candles.last().unwrap().close, Decimal::new(1015, 1));
    }
}
