use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

const MAGIC: &str = "QSHELL-CONFIG-ENC";
const VERSION: u32 = 1;
const PBKDF2_ROUNDS: u32 = 200_000;

#[derive(Serialize, Deserialize)]
struct EncryptedEnvelope {
    magic: String,
    version: u32,
    kdf: String,
    rounds: u32,
    salt: String,
    nonce: String,
    data: String,
}

/// 通过文件头标识判断是否为加密配置（不解析内容，未解锁时也能用）。
pub fn is_encrypted(raw: &[u8]) -> bool {
    raw.windows(MAGIC.len()).any(|window| window == MAGIC.as_bytes())
}

fn derive_key(password: &str, salt: &[u8], rounds: u32) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, rounds, &mut key);
    key
}

/// 用主密码加密配置（AES-256-GCM + PBKDF2-SHA256）。
pub fn encrypt(raw: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let mut salt = [0u8; 16];
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let key = derive_key(password, &salt, PBKDF2_ROUNDS);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("初始化加密器失败: {}", e))?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), raw)
        .map_err(|e| format!("加密配置失败: {}", e))?;

    let b64 = base64::engine::general_purpose::STANDARD;
    let envelope = EncryptedEnvelope {
        magic: MAGIC.to_string(),
        version: VERSION,
        kdf: "pbkdf2-sha256".to_string(),
        rounds: PBKDF2_ROUNDS,
        salt: b64.encode(salt),
        nonce: b64.encode(nonce_bytes),
        data: b64.encode(ciphertext),
    };
    serde_json::to_vec_pretty(&envelope).map_err(|e| format!("序列化加密配置失败: {}", e))
}

/// 解密配置；密码错误或内容被篡改都会失败。
pub fn decrypt(raw: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let envelope: EncryptedEnvelope =
        serde_json::from_slice(raw).map_err(|_| "配置文件已加密，但格式无法识别".to_string())?;
    if envelope.magic != MAGIC {
        return Err("不是 QShell 加密配置".to_string());
    }
    if envelope.version != VERSION {
        return Err(format!("不支持的加密配置版本: {}", envelope.version));
    }
    let b64 = base64::engine::general_purpose::STANDARD;
    let salt = b64.decode(&envelope.salt).map_err(|e| format!("salt 解析失败: {}", e))?;
    let nonce_bytes = b64.decode(&envelope.nonce).map_err(|e| format!("nonce 解析失败: {}", e))?;
    let data = b64.decode(&envelope.data).map_err(|e| format!("密文解析失败: {}", e))?;

    let key = derive_key(password, &salt, envelope.rounds);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("初始化解密器失败: {}", e))?;
    cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), data.as_ref())
        .map_err(|_| "主密码错误，或配置文件已损坏".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_then_decrypt_roundtrip() {
        let plain = br#"{"devices":[],"managed_hosts":[]}"#;
        let password = "correct horse battery staple";
        let encrypted = encrypt(plain, password).expect("encrypt");
        assert!(is_encrypted(&encrypted));
        assert!(!is_encrypted(plain));
        let decrypted = decrypt(&encrypted, password).expect("decrypt");
        assert_eq!(decrypted, plain);
    }

    #[test]
    fn wrong_password_is_rejected() {
        let encrypted = encrypt(b"secret", "right-password").expect("encrypt");
        assert!(decrypt(&encrypted, "wrong-password").is_err());
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let encrypted = encrypt(b"secret", "pw").expect("encrypt");
        let mut envelope: serde_json::Value = serde_json::from_slice(&encrypted).unwrap();
        envelope["data"] = serde_json::Value::String("AAAA".to_string());
        let tampered = serde_json::to_vec(&envelope).unwrap();
        assert!(decrypt(&tampered, "pw").is_err());
    }
}
