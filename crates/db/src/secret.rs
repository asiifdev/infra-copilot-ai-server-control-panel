//! Secret-bearing types. Their `Debug` never prints the value (CLAUDE.md rule 2).

use std::fmt;

/// A text secret (password hash, token hash) that prints as `[REDACTED]`.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretText(String);

impl SecretText {
    /// Wraps a value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw value; call only where it is handed to the database or a verifier.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretText([REDACTED])")
    }
}

/// An encrypted secret as stored: opaque ciphertext plus the id of the master key that sealed it.
///
/// This crate never encrypts or decrypts; that belongs to a crypto layer that is not chosen yet
/// (decision 0009). `Debug` shows only the length and key id.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedSecret {
    /// AES-256-GCM output (nonce and tag included) produced by the crypto layer.
    pub ciphertext: Vec<u8>,
    /// Which master key version sealed it, for rotation.
    pub key_id: String,
}

impl fmt::Debug for SealedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SealedSecret")
            .field(
                "ciphertext",
                &format_args!("[{} bytes]", self.ciphertext.len()),
            )
            .field("key_id", &self.key_id)
            .finish()
    }
}
