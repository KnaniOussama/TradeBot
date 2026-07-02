//! OHLCV candle aggregator. Port of `tradebot/data/ohlcv.py`.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use tokio::sync::mpsc;
use tradebot_storage::{JsonStorage, OhlcvCandle, StorageError};

use crate::prices::PriceTick;

/// Bucket width in seconds for each supported timeframe. Matches
/// `TIMEFRAME_SECONDS` in `tradebot/data/ohlcv.py`.
pub fn timeframe_seconds(timeframe: &str) -> Option<i64> {
    match timeframe {
        "5s" => Some(5),
        "1m" => Some(60),
        "15m" => Some(900),
        "1h" => Some(3600),
        _ => None,
    }
}

/// Floor `ts` to the start of its bucket for `timeframe`.
///
/// # Panics
/// Panics if `timeframe` is not one of the supported values, matching the
/// Python `KeyError` from indexing `TIMEFRAME_SECONDS`.
pub fn bucket_for(ts: DateTime<Utc>, timeframe: &str) -> DateTime<Utc> {
    let secs =
        timeframe_seconds(timeframe).unwrap_or_else(|| panic!("unknown timeframe: {timeframe}"));
    let epoch = ts.timestamp();
    let floored = epoch - epoch.rem_euclid(secs);
    Utc.timestamp_opt(floored, 0)
        .single()
        .expect("floored timestamp is always representable")
}

#[derive(Debug, Clone)]
struct BucketState {
    bucket_start: DateTime<Utc>,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

fn f64_to_money(value: f64) -> Decimal {
    Decimal::from_f64_retain(value).unwrap_or_default()
}

/// Consumes `PriceTick`s from a channel and writes one OHLCV candle per
/// timeframe bucket per pair to `storage`.
pub struct OhlcvAggregator<'a> {
    storage: &'a JsonStorage,
    source: mpsc::Receiver<PriceTick>,
    timeframe: String,
    state: HashMap<String, BucketState>,
    stop_after_drain: bool,
}

impl<'a> OhlcvAggregator<'a> {
    pub fn new(
        storage: &'a JsonStorage,
        source: mpsc::Receiver<PriceTick>,
        timeframe: impl Into<String>,
    ) -> Self {
        Self {
            storage,
            source,
            timeframe: timeframe.into(),
            state: HashMap::new(),
            stop_after_drain: false,
        }
    }

    pub fn stop_after_drain(&mut self) {
        self.stop_after_drain = true;
    }

    /// Drain ticks from `source`, writing a candle each time a pair's bucket
    /// rolls over. Returns once `stop_after_drain` has been set and no tick
    /// arrives within the poll window, or once the channel is permanently
    /// closed (no `PriceFeed` side left to send more ticks).
    pub async fn run(&mut self) -> Result<(), StorageError> {
        loop {
            match tokio::time::timeout(Duration::from_millis(50), self.source.recv()).await {
                Ok(Some(tick)) => self.handle(tick)?,
                Ok(None) => return Ok(()),
                Err(_elapsed) => {
                    if self.stop_after_drain {
                        return Ok(());
                    }
                }
            }
        }
    }

    fn handle(&mut self, tick: PriceTick) -> Result<(), StorageError> {
        let bucket = bucket_for(tick.sampled_at, &self.timeframe);
        match self.state.get_mut(&tick.pair) {
            None => {
                self.state.insert(
                    tick.pair,
                    BucketState {
                        bucket_start: bucket,
                        open: tick.price,
                        high: tick.price,
                        low: tick.price,
                        close: tick.price,
                    },
                );
                Ok(())
            }
            Some(cur) if bucket > cur.bucket_start => {
                let finished = cur.clone();
                self.write(&tick.pair, &finished)?;
                self.state.insert(
                    tick.pair,
                    BucketState {
                        bucket_start: bucket,
                        open: tick.price,
                        high: tick.price,
                        low: tick.price,
                        close: tick.price,
                    },
                );
                Ok(())
            }
            Some(cur) => {
                cur.high = cur.high.max(tick.price);
                cur.low = cur.low.min(tick.price);
                cur.close = tick.price;
                Ok(())
            }
        }
    }

    fn write(&self, pair: &str, state: &BucketState) -> Result<(), StorageError> {
        let candle = OhlcvCandle {
            pair: pair.to_string(),
            timeframe: self.timeframe.clone(),
            bucket_start: state.bucket_start,
            open: f64_to_money(state.open),
            high: f64_to_money(state.high),
            low: f64_to_money(state.low),
            close: f64_to_money(state.close),
            volume_quote: Decimal::ZERO,
        };
        self.storage.upsert_ohlcv(&candle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_for_floors_to_minute() {
        let ts = Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 30).unwrap();
        let bucket = bucket_for(ts, "1m");
        assert_eq!(bucket, Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap());
    }

    #[test]
    fn bucket_for_floors_to_hour() {
        let ts = Utc.with_ymd_and_hms(2026, 5, 3, 12, 45, 10).unwrap();
        let bucket = bucket_for(ts, "1h");
        assert_eq!(bucket, Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap());
    }
}
