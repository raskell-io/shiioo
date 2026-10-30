//! Where harness runs get their secrets from, and how a spec refers to them.
//!
//! A run's spec never holds a secret value: `env = { GH_TOKEN =
//! "secret:gh-token" }` names one, and the worker resolves the name
//! through a [`SecretSource`] when the harness starts. The binary provides
//! the encrypted store in `.shiioo/` (`shiioo secret set …`); tests use a
//! map.

use std::collections::BTreeMap;

/// Prefix that marks an environment value as a secret reference.
pub const SECRET_PREFIX: &str = "secret:";

/// The secret the `container` level hands Claude Code as
/// `CLAUDE_CODE_OAUTH_TOKEN` (made with `claude setup-token`).
pub const CLAUDE_OAUTH_TOKEN: &str = "claude-code-oauth-token";

pub trait SecretSource: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
    /// Every `(name, value)` pair, for redaction.
    fn all(&self) -> Vec<(String, String)>;
}

/// No secrets configured: every reference is unresolved.
pub struct NoSecrets;

impl SecretSource for NoSecrets {
    fn get(&self, _name: &str) -> Option<String> {
        None
    }

    fn all(&self) -> Vec<(String, String)> {
        Vec::new()
    }
}

impl SecretSource for BTreeMap<String, String> {
    fn get(&self, name: &str) -> Option<String> {
        BTreeMap::get(self, name).cloned()
    }

    fn all(&self) -> Vec<(String, String)> {
        self.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
}

/// Turns a spec's `env` into the values to hand the process. A reference to
/// a secret that is not set is an error naming the secret, so the owner
/// learns what to `shiioo secret set`.
pub fn resolve_env(
    env: &BTreeMap<String, String>,
    secrets: &dyn SecretSource,
) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::with_capacity(env.len());
    for (key, value) in env {
        match value.strip_prefix(SECRET_PREFIX) {
            Some(name) => match secrets.get(name) {
                Some(v) => out.push((key.clone(), v)),
                None => {
                    return Err(format!(
                        "secret {name} (for {key}) is not set; run `shiioo secret set {name}`"
                    ));
                }
            },
            None => out.push((key.clone(), value.clone())),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_references_and_passes_literals() {
        let mut secrets = BTreeMap::new();
        secrets.insert("gh-token".to_owned(), "ghp_x".to_owned());
        let mut env = BTreeMap::new();
        env.insert("GH_TOKEN".to_owned(), "secret:gh-token".to_owned());
        env.insert("RUST_LOG".to_owned(), "info".to_owned());
        let resolved = resolve_env(&env, &secrets).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            resolved,
            vec![
                ("GH_TOKEN".to_owned(), "ghp_x".to_owned()),
                ("RUST_LOG".to_owned(), "info".to_owned())
            ]
        );
    }

    #[test]
    fn a_missing_secret_names_itself() {
        let mut env = BTreeMap::new();
        env.insert("GH_TOKEN".to_owned(), "secret:gh-token".to_owned());
        match resolve_env(&env, &NoSecrets) {
            Err(err) => {
                assert!(err.contains("gh-token") && err.contains("shiioo secret set"), "{err}");
            }
            Ok(resolved) => panic!("resolved a missing secret: {resolved:?}"),
        }
    }
}
