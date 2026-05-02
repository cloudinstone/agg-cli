use std::process::Command;
use aes::Aes128;
use cbc::cipher::{BlockEncryptMut, KeyIvInit};
use cbc::cipher::block_padding::Pkcs7;
use pbkdf2::pbkdf2_hmac;
use sha1::Sha1;

type Aes128CbcEnc = cbc::Encryptor<Aes128>;

const CBC_IV: [u8; 16] = [b' '; 16];
const SALT: &[u8] = b"saltysalt";

pub fn get_macos_safe_storage_password() -> Option<String> {
    let output = Command::new("security")
        .args(&["find-generic-password", "-w", "-s", "Antigravity Safe Storage"])
        .output()
        .ok()?;
    if output.status.success() {
        let password = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !password.is_empty() {
            return Some(password);
        }
    }
    None
}

pub fn derive_key(password: &str) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2_hmac::<Sha1>(password.as_bytes(), SALT, 1003, &mut key);
    key
}

pub fn encrypt_v10(plaintext: &[u8], key: &[u8; 16]) -> Result<Vec<u8>, String> {
    let cipher = Aes128CbcEnc::new_from_slices(key, &CBC_IV)
        .map_err(|e| format!("Init encryptor failed: {}", e))?;
    
    let mut buf = plaintext.to_vec();
    let msg_len = buf.len();
    let pad_len = 16 - (msg_len % 16);
    buf.resize(msg_len + pad_len, 0);
    
    let ciphertext = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buf, msg_len)
        .map_err(|e| format!("AES-CBC encryption failed: {}", e))?
        .to_vec();

    let mut result = Vec::with_capacity(3 + ciphertext.len());
    result.extend_from_slice(b"v10");
    result.extend_from_slice(&ciphertext);
    Ok(result)
}
