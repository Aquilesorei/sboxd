use std::process::ExitCode;

use crate::cli::{Cli, LintCommand, OutputFormat};
use crate::config::{LoadOptions, load_config, model::Config};
use crate::error::SboxError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LintLevel {
    Error,
    Warn,
}

/// `network_allow` is hostname/DNS-based and does not prevent raw-IP egress.
fn rule_network_allow_raw_ip_bypass(config: &Config) -> Vec<LintResult> {
    config
        .profiles
        .iter()
        .filter(|(_, profile)| profile.network.as_deref() == Some("on") && !profile.network_allow.is_empty())
        .map(|(name, _)| {
            LintResult::warn(
                "network-allow-raw-ip-bypass",
                format!(
                    "profile `{name}` uses `network_allow` — this restricts DNS/hostnames but does not block direct IP connections"
                ),
                "treat `network_allow` as a DNS allow-list; for stronger egress control, use host firewalling or a backend that supports IP-level filtering",
            )
        })
        .collect()
}

#[derive(Debug, Clone)]
struct LintResult {
    rule: &'static str,
    level: LintLevel,
    message: String,
    /// Suggestion for how to fix it.
    fix: &'static str,
}

impl LintResult {
    fn error(rule: &'static str, message: String, fix: &'static str) -> Self {
        Self {
            rule,
            level: LintLevel::Error,
            message,
            fix,
        }
    }
    fn warn(rule: &'static str, message: String, fix: &'static str) -> Self {
        Self {
            rule,
            level: LintLevel::Warn,
            message,
            fix,
        }
    }
}

