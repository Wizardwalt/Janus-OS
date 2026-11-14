//! Cryptographic primitives for Monolith and Pip-Boy enrollment.
//!
//! The device generates and retains its Ed25519 private key. Janus receives
//! only the public key and a signature over a short-lived, owner-bound challenge.

use anyhow::{bail, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signature, VerifyingKey};

const DOMAIN: &[u8] = b"JANUS-ARIA-DEVICE-ENROLLMENT-V1\0";
const MAX_CHALLENGE_LIFETIME_SECS: i64 = 300;
const MAX_CLOCK_SKEW_SECS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentClaims {
    pub challenge_id: String,
    pub nonce: String,
    pub device_id: String,
    pub device_type: String,
    pub owner_user_id: String,
    pub issued_at_unix: i64,
    pub expires_at_unix: i64,
    pub protocol_version: u32,
}

impl EnrollmentClaims {
    /// Encode fields unambiguously and in a fixed order. Length-prefixing
    /// prevents delimiter ambiguity; the domain tag prevents cross-protocol use.
    pub fn signing_bytes(&self) -> Result<Vec<u8>> {
        validate_text("challenge_id", &self.challenge_id, 128)?;
        validate_text("nonce", &self.nonce, 256)?;
        validate_text("device_id", &self.device_id, 128)?;
        validate_text("owner_user_id", &self.owner_user_id, 128)?;
        if !matches!(self.device_type.as_str(), "monolith" | "pipboy") {
            bail!("unsupported device type");
        }
        if self.protocol_version != 1 {
            bail!("unsupported enrollment protocol version");
        }
        if self.issued_at_unix < 0
            || self.expires_at_unix <= self.issued_at_unix
            || self.expires_at_unix - self.issued_at_unix > MAX_CHALLENGE_LIFETIME_SECS
        {
            bail!("invalid challenge lifetime");
        }

        let mut bytes = Vec::with_capacity(512);
        bytes.extend_from_slice(DOMAIN);
        for field in [
            self.challenge_id.as_str(),
            self.nonce.as_str(),
            self.device_id.as_str(),
            self.device_type.as_str(),
            self.owner_user_id.as_str(),
        ] {
            let length = u32::try_from(field.len())?;
            bytes.extend_from_slice(&length.to_be_bytes());
            bytes.extend_from_slice(field.as_bytes());
        }
        bytes.extend_from_slice(&self.issued_at_unix.to_be_bytes());
        bytes.extend_from_slice(&self.expires_at_unix.to_be_bytes());
        bytes.extend_from_slice(&self.protocol_version.to_be_bytes());
        Ok(bytes)
    }
}

fn validate_text(name: &str, value: &str, max_len: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max_len {
        bail!("invalid {name}");
    }
    Ok(())
}

/// Verify a device proof-of-possession. The caller must additionally consume
/// the challenge atomically in persistent storage before accepting enrollment.
pub fn verify_device_proof(
    claims: &EnrollmentClaims,
    public_key_base64: &str,
    signature_base64: &str,
    now_unix: i64,
) -> Result<()> {
    let bytes = claims.signing_bytes()?;
    if claims.issued_at_unix > now_unix.saturating_add(MAX_CLOCK_SKEW_SECS)
        || claims.expires_at_unix <= now_unix
    {
        bail!("enrollment challenge is not currently valid");
    }

    let public_key = URL_SAFE_NO_PAD.decode(public_key_base64)?;
    let public_key: [u8; 32] = public_key
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid Ed25519 public key length"))?;
    let verifying_key = VerifyingKey::from_bytes(&public_key)?;
    let signature_bytes = URL_SAFE_NO_PAD.decode(signature_base64)?;
    let signature = Signature::from_slice(&signature_bytes)?;
    verifying_key
        .verify_strict(&bytes, &signature)
        .map_err(|_| anyhow::anyhow!("device proof-of-possession failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use ed25519_dalek::{Signer, SigningKey};

    fn claims() -> EnrollmentClaims {
        EnrollmentClaims {
            challenge_id: "challenge-123".into(),
            nonce: "a-random-server-nonce".into(),
            device_id: "monolith-0001".into(),
            device_type: "monolith".into(),
            owner_user_id: "owner-42".into(),
            issued_at_unix: 1_000,
            expires_at_unix: 1_180,
            protocol_version: 1,
        }
    }

    fn proof(claims: &EnrollmentClaims, key: &SigningKey) -> (String, String) {
        let public_key = URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes());
        let signature = URL_SAFE_NO_PAD.encode(key.sign(&claims.signing_bytes().unwrap()).to_bytes());
        (public_key, signature)
    }

    #[test]
    fn accepts_a_valid_owner_bound_device_signature() {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let claims = claims();
        let (public_key, signature) = proof(&claims, &key);
        assert!(verify_device_proof(&claims, &public_key, &signature, 1_050).is_ok());
    }

    #[test]
    fn rejects_a_signature_from_another_device_key() {
        let claims = claims();
        let (_, signature) = proof(&claims, &SigningKey::from_bytes(&[7u8; 32]));
        let (wrong_key, _) = proof(&claims, &SigningKey::from_bytes(&[8u8; 32]));
        assert!(verify_device_proof(&claims, &wrong_key, &signature, 1_050).is_err());
    }

    #[test]
    fn rejects_tampered_owner_or_device_claims() {
        let claims = claims();
        let (public_key, signature) = proof(&claims, &SigningKey::from_bytes(&[7u8; 32]));
        let mut tampered = claims.clone();
        tampered.owner_user_id = "different-owner".into();
        assert!(verify_device_proof(&tampered, &public_key, &signature, 1_050).is_err());
        let mut tampered = claims;
        tampered.device_type = "pipboy".into();
        assert!(verify_device_proof(&tampered, &public_key, &signature, 1_050).is_err());
    }

    #[test]
    fn rejects_expired_and_overlong_challenges() {
        let claims = claims();
        let (public_key, signature) = proof(&claims, &SigningKey::from_bytes(&[7u8; 32]));
        assert!(verify_device_proof(&claims, &public_key, &signature, 1_180).is_err());
        let mut overlong = claims;
        overlong.expires_at_unix = overlong.issued_at_unix + 301;
        assert!(overlong.signing_bytes().is_err());
    }

    #[test]
    fn rejects_unsupported_device_types_and_protocol_versions() {
        let mut claims = claims();
        claims.device_type = "unknown".into();
        assert!(claims.signing_bytes().is_err());
        claims.device_type = "monolith".into();
        claims.protocol_version = 2;
        assert!(claims.signing_bytes().is_err());
    }
}
