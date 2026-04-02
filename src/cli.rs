use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "sbox",
    version,
    about = "Policy-driven sandboxed command runner"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[arg(long, global = true)]
    pub workspace: Option<PathBuf>,

    #[arg(long, global = true, value_enum)]
    pub backend: Option<CliBackendKind>,

    #[arg(long, global = true)]
    pub image: Option<String>,

    #[arg(long, global = true)]
    pub profile: Option<String>,

    #[arg(long, global = true, value_enum)]
    pub mode: Option<CliExecutionMode>,

    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[arg(long, global = true)]
    pub quiet: bool,

    #[arg(long, global = true)]
    pub strict_security: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    Init(InitCommand),
    Run(RunCommand),
    Exec(ExecCommand),
    Shell(ShellCommand),
    Plan(PlanCommand),
    Doctor(DoctorCommand),
    Clean(CleanCommand),
}

#[derive(Debug, Clone, Args)]
pub struct InitCommand {
    #[arg(long)]
    pub force: bool,

    #[arg(long)]
    pub preset: Option<String>,

    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
#[command(trailing_var_arg = true)]
pub struct RunCommand {
    #[arg(required = true, num_args = 1.., allow_hyphen_values = true)]
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Args)]
#[command(trailing_var_arg = true)]
pub struct ExecCommand {
    pub profile: String,

    #[arg(required = true, num_args = 1.., allow_hyphen_values = true)]
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Args, Default)]
pub struct ShellCommand {
    #[arg(long)]
    pub shell: Option<String>,
}

#[derive(Debug, Clone, Args)]
#[command(trailing_var_arg = true)]
pub struct PlanCommand {
    #[arg(required = true, num_args = 1.., allow_hyphen_values = true)]
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Args, Default)]
pub struct DoctorCommand {
    #[arg(long)]
    pub strict: bool,
}

#[derive(Debug, Clone, Args, Default)]
pub struct CleanCommand {
    #[arg(long)]
    pub sessions: bool,

    #[arg(long)]
    pub images: bool,

    #[arg(long)]
    pub caches: bool,

    #[arg(long)]
    pub all: bool,

    #[arg(long = "global")]
    pub global_scope: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliBackendKind {
    Podman,
    Docker,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliExecutionMode {
    Host,
    Sandbox,
}
