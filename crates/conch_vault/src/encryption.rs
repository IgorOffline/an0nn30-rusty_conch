use crate::error::VaultError;
use crate::model::Vault;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::Argon2;
use rand::RngCore;
use std::path::Path;

const MAGIC: &[u8; 8] = b"CONCHVLT";
const FORMAT_VERSION: u32 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

const ARGON2_M_COST: u32 = 65536;
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 4;

pub fn derive_key(password: &[u8], salt: &[u8]) -> Result<[u8; KEY_LEN], VaultError> {
    let params = argon2::Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(KEY_LEN))
        .map_err(|e| VaultError::Encryption(e.to_string()))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon2
        .hash_password_into(password, salt, &mut key)
        .map_err(|e| VaultError::Encryption(e.to_string()))?;
    Ok(key)
}

pub fn encrypt_vault(vault: &Vault, password: &[u8]) -> Result<Vec<u8>, VaultError> {
    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    let key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| VaultError::Encryption(e.to_string()))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let payload = bincode::serialize(vault)
        .map_err(|e| VaultError::Serialization(e.to_string()))?;
    let ciphertext = cipher
        .encrypt(nonce, payload.as_ref())
        .map_err(|e| VaultError::Encryption(e.to_string()))?;
    let mut output = Vec::new();
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

pub fn decrypt_vault(data: &[u8], password: &[u8]) -> Result<Vault, VaultError> {
    let header_len = MAGIC.len() + 4 + SALT_LEN + NONCE_LEN;
    if data.len() < header_len {
        return Err(VaultError::Corrupted("file too short".into()));
    }
    if &data[..8] != MAGIC {
        return Err(VaultError::Corrupted("invalid magic bytes".into()));
    }
    let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
    if version != FORMAT_VERSION {
        return Err(VaultError::Corrupted(format!("unsupported version: {version}")));
    }
    let salt = &data[12..12 + SALT_LEN];
    let nonce_bytes = &data[12 + SALT_LEN..12 + SALT_LEN + NONCE_LEN];
    let ciphertext = &data[header_len..];
    let key = derive_key(password, salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| VaultError::Encryption(e.to_string()))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| VaultError::WrongPassword)?;
    bincode::deserialize(&plaintext)
        .map_err(|e| VaultError::Serialization(e.to_string()))
}

pub fn generate_salt() -> [u8; SALT_LEN] {
    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

pub fn save_vault_file(path: &Path, vault: &Vault, password: &[u8]) -> Result<(), VaultError> {
    let data = encrypt_vault(vault, password)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, data)?;
    Ok(())
}

pub struct CachedKey {
    pub derived_key: [u8; KEY_LEN],
    pub salt: [u8; SALT_LEN],
}

pub fn save_vault_file_with_key(path: &Path, vault: &Vault, cached: &CachedKey) -> Result<(), VaultError> {
    let cipher = Aes256Gcm::new_from_slice(&cached.derived_key)
        .map_err(|e| VaultError::Encryption(e.to_string()))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let payload = bincode::serialize(vault)
        .map_err(|e| VaultError::Serialization(e.to_string()))?;
    let ciphertext = cipher.encrypt(nonce, payload.as_ref())
        .map_err(|e| VaultError::Encryption(e.to_string()))?;
    let mut output = Vec::new();
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    output.extend_from_slice(&cached.salt);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, output)?;
    Ok(())
}

pub fn load_vault_file(path: &Path, password: &[u8]) -> Result<(Vault, CachedKey), VaultError> {
    if !path.exists() {
        return Err(VaultError::NotFound);
    }
    let data = std::fs::read(path)?;
    let header_len = MAGIC.len() + 4 + SALT_LEN + NONCE_LEN;
    if data.len() < header_len {
        return Err(VaultError::Corrupted("file too short".into()));
    }
    if &data[..8] != MAGIC {
        return Err(VaultError::Corrupted("invalid magic bytes".into()));
    }
    let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
    if version != FORMAT_VERSION {
        return Err(VaultError::Corrupted(format!("unsupported version: {version}")));
    }
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&data[12..12 + SALT_LEN]);
    let derived_key = derive_key(password, &salt)?;
    let nonce_bytes = &data[12 + SALT_LEN..12 + SALT_LEN + NONCE_LEN];
    let ciphertext = &data[header_len..];
    let cipher = Aes256Gcm::new_from_slice(&derived_key)
        .map_err(|e| VaultError::Encryption(e.to_string()))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext)
        .map_err(|_| VaultError::WrongPassword)?;
    let vault: Vault = bincode::deserialize(&plaintext)
        .map_err(|e| VaultError::Serialization(e.to_string()))?;
    Ok((vault, CachedKey { derived_key, salt }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AuthMethod, VaultAccount, VaultSettings};

    fn make_test_vault() -> Vault {
        Vault {
            version: 1,
            accounts: vec![VaultAccount {
                id: uuid::Uuid::new_v4(),
                display_name: "Test".into(),
                username: "testuser".into(),
                auth: AuthMethod::Password("secret123".into()),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }],
            settings: VaultSettings::default(),
        }
    }

    #[test]
    fn derive_key_deterministic_for_same_inputs() {
        let password = b"master-password";
        let salt = b"1234567890123456";
        let key1 = derive_key(password, salt).unwrap();
        let key2 = derive_key(password, salt).unwrap();
        assert_eq!(key1, key2);
    }

    #[test]
    fn derive_key_different_for_different_passwords() {
        let salt = b"1234567890123456";
        let key1 = derive_key(b"password1", salt).unwrap();
        let key2 = derive_key(b"password2", salt).unwrap();
        assert_ne!(key1, key2);
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let vault = make_test_vault();
        let password = b"test-master-password";
        let encrypted = encrypt_vault(&vault, password).unwrap();
        let decrypted = decrypt_vault(&encrypted, password).unwrap();
        assert_eq!(decrypted.version, vault.version);
        assert_eq!(decrypted.accounts.len(), 1);
        assert_eq!(decrypted.accounts[0].username, "testuser");
    }

    #[test]
    fn decrypt_with_wrong_password_fails() {
        let vault = make_test_vault();
        let encrypted = encrypt_vault(&vault, b"correct").unwrap();
        let result = decrypt_vault(&encrypted, b"wrong");
        assert!(matches!(result, Err(VaultError::WrongPassword)));
    }

    #[test]
    fn decrypt_truncated_data_fails() {
        let result = decrypt_vault(b"too short", b"password");
        assert!(matches!(result, Err(VaultError::Corrupted(_))));
    }

    #[test]
    fn decrypt_bad_magic_fails() {
        let mut data = vec![0u8; 100];
        data[..8].copy_from_slice(b"BADMAGIC");
        let result = decrypt_vault(&data, b"password");
        assert!(matches!(result, Err(VaultError::Corrupted(_))));
    }

    #[test]
    fn save_and_load_vault_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let vault = make_test_vault();
        let password = b"file-test-password";
        save_vault_file(&path, &vault, password).unwrap();
        assert!(path.exists());
        let (loaded, _cached) = load_vault_file(&path, password).unwrap();
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.accounts[0].username, "testuser");
    }

    #[test]
    fn load_nonexistent_vault_file_returns_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.enc");
        let result = load_vault_file(&path, b"password");
        assert!(matches!(result, Err(VaultError::NotFound)));
    }
}
