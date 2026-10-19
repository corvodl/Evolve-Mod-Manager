//! Offline Developer lock.
//!
//! Only a salted Argon2id verifier is compiled into the client. The password
//! itself is never stored in the source, executable resources or logs.
//! This is a local UI gate, NOT a security boundary against someone who can
//! modify the executable or perform offline password guesses.

use argon2::{
    Argon2,
    PasswordHash,
    PasswordVerifier,
};

// PHC format: Argon2id v19, 64 MiB, three iterations, one lane, random salt.
// The source contains a one-way verifier only. Do not add plaintext fixtures.
const DEVELOPER_VERIFIER: &str =
    "$argon2id$v=19$m=65536,t=3,p=1$7rV5JYtexiPG99mNjtK4NQ$HBfF9eoyudPzPaWzfdcQmTIKtiJTf/vDaMSb012ryAk";

pub(super) fn verify(password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(DEVELOPER_VERIFIER) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::verify;

    #[test]
    fn invalid_or_blank_inputs_remain_locked() {
        assert!(!verify(""));
        assert!(!verify("unrelated-input"));
        assert!(!verify("UNRELATED-INPUT"));
    }
}
