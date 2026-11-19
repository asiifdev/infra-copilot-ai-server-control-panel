//! SHA-256 used to chain audit entries, backed by the audited `sha2` crate (decision 0012).
//!
//! The NIST test vectors below stay as a guard that this wrapper keeps producing the standard
//! lowercase hex digest, so stored hashes remain verifiable across dependency upgrades.

use sha2::{Digest, Sha256};
use std::fmt::Write;

/// SHA-256 of `data` as 64 lowercase hex characters.
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for byte in digest {
        // Writing to a `String` cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::sha256_hex;

    // NIST FIPS 180-4 / CAVP example vectors.
    #[test]
    fn nist_empty() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn nist_abc() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn nist_two_block_message() {
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn nist_896_bit_message() {
        assert_eq!(
            sha256_hex(
                b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"
            ),
            "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1"
        );
    }

    #[test]
    fn nist_million_a() {
        assert_eq!(
            sha256_hex(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn padding_boundaries() {
        // 55, 56 and 64 byte inputs exercise the one-block/two-block padding edge.
        for len in [55usize, 56, 63, 64, 65] {
            assert_eq!(sha256_hex(&vec![0x61; len]).len(), 64);
        }
        assert_eq!(
            sha256_hex(&[0x61; 64]),
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
        );
    }
}
