use std::process::{Command, Stdio};

use crate::error::SboxError;

pub(crate) struct FirewallSpec {
    pub allow_ips: Vec<String>,
}

pub(crate) fn apply_firewall_in_netns(pid: i32, spec: &FirewallSpec) -> Result<(), SboxError> {
    // Best-effort minimal egress firewall using nftables in the target network namespace.
    // This function assumes the caller has already ensured Linux + root.

    let mut allowed_v4: Vec<String> = Vec::new();
    let mut allowed_v6: Vec<String> = Vec::new();
    for ip in &spec.allow_ips {
        if ip.contains(':') {
            allowed_v6.push(ip.clone());
        } else {
            allowed_v4.push(ip.clone());
        }
    }

    // Build a small ruleset. We accept established traffic, allow loopback, and allow TCP 80/443
    // to the resolved allowlist. Everything else is dropped.
    let mut rules = String::new();
    rules.push_str("flush table inet sbox\n");
    rules.push_str("table inet sbox {\n");
    rules.push_str("  chain output {\n");
    rules.push_str("    type filter hook output priority 0; policy drop;\n");
    rules.push_str("    oifname \"lo\" accept\n");
    rules.push_str("    ct state established,related accept\n");

    if !allowed_v4.is_empty() {
        rules.push_str(&format!(
            "    ip daddr {{ {} }} tcp dport {{ 80, 443 }} accept\n",
            allowed_v4.join(", ")
        ));
    }
    if !allowed_v6.is_empty() {
        rules.push_str(&format!(
            "    ip6 daddr {{ {} }} tcp dport {{ 80, 443 }} accept\n",
            allowed_v6.join(", ")
        ));
    }

    rules.push_str("  }\n");
    rules.push_str("}\n");

    let mut child = Command::new("nsenter");
    child.args(["-t", &pid.to_string(), "-n", "nft", "-f", "-"]);
    child.stdin(Stdio::piped());
    child.stdout(Stdio::null());
    child.stderr(Stdio::null());

    let mut child = child
        .spawn()
        .map_err(|source| SboxError::CommandSpawn {
            program: "nsenter".to_string(),
            source,
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write as _;
        stdin.write_all(rules.as_bytes()).ok();
    }

    let status = child
        .wait()
        .map_err(|source| SboxError::CommandSpawn {
            program: "nsenter".to_string(),
            source,
        })?;

    if !status.success() {
        return Err(SboxError::FirewallPolicyUnavailable {
            reason: "failed to apply nftables firewall rules inside container netns".to_string(),
        });
    }

    Ok(())
}