pub fn execute(cli: &Cli, command: &LintCommand) -> Result<ExitCode, SboxError> {
    let loaded = load_config(&LoadOptions {
        workspace: cli.workspace.clone(),
        config: cli.config.clone(),
    })?;

    let findings = run_all_rules(&loaded.config);

    match cli.output_format {
        OutputFormat::Json => print_json(&findings, &loaded.config_path.display().to_string()),
        OutputFormat::Text => print_text(&findings, &loaded.config_path.display().to_string()),
    }

    let errors = findings
        .iter()
        .filter(|f| f.level == LintLevel::Error)
        .count();
    let warns = findings
        .iter()
        .filter(|f| f.level == LintLevel::Warn)
        .count();

    if errors > 0 {
        return Ok(ExitCode::from(2));
    }
    if warns > 0 && command.strict {
        return Ok(ExitCode::from(2));
    }
    if warns > 0 {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

// ── Rules ────────────────────────────────────────────────────────────────────

fn run_all_rules(config: &Config) -> Vec<LintResult> {
    let mut results = Vec::new();
    results.extend(rule_unpinned_image(config));
    results.extend(rule_full_network_no_allowlist(config));
    results.extend(rule_broad_network_allow(config));
    results.extend(rule_network_allow_raw_ip_bypass(config));
    results.extend(rule_sensitive_passthrough_not_denied(config));
    results.extend(rule_install_profile_no_role(config));
    results.extend(rule_privileged_identity(config));
    results.extend(rule_no_profiles_or_dispatch(config));
    results
}

/// Image is mutable reference with no digest and no `require_pinned_image`.
fn rule_unpinned_image(config: &Config) -> Vec<LintResult> {
    let pinned_globally = config
        .runtime
        .as_ref()
        .and_then(|rt| rt.require_pinned_image)
        .unwrap_or(false);

    if pinned_globally {
        return vec![];
    }

    let image = match config.image.as_ref() {
        Some(i) => i,
        None => return vec![],
    };

    // Already pinned if a digest is set, or if it's a local build.
    if image.digest.is_some() || image.build.is_some() {
        return vec![];
    }

    vec![LintResult::warn(
        "unpinned-image",
        format!(
            "image `{}` is a mutable tag — it may change between runs",
            image.reference.as_deref().unwrap_or("<unknown>")
        ),
        "add `image.digest: sha256:...` or set `runtime.require_pinned_image: true`",
    )]
}

/// A profile has `network: on` but no `network_allow` — full internet access.
fn rule_full_network_no_allowlist(config: &Config) -> Vec<LintResult> {
    config
        .profiles
        .iter()
        .filter(|(_, profile)| {
            profile.network.as_deref() == Some("on") && profile.network_allow.is_empty()
        })
        .map(|(name, _)| {
            LintResult::warn(
                "unrestricted-network",
                format!(
                    "profile `{name}` has `network: on` with no `network_allow` — \
                     full internet access inside the sandbox"
                ),
                "add `network_allow` with specific hostnames, or set `network: off`",
            )
        })
        .collect()
}

/// A `network_allow` entry looks overly broad (e.g. `*`, `github.com`, `0.0.0.0`).
fn rule_broad_network_allow(config: &Config) -> Vec<LintResult> {
    const BROAD: &[&str] = &["*", "0.0.0.0", "github.com", "raw.githubusercontent.com"];

    config
        .profiles
        .iter()
        .flat_map(|(name, profile)| {
            profile.network_allow.iter().filter_map(move |host| {
                if BROAD
                    .iter()
                    .any(|&b| host == b || host.ends_with(&format!(".{b}")))
                {
                    Some(LintResult::warn(
                        "broad-network-allow",
                        format!(
                            "profile `{name}` allows `{host}` — this may grant broader \
                             network access than intended"
                        ),
                        "restrict to the specific registry hostname required by this profile",
                    ))
                } else {
                    None
                }
            })
        })
        .collect()
}

/// A sensitive env var is in `pass_through` but not in `deny`.
fn rule_sensitive_passthrough_not_denied(config: &Config) -> Vec<LintResult> {
    const SENSITIVE_PATTERNS: &[&str] = &[
        "TOKEN",
        "SECRET",
        "KEY",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "AUTH",
    ];

    let env = match config.environment.as_ref() {
        Some(e) => e,
        None => return vec![],
    };

    env.pass_through
        .iter()
        .filter(|var| {
            let upper = var.to_uppercase();
            SENSITIVE_PATTERNS.iter().any(|p| upper.contains(p))
                && !env.deny.iter().any(|d| d == *var)
        })
        .map(|var| {
            LintResult::error(
                "sensitive-passthrough",
                format!(
                    "`{var}` looks like a credential and is in `pass_through` but not `deny` — \
                     it will be visible to code running inside the sandbox"
                ),
                "add the variable to `environment.deny` or remove it from `pass_through`",
            )
        })
        .collect()
}

/// A profile routes install-style commands but has no `role: install`.
fn rule_install_profile_no_role(config: &Config) -> Vec<LintResult> {
    const INSTALL_VERBS: &[&str] = &["install", "add", "sync", "fetch", "get", "update"];

    let mut results = Vec::new();

    for (dispatch_name, rule) in &config.dispatch {
        let profile = match config.profiles.get(&rule.profile) {
            Some(p) => p,
            None => continue,
        };

        if profile.role.is_some() {
            continue;
        }

        let has_install_pattern = rule
            .patterns
            .iter()
            .any(|pat| INSTALL_VERBS.iter().any(|verb| pat.contains(verb)));

        if has_install_pattern {
            results.push(LintResult::warn(
                "install-role-missing",
                format!(
                    "dispatch rule `{dispatch_name}` routes install-like commands to profile \
                     `{}` but the profile has no `role: install`",
                    rule.profile
                ),
                "add `role: install` to the profile to enable lockfile checks and install-style audit",
            ));
        }
    }

    results
}

/// Identity explicitly sets uid 0 — container runs as root.
fn rule_privileged_identity(config: &Config) -> Vec<LintResult> {
    let is_root = config
        .identity
        .as_ref()
        .and_then(|id| id.uid)
        .is_some_and(|uid| uid == 0);

    if !is_root {
        return vec![];
    }

    vec![LintResult::warn(
        "root-identity",
        "identity sets `uid: 0` — the container runs as root; files created inside will be \
         root-owned on the host (non-rootless Docker) and the attack surface is larger"
            .to_string(),
        "remove `identity.uid` to let sbox map to your host user (default behaviour)",
    )]
}

/// Config has neither profiles nor dispatch rules — sbox will only use the default profile.
fn rule_no_profiles_or_dispatch(config: &Config) -> Vec<LintResult> {
    // package_manager: elaboration adds synthetic profiles/dispatch, so skip if set.
    if config.package_manager.is_some() {
        return vec![];
    }

    if config.profiles.is_empty() && config.dispatch.is_empty() {
        return vec![LintResult::warn(
            "no-policy",
            "no profiles or dispatch rules are defined — all commands use the same default policy"
                .to_string(),
            "add profiles and dispatch rules, or use `package_manager:` for a preset",
        )];
    }

    vec![]
}

// ── Output ───────────────────────────────────────────────────────────────────

fn print_text(findings: &[LintResult], config_path: &str) {
    let errors = findings
        .iter()
        .filter(|f| f.level == LintLevel::Error)
        .count();
    let warns = findings
        .iter()
        .filter(|f| f.level == LintLevel::Warn)
        .count();

    println!("sbox lint: {config_path}");
    println!();

    if findings.is_empty() {
        println!("✓ no issues found");
        return;
    }

    for f in findings {
        let label = match f.level {
            LintLevel::Error => "error",
            LintLevel::Warn => "warn ",
        };
        println!("[{label}] {} — {}", f.rule, f.message);
        println!("        fix: {}", f.fix);
        println!();
    }

    println!("{errors} error(s), {warns} warning(s)");
}

fn print_json(findings: &[LintResult], config_path: &str) {
    let items: Vec<serde_json::Value> = findings
        .iter()
        .map(|f| {
            serde_json::json!({
                "rule": f.rule,
                "level": match f.level { LintLevel::Error => "error", LintLevel::Warn => "warn" },
                "message": f.message,
                "fix": f.fix,
            })
        })
        .collect();

    let out = serde_json::json!({
        "config": config_path,
        "errors": findings.iter().filter(|f| f.level == LintLevel::Error).count(),
        "warnings": findings.iter().filter(|f| f.level == LintLevel::Warn).count(),
        "findings": items,
    });

    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{
        Config, DispatchRule, EnvironmentConfig, ExecutionMode, ImageConfig, ProfileConfig,
        ProfileRole,
    };
    use std::collections::BTreeMap;

    fn empty_config() -> Config {
        Config {
            version: 1,
            runtime: None,
            workspace: None,
            identity: None,
            image: None,
            environment: None,
            mounts: vec![],
            caches: vec![],
            secrets: vec![],
            profiles: Default::default(),
            dispatch: Default::default(),
            package_manager: None,
            commands: Default::default(),
        }
    }

    fn make_profile(network: &str, role: Option<ProfileRole>) -> ProfileConfig {
        ProfileConfig {
            mode: ExecutionMode::Sandbox,
            image: None,
            network: Some(network.to_string()),
            writable: Some(true),
            require_pinned_image: None,
            require_lockfile: None,
            role,
            lockfile_files: vec![],
            pre_run: vec![],
            network_policy: crate::config::model::NetworkPolicy::Dns,
            network_allow: vec![],
            ports: vec![],
            capabilities: None,
            no_new_privileges: Some(true),
            read_only_rootfs: None,
            reuse_container: None,
            shell: None,
            writable_paths: None,
            compose: None,
        }
    }

    #[test]
    fn lint_clean_config_has_no_findings() {
        let mut config = empty_config();
        config.image = Some(ImageConfig {
            reference: Some("node:22".to_string()),
            digest: Some("sha256:abc".to_string()),
            verify_signature: None,
            build: None,
            preset: None,
            pull_policy: None,
            tag: None,
        });
        config.package_manager = Some(crate::config::model::PackageManagerConfig {
            name: "npm".to_string(),
            install_writable: None,
            build_writable: None,
            network_allow: None,
            pre_run: None,
        });
        let findings = run_all_rules(&config);
        assert!(
            findings.is_empty(),
            "expected no findings, got: {findings:?}"
        );
    }

    #[test]
    fn lint_warns_on_unpinned_image() {
        let mut config = empty_config();
        config.image = Some(ImageConfig {
            reference: Some("node:22".to_string()),
            digest: None,
            verify_signature: None,
            build: None,
            preset: None,
            pull_policy: None,
            tag: None,
        });
        let findings = run_all_rules(&config);
        assert!(findings.iter().any(|f| f.rule == "unpinned-image"));
    }

    #[test]
    fn lint_warns_on_unrestricted_network() {
        let mut config = empty_config();
        config
            .profiles
            .insert("install".to_string(), make_profile("on", None));
        let findings = run_all_rules(&config);
        assert!(findings.iter().any(|f| f.rule == "unrestricted-network"));
    }

    #[test]
    fn lint_errors_on_sensitive_passthrough_not_denied() {
        let mut config = empty_config();
        config.environment = Some(EnvironmentConfig {
            pass_through: vec!["NPM_TOKEN".to_string()],
            set: BTreeMap::new(),
            deny: vec![],
        });
        let findings = run_all_rules(&config);
        assert!(
            findings
                .iter()
                .any(|f| f.rule == "sensitive-passthrough" && f.level == LintLevel::Error)
        );
    }

    #[test]
    fn lint_no_error_when_sensitive_var_is_also_denied() {
        let mut config = empty_config();
        config.environment = Some(EnvironmentConfig {
            pass_through: vec!["NPM_TOKEN".to_string()],
            set: BTreeMap::new(),
            deny: vec!["NPM_TOKEN".to_string()],
        });
        let findings = run_all_rules(&config);
        assert!(!findings.iter().any(|f| f.rule == "sensitive-passthrough"));
    }

    #[test]
    fn lint_warns_on_install_pattern_without_role() {
        let mut config = empty_config();
        config
            .profiles
            .insert("install".to_string(), make_profile("on", None));
        config.dispatch.insert(
            "npm-install".to_string(),
            DispatchRule {
                patterns: vec!["npm install*".to_string()],
                profile: "install".to_string(),
            },
        );
        let findings = run_all_rules(&config);
        assert!(findings.iter().any(|f| f.rule == "install-role-missing"));
    }

    #[test]
    fn lint_no_warn_when_install_profile_has_role() {
        let mut config = empty_config();
        config.profiles.insert(
            "install".to_string(),
            make_profile("on", Some(ProfileRole::Install)),
        );
        config.dispatch.insert(
            "npm-install".to_string(),
            DispatchRule {
                patterns: vec!["npm install*".to_string()],
                profile: "install".to_string(),
            },
        );
        let findings = run_all_rules(&config);
        assert!(!findings.iter().any(|f| f.rule == "install-role-missing"));
    }
}
