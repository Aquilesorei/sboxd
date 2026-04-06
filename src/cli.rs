use std::path::PathBuf;

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

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
    Shim(ShimCommand),
    Bootstrap(BootstrapCommand),
    Audit(AuditCommand),
    Completions(CompletionsCommand),
}

#[derive(Debug, Clone, Args)]
pub struct InitCommand {
    #[arg(long)]
    pub force: bool,

    #[arg(long)]
    pub preset: Option<String>,

    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Launch an interactive wizard to generate sbox.yaml
    #[arg(long, short = 'i')]
    pub interactive: bool,

    /// Auto-detect the package manager from an existing lockfile in the current directory
    /// and generate a matching preset config (skips the wizard)
    #[arg(long, conflicts_with_all = ["preset", "interactive"])]
    pub from_lockfile: bool,
}

#[derive(Debug, Clone, Args)]
#[command(trailing_var_arg = true)]
pub struct RunCommand {
    /// Print the resolved plan and backend command without executing
    #[arg(long)]
    pub dry_run: bool,

    /// Pass an extra environment variable into the sandbox, e.g. -e FOO=bar (repeatable)
    #[arg(short = 'e', long = "env", value_name = "NAME=VALUE")]
    pub env: Vec<String>,

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
    #[arg(long)]
    pub show_command: bool,

    /// Run the ecosystem's audit tool (npm audit, cargo audit, etc.) and append findings
    #[arg(long)]
    pub audit: bool,

    /// Omit to show the policy for the profile selected by --profile without a specific command.
    #[arg(num_args = 0.., allow_hyphen_values = true)]
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

/// Scan the project's lockfile for known-malicious or vulnerable package versions.
/// Delegates to the ecosystem's native audit tool (npm audit, cargo audit, etc.)
/// and runs on the host (not in a sandbox) so it can reach advisory databases.
#[derive(Debug, Clone, Args, Default)]
pub struct AuditCommand {
    /// Extra arguments forwarded to the underlying audit tool.
    #[arg(num_args = 0.., allow_hyphen_values = true)]
    pub extra_args: Vec<String>,
}

/// Generate the package lockfile inside the sandbox without running install scripts.
/// Requires `package_manager:` to be configured in sbox.yaml.
/// After bootstrap, run `sbox run -- <rebuild-command>` to execute scripts with network off.
#[derive(Debug, Clone, Args, Default)]
pub struct BootstrapCommand {}

/// Print shell completion script to stdout.
/// Pipe the output to your shell's completion directory, e.g.:
///   sbox completions bash > /etc/bash_completion.d/sbox
///   sbox completions zsh  > ~/.zsh/completions/_sbox
#[derive(Debug, Clone, Args)]
pub struct CompletionsCommand {
    pub shell: Shell,
}

pub fn generate_completions(shell: Shell) {
    use std::io;
    clap_complete::generate(shell, &mut Cli::command(), "sbox", &mut io::stdout());
}

#[derive(Debug, Clone, Args)]
pub struct ShimCommand {
    /// Directory to write shim scripts into (default: ~/.local/bin)
    #[arg(long)]
    pub dir: Option<PathBuf>,

    /// Overwrite existing shim files
    #[arg(long)]
    pub force: bool,

    /// Print what would be created without writing anything
    #[arg(long)]
    pub dry_run: bool,

    /// Check whether shims are installed and appear in PATH before the real binaries; exit 1 if any are missing or shadowed
    #[arg(long)]
    pub verify: bool,
}
