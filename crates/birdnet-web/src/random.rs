//! Operating-system randomness, in one place.
//!
//! Session ids, CSP nonces, share keys and stream tokens all need bytes an
//! attacker cannot predict. They used to draw them from
//! `password_hash::rand_core::OsRng`, a re-export that came with the password
//! hashing crate and went when it was upgraded; asking the OS directly says
//! what is meant and has no other reason to change.

/// Fill `bytes` from the operating system's CSPRNG (`getrandom(2)` on Linux).
///
/// # Errors
///
/// The OS refused to produce randomness, which on Linux means `getrandom`
/// itself failed. Every caller decides for itself what to do about that; none
/// of them may carry on with the bytes it passed in.
pub fn fill(bytes: &mut [u8]) -> Result<(), getrandom::Error> {
    getrandom::fill(bytes)
}
