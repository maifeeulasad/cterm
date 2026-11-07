//! Client-side auth plumbing: where the bearer token lives and the CLI
//! commands that manage it.
//!
//! The daemon (in `cterm-daemon::auth`) only knows how to *verify* a token.
//! Everything about *producing* and *storing* one lives here, because the
//! choice of backend (OS keyring vs. plain env var vs. an in-memory mock in
//! tests) is a client concern. Keeping it on this side also means the
//! daemon doesn't link the `keyring` crate — one less dep in the server
//! process.
//!
//! ## Precedence the client uses
//!
//! When the client needs a token (e.g. `cterm execute` is about to
//! connect), [`resolve_token`] returns the first of:
//!
//! 1. `CTERM_AUTH_TOKEN=<secret>` from the environment. Dev/CI escape hatch.
//! 2. Whatever the active [`KeyStore`] has under
//!    (`service = "cterm", user = "daemon-token"`).
//! 3. `None` — unauthenticated client.
//!
//! An attempt to read the keyring that fails for environment reasons
//! (no DBus on a headless Linux box, keychain locked, …) degrades to #3
//! and prints a short warning; nothing crashes.

use crate::error::{CliError, CliResult};

/// Name reserved for this tool in the OS secret store.
pub const SERVICE: &str = "cterm";
/// Account name under [`SERVICE`] where the daemon's bearer token lives.
pub const ACCOUNT_DAEMON_TOKEN: &str = "daemon-token";

/// Env var that short-circuits the keyring. Set it to a raw token string.
pub const ENV_TOKEN: &str = "CTERM_AUTH_TOKEN";

/// Where tokens are kept. Implementations can map to the OS keyring, an
/// in-memory store (tests), a file, or any other backend.
///
/// Implementations are expected to be cheap to clone (they wrap handles,
/// not the secrets themselves) and safe to share across tasks.
pub trait KeyStore: Send + Sync {
    /// Reads the stored token, or `Ok(None)` if the slot is empty.
    ///
    /// Returning `Err` means the backend itself failed (keychain locked,
    /// DBus unavailable), not "no token". Callers typically fall back to
    /// "no token" on an error and surface a warning to the operator.
    fn get(&self, service: &str, account: &str) -> CliResult<Option<String>>;

    /// Stores `token` under `(service, account)`, overwriting any existing value.
    fn set(&self, service: &str, account: &str, token: &str) -> CliResult<()>;

    /// Removes the token. `Ok(())` when the slot is already empty.
    fn delete(&self, service: &str, account: &str) -> CliResult<()>;
}

/// OS-native keyring backend (Keychain / Credential Manager / Secret Service).
///
/// Thin wrapper around the `keyring` crate so the rest of the CLI never
/// imports it directly; swapping the crate (or stubbing it in tests)
/// stays a one-file change.
#[derive(Default, Clone, Copy)]
pub struct SystemKeyring;

impl KeyStore for SystemKeyring {
    fn get(&self, service: &str, account: &str) -> CliResult<Option<String>> {
        let entry = keyring::Entry::new(service, account).map_err(keyring_err)?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_err(e)),
        }
    }

    fn set(&self, service: &str, account: &str, token: &str) -> CliResult<()> {
        let entry = keyring::Entry::new(service, account).map_err(keyring_err)?;
        entry.set_password(token).map_err(keyring_err)
    }

    fn delete(&self, service: &str, account: &str) -> CliResult<()> {
        let entry = keyring::Entry::new(service, account).map_err(keyring_err)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_err(e)),
        }
    }
}

fn keyring_err(e: keyring::Error) -> CliError {
    CliError::Msg(format!("keyring: {e}"))
}

/// In-process store used by unit tests. Not compiled into release binaries
/// so an operator inside a container (no keyring available) must either set
/// `CTERM_AUTH_TOKEN` or run with a keyring daemon.
#[cfg(test)]
#[derive(Default, Clone)]
pub struct MemoryKeyStore {
    inner: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<(String, String), String>>,
    >,
}

#[cfg(test)]
impl KeyStore for MemoryKeyStore {
    fn get(&self, service: &str, account: &str) -> CliResult<Option<String>> {
        Ok(self
            .inner
            .lock()
            .expect("MemoryKeyStore mutex poisoned")
            .get(&(service.to_owned(), account.to_owned()))
            .cloned())
    }

