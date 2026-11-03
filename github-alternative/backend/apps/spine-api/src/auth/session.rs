use rand::RngCore;

pub const DEFAULT_SESSION_TTL_NANOS: i64 = 7 * 24 * 3600 * 1_000_000_000; // 7 days

pub fn generate_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("sp_sess_{}", hex::encode(bytes))
}

pub fn hash_session_token(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}
