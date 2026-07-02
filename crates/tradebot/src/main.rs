//! TradeBot CLI. Port of `tradebot/main.py`: the `click` command group
//! (`init`, `start`, `wallet`) and the `_run` / `_run_loop` async wiring
//! that builds the full trading engine and runs it.

mod cli;
mod wiring;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use cli::{cmd_init, cmd_wallet, WalletAction};

#[derive(Parser)]
#[command(name = "tradebot", version, about = "TradeBot CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scaffold a default config file.
    Init {
        #[arg(long = "config", default_value = "tradebot.config.json")]
        config: PathBuf,
    },
    /// Launch the trading engine.
    Start {
        #[arg(long = "config", default_value = "tradebot.config.json")]
        config: PathBuf,
        #[arg(long = "mode", value_enum, default_value_t = RunMode::Demo)]
        mode: RunMode,
        /// Required for --mode real (safety gate).
        #[arg(long = "confirm-real")]
        confirm_real: bool,
        #[arg(long = "keystore")]
        keystore: Option<PathBuf>,
        #[arg(long = "passphrase-env", default_value = "TRADEBOT_PASSPHRASE")]
        passphrase_env: String,
        /// Wipe saved portfolio + risk state before starting (keeps trade log).
        #[arg(long = "reset")]
        reset: bool,
        /// Test-only: run at most N decision cycles then exit instead of
        /// running forever. Not part of the Python CLI; added so
        /// integration tests can exercise `start` deterministically.
        #[arg(long = "max-cycles", hide = true)]
        max_cycles: Option<u64>,
    },
    /// Manage the bot's encrypted keypair.
    Wallet {
        #[arg(value_enum)]
        action: WalletAction,
        #[arg(long = "keystore")]
        keystore: PathBuf,
        #[arg(long = "passphrase-env", default_value = "TRADEBOT_PASSPHRASE")]
        passphrase_env: String,
    },
}

#[derive(Copy, Clone, ValueEnum)]
pub enum RunMode {
    Demo,
    Real,
}

impl From<RunMode> for tradebot_common::Mode {
    fn from(mode: RunMode) -> Self {
        match mode {
            RunMode::Demo => tradebot_common::Mode::Demo,
            RunMode::Real => tradebot_common::Mode::Real,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { config } => cmd_init(&config),
        Command::Wallet {
            action,
            keystore,
            passphrase_env,
        } => cmd_wallet(action, &keystore, &passphrase_env),
        Command::Start {
            config,
            mode,
            confirm_real,
            keystore,
            passphrase_env,
            reset,
            max_cycles,
        } => {
            let rt = tokio::runtime::Runtime::new().expect("failed to build tokio runtime");
            rt.block_on(cli::cmd_start(
                config,
                mode,
                confirm_real,
                keystore,
                passphrase_env,
                reset,
                max_cycles,
            ))
        }
    }
}
