use clap::Parser;
use sbox::cli::{
    Cli, CliBackendKind, CliExecutionMode, Commands, DoctorCommand, ExecCommand, InitCommand,
    PlanCommand, RunCommand, ShellCommand,
};
use std::path::Path;

#[test]
fn parse_run_command() {
    let args = vec!["sbox", "run", "--", "npm", "install", "lodash"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Run(RunCommand { command }) => {
            assert_eq!(command, vec!["npm", "install", "lodash"]);
        }
        other => panic!("expected run command, got {other:?}"),
    }
}

#[test]
fn parse_exec_command() {
    let args = vec!["sbox", "exec", "install", "--", "uv", "sync"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Exec(ExecCommand { profile, command }) => {
            assert_eq!(profile, "install");
            assert_eq!(command, vec!["uv", "sync"]);
        }
        other => panic!("expected exec command, got {other:?}"),
    }
}

#[test]
fn parse_plan_command() {
    let args = vec!["sbox", "plan", "--", "cargo", "build"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Plan(PlanCommand { command }) => {
            assert_eq!(command, vec!["cargo", "build"]);
        }
        other => panic!("expected plan command, got {other:?}"),
    }
}

#[test]
fn parse_init_command() {
    let args = vec!["sbox", "init"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Init(InitCommand {
            force,
            preset,
            output,
        }) => {
            assert!(!force);
            assert!(preset.is_none());
            assert!(output.is_none());
        }
        other => panic!("expected init command, got {other:?}"),
    }
}

#[test]
fn parse_init_with_preset() {
    let args = vec!["sbox", "init", "--preset", "python"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Init(InitCommand { preset, .. }) => {
            assert_eq!(preset.as_deref(), Some("python"));
        }
        other => panic!("expected init command, got {other:?}"),
    }
}

#[test]
fn parse_init_with_force() {
    let args = vec!["sbox", "init", "--force"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Init(InitCommand { force, .. }) => {
            assert!(force);
        }
        other => panic!("expected init command, got {other:?}"),
    }
}

#[test]
fn parse_init_with_output() {
    let args = vec!["sbox", "init", "--output", "custom/sbox.yaml"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Init(InitCommand { output, .. }) => {
            let p: &Path = Path::new("custom/sbox.yaml");
            assert_eq!(output.as_deref(), Some(p));
        }
        other => panic!("expected init command, got {other:?}"),
    }
}

#[test]
fn parse_shell_command() {
    let args = vec!["sbox", "shell"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Shell(ShellCommand { shell }) => {
            assert!(shell.is_none());
        }
        other => panic!("expected shell command, got {other:?}"),
    }
}

#[test]
fn parse_shell_with_shell_override() {
    let args = vec!["sbox", "shell", "--shell", "/bin/zsh"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Shell(ShellCommand { shell }) => {
            assert_eq!(shell.as_deref(), Some("/bin/zsh"));
        }
        other => panic!("expected shell command, got {other:?}"),
    }
}

#[test]
fn parse_doctor_command() {
    let args = vec!["sbox", "doctor"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Doctor(DoctorCommand { strict }) => {
            assert!(!strict);
        }
        other => panic!("expected doctor command, got {other:?}"),
    }
}

#[test]
fn parse_doctor_with_strict() {
    let args = vec!["sbox", "doctor", "--strict"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Doctor(DoctorCommand { strict }) => {
            assert!(strict);
        }
        other => panic!("expected doctor command, got {other:?}"),
    }
}

#[test]
fn global_config_flag() {
    let args = vec![
        "sbox",
        "--config",
        "/path/to/config.yaml",
        "plan",
        "--",
        "echo",
    ];
    let cli = Cli::parse_from(args);

    let p = Path::new("/path/to/config.yaml");
    assert_eq!(cli.config.as_deref(), Some(p));
}

#[test]
fn global_workspace_flag() {
    let args = vec!["sbox", "--workspace", "/project", "plan", "--", "echo"];
    let cli = Cli::parse_from(args);

    let p = Path::new("/project");
    assert_eq!(cli.workspace.as_deref(), Some(p));
}

