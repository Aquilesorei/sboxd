//! Forced local egress proxy.
//!
//! Landlock's network rules key on port number only, not destination host —
//! `AccessNet::ConnectTcp` can't express "allow api.stripe.com, block
//! attacker.example.com" on its own. So instead of opening the firewall to
//! all outbound ports, sbox opens it to exactly one: a local proxy it
//! controls. The proxy is the only thing with real network access and it
//! enforces the host allowlist that Landlock can't.
//!
//! ponytail: plain std TCP proxy, HTTP CONNECT (HTTPS) + raw HTTP forwarding,
//! no TLS termination or payload inspection. Upgrade to request-shape rules
//! only if a real incident demands it — see CLAUDE.md.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

const MAX_HEAD_BYTES: usize = 16 * 1024;

pub struct EgressProxy;

impl EgressProxy {
    /// Binds an ephemeral localhost port and serves it in a background
    /// thread for the lifetime of the process. Returns the bound port so
    /// the caller can point the child's HTTP(S)_PROXY env vars at it and
    /// carve a matching Landlock exception.
    pub fn spawn(allowlist: Vec<String>) -> io::Result<u16> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let allowlist: Arc<Vec<String>> = Arc::new(allowlist);

        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let allowlist = Arc::clone(&allowlist);
                thread::spawn(move || {
                    let _ = handle_conn(stream, &allowlist);
                });
            }
        });

        Ok(port)
    }
}

fn handle_conn(mut client: TcpStream, allowlist: &[String]) -> io::Result<()> {
    let head = read_head(&mut client)?;
    let head_str = String::from_utf8_lossy(&head);
    let mut lines = head_str.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");

    let host = if method.eq_ignore_ascii_case("CONNECT") {
        target.split(':').next().unwrap_or("").to_string()
    } else {
        lines
            .find_map(|l| l.to_ascii_lowercase().starts_with("host:").then(|| {
                l[5..].trim().split(':').next().unwrap_or("").to_string()
            }))
            .unwrap_or_default()
    };

    if !host_allowed(&host, allowlist) {
        eprintln!("[sbox] blocked outbound connection to '{host}' (not in --allow-net-out list)");
        client.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n")?;
        return Ok(());
    }

    if method.eq_ignore_ascii_case("CONNECT") {
        let upstream = TcpStream::connect(target)?;
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
        splice(client, upstream)
    } else {
        let mut upstream = TcpStream::connect(format!("{host}:80"))?;
        upstream.write_all(&head)?;
        splice(client, upstream)
    }
}

fn host_allowed(host: &str, allowlist: &[String]) -> bool {
    if host.is_empty() {
        return false;
    }
    allowlist
        .iter()
        .any(|allowed| host == allowed || host.ends_with(&format!(".{allowed}")))
}

/// Reads bytes up to and including the terminating blank line of an HTTP
/// request head. Byte-at-a-time is fine here — heads are small and this
/// isn't a hot path.
fn read_head(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if stream.read(&mut byte)? == 0 {
            break;
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") || buf.len() >= MAX_HEAD_BYTES {
            break;
        }
    }
    Ok(buf)
}

fn splice(mut client: TcpStream, mut upstream: TcpStream) -> io::Result<()> {
    let mut client_w = client.try_clone()?;
    let mut upstream_r = upstream.try_clone()?;
    let up = thread::spawn(move || {
        let _ = io::copy(&mut upstream_r, &mut client_w);
        let _ = client_w.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut client, &mut upstream);
    let _ = upstream.shutdown(Shutdown::Write);
    let _ = up.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_allowed_matches_exact_and_subdomain() {
        let allow = vec!["api.stripe.com".to_string(), "github.com".to_string()];
        assert!(host_allowed("api.stripe.com", &allow));
        assert!(host_allowed("codeload.github.com", &allow));
        assert!(!host_allowed("evil.com", &allow));
        assert!(!host_allowed("", &allow));
    }

    #[test]
    fn proxy_blocks_non_allowlisted_connect() {
        let port = EgressProxy::spawn(vec!["example.com".to_string()]).unwrap();
        let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
        conn.write_all(b"CONNECT evil.com:443 HTTP/1.1\r\nHost: evil.com\r\n\r\n")
            .unwrap();
        let mut resp = [0u8; 32];
        let n = conn.read(&mut resp).unwrap();
        assert!(String::from_utf8_lossy(&resp[..n]).starts_with("HTTP/1.1 403"));
    }
}
