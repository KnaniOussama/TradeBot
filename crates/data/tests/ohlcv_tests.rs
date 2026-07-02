use chrono::{TimeZone, Timelike, Utc};
use rust_decimal::prelude::ToPrimitive;
use tokio::sync::mpsc;
use tradebot_data::{OhlcvAggregator, PriceTick};
use tradebot_storage::JsonStorage;

fn tick(pair: &str, price: f64, ts: chrono::DateTime<Utc>) -> PriceTick {
    PriceTick {
        pair: pair.to_string(),
        price,
        sampled_at: ts,
        in_amount_quote: 10.0,
        price_impact_pct: 0.0,
    }
}

#[tokio::test]
async fn aggregator_writes_one_candle_per_bucket() {
    let tmp = tempfile::tempdir().expect("tmp dir");
    let storage = JsonStorage::new(tmp.path()).expect("storage");
    let (tx, rx) = mpsc::channel(16);
    let mut agg = OhlcvAggregator::new(&storage, rx, "1m");

    let base = Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 30).unwrap();
    tx.send(tick("SOL/USDC", 100.0, base)).await.unwrap();
    tx.send(tick("SOL/USDC", 102.0, base.with_second(45).unwrap()))
        .await
        .unwrap();
    tx.send(tick("SOL/USDC", 99.0, base.with_second(55).unwrap()))
        .await
        .unwrap();
    // next minute -> flushes the previous bucket
    tx.send(tick(
        "SOL/USDC",
        101.0,
        base.with_minute(1).unwrap().with_second(5).unwrap(),
    ))
    .await
    .unwrap();

    agg.stop_after_drain();
    agg.run().await.expect("run should succeed");
    drop(tx);

    let candles = storage.load_ohlcv("SOL/USDC", "1m", 10);
    assert_eq!(candles.len(), 1);
    let row = &candles[0];
    assert_eq!(row.open.to_f64().unwrap(), 100.0);
    assert_eq!(row.high.to_f64().unwrap(), 102.0);
    assert_eq!(row.low.to_f64().unwrap(), 99.0);
    assert_eq!(row.close.to_f64().unwrap(), 99.0);
}

#[tokio::test]
async fn aggregator_separates_pairs() {
    let tmp = tempfile::tempdir().expect("tmp dir");
    let storage = JsonStorage::new(tmp.path()).expect("storage");
    let (tx, rx) = mpsc::channel(16);
    let mut agg = OhlcvAggregator::new(&storage, rx, "1m");

    let base = Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap();
    tx.send(tick("SOL/USDC", 100.0, base)).await.unwrap();
    tx.send(tick("JUP/USDC", 1.5, base)).await.unwrap();
    tx.send(tick("SOL/USDC", 110.0, base.with_minute(1).unwrap()))
        .await
        .unwrap();
    tx.send(tick("JUP/USDC", 1.6, base.with_minute(1).unwrap()))
        .await
        .unwrap();

    agg.stop_after_drain();
    agg.run().await.expect("run should succeed");
    drop(tx);

    let sol = storage.load_ohlcv("SOL/USDC", "1m", 10);
    let jup = storage.load_ohlcv("JUP/USDC", "1m", 10);
    assert_eq!(sol.len(), 1);
    assert_eq!(jup.len(), 1);
}