#[test]
fn global_backend_podman() {
    let args = vec!["sbox", "--backend", "podman", "plan", "--", "echo"];
    let cli = Cli::parse_from(args);

    assert!(matches!(cli.backend, Some(CliBackendKind::Podman)));
}

#[test]
fn global_backend_docker() {
    let args = vec!["sbox", "--backend", "docker", "plan", "--", "echo"];
    let cli = Cli::parse_from(args);

    assert!(matches!(cli.backend, Some(CliBackendKind::Docker)));
}

#[test]
fn global_image_override() {
    let args = vec!["sbox", "--image", "rust:1.80", "plan", "--", "cargo"];
    let cli = Cli::parse_from(args);

    assert_eq!(cli.image.as_deref(), Some("rust:1.80"));
}

#[test]
fn global_profile_override() {
    let args = vec!["sbox", "--profile", "build", "plan", "--", "cargo"];
    let cli = Cli::parse_from(args);

    assert_eq!(cli.profile.as_deref(), Some("build"));
}

#[test]
fn global_mode_host() {
    let args = vec!["sbox", "--mode", "host", "run", "--", "echo"];
    let cli = Cli::parse_from(args);

    assert!(matches!(cli.mode, Some(CliExecutionMode::Host)));
}

#[test]
fn global_mode_sandbox() {
    let args = vec!["sbox", "--mode", "sandbox", "run", "--", "echo"];
    let cli = Cli::parse_from(args);

    assert!(matches!(cli.mode, Some(CliExecutionMode::Sandbox)));
}

#[test]
fn global_strict_security_flag() {
    let args = vec!["sbox", "--strict-security", "run", "--", "npm"];
    let cli = Cli::parse_from(args);

    assert!(cli.strict_security);
}

#[test]
fn global_verbose_flag() {
    let args = vec!["sbox", "-v", "plan", "--", "echo"];
    let cli = Cli::parse_from(args);

    assert_eq!(cli.verbose, 1);
}

#[test]
fn global_verbose_multiple() {
    let args = vec!["sbox", "-vvv", "plan", "--", "echo"];
    let cli = Cli::parse_from(args);

    assert_eq!(cli.verbose, 3);
}

#[test]
fn global_quiet_flag() {
    let args = vec!["sbox", "--quiet", "plan", "--", "echo"];
    let cli = Cli::parse_from(args);

    assert!(cli.quiet);
}

#[test]
fn combined_global_flags() {
    let args = vec![
        "sbox",
        "--config",
        "/cfg.yaml",
        "--workspace",
        "/work",
        "--backend",
        "podman",
        "--strict-security",
        "-vv",
        "plan",
        "--",
        "cargo",
    ];
    let cli = Cli::parse_from(args);

    assert!(cli.config.is_some());
    assert!(cli.workspace.is_some());
    assert!(matches!(cli.backend, Some(CliBackendKind::Podman)));
    assert!(cli.strict_security);
    assert_eq!(cli.verbose, 2);
}

#[test]
fn command_after_double_dash_preserved() {
    let args = vec!["sbox", "run", "--", "echo", "-n", "hello world"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Run(RunCommand { command }) => {
            assert_eq!(command, vec!["echo", "-n", "hello world"]);
        }
        other => panic!("expected run command, got {other:?}"),
    }
}

#[test]
fn command_with_hyphen_values() {
    let args = vec!["sbox", "run", "--", "node", "--version"];
    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Run(RunCommand { command }) => {
            assert_eq!(command, vec!["node", "--version"]);
        }
        other => panic!("expected run command, got {other:?}"),
    }
}

#[test]
fn version_flag() {
    let args = vec!["sbox", "--version"];
    let result = Cli::try_parse_from(args);
    assert!(result.is_err());
}

#[test]
fn help_flag() {
    let args = vec!["sbox", "--help"];
    let result = Cli::try_parse_from(args);
    assert!(result.is_err());
}

#[test]
fn subcommand_help() {
    let args = vec!["sbox", "run", "--help"];
    let result = Cli::try_parse_from(args);
    assert!(result.is_err());
}
