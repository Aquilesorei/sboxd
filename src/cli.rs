use std::path::PathBuf;

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

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

    /// Output format: text (default) or json
    #[arg(
        long = "output-format",
        global = true,
        value_enum,
        default_value = "text"
    )]
    pub output_format: OutputFormat,

    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Internal: used to capture unknown subcommands as custom commands
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub custom_command: Vec<String>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    #[command(visible_alias = "i")]
    Init(InitCommand),
    #[command(visible_alias = "r")]
    Run(RunCommand),
    #[command(visible_alias = "e")]
    Exec(ExecCommand),
    #[command(visible_alias = "sh")]
    Shell(ShellCommand),
    #[command(visible_alias = "p")]
    Plan(PlanCommand),
    #[command(visible_alias = "d")]
    Doctor(DoctorCommand),
    #[command(visible_alias = "c")]
    Clean(CleanCommand),
    Shim(ShimCommand),
    #[command(visible_alias = "b")]
    Bootstrap(BootstrapCommand),
    #[command(visible_alias = "a")]
    Audit(AuditCommand),
    Harden(HardenCommand),
    Completions(CompletionsCommand),
    #[command(visible_alias = "s")]
    Status(StatusCommand),
    #[command(visible_alias = "l")]
    Logs(LogsCommand),
    #[command(visible_alias = "ex")]
    Explain(ExplainCommand),
    Lint(LintCommand),
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

    /// In interactive mode, prompt from scratch instead of auto-applying detected defaults
    #[arg(long, requires = "interactive")]
    pub all: bool,

    /// Auto-detect the package manager from an existing lockfile in the current directory
    /// and generate a matching preset config (skips the wizard)
    #[arg(long, conflicts_with_all = ["preset", "interactive", "all"])]
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

#[derive(Debug, Clone, Args, Default)]
pub struct HardenCommand {
    /// Overwrite the generated hardening file if it already exists
    #[arg(long)]
    pub write: bool,

    /// Print the generated override instead of only writing the file
    #[arg(long)]
    pub diff: bool,

    /// Run docker compose with the generated override after writing it
    #[arg(long)]
    pub run: bool,

    /// Compose file to harden
    #[arg(long)]
    pub compose_file: Option<PathBuf>,

    /// Output path for the generated compose override
    #[arg(long)]
    pub output: Option<PathBuf>,
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

/// List running sbox-managed containers for the current workspace.
#[derive(Debug, Clone, Args, Default)]
pub struct StatusCommand {
    /// Show containers from all workspaces, not just the current one
    #[arg(long)]
    pub all: bool,
}

/// Stream or tail logs from a running sbox reusable session.
#[derive(Debug, Clone, Args, Default)]
pub struct LogsCommand {
    /// Profile name to fetch logs for (defaults to most recently started)
    pub profile: Option<String>,

    /// Follow log output (like `tail -f`)
    #[arg(short, long)]
    pub follow: bool,

    /// Number of lines to show from the end of the logs
    #[arg(long, default_value = "50")]
    pub tail: u32,
}

/// Explain in plain language what sbox would do for a given command.
#[derive(Debug, Clone, Args)]
#[command(trailing_var_arg = true)]
pub struct ExplainCommand {
    #[arg(required = true, num_args = 1.., allow_hyphen_values = true)]
    pub command: Vec<String>,
}

/// Statically analyse sbox.yaml for security antipatterns.
#[derive(Debug, Clone, Args, Default)]
pub struct LintCommand {
    /// Treat warnings as errors (exit 2 instead of 0 on warnings)
    #[arg(long)]
    pub strict: bool,
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
