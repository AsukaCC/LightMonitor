use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rand::RngCore;
use std::fs;
use std::path::Path;

const PREFIX: &str = "enc:v1:";

#[derive(Clone)]
pub struct CredentialCipher {
    cipher: Aes256Gcm,
}

impl CredentialCipher {
    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        let key = if path.is_file() {
            let encoded = fs::read_to_string(path)
                .with_context(|| format!("read credential key {}", path.display()))?;
            let decoded = STANDARD
                .decode(encoded.trim())
                .context("credential key is not valid base64")?;
            if decoded.len() != 32 {
                bail!("credential key must contain exactly 32 bytes");
            }
            decoded
        } else {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut generated = vec![0_u8; 32];
            rand::thread_rng().fill_bytes(&mut generated);
            fs::write(path, format!("{}\n", STANDARD.encode(&generated)))?;
            set_private_permissions(path)?;
            generated
        };

        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| anyhow::anyhow!("failed to initialize credential cipher"))?;
        Ok(Self { cipher })
    }

    pub fn is_encrypted(value: &str) -> bool {
        value.starts_with(PREFIX)
    }

    pub fn encrypt(&self, plaintext: &str) -> anyhow::Result<String> {
        if plaintext.is_empty() {
            return Ok(String::new());
        }
        let mut nonce_bytes = [0_u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
            .map_err(|_| anyhow::anyhow!("failed to encrypt SSH credential"))?;
        Ok(format!(
            "{PREFIX}{}:{}",
            STANDARD.encode(nonce_bytes),
            STANDARD.encode(ciphertext)
        ))
    }

    pub fn decrypt(&self, stored: &str) -> anyhow::Result<String> {
        if stored.is_empty() {
            return Ok(String::new());
        }
        let Some(payload) = stored.strip_prefix(PREFIX) else {
            return Ok(stored.to_string());
        };
        let (nonce, ciphertext) = payload
            .split_once(':')
            .context("invalid encrypted SSH credential")?;
        let nonce = STANDARD.decode(nonce)?;
        if nonce.len() != 12 {
            bail!("invalid encrypted SSH credential nonce");
        }
        let ciphertext = STANDARD.decode(ciphertext)?;
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
            .map_err(|_| anyhow::anyhow!("failed to decrypt SSH credential"))?;
        String::from_utf8(plaintext).context("SSH credential is not valid UTF-8")
    }
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn encrypts_with_random_nonces_and_decrypts() {
        let path = std::env::temp_dir().join(format!("lightmonitor-key-{}", Uuid::new_v4()));
        let cipher = CredentialCipher::load_or_create(&path).unwrap();
        let first = cipher.encrypt("secret").unwrap();
        let second = cipher.encrypt("secret").unwrap();
        assert_ne!(first, second);
        assert!(CredentialCipher::is_encrypted(&first));
        assert_eq!(cipher.decrypt(&first).unwrap(), "secret");
        let _ = fs::remove_file(path);
    }
}
