use rand::RngCore;

pub fn generate_pat() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("sp_pat_{}", hex::encode(bytes))
}

pub fn hash_pat(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

pub fn has_scope(scopes: &[String], required: &str) -> bool {
    scopes
        .iter()
        .any(|s| s == required || s == "admin" || s == "*")
}
