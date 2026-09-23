//! 通用哈希、摘要与 PKCE 挑战码生成工具

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::Rng;
use sha2::{Digest, Sha256};

/// 计算字节切片的 32 字节 SHA-256 原始摘要
pub fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// 计算字节切片的 SHA-256 十六进制小写摘要
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = sha256_bytes(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02x}", b);
    }
    out
}

/// 计算多段字节切片拼接后的 SHA-256 十六进制小写摘要
pub fn sha256_multi_hex(chunks: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for chunk in chunks {
        hasher.update(chunk);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02x}", b);
    }
    out
}

/// 计算字符串的 SHA-256 十六进制小写摘要
pub fn sha256_str(text: &str) -> String {
    sha256_hex(text.as_bytes())
}

/// URL-Safe 无填充 Base64 编码
pub fn base64_url_encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// 统一生成 OAuth2 PKCE 三元组：`(code_verifier, code_challenge, state)`
pub fn generate_pkce_params() -> (String, String, String) {
    let mut verifier_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut verifier_bytes);
    let verifier = base64_url_encode(&verifier_bytes);

    let challenge = base64_url_encode(&sha256_bytes(verifier.as_bytes()));

    let mut state_bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut state_bytes);
    let state = base64_url_encode(&state_bytes);

    (verifier, challenge, state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hex() {
        assert_eq!(
            sha256_str("hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_pkce_params_generation() {
        let (verifier, challenge, state) = generate_pkce_params();
        assert!(!verifier.is_empty());
        assert!(!challenge.is_empty());
        assert!(!state.is_empty());
        assert_ne!(verifier, challenge);
    }
}
