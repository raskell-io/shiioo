//! Network egress for `container` runs: an allow-listing HTTP proxy per
//! run.
//!
//! The harness container joins an *internal* Docker network, which has no
//! route anywhere. The only way out is a tinyproxy sidecar that sits on
//! both that network and the default bridge, and forwards `CONNECT`
//! (TLS, port 443) and plain HTTP only to hosts on the allow list. The
//! harness learns about it through `HTTPS_PROXY` and friends; a program
//! that ignores them simply cannot connect. The office's own API host is
//! always on the list, so the permission hook works.

use super::{SandboxError, output, short_hash};
use std::path::Path;

/// The internal network every sandboxed harness joins.
pub const NETWORK: &str = "shiioo-sandbox";
pub const PROXY_PORT: u16 = 3128;
pub const DOCKERFILE: &str = include_str!("egress.Dockerfile");

/// The sidecar's image tag, derived from its Dockerfile.
pub fn image() -> String {
    format!("shiioo-egress:{}", short_hash(DOCKERFILE))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub name: String,
    pub url: String,
    pub hosts: Vec<String>,
}

/// tinyproxy's configuration: filter by host with shell patterns, deny by
/// default, TLS tunnels to 443 only.
pub fn tinyproxy_conf() -> String {
    format!(
        "Port {PROXY_PORT}\n\
         Listen 0.0.0.0\n\
         Timeout 600\n\
         MaxClients 64\n\
         PidFile \"/tmp/tinyproxy.pid\"\n\
         LogLevel Connect\n\
         DisableViaHeader Yes\n\
         Filter \"/etc/tinyproxy/filter\"\n\
         FilterType fnmatch\n\
         FilterDefaultDeny Yes\n\
         ConnectPort 443\n"
    )
}

/// One pattern per line; the office's host is always allowed.
pub fn filter_file(hosts: &[String]) -> String {
    let mut lines: Vec<&str> = vec!["host.docker.internal"];
    lines.extend(hosts.iter().map(String::as_str).filter(|h| !h.is_empty()));
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// The variables that send a process's traffic through `url`.
pub fn proxy_env(url: &str) -> Vec<(String, String)> {
    ["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"]
        .into_iter()
        .map(|k| (k.to_owned(), url.to_owned()))
        .collect()
}

pub async fn ensure_network(engine: &str) -> Result<(), SandboxError> {
    if output(engine, &["network", "inspect", NETWORK]).await.is_ok() {
        return Ok(());
    }
    output(engine, &["network", "create", "--internal", NETWORK]).await?;
    Ok(())
}

/// Starts the proxy for one run. `dir` receives the configuration files
/// and is mounted read-only into the sidecar.
pub async fn start(
    engine: &str,
    dir: &Path,
    short_run: &str,
    hosts: &[String],
) -> Result<Proxy, SandboxError> {
    ensure_network(engine).await?;
    super::container::ensure_image(engine, &image(), DOCKERFILE, dir).await?;
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("tinyproxy.conf"), tinyproxy_conf())?;
    std::fs::write(dir.join("filter"), filter_file(hosts))?;
    let name = format!("shiioo-egress-{short_run}");
    super::container::remove_container(engine, &name).await;
    // `--mount` fails loudly when the host path is not visible to the
    // engine (a `-v` would silently create an empty directory instead).
    let conf = format!(
        "type=bind,src={},dst=/etc/tinyproxy,readonly",
        dir.to_string_lossy()
    );
    output(
        engine,
        &[
            "run",
            "-d",
            "--name",
            &name,
            "--network",
            NETWORK,
            "--read-only",
            "--tmpfs",
            "/tmp:rw,size=64m",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--pids-limit",
            "128",
            "--add-host",
            "host.docker.internal:host-gateway",
            "--mount",
            &conf,
            &image(),
        ],
    )
    .await?;
    // A proxy that died on its configuration must not pass for one that
    // runs; its log says why.
    let running = output(engine, &["inspect", "-f", "{{.State.Running}}", &name]).await;
    if !matches!(running.as_deref(), Ok("true")) {
        let logs = output(engine, &["logs", &name]).await.unwrap_or_default();
        super::container::remove_container(engine, &name).await;
        return Err(SandboxError::Command {
            program: engine.into(),
            args: format!("run {name}"),
            stderr: format!("the egress proxy did not start: {}", logs.trim()),
        });
    }
    // The second leg: the default bridge, which has the way out.
    if let Err(e) = output(engine, &["network", "connect", "bridge", &name]).await {
        super::container::remove_container(engine, &name).await;
        return Err(e);
    }
    Ok(Proxy {
        name: name.clone(),
        url: format!("http://{name}:{PROXY_PORT}"),
        hosts: hosts.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_always_allows_the_office_host() {
        let f = filter_file(&["github.com".into(), "*.github.com".into(), String::new()]);
        assert_eq!(f, "host.docker.internal\ngithub.com\n*.github.com\n");
    }

    #[test]
    fn conf_denies_by_default_and_tunnels_only_tls() {
        let c = tinyproxy_conf();
        assert!(c.contains("FilterDefaultDeny Yes"));
        assert!(c.contains("FilterType fnmatch"));
        assert!(c.contains("ConnectPort 443"));
        assert!(!c.contains("ConnectPort 80"));
    }

    #[test]
    fn proxy_env_covers_both_spellings() {
        let env = proxy_env("http://p:3128");
        assert_eq!(env.len(), 4);
        assert!(env.iter().all(|(_, v)| v == "http://p:3128"));
    }
}
