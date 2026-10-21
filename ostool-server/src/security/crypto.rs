use anyhow::{Context, ensure};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use rand::RngCore;
use ring::{aead, hmac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Redis/SQL TLS builders need one explicit provider when both TLS backends are linked.
pub fn initialize_tls() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
pub fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub fn proof_hash(purpose: &str, id: uuid::Uuid, code: &str) -> String {
    hash(&format!("ostool:{purpose}:{id}:{code}"))
}
pub fn equal(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}
pub fn validate_password(password: &str) -> anyhow::Result<()> {
    ensure!(
        password.chars().count() >= 15 && password.len() <= 1024,
        "密码需要至少 15 个字符，且不超过 1024 字节"
    );
    let lower = password.to_lowercase();
    ensure!(
        ![
            "passwordpassword",
            "123456789012345",
            "qwertyuiopasdfgh",
            "adminadminadmin"
        ]
        .iter()
        .any(|p| lower == *p)
            && !password.chars().all(|c| password.starts_with(c)),
        "请使用不易猜测的密码或长口令"
    );
    Ok(())
}
pub fn seal(key: &str, context: &str, plaintext: &[u8]) -> anyhow::Result<String> {
    let key = STANDARD.decode(key)?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &key)
            .map_err(|_| anyhow::anyhow!("invalid MFA encryption key"))?,
    );
    let mut nonce = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce);
    let mut ciphertext = plaintext.to_vec();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(context.as_bytes()),
        &mut ciphertext,
    )
    .map_err(|_| anyhow::anyhow!("cannot encrypt MFA secret"))?;
    let mut output = nonce.to_vec();
    output.extend(ciphertext);
    Ok(format!("v1:{}", STANDARD.encode(output)))
}
pub fn open(key: &str, context: &str, value: &str) -> anyhow::Result<Vec<u8>> {
    let mut bytes = STANDARD.decode(
        value
            .strip_prefix("v1:")
            .context("unsupported MFA secret version")?,
    )?;
    ensure!(bytes.len() >= 28, "invalid encrypted MFA secret");
    let nonce: [u8; 12] = bytes[..12].try_into()?;
    let key = STANDARD.decode(key)?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &key)
            .map_err(|_| anyhow::anyhow!("invalid MFA encryption key"))?,
    );
    Ok(key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(context.as_bytes()),
            &mut bytes[12..],
        )
        .map_err(|_| anyhow::anyhow!("cannot decrypt MFA secret"))?
        .to_vec())
}
pub fn totp(secret: &[u8], step: i64) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let tag = hmac::sign(&key, &step.to_be_bytes());
    let bytes = tag.as_ref();
    let offset = (bytes[19] & 15) as usize;
    let value = u32::from_be_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("HMAC SHA1 slice"),
    ) & 0x7fffffff;
    format!("{:06}", value % 1_000_000)
}
pub fn verify_totp(secret: &[u8], code: &str, now: i64, last: i64) -> Option<i64> {
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    ((now / 30 - 1)..=(now / 30 + 1))
        .filter(|step| *step > last)
        .find(|step| equal(&totp(secret, *step), code))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn totp_matches_rfc6238_sha1_vectors_and_rejects_replay() {
        let secret = b"12345678901234567890";
        for (seconds, code) in [
            (59, "287082"),
            (1111111109, "081804"),
            (1111111111, "050471"),
            (1234567890, "005924"),
            (2000000000, "279037"),
            (20000000000, "353130"),
        ] {
            assert_eq!(totp(secret, seconds / 30), code);
            assert_eq!(verify_totp(secret, code, seconds, -1), Some(seconds / 30));
            assert_eq!(verify_totp(secret, code, seconds, seconds / 30), None);
        }
        assert!(verify_totp(secret, "not-a-code", 59, -1).is_none());
    }
    #[test]
    fn encryption_is_randomized_authenticated_and_bound_to_its_owner() {
        let key = STANDARD.encode([1u8; 32]);
        let encrypted = seal(&key, "user-one", b"top-secret-seed").unwrap();
        assert_eq!(
            open(&key, "user-one", &encrypted).unwrap(),
            b"top-secret-seed"
        );
        assert_ne!(
            encrypted,
            seal(&key, "user-one", b"top-secret-seed").unwrap()
        );
        assert!(open(&key, "user-two", &encrypted).is_err());
        assert!(open(&STANDARD.encode([2u8; 32]), "user-one", &encrypted).is_err());
    }
    #[test]
    fn password_length_counts_characters_and_allows_long_passphrases() {
        assert!(validate_password("密码密码").is_err());
        assert!(validate_password("a long and memorable passphrase").is_ok());
        assert!(validate_password(&"x".repeat(20)).is_err());
        assert!(validate_password(&"a ".repeat(600)).is_err());
    }
}
