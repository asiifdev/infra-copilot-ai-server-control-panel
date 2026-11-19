//! Sealing secrets at rest (ADR 0021, CLAUDE.md rule 10, docs/design.md 3.1).
//!
//! * Algorithm: AES-256-GCM from `ring`, a fresh random 96-bit nonce per seal.
//! * Stored form: `version (1 byte) || nonce (12) || ciphertext || tag (16)`.
//! * The associated data binds a ciphertext to its **org, purpose and key id**, so a sealed value
//!   copied to another org's row, another purpose, or re-labelled with another key id fails to
//!   open instead of decrypting.
//! * The master key never touches the database. It comes from the environment as
//!   `<key_id>:<64 hex chars>`; retired keys stay available for opening so rotation does not
//!   strand old rows.
//!
//! Plaintext lives in memory only inside a [`Secret`] or a [`zeroize::Zeroizing`] buffer.

use core_domain::secret::Secret;
use db::{Connection, OrgId, SealedSecret, SecretId};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use std::fmt;
use zeroize::{Zeroize, Zeroizing};

/// Format version byte of a sealed value.
const VERSION: u8 = 1;
/// AES-256 key length.
const KEY_LEN: usize = 32;
/// GCM tag length.
const TAG_LEN: usize = 16;
/// Name of the environment variable holding the current key.
pub const ENV_KEY: &str = "NODIVARA_MASTER_KEY";
/// Name of the environment variable holding retired keys (comma separated), kept for opening.
pub const ENV_RETIRED_KEYS: &str = "NODIVARA_MASTER_KEYS_RETIRED";

/// Why a vault operation failed. Closed set; never carries key material or plaintext.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultError {
    /// The key specification is not `<id>:<64 hex chars>`.
    BadKeySpec,
    /// The environment variable is not set.
    NoKey,
    /// The sealed value names a key id this vault does not hold.
    UnknownKey,
    /// The sealed value is malformed, was tampered with, or belongs to another org or purpose.
    Corrupt,
    /// The system random source failed.
    Random,
    /// The opened bytes are not valid UTF-8 text.
    NotText,
    /// The database refused the operation.
    Database,
    /// The secret does not exist in this org.
    NotFound,
}

impl VaultError {
    /// Stable machine code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::BadKeySpec => "vault_bad_key_spec",
            Self::NoKey => "vault_no_key",
            Self::UnknownKey => "vault_unknown_key",
            Self::Corrupt => "vault_corrupt",
            Self::Random => "vault_random",
            Self::NotText => "vault_not_text",
            Self::Database => "vault_database",
            Self::NotFound => "vault_not_found",
        }
    }
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for VaultError {}

impl From<db::Error> for VaultError {
    fn from(error: db::Error) -> Self {
        match error {
            db::Error::NotFound => Self::NotFound,
            _ => Self::Database,
        }
    }
}

/// A 256-bit key. Wiped on drop; `Debug` shows nothing.
struct MasterKey([u8; KEY_LEN]);

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

struct NamedKey {
    id: String,
    key: LessSafeKey,
    // Kept so the key material is wiped when the vault is dropped.
    _material: MasterKey,
}

/// Holds the current key and any retired ones.
pub struct Vault {
    current: NamedKey,
    retired: Vec<NamedKey>,
    rng: SystemRandom,
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Vault(current={}, retired={})",
            self.current.id,
            self.retired.len()
        )
    }
}

impl Vault {
    /// Builds a vault from key specifications `<id>:<64 hex chars>`.
    ///
    /// # Errors
    /// [`VaultError::BadKeySpec`] for a malformed spec or a repeated key id.
    pub fn new(current: &str, retired: &[&str]) -> Result<Self, VaultError> {
        let current = parse_key(current)?;
        let mut keys = Vec::with_capacity(retired.len());
        for spec in retired {
            let key = parse_key(spec)?;
            if key.id == current.id || keys.iter().any(|k: &NamedKey| k.id == key.id) {
                return Err(VaultError::BadKeySpec);
            }
            keys.push(key);
        }
        Ok(Self {
            current,
            retired: keys,
            rng: SystemRandom::new(),
        })
    }

    /// Reads [`ENV_KEY`] and [`ENV_RETIRED_KEYS`].
    ///
    /// # Errors
    /// [`VaultError::NoKey`] when the current key is unset; [`VaultError::BadKeySpec`] otherwise.
    pub fn from_env() -> Result<Self, VaultError> {
        let current = Zeroizing::new(std::env::var(ENV_KEY).map_err(|_| VaultError::NoKey)?);
        let retired = Zeroizing::new(std::env::var(ENV_RETIRED_KEYS).unwrap_or_default());
        let specs: Vec<&str> = retired
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        Self::new(current.trim(), &specs)
    }

    /// The id new secrets are sealed under.
    #[must_use]
    pub fn current_key_id(&self) -> &str {
        &self.current.id
    }

    /// Seals `plaintext` for `org` and `purpose`.
    ///
    /// # Errors
    /// [`VaultError::Random`] if the system random source fails.
    pub fn seal(
        &self,
        org: &OrgId,
        purpose: &str,
        plaintext: &[u8],
    ) -> Result<SealedSecret, VaultError> {
        let mut nonce = [0_u8; NONCE_LEN];
        self.rng.fill(&mut nonce).map_err(|_| VaultError::Random)?;
        self.seal_with_nonce(org, purpose, plaintext, nonce)
    }

