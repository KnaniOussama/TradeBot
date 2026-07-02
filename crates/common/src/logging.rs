use tracing::level_filters::LevelFilter;

/// Parse a config `log_level` string (as used by the Python engine: "INFO",
/// "DEBUG", etc.) into a tracing filter. Unknown values fall back to INFO.
pub fn parse_level(level: &str) -> LevelFilter {
    match level.to_ascii_uppercase().as_str() {
        "TRACE" => LevelFilter::TRACE,
        "DEBUG" => LevelFilter::DEBUG,
        "INFO" => LevelFilter::INFO,
        "WARN" | "WARNING" => LevelFilter::WARN,
        "ERROR" => LevelFilter::ERROR,
        _ => LevelFilter::INFO,
    }
}

/// Initialize global JSON logging at the given level. Safe to call once; a
/// second call is a no-op (returns without panicking).
pub fn init_logging(level: &str, json: bool) {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::builder()
        .with_default_directive(parse_level(level).into())
        .from_env_lossy();

    let builder = fmt().with_env_filter(filter);
    let result = if json {
        builder.json().try_init()
    } else {
        builder.try_init()
    };
    // Ignore "already initialized" — callers may invoke this more than once
    // across tests or re-entrant setup.
    let _ = result;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::level_filters::LevelFilter;

    #[test]
    fn parses_known_levels_case_insensitively() {
        assert_eq!(parse_level("info"), LevelFilter::INFO);
        assert_eq!(parse_level("DEBUG"), LevelFilter::DEBUG);
        assert_eq!(parse_level("WARNING"), LevelFilter::WARN);
        assert_eq!(parse_level("error"), LevelFilter::ERROR);
    }

    #[test]
    fn unknown_level_falls_back_to_info() {
        assert_eq!(parse_level("bogus"), LevelFilter::INFO);
    }

    #[test]
    fn init_logging_does_not_panic_when_called_twice() {
        init_logging("INFO", true);
        init_logging("DEBUG", true);
    }
}