    fn set(&self, service: &str, account: &str, token: &str) -> CliResult<()> {
        self.inner
            .lock()
            .expect("MemoryKeyStore mutex poisoned")
            .insert((service.to_owned(), account.to_owned()), token.to_owned());
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> CliResult<()> {
        self.inner
            .lock()
            .expect("MemoryKeyStore mutex poisoned")
            .remove(&(service.to_owned(), account.to_owned()));
        Ok(())
    }
}

/// Resolve the bearer token the client should present to the daemon, in
/// order: env var, then the given keyring. Returns `None` for
/// "connect unauthenticated".
///
/// Keyring failures are swallowed and reported to stderr as a one-line
/// warning so an unreadable keychain never bricks a `cterm execute` call
/// — the caller just connects unauthenticated and the daemon rejects
/// them with a clear `UNAUTHENTICATED` if auth is on.
pub fn resolve_token(store: &dyn KeyStore) -> Option<String> {
    if let Ok(t) = std::env::var(ENV_TOKEN) {
        if !t.is_empty() {
            return Some(t);
        }
    }
    match store.get(SERVICE, ACCOUNT_DAEMON_TOKEN) {
        Ok(opt) => opt.filter(|s| !s.is_empty()),
        Err(e) => {
            eprintln!("cterm: keyring unavailable ({e}); connecting unauthenticated");
            None
        }
    }
}

/// Generates a fresh bearer token. 128 bits of entropy (two UUIDv4s worth
/// of randomness compressed into one hex string) is overkill for a
/// loopback-only service but costs nothing.
///
/// Returned as lowercase hex, no padding, no separators — easy to paste
/// into an env var and friendly to shell escaping.
#[must_use]
pub fn generate_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

// -------- Subcommand handlers --------

/// `cterm auth enable [--token <t>] [--print]` — store a token in the
/// keyring and tell the operator what to do next.
///
/// If `--token` is omitted, a fresh token is generated. `--print` echoes
/// the token to stdout *once* so the operator can copy it into
/// `CTERM_AUTH_TOKEN` on hosts that lack a keyring.
pub fn enable(store: &dyn KeyStore, provided: Option<String>, print: bool) -> CliResult<String> {
    let token = provided.unwrap_or_else(generate_token);
    store.set(SERVICE, ACCOUNT_DAEMON_TOKEN, &token)?;
    if print {
        println!("{token}");
    }
    Ok(token)
}

/// `cterm auth disable` — remove the token from the keyring. The daemon
/// itself only drops back to open mode on restart (and only if nothing
/// in its own env points at a token).
pub fn disable(store: &dyn KeyStore) -> CliResult<()> {
    store.delete(SERVICE, ACCOUNT_DAEMON_TOKEN)
}

/// `cterm auth rotate [--print]` — generate a new token, store it,
/// return it. Callers must restart the daemon for the new value to take
/// effect; the command prints that as a reminder.
pub fn rotate(store: &dyn KeyStore, print: bool) -> CliResult<String> {
    let token = generate_token();
    store.set(SERVICE, ACCOUNT_DAEMON_TOKEN, &token)?;
    if print {
        println!("{token}");
    }
    Ok(token)
}

/// Status returned by [`status`].
pub struct AuthStatus {
    /// True if any token (env or keyring) would be presented right now.
    pub has_token: bool,
    /// Source of the active token, if any.
    pub source: Option<AuthSource>,
    /// Hex fingerprint (first 8 chars of the token) for operator eyeballing.
    /// Never the full secret.
    pub fingerprint: Option<String>,
}

/// Where a resolved token came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSource {
    /// `CTERM_AUTH_TOKEN` env var.
    Env,
    /// OS keyring.
    Keyring,
}

/// `cterm auth status` — reports which source would win, without
/// printing the token itself.
pub fn status(store: &dyn KeyStore) -> AuthStatus {
    if let Ok(t) = std::env::var(ENV_TOKEN) {
        if !t.is_empty() {
            return AuthStatus {
                has_token: true,
                source: Some(AuthSource::Env),
                fingerprint: Some(short(&t)),
            };
        }
    }
    match store.get(SERVICE, ACCOUNT_DAEMON_TOKEN) {
        Ok(Some(t)) if !t.is_empty() => AuthStatus {
            has_token: true,
            source: Some(AuthSource::Keyring),
            fingerprint: Some(short(&t)),
        },
        _ => AuthStatus {
            has_token: false,
            source: None,
            fingerprint: None,
        },
    }
}

fn short(t: &str) -> String {
    t.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> MemoryKeyStore {
        MemoryKeyStore::default()
    }

    #[test]
    fn generate_is_nonempty_hex() {
        let t = generate_token();
        assert_eq!(t.len(), 32);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn enable_then_status_reports_keyring_source() {
        let s = store();
        let saved = enable(&s, None, false).unwrap();
        let st = status(&s);
        assert!(st.has_token);
        assert_eq!(st.source, Some(AuthSource::Keyring));
        assert_eq!(st.fingerprint.as_deref(), Some(&saved[..8]));
    }

    #[test]
    fn disable_clears_stored_token() {
        let s = store();
        enable(&s, Some("abc".into()), false).unwrap();
        disable(&s).unwrap();
        assert!(!status(&s).has_token);
    }

    #[test]
    fn rotate_returns_fresh_value() {
        let s = store();
        let a = enable(&s, None, false).unwrap();
        let b = rotate(&s, false).unwrap();
        assert_ne!(a, b);
        assert_eq!(s.get(SERVICE, ACCOUNT_DAEMON_TOKEN).unwrap().unwrap(), b);
    }

    #[test]
    fn resolve_prefers_env_over_keyring() {
        // Keep this test module in sync with env — it doesn't race with others
        // because we're the only ones touching this key.
        let s = store();
        enable(&s, Some("from-keyring".into()), false).unwrap();
        temp_env(ENV_TOKEN, Some("from-env"), || {
            assert_eq!(resolve_token(&s).as_deref(), Some("from-env"));
        });
    }

    #[test]
    fn resolve_falls_back_to_keyring_when_env_unset() {
        let s = store();
        enable(&s, Some("in-keyring".into()), false).unwrap();
        temp_env(ENV_TOKEN, None, || {
            assert_eq!(resolve_token(&s).as_deref(), Some("in-keyring"));
        });
    }

    #[test]
    fn resolve_none_when_nothing_configured() {
        let s = store();
        temp_env(ENV_TOKEN, None, || {
            assert_eq!(resolve_token(&s), None);
        });
    }

    // Serialize env mutations across tests in this module without `unsafe` —
    // we hold a module-level Mutex, and the std::env::set_var/remove_var in
    // Rust 2021 is a safe fn.
    fn temp_env(key: &str, value: Option<&str>, body: impl FnOnce()) {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let prior = std::env::var(key).ok();
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        match prior {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        if let Err(e) = out {
            std::panic::resume_unwind(e);
        }
    }
}
