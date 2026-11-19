use super::*;
use db::{create_org, open_in_memory};

const KEY1: &str = "k1:000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const KEY2: &str = "k2:ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100";

fn org(n: u8) -> OrgId {
    match OrgId::parse(&format!("00000000-0000-4000-8000-0000000000{n:02x}")) {
        Ok(o) => o,
        Err(_) => unreachable!("valid test id"),
    }
}

fn vault() -> Vault {
    match Vault::new(KEY1, &[]) {
        Ok(v) => v,
        Err(_) => unreachable!("valid test key"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// McGrew and Viega, "The Galois/Counter Mode of Operation", test case 16 (AES-256, 96-bit IV,
/// with AAD), the vector ring itself is checked against. Proves we call ring correctly.
#[test]
fn ring_matches_the_published_aes_256_gcm_vector() {
    let key = unhex("feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308");
    let nonce = unhex("cafebabefacedbaddecaf888");
    let aad = unhex("feedfacedeadbeeffeedfacedeadbeefabaddad2");
    let mut data = unhex(
        "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
    );
    let unbound = UnboundKey::new(&AES_256_GCM, &key).map_err(|_| ());
    let Ok(unbound) = unbound else {
        unreachable!("32-byte key")
    };
    let key = LessSafeKey::new(unbound);
    let Ok(nonce) = Nonce::try_assume_unique_for_key(&nonce) else {
        unreachable!("12-byte nonce")
    };
    assert!(
        key.seal_in_place_append_tag(nonce, Aad::from(aad.as_slice()), &mut data)
            .is_ok()
    );
    assert_eq!(
        hex(&data),
        "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662\
         76fc6ece0f4e1768cddf8853bb2d551b"
    );
}

#[test]
fn seals_and_opens_text_and_bytes() {
    let v = vault();
    let o = org(1);
    let sealed = v
        .seal(&o, "ai_provider_key", b"sk-test-0123456789")
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(sealed.key_id, "k1");
    assert_eq!(sealed.ciphertext[0], VERSION);
    assert_eq!(sealed.ciphertext.len(), 1 + NONCE_LEN + 18 + TAG_LEN);
    assert!(!sealed.ciphertext.windows(10).any(|w| w == b"0123456789"));
    let back = v.open_secret(&o, "ai_provider_key", &sealed);
    assert_eq!(
        back.map(|s| s.expose_secret().to_owned()),
        Ok("sk-test-0123456789".to_owned())
    );
    let empty = v.seal(&o, "p", b"").unwrap_or_else(|_| unreachable!());
    assert_eq!(v.open(&o, "p", &empty).map(|b| b.len()), Ok(0));
}

#[test]
fn every_seal_uses_a_fresh_nonce() {
    let v = vault();
    let o = org(1);
    let mut seen = std::collections::HashSet::new();
    for _ in 0..2000 {
        let s = v.seal(&o, "p", b"same").unwrap_or_else(|_| unreachable!());
        assert!(
            seen.insert(s.ciphertext[1..=NONCE_LEN].to_vec()),
            "nonce reused"
        );
    }
}

#[test]
fn a_sealed_value_does_not_open_for_another_org_purpose_or_key_label() {
    let v = vault();
    let sealed = v
        .seal(&org(1), "ai_provider_key", b"secret")
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(
        v.open(&org(2), "ai_provider_key", &sealed).err(),
        Some(VaultError::Corrupt)
    );
    assert_eq!(
        v.open(&org(1), "other_purpose", &sealed).err(),
        Some(VaultError::Corrupt)
    );
    let relabelled = SealedSecret {
        key_id: "k2".into(),
        ..sealed.clone()
    };
    assert_eq!(
        v.open(&org(1), "ai_provider_key", &relabelled).err(),
        Some(VaultError::UnknownKey)
    );
    assert!(v.open(&org(1), "ai_provider_key", &sealed).is_ok());
}

#[test]
fn any_flipped_bit_or_truncation_is_refused() {
    let v = vault();
    let o = org(1);
    let sealed = v
        .seal(&o, "p", b"payload-payload")
        .unwrap_or_else(|_| unreachable!());
    for i in 0..sealed.ciphertext.len() {
        let mut bad = sealed.clone();
        bad.ciphertext[i] ^= 0x01;
        assert_eq!(
            v.open(&o, "p", &bad).err(),
            Some(VaultError::Corrupt),
            "byte {i}"
        );
    }
    for len in [
        0,
        1,
        NONCE_LEN,
        NONCE_LEN + TAG_LEN,
        sealed.ciphertext.len() - 1,
    ] {
        let bad = SealedSecret {
            ciphertext: sealed.ciphertext[..len].to_vec(),
            key_id: "k1".into(),
        };
        assert_eq!(
            v.open(&o, "p", &bad).err(),
            Some(VaultError::Corrupt),
            "len {len}"
        );
    }
}

#[test]
fn associated_data_fields_cannot_bleed_into_each_other() {
    let o = org(1);
    assert_ne!(
        associated_data(&o, "ab", "c"),
        associated_data(&o, "a", "bc")
    );
}

#[test]
fn key_specs_are_validated_strictly() {
    for bad in [
        "",
        "k1",
        "k1:",
        ":00",
        "k1:0011",
        "k1:zz0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        "k 1:000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        "k1:000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f00",
    ] {
        assert_eq!(
            Vault::new(bad, &[]).err(),
            Some(VaultError::BadKeySpec),
            "{bad:?}"
        );
    }
    assert_eq!(
        Vault::new(KEY1, &[KEY1]).err(),
        Some(VaultError::BadKeySpec),
        "same id twice"
    );
    assert!(Vault::new(KEY1, &[KEY2]).is_ok());
}

#[test]
fn debug_output_shows_ids_only() {
    let v = Vault::new(KEY1, &[KEY2]).unwrap_or_else(|_| unreachable!());
    let text = format!("{v:?}");
    assert_eq!(text, "Vault(current=k1, retired=1)");
    assert!(!text.contains("0001") && !text.contains("ffee"));
}

#[test]
fn rotation_keeps_old_rows_readable_and_moves_them_to_the_new_key() {
    let conn = open_in_memory().unwrap_or_else(|_| unreachable!());
    let o = create_org(&conn, "a", "a", "free")
        .unwrap_or_else(|_| unreachable!())
        .id;
    let old = Vault::new(KEY1, &[]).unwrap_or_else(|_| unreachable!());
    let id = old
        .store(&conn, &o, "ai_provider_key", b"the-key")
        .unwrap_or_else(|_| unreachable!());

    // Key changed: k2 is current, k1 retired.
    let new = Vault::new(KEY2, &[KEY1]).unwrap_or_else(|_| unreachable!());
    assert_eq!(
        new.load(&conn, &o, &id)
            .map(|s| s.expose_secret().to_owned()),
        Ok("the-key".to_owned())
    );
    // The old vault alone cannot read what k2 sealed, and the new one alone cannot read k1.
    assert_eq!(new.rotate_org(&conn, &o, std::slice::from_ref(&id)), Ok(1));
    assert_eq!(
        new.rotate_org(&conn, &o, std::slice::from_ref(&id)),
        Ok(0),
        "already current"
    );
    let only_new = Vault::new(KEY2, &[]).unwrap_or_else(|_| unreachable!());
    assert_eq!(
        only_new
            .load(&conn, &o, &id)
            .map(|s| s.expose_secret().to_owned()),
        Ok("the-key".to_owned())
    );
    assert_eq!(old.load(&conn, &o, &id).err(), Some(VaultError::UnknownKey));
}

#[test]
fn stored_secrets_are_scoped_to_their_org_and_hold_no_plaintext() {
    let conn = open_in_memory().unwrap_or_else(|_| unreachable!());
    let a = create_org(&conn, "a", "a", "free")
        .unwrap_or_else(|_| unreachable!())
        .id;
    let b = create_org(&conn, "b", "b", "free")
        .unwrap_or_else(|_| unreachable!())
        .id;
    let v = vault();
    let id = v
        .store(&conn, &a, "ai_provider_key", b"needle-needle-needle")
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(v.load(&conn, &b, &id).err(), Some(VaultError::NotFound));
    let raw: Vec<u8> = conn
        .query_row("SELECT ciphertext FROM secrets", [], |r| r.get(0))
        .unwrap_or_else(|_| unreachable!());
    assert!(!raw.windows(6).any(|w| w == b"needle"));
    // Even a database-level copy into org B's row does not open: the AAD names org A.
    let (purpose, sealed) = db::get_secret(&conn, &a, &id).unwrap_or_else(|_| unreachable!());
    let copied = db::put_secret(&conn, &b, &purpose, &sealed).unwrap_or_else(|_| unreachable!());
    assert_eq!(v.load(&conn, &b, &copied).err(), Some(VaultError::Corrupt));
}
