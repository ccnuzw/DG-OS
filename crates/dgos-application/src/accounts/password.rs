//! Password handling for local accounts (D087, D090).
//!
//! Argon2id with m = 19 MiB, t = 2, p = 1; the parameters are stored in the
//! PHC string, so a hash made with other parameters still verifies and is
//! re-hashed after a successful sign-in. Every function here is CPU-bound and
//! is only called from `spawn_blocking`.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Deserializer};
use std::sync::OnceLock;
use zeroize::Zeroizing;

pub const PASSWORD_MEMORY_KIB: u32 = 19 * 1024;
pub const PASSWORD_ITERATIONS: u32 = 2;
pub const PASSWORD_PARALLELISM: u32 = 1;

/// A password received from the renderer. The buffer is zeroed on drop,
/// `Debug` never prints it, and nothing trims it.
pub struct Password(Zeroizing<String>);

impl Password {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    /// Length in Unicode scalar values (the D087 rule).
    pub(crate) fn chars(&self) -> usize {
        self.0.chars().count()
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(***)")
    }
}

impl<'de> Deserialize<'de> for Password {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Password::new)
    }
}

fn argon2() -> Argon2<'static> {
    let params = Params::new(
        PASSWORD_MEMORY_KIB,
        PASSWORD_ITERATIONS,
        PASSWORD_PARALLELISM,
        None,
    )
    .expect("valid Argon2id parameters");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// A PHC string for `password` with a fresh random salt.
pub(crate) fn hash(password: &Password) -> Option<String> {
    let salt = SaltString::generate(&mut OsRng);
    argon2()
        .hash_password(password.expose().as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .ok()
}

/// Whether `password` matches `phc` (with the parameters stored in `phc`);
/// on a match whose parameters differ from the current ones, also a new
/// hash with the current parameters (technical design B08-37).
pub(crate) fn verify(password: &Password, phc: &str) -> (bool, Option<String>) {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return (false, None);
    };
    if argon2()
        .verify_password(password.expose().as_bytes(), &parsed)
        .is_err()
    {
        return (false, None);
    }
    let current = Params::try_from(&parsed).is_ok_and(|params| {
        parsed.algorithm.as_str() == "argon2id"
            && params.m_cost() == PASSWORD_MEMORY_KIB
            && params.t_cost() == PASSWORD_ITERATIONS
            && params.p_cost() == PASSWORD_PARALLELISM
    });
    (true, if current { None } else { hash(password) })
}

/// The same work as a real verification, for an unknown username, so the
/// response time does not tell whether the name exists (D090).
pub(crate) fn verify_dummy(password: &Password) {
    let _ = verify(password, dummy_hash());
}

/// A fixed hash with the current parameters. Computed once; the account
/// service warms it at startup so the first unknown name is not slower.
pub(crate) fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        hash(&Password::new("dgos-unknown-account".into())).expect("dummy password hash")
    })
}
