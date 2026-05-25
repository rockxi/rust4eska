use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{
    password_hash::{PasswordHasher, SaltString},
    Argon2,
};
use rand::{rngs::OsRng, RngCore};
use zeroize::Zeroize;

pub fn derive_key(password: &str, salt: &str) -> anyhow::Result<[u8; 32]> {
    let mut password_bytes = password.as_bytes().to_vec();
    let salt_bytes = SaltString::from_b64(salt).map_err(|e| anyhow::anyhow!("Invalid salt: {}", e))?;
    
    let argon2 = Argon2::default();
    let mut key = [0u8; 32];
    
    let hash = argon2
        .hash_password(password_bytes.as_slice(), &salt_bytes)
        .map_err(|e| anyhow::anyhow!("Hash error: {}", e))?;
    
    let output = hash.hash.ok_or_else(|| anyhow::anyhow!("No hash output"))?;
    key.copy_from_slice(&output.as_bytes()[..32]);
    
    password_bytes.zeroize();
    Ok(key)
}

pub fn derive_key_simple(password: &str, salt: &[u8]) -> anyhow::Result<[u8; 32]> {
    let mut key = [0u8; 32];
    let argon2 = Argon2::default();
    argon2.hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow::anyhow!("Argon2 error: {}", e))?;
    Ok(key)
}

pub fn encrypt(key: &[u8; 32], data: &[u8]) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    
    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| anyhow::anyhow!("Encryption error: {}", e))?;
    
    Ok((ciphertext, nonce_bytes.to_vec()))
}

pub fn decrypt(key: &[u8; 32], ciphertext: &[u8], nonce_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Nonce::from_slice(nonce_bytes);
    
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("Decryption error: {}", e))?;
    
    Ok(plaintext)
}

pub fn generate_nonce() -> [u8; 12] {
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    nonce
}

pub fn generate_salt() -> String {
    SaltString::generate(&mut OsRng).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encryption_decryption() -> anyhow::Result<()> {
        let key = [42u8; 32];
        let data = b"hello world secret message";
        
        let (encrypted, nonce) = encrypt(&key, data)?;
        assert_ne!(data.to_vec(), encrypted);
        
        let decrypted = decrypt(&key, &encrypted, &nonce)?;
        assert_eq!(data.to_vec(), decrypted);
        Ok(())
    }

    #[test]
    fn test_key_derivation() -> anyhow::Result<()> {
        let password = "my-super-password";
        let salt_bytes = b"fixed-salt";
        
        let key1 = derive_key_simple(password, salt_bytes)?;
        let key2 = derive_key_simple(password, salt_bytes)?;
        let key3 = derive_key_simple("other", salt_bytes)?;
        
        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        Ok(())
    }
}
