use crate::core::error::ForgeError;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

pub fn validate_password_strength(password: &str) -> Result<(), ForgeError> {
    if password.len() < 8 {
        return Err(ForgeError::BadRequest(
            "Password must be at least 8 characters long".into(),
        ));
    }
    let has_uppercase = password.chars().any(|c| c.is_uppercase());
    let has_lowercase = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());

    if !has_uppercase || !has_lowercase || !has_digit {
        return Err(ForgeError::BadRequest(
            "Password must contain at least one uppercase letter, one lowercase letter, and one number".into(),
        ));
    }
    Ok(())
}

pub fn hash_password(password: &str) -> Result<String, ForgeError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| ForgeError::Internal(format!("Password hashing failed: {}", e)))?
        .to_string();
    Ok(hash)
}

pub fn verify_password(password: &str, hash_str: &str) -> Result<bool, ForgeError> {
    let parsed_hash = match PasswordHash::new(hash_str) {
        Ok(h) => h,
        Err(_) => return Ok(false),
    };
    let argon2 = Argon2::default();
    Ok(argon2
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}
