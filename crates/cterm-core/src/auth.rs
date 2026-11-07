//! Authentication seam for the agent-facing control surface.
//!
//! Auth is a *port*: the daemon knows "someone presented a credential of some
//! shape; is it allowed to act?" without knowing how it was issued, stored, or
//! rotated. That keeps the gRPC adapter small, lets the daemon stay
//! testable against an in-memory fake, and lets callers opt into the
//! concrete scheme they want (today: a bearer token in OS keyring; tomorrow:
//! TLS client certs, mTLS, or anything else) without touching service code.
//!
//! The default is [`NoopVerifier`], which accepts everything. This preserves
//! the current "local-first, open on 127.0.0.1" behavior. Flipping to
//! `TokenVerifier` (in `cterm-daemon`) is explicit and opt-in.

use crate::error::{Error, Result};

/// Verifies a credential presented by a gRPC caller.
///
/// Implementations must be side-effect-free, cheap, and safe to share across
/// tasks (`Send + Sync`): the daemon calls this on every RPC.
///
/// The argument is the raw bearer token the client sent (what came after
/// `Bearer ` in the `authorization` metadata), or `None` if the client did
/// not send the header at all. Distinguishing "sent nothing" from "sent the
/// wrong value" is the verifier's business: a strict verifier should reject
/// both the same way so callers can't fingerprint which case they hit.
pub trait AuthVerifier: Send + Sync {
    /// `Ok(())` to allow, `Err(Error::Forbidden)` to reject with a permission
    /// error that the gRPC adapter maps to `UNAUTHENTICATED`.
    fn verify(&self, token: Option<&str>) -> Result<()>;
}

/// The default: accepts every request, with or without a credential.
///
/// This matches cterm's historical behavior: the daemon listens on
/// `127.0.0.1`, and local processes are trusted. Opt in to a real verifier
/// to raise the bar.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopVerifier;

impl AuthVerifier for NoopVerifier {
    fn verify(&self, _token: Option<&str>) -> Result<()> {
        Ok(())
    }
}

/// Convenience: build the reject that the gRPC adapter turns into
/// `UNAUTHENTICATED`. Implementations should prefer this over constructing
/// their own `Error::Forbidden` so the operator-visible message stays
/// consistent.
#[must_use]
pub fn deny() -> Error {
    Error::Forbidden("authentication required".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_accepts_without_token() {
        assert!(NoopVerifier.verify(None).is_ok());
    }

    #[test]
    fn noop_accepts_with_token() {
        assert!(NoopVerifier.verify(Some("anything")).is_ok());
    }
}
