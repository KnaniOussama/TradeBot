use serde::{Deserialize, Serialize};
use std::fmt;

/// Trading mode. Serializes to the lowercase strings the Python engine uses
/// in state-file names (`portfolio.demo.json`, etc.) and config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Demo,
    Real,
    Backtest,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Mode::Demo => "demo",
            Mode::Real => "real",
            Mode::Backtest => "backtest",
        };
        f.write_str(s)
    }
}

/// Candle timeframe. Serializes to the exact tokens used as config keys and
/// OHLCV filename suffixes (`5s`, `1m`, `15m`, `1h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Timeframe {
    #[serde(rename = "5s")]
    FiveS,
    #[serde(rename = "1m")]
    OneM,
    #[serde(rename = "15m")]
    FifteenM,
    #[serde(rename = "1h")]
    OneH,
}

impl Timeframe {
    /// Bucket width in seconds. Mirrors `_TIMEFRAME_SECONDS` in the Python loop.
    pub fn seconds(self) -> u32 {
        match self {
            Timeframe::FiveS => 5,
            Timeframe::OneM => 60,
            Timeframe::FifteenM => 900,
            Timeframe::OneH => 3600,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Mode::Demo).unwrap(), "\"demo\"");
        assert_eq!(serde_json::to_string(&Mode::Real).unwrap(), "\"real\"");
        assert_eq!(
            serde_json::to_string(&Mode::Backtest).unwrap(),
            "\"backtest\""
        );
    }

    #[test]
    fn mode_display_matches_serialization() {
        assert_eq!(Mode::Demo.to_string(), "demo");
    }

    #[test]
    fn timeframe_roundtrips_through_tokens() {
        for (tf, token) in [
            (Timeframe::FiveS, "\"5s\""),
            (Timeframe::OneM, "\"1m\""),
            (Timeframe::FifteenM, "\"15m\""),
            (Timeframe::OneH, "\"1h\""),
        ] {
            assert_eq!(serde_json::to_string(&tf).unwrap(), token);
            let back: Timeframe = serde_json::from_str(token).unwrap();
            assert_eq!(back, tf);
        }
    }

    #[test]
    fn timeframe_seconds() {
        assert_eq!(Timeframe::FiveS.seconds(), 5);
        assert_eq!(Timeframe::OneM.seconds(), 60);
        assert_eq!(Timeframe::FifteenM.seconds(), 900);
        assert_eq!(Timeframe::OneH.seconds(), 3600);
    }
}
