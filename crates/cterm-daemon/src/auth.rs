//! Token-based auth for the gRPC `Terminal` service.
//!
//! This module holds the concrete auth adapters the daemon hands to tonic:
//!
//! * [`TokenVerifier`] — a bearer-token verifier, the single opt-in scheme
//!   cterm-daemon ships today. One expected value; constant-time compare so
//!   mismatches don't leak via timing.
//! * [`verify_request`] — the tonic interceptor that pulls `authorization`
//!   out of request metadata and delegates to an [`AuthVerifier`].
//! * [`verifier_from_env`] — the env-driven factory used by `main.rs`.
//!
//! The daemon owns an `Arc<dyn AuthVerifier>` built once at startup. Each
//! RPC is wrapped by the interceptor so no service method has to repeat
//! the check. Opt-out is the default: without `CTERM_AUTH_TOKEN` or
//! `CTERM_AUTH=on`, [`verifier_from_env`] returns a `NoopVerifier` and the
//! service behaves exactly as it did before 0.6.0.

use std::sync::Arc;

use cterm_core::{deny, AuthVerifier, NoopVerifier, Result};
use tonic::service::Interceptor;
use tonic::{Request, Status};

/// Shared handle to whichever verifier the daemon is running.
///
/// Used by the daemon so the interceptor can be built from the same
/// instance that owns any future state (rate limits, revocations, etc.).
pub type Verifier = Arc<dyn AuthVerifier>;

/// Bearer-token verifier. One expected value; constant-time compare.
///
/// Byte-level equality instead of string `==` so an attacker cannot
/// distinguish "first byte matched" from "first byte wrong" by timing.
/// On a loopback service that's borderline paranoid (an attacker with
/// local code execution already has easier wins), but it costs nothing
/// and matches how every other token verifier in the ecosystem behaves.
pub struct TokenVerifier {
    expected: Vec<u8>,
}

impl TokenVerifier {
    /// Builds a verifier that accepts exactly `token`.
    ///
    /// The token is stored as bytes internally; the caller keeps ownership
    /// of its plaintext string.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            expected: token.into().into_bytes(),
        }
    }
}

impl AuthVerifier for TokenVerifier {
    fn verify(&self, token: Option<&str>) -> Result<()> {
        let presented = token.unwrap_or("").as_bytes();
        if constant_time_eq(presented, &self.expected) {
            Ok(())
        } else {
            Err(deny())
        }
    }
}

/// Constant-time byte comparison. `a != b in length` always fails, but
/// without an early return that would leak the length comparison itself.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    // Fold length mismatch into the final bit so we still compare over
    // `max(a.len(), b.len())` bytes.
    let max = a.len().max(b.len());
    let mut diff: u8 = (a.len() ^ b.len()) as u8;
    for i in 0..max {
        let x = *a.get(i).unwrap_or(&0);
        let y = *b.get(i).unwrap_or(&0);
        diff |= x ^ y;
    }
    diff == 0
}

/// Factory: builds the verifier the daemon will run from.
///
/// Precedence:
/// 1. `CTERM_AUTH_TOKEN=<secret>` — use that token directly. Highest
///    priority so operators can smoke-test without touching a keyring.
/// 2. `CTERM_AUTH=on` combined with a `token` passed by the caller (the
///    binary pulls it from the OS keyring and hands it to us). Keeps this
///    crate free of a keyring dependency.
/// 3. Anything else — [`NoopVerifier`]. This is the default; auth is
///    opt-in, so no config means no enforcement.
///
/// The returned pair is `(verifier, enabled)`: `enabled` lets `main.rs`
/// log clearly whether auth is on or off, without inspecting the trait
/// object's concrete type.
#[must_use]
pub fn verifier_from_env(keyring_token: Option<String>) -> (Verifier, bool) {
    if let Ok(token) = std::env::var("CTERM_AUTH_TOKEN") {
        if !token.is_empty() {
            return (Arc::new(TokenVerifier::new(token)), true);
        }
    }

    let requested = std::env::var("CTERM_AUTH")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "on" | "1" | "true"))
        .unwrap_or(false);

    if requested {
        if let Some(token) = keyring_token {
            if !token.is_empty() {
                return (Arc::new(TokenVerifier::new(token)), true);
            }
        }
    }

    (Arc::new(NoopVerifier), false)
}

/// tonic [`Interceptor`] that pulls the bearer token from `authorization`
/// metadata and delegates to an [`AuthVerifier`]. Install on every service
/// via `TerminalServer::with_interceptor` (or the lib's `terminal_server`
/// helper, which accepts an auth parameter in 0.6.0).
#[derive(Clone)]
pub struct AuthInterceptor {
    verifier: Verifier,
}

impl AuthInterceptor {
    /// Wraps a shared verifier.
    #[must_use]
    pub fn new(verifier: Verifier) -> Self {
        Self { verifier }
    }
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, request: Request<()>) -> std::result::Result<Request<()>, Status> {
        let token = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_bearer);

        self.verifier
            .verify(token.as_deref())
            .map(|_| request)
            .map_err(|_| Status::unauthenticated("invalid or missing bearer token"))
    }
}

/// Strip the `Bearer ` prefix from an `authorization` header value, keeping
/// the rest verbatim. Returns `None` if the header isn't a bearer scheme.
fn parse_bearer(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    // ASCII case-insensitive on the scheme word only; the token after is case-sensitive.
    let (scheme, rest) = trimmed.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("Bearer") {
        Some(rest.trim().to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_verifier_accepts_exact_match() {
        let v = TokenVerifier::new("secret");
        assert!(v.verify(Some("secret")).is_ok());
    }

    #[test]
    fn token_verifier_rejects_wrong_token() {
        let v = TokenVerifier::new("secret");
        assert!(v.verify(Some("other")).is_err());
    }

    #[test]
    fn token_verifier_rejects_missing_token() {
        let v = TokenVerifier::new("secret");
        assert!(v.verify(None).is_err());
    }

    #[test]
    fn token_verifier_rejects_empty_presented() {
        let v = TokenVerifier::new("secret");
        assert!(v.verify(Some("")).is_err());
    }

    #[test]
    fn token_verifier_rejects_prefix_of_secret() {
        let v = TokenVerifier::new("secretlong");
        assert!(v.verify(Some("secret")).is_err());
    }

    #[test]
    fn constant_time_eq_ok() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn parse_bearer_variants() {
        assert_eq!(parse_bearer("Bearer abc"), Some("abc".to_owned()));
        assert_eq!(parse_bearer("bearer abc"), Some("abc".to_owned()));
        assert_eq!(parse_bearer("  Bearer   xyz "), Some("xyz".to_owned()));
        assert_eq!(parse_bearer("Basic abc"), None);
        assert_eq!(parse_bearer("Bearer"), None);
    }

    // Env-touching tests live in the integration test file (tests/auth_env.rs)
    // to avoid racing with other unit tests in parallel — this module stays
    // pure so forbid(unsafe_code) can stand.
}
