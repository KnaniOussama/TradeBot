//! Command implementations. Mirrors `init_cmd`, `wallet_cmd`, and
//! `start_cmd` in `tradebot/main.py`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::ValueEnum;

use tradebot_common::Mode;
use tradebot_config::{default_config, load_config, save_config};
use tradebot_wallet::{generate_bot_keypair, load_bot_keypair, save_bot_keypair, BotKeypair};

use crate::wiring;
use crate::RunMode;

#[derive(Copy, Clone, ValueEnum)]
pub enum WalletAction {
    Generate,
    Show,
}

/// Scaffold a default config file. Mirrors `init_cmd` in main.py: refuses to
/// overwrite an existing file (exit code 2).
pub fn cmd_init(config_path: &Path) -> ExitCode {
    if config_path.exists() {
        eprintln!(
            "ERROR: {} already exists; refusing to overwrite",
            config_path.display()
        );
        return ExitCode::from(2);
    }
    let cfg = default_config();
    if let Err(e) = save_config(config_path, &cfg) {
        eprintln!("ERROR: failed to write config: {e}");
        return ExitCode::FAILURE;
    }
    println!("Wrote default config to {}", config_path.display());
    println!("Edit it (or use the dashboard Settings tab after `tradebot start`).");
    ExitCode::SUCCESS
}

/// Manage the bot's encrypted keypair. Mirrors `wallet_cmd` in main.py: both
/// actions require the passphrase env var; `generate` refuses to overwrite
/// an existing keystore.
pub fn cmd_wallet(action: WalletAction, keystore: &Path, passphrase_env: &str) -> ExitCode {
    let passphrase = match std::env::var(passphrase_env) {
        Ok(p) if !p.is_empty() => p,
        _ => {
            eprintln!("ERROR: set {passphrase_env} in your environment first");
            return ExitCode::from(2);
        }
    };
    match action {
        WalletAction::Generate => {
            if keystore.exists() {
                eprintln!(
                    "ERROR: refusing to overwrite existing keystore at {}",
                    keystore.display()
                );
                return ExitCode::from(2);
            }
            let kp = generate_bot_keypair();
            if let Err(e) = save_bot_keypair(&kp, keystore, &passphrase) {
                eprintln!("ERROR: failed to save keystore: {e}");
                return ExitCode::FAILURE;
            }
            println!("Generated keystore at {}", keystore.display());
            println!("Bot address: {}", kp.address);
            ExitCode::SUCCESS
        }
        WalletAction::Show => match load_bot_keypair(keystore, &passphrase) {
            Ok(kp) => {
                println!("Bot address: {}", kp.address);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("ERROR: failed to load keystore: {e}");
                ExitCode::FAILURE
            }
        },
    }
}

/// Launch engine. Mirrors `start_cmd` in main.py: loads config, configures
/// logging, optionally resets saved state, gates real mode behind
/// `--confirm-real` and a loaded keystore, then hands off to the async
/// engine wiring in [`wiring::run`].
#[allow(clippy::too_many_arguments)]
pub async fn cmd_start(
    config_path: PathBuf,
    mode: RunMode,
    confirm_real: bool,
    keystore: Option<PathBuf>,
    passphrase_env: String,
    reset: bool,
    max_cycles: Option<u64>,
) -> ExitCode {
    if !config_path.exists() {
        eprintln!(
            "ERROR: config not found: {} (run `tradebot init` first)",
            config_path.display()
        );
        return ExitCode::from(2);
    }
    let cfg = match load_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR: failed to load config: {e}");
            return ExitCode::FAILURE;
        }
    };
    tradebot_common::init_logging(&cfg.app.log_level, true);

    let mode: Mode = mode.into();

    if reset {
        for path in wiring::state_file_paths(&cfg.data_dir, mode) {
            if path.exists() {
                if let Err(e) = std::fs::remove_file(&path) {
                    tracing::warn!(path = %path.display(), error = %e, "reset_remove_failed");
                }
            }
        }
        println!("Wiped saved state for mode={mode}");
    }

    if matches!(mode, Mode::Real) && !confirm_real {
        tracing::error!(
            hint = "re-run with --confirm-real",
            "real_mode_requires_confirm"
        );
        return ExitCode::from(2);
    }

    let mut bot_keypair: Option<BotKeypair> = None;
    if matches!(mode, Mode::Real) {
        let passphrase = match std::env::var(&passphrase_env) {
            Ok(p) if !p.is_empty() => p,
            _ => {
                tracing::error!(
                    env_var = passphrase_env.as_str(),
                    "keystore_passphrase_missing"
                );
                return ExitCode::from(2);
            }
        };
        let ks_path = keystore
            .clone()
            .unwrap_or_else(|| PathBuf::from(&cfg.app.keystore_path));
        match load_bot_keypair(&ks_path, &passphrase) {
            Ok(kp) => {
                tracing::info!(address = kp.address.as_str(), "keystore_loaded");
                bot_keypair = Some(kp);
            }
            Err(e) => {
                eprintln!("ERROR: failed to load keystore: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    match wiring::run(mode, cfg, bot_keypair, max_cycles).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "run_failed");
            ExitCode::FAILURE
        }
    }
}
