use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "sbox",
    author = "Aquilesorei",
    version = "0.2.0",
    about = "Zero-config native Linux security sandbox for development commands",
    allow_external_subcommands = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Explicitly run a command in the native sandbox
    Run {
        /// The command to execute
        command: String,
        /// Arguments for the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Manage transparent shell shims
    Shim {
        #[command(subcommand)]
        action: ShimAction,
    },
    /// Catch external subcommands (e.g. `sbox npm install` -> `npm` with args `["install"]`)
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Subcommand, Debug)]
pub enum ShimAction {
    /// Install shims for common dev tools into ~/.local/share/sbox/shims
    Install,
    /// Verify active shims on PATH
    Verify,
}