    fn seal_with_nonce(
        &self,
        org: &OrgId,
        purpose: &str,
        plaintext: &[u8],
        nonce: [u8; NONCE_LEN],
    ) -> Result<SealedSecret, VaultError> {
        let mut out = Vec::with_capacity(1 + NONCE_LEN + plaintext.len() + TAG_LEN);
        out.push(VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(plaintext);
        let aad = associated_data(org, purpose, &self.current.id);
        let mut body = out.split_off(1 + NONCE_LEN);
        let result = self.current.key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad.as_slice()),
            &mut body,
        );
        if result.is_err() {
            body.zeroize();
            return Err(VaultError::Corrupt);
        }
        out.extend_from_slice(&body);
        Ok(SealedSecret {
            ciphertext: out,
            key_id: self.current.id.clone(),
        })
    }

    /// Opens a sealed value back into bytes that are wiped on drop.
    ///
    /// # Errors
    /// [`VaultError::UnknownKey`] for a key id this vault lacks; [`VaultError::Corrupt`] for a
    /// malformed or tampered value, or one sealed for another org or purpose.
    pub fn open(
        &self,
        org: &OrgId,
        purpose: &str,
        sealed: &SealedSecret,
    ) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        let named = std::iter::once(&self.current)
            .chain(&self.retired)
            .find(|k| k.id == sealed.key_id)
            .ok_or(VaultError::UnknownKey)?;
        let bytes = &sealed.ciphertext;
        if bytes.len() < 1 + NONCE_LEN + TAG_LEN || bytes.first() != Some(&VERSION) {
            return Err(VaultError::Corrupt);
        }
        let nonce = Nonce::try_assume_unique_for_key(&bytes[1..=NONCE_LEN])
            .map_err(|_| VaultError::Corrupt)?;
        let mut buffer = Zeroizing::new(bytes[1 + NONCE_LEN..].to_vec());
        let aad = associated_data(org, purpose, &sealed.key_id);
        let plain_len = named
            .key
            .open_in_place(nonce, Aad::from(aad.as_slice()), &mut buffer)
            .map_err(|_| VaultError::Corrupt)?
            .len();
        buffer.truncate(plain_len);
        Ok(buffer)
    }

    /// Opens a sealed value as text.
    ///
    /// # Errors
    /// As [`Vault::open`], plus [`VaultError::NotText`] for bytes that are not UTF-8.
    pub fn open_secret(
        &self,
        org: &OrgId,
        purpose: &str,
        sealed: &SealedSecret,
    ) -> Result<Secret, VaultError> {
        let bytes = self.open(org, purpose, sealed)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| VaultError::NotText)?;
        Ok(Secret::new(text))
    }

    /// Seals `plaintext` and stores it for the org.
    ///
    /// # Errors
    /// Sealing or database failure.
    pub fn store(
        &self,
        conn: &Connection,
        org: &OrgId,
        purpose: &str,
        plaintext: &[u8],
    ) -> Result<SecretId, VaultError> {
        let sealed = self.seal(org, purpose, plaintext)?;
        Ok(db::put_secret(conn, org, purpose, &sealed)?)
    }

    /// Loads a stored secret as text. The purpose is read from the row, so a caller cannot ask
    /// for a secret under a different purpose than it was sealed for.
    ///
    /// # Errors
    /// [`VaultError::NotFound`] when it is not in this org, else as [`Vault::open_secret`].
    pub fn load(
        &self,
        conn: &Connection,
        org: &OrgId,
        id: &SecretId,
    ) -> Result<Secret, VaultError> {
        let (purpose, sealed) = db::get_secret(conn, org, id)?;
        self.open_secret(org, &purpose, &sealed)
    }

    /// Re-seals every secret of the org that is not under the current key, and returns how
    /// many were rotated. Run after changing the current key.
    ///
    /// # Errors
    /// Any open, seal or database failure; secrets already rotated stay rotated.
    pub fn rotate_org(
        &self,
        conn: &Connection,
        org: &OrgId,
        ids: &[SecretId],
    ) -> Result<usize, VaultError> {
        let mut rotated = 0;
        for id in ids {
            let (purpose, sealed) = db::get_secret(conn, org, id)?;
            if sealed.key_id == self.current.id {
                continue;
            }
            let plain = self.open(org, &purpose, &sealed)?;
            let fresh = self.seal(org, &purpose, &plain)?;
            db::replace_secret(conn, org, id, &fresh)?;
            rotated += 1;
        }
        Ok(rotated)
    }
}

/// `nodivara-vault/v1 | org | purpose | key id`, each length-prefixed so no value can bleed into
/// the next.
fn associated_data(org: &OrgId, purpose: &str, key_id: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(64 + purpose.len() + key_id.len());
    aad.extend_from_slice(b"nodivara-vault/v1");
    for part in [org.as_str(), purpose, key_id] {
        aad.extend_from_slice(&(part.len() as u64).to_be_bytes());
        aad.extend_from_slice(part.as_bytes());
    }
    aad
}

fn parse_key(spec: &str) -> Result<NamedKey, VaultError> {
    let (id, hex) = spec.split_once(':').ok_or(VaultError::BadKeySpec)?;
    let id_ok = !id.is_empty()
        && id.len() <= 100
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if !id_ok || hex.len() != KEY_LEN * 2 {
        return Err(VaultError::BadKeySpec);
    }
    let mut material = MasterKey([0; KEY_LEN]);
    for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
        let hi = nibble(pair[0]).ok_or(VaultError::BadKeySpec)?;
        let lo = nibble(pair[1]).ok_or(VaultError::BadKeySpec)?;
        material.0[i] = (hi << 4) | lo;
    }
    let unbound = UnboundKey::new(&AES_256_GCM, &material.0).map_err(|_| VaultError::BadKeySpec)?;
    Ok(NamedKey {
        id: id.to_owned(),
        key: LessSafeKey::new(unbound),
        _material: material,
    })
}

const fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
