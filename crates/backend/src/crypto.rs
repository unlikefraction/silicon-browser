//! Small authenticated encryption boundary for provider URLs and command logs.

use aes_gcm::aead::{Aead, KeyInit, OsRng, Payload, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct SecretBox {
    cipher: Aes256Gcm,
    key_id: String,
}

impl SecretBox {
    pub fn new(key: &[u8; 32]) -> Self {
        let digest = Sha256::digest(key);
        Self {
            cipher: Aes256Gcm::new_from_slice(key).expect("AES-256 accepts a 32-byte key"),
            // This is a non-secret key selector, not key material. Keeping it in
            // the envelope makes rotation failures explicit instead of looking
            // like arbitrary ciphertext corruption.
            key_id: hex::encode(&digest[..8]),
        }
    }

    /// Seal a value for one logical database field/resource. AES-GCM authenticates
    /// the context as AAD, so a valid ciphertext copied across tenants or columns
    /// cannot be decrypted there.
    pub fn seal_for(&self, context: &str, plaintext: &str) -> Result<String, String> {
        validate_context(context)?;
        let mut nonce = [0_u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let encrypted = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext.as_bytes(), aad: context.as_bytes() })
            .map_err(|_| "encryption failed")?;
        let mut payload = Vec::with_capacity(nonce.len() + encrypted.len());
        payload.extend_from_slice(&nonce);
        payload.extend_from_slice(&encrypted);
        Ok(format!("sb1.{}.{}", self.key_id, URL_SAFE_NO_PAD.encode(payload)))
    }

    pub fn open_for(&self, context: &str, envelope: &str) -> Result<String, String> {
        validate_context(context)?;
        let mut parts = envelope.split('.');
        if parts.next() != Some("sb1") || parts.next() != Some(self.key_id.as_str()) {
            return Err("encrypted value uses an unknown format or key".into());
        }
        let encoded = parts.next().ok_or("encrypted value is malformed")?;
        if parts.next().is_some() {
            return Err("encrypted value is malformed".into());
        }
        let payload = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| "encrypted value is malformed")?;
        let (nonce, encrypted) = payload.split_at_checked(12).ok_or("encrypted value is too short")?;
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), Payload { msg: encrypted, aad: context.as_bytes() })
            .map_err(|_| "encrypted value failed authentication")?;
        String::from_utf8(plaintext).map_err(|_| "encrypted value is not UTF-8".into())
    }
}

fn validate_context(context: &str) -> Result<(), String> {
    if context.is_empty() || context.len() > 1_024 || context.bytes().any(|byte| byte == 0) {
        Err("encryption context is invalid".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_round_trip_and_tampering_fails_closed() {
        let secrets = SecretBox::new(&[7; 32]);
        let sealed = secrets.seal_for("test:runtime-url", "wss://contains-a-secret").unwrap();
        assert!(sealed.starts_with("sb1."));
        assert!(!sealed.contains("secret"));
        assert_eq!(secrets.open_for("test:runtime-url", &sealed).unwrap(), "wss://contains-a-secret");
        let mut bytes = sealed.into_bytes();
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
        assert!(secrets.open_for("test:runtime-url", &String::from_utf8(bytes).unwrap()).is_err());
    }

    #[test]
    fn contexts_and_key_ids_prevent_ciphertext_swaps() {
        let first = SecretBox::new(&[7; 32]);
        let second = SecretBox::new(&[8; 32]);
        let sealed = first.seal_for("org:o1/session:s1/cdp", "wss://secret").unwrap();
        assert_eq!(first.open_for("org:o1/session:s1/cdp", &sealed).unwrap(), "wss://secret");
        assert!(first.open_for("org:o2/session:s1/cdp", &sealed).is_err());
        assert!(first.open_for("org:o1/session:s1/live", &sealed).is_err());
        assert!(second.open_for("org:o1/session:s1/cdp", &sealed).is_err());
    }
}
