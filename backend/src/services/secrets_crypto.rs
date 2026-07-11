//! Envelope encryption for the Secrets subsystem. The control plane is the
//! sole cryptographic authority: every secret value is encrypted under a
//! fresh per-secret 32-byte data-encryption key (DEK) with AES-256-GCM, and
//! the DEK is wrapped by the master key from `SECRETS_MASTER_KEY`. Postgres
//! only ever sees ciphertext, nonces, and the wrapped DEK; decrypted values
//! exist briefly in `Zeroizing` buffers at scheduler dispatch and are wiped
//! on drop. The secret row's UUID rides along as AEAD associated data, so a
//! ciphertext copied onto another row fails authentication.

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use zeroize::Zeroizing;

/// AES-GCM standard 96-bit nonce.
const NONCE_LEN: usize = 12;
/// Only one master-key generation exists; the column is a rotation seam.
const KEY_VERSION: i32 = 1;

/// Everything persisted for one secret value. No field is sensitive on its
/// own — decryption requires the master key held only in process memory.
pub struct EncryptedSecret {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub wrapped_dek: Vec<u8>,
    pub dek_nonce: Vec<u8>,
    pub key_version: i32,
}

pub struct SecretsCrypto {
    master_key: Zeroizing<[u8; 32]>,
}

impl SecretsCrypto {
    pub fn new(key: &[u8]) -> anyhow::Result<Self> {
        let key: [u8; 32] = key
            .try_into()
            .map_err(|_| anyhow::anyhow!("secrets master key must be exactly 32 bytes"))?;
        Ok(Self {
            master_key: Zeroizing::new(key),
        })
    }

    /// Envelope-encrypt one value: fresh OS-RNG DEK and nonces per call, so
    /// two encryptions of the same plaintext never produce related output.
    /// `aad` is the secret row's UUID bytes.
    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> anyhow::Result<EncryptedSecret> {
        let mut dek = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(dek.as_mut());
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let mut dek_nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut dek_nonce);

        let value_cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(dek.as_ref()));
        let ciphertext = value_cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad })
            .map_err(|_| anyhow::anyhow!("secret encryption failed"))?;

        let master_cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(self.master_key.as_ref()));
        let wrapped_dek = master_cipher
            .encrypt(
                Nonce::from_slice(&dek_nonce),
                Payload {
                    msg: dek.as_ref(),
                    aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("secret key wrapping failed"))?;

        Ok(EncryptedSecret {
            ciphertext,
            nonce: nonce.to_vec(),
            wrapped_dek,
            dek_nonce: dek_nonce.to_vec(),
            key_version: KEY_VERSION,
        })
    }

    /// Unwrap the DEK and decrypt the value. Errors are deliberately opaque
    /// (wrong key, tampering, and row-swap are indistinguishable); callers
    /// map them to a static category and never surface detail.
    pub fn decrypt(&self, enc: &EncryptedSecret, aad: &[u8]) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        if enc.key_version != KEY_VERSION {
            anyhow::bail!("secret decryption failed");
        }
        if enc.nonce.len() != NONCE_LEN || enc.dek_nonce.len() != NONCE_LEN {
            anyhow::bail!("secret decryption failed");
        }

        let master_cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(self.master_key.as_ref()));
        let dek = Zeroizing::new(
            master_cipher
                .decrypt(
                    Nonce::from_slice(&enc.dek_nonce),
                    Payload {
                        msg: enc.wrapped_dek.as_slice(),
                        aad,
                    },
                )
                .map_err(|_| anyhow::anyhow!("secret decryption failed"))?,
        );
        if dek.len() != 32 {
            anyhow::bail!("secret decryption failed");
        }

        let value_cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(dek.as_ref()));
        let plaintext = value_cipher
            .decrypt(
                Nonce::from_slice(&enc.nonce),
                Payload {
                    msg: enc.ciphertext.as_slice(),
                    aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("secret decryption failed"))?;
        Ok(Zeroizing::new(plaintext))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crypto() -> SecretsCrypto {
        SecretsCrypto::new(&[7u8; 32]).unwrap()
    }

    #[test]
    fn rejects_wrong_key_length() {
        assert!(SecretsCrypto::new(&[0u8; 16]).is_err());
        assert!(SecretsCrypto::new(&[0u8; 33]).is_err());
    }

    #[test]
    fn round_trip() {
        let c = crypto();
        let aad = uuid::Uuid::new_v4();
        let enc = c.encrypt(b"hunter2-but-longer", aad.as_bytes()).unwrap();
        let out = c.decrypt(&enc, aad.as_bytes()).unwrap();
        assert_eq!(out.as_slice(), b"hunter2-but-longer");
    }

    #[test]
    fn same_plaintext_encrypts_differently() {
        let c = crypto();
        let aad = uuid::Uuid::new_v4();
        let a = c.encrypt(b"same-value", aad.as_bytes()).unwrap();
        let b = c.encrypt(b"same-value", aad.as_bytes()).unwrap();
        assert_ne!(a.ciphertext, b.ciphertext);
        assert_ne!(a.wrapped_dek, b.wrapped_dek);
        assert_ne!(a.nonce, b.nonce);
    }

    #[test]
    fn tampering_is_rejected() {
        let c = crypto();
        let aad = uuid::Uuid::new_v4();
        let enc = c.encrypt(b"tamper-target", aad.as_bytes()).unwrap();

        let mut bad = EncryptedSecret {
            ciphertext: enc.ciphertext.clone(),
            nonce: enc.nonce.clone(),
            wrapped_dek: enc.wrapped_dek.clone(),
            dek_nonce: enc.dek_nonce.clone(),
            key_version: enc.key_version,
        };
        bad.ciphertext[0] ^= 1;
        assert!(c.decrypt(&bad, aad.as_bytes()).is_err());

        bad.ciphertext = enc.ciphertext.clone();
        bad.wrapped_dek[0] ^= 1;
        assert!(c.decrypt(&bad, aad.as_bytes()).is_err());

        bad.wrapped_dek = enc.wrapped_dek.clone();
        bad.nonce[0] ^= 1;
        assert!(c.decrypt(&bad, aad.as_bytes()).is_err());
    }

    #[test]
    fn swapped_row_id_fails_authentication() {
        let c = crypto();
        let enc = c
            .encrypt(b"bound-to-row", uuid::Uuid::new_v4().as_bytes())
            .unwrap();
        assert!(c.decrypt(&enc, uuid::Uuid::new_v4().as_bytes()).is_err());
    }

    #[test]
    fn wrong_master_key_fails() {
        let aad = uuid::Uuid::new_v4();
        let enc = crypto().encrypt(b"key-mismatch", aad.as_bytes()).unwrap();
        let other = SecretsCrypto::new(&[8u8; 32]).unwrap();
        assert!(other.decrypt(&enc, aad.as_bytes()).is_err());
    }
}
