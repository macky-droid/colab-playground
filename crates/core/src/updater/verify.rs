//! Whether downloaded release files are the ones the project published.
//!
//! The release workflow signs, with the project's Ed25519 key, a line naming the release tag
//! followed by the release's `SHA256SUMS` ([`signed_message`]). The app checks that signature
//! against the keys compiled into it, then the download's SHA-256 against the list. The tag is
//! part of what is signed because every release lists the same version-less file names: without
//! it, an older release's signed list and files would pass for a newer release.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, VerifyingKey};

use crate::error::{Error, ErrorKind, Result};

/// The public keys an update may be signed with (see the file for its format). The release
/// workflow checks every new signature against the same file.
pub const TRUSTED_KEYS: &str = include_str!("../../update-keys.txt");

/// A raw Ed25519 public key.
pub type PublicKey = [u8; 32];

pub type Sha256 = [u8; 32];

fn unverified(detail: impl Into<String>) -> Error {
    Error::new(ErrorKind::UpdateUnverified).detail(detail)
}

/// The bytes the release workflow signs for release `tag`.
pub fn signed_message(tag: &str, sums: &[u8]) -> Vec<u8> {
    let mut message = format!("colab-playground {tag}\n").into_bytes();
    message.extend_from_slice(sums);
    message
}

/// Parses text in the format of [`TRUSTED_KEYS`]. A line that is not a valid key is an error,
/// never skipped: a typo must not leave a build trusting fewer keys than intended.
pub fn parse_keys(text: &str) -> Result<Vec<PublicKey>> {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty())
        .map(|line| {
            let invalid = || {
                Error::new(ErrorKind::Parse)
                    .with("key", line)
                    .detail("not a base64-encoded Ed25519 public key")
            };
            let key: PublicKey = B64
                .decode(line)
                .ok()
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or_else(invalid)?;
            VerifyingKey::from_bytes(&key).map_err(|_| invalid())?;
            Ok(key)
        })
        .collect()
}

pub fn trusted_keys() -> Result<Vec<PublicKey>> {
    parse_keys(TRUSTED_KEYS)
}

/// Checks `signature` (64 raw bytes) over `message`; a match with any one of `keys` is enough,
/// so a new key can be added before the old one is retired.
pub fn check_signature(message: &[u8], signature: &[u8], keys: &[PublicKey]) -> Result<()> {
    let signature: [u8; 64] = signature
        .try_into()
        .map_err(|_| unverified("the signature is not 64 bytes"))?;
    let signature = Signature::from_bytes(&signature);
    if keys.is_empty() {
        return Err(unverified("this build trusts no update key"));
    }
    let matches = keys.iter().any(|key| {
        VerifyingKey::from_bytes(key)
            .is_ok_and(|key| key.verify_strict(message, &signature).is_ok())
    });
    if matches {
        Ok(())
    } else {
        Err(unverified("the signature matches none of the trusted keys"))
    }
}

/// The SHA-256 that `sums` (the format `sha256sum` writes) lists for `name`; `None` when the
/// list does not name it. A list that is not in that format is an error.
pub fn listed_hash(sums: &[u8], name: &str) -> Result<Option<Sha256>> {
    let text = std::str::from_utf8(sums).map_err(|_| unverified("SHA256SUMS is not text"))?;
    for line in text.lines().map(|line| line.trim_end_matches('\r')) {
        if line.trim().is_empty() {
            continue;
        }
        // `<64 hex digits><space><' ' for text mode or '*' for binary mode><file name>`
        let malformed = || unverified(format!("SHA256SUMS has a malformed line: {line:?}"));
        let (hash, rest) = line.split_at_checked(64).ok_or_else(malformed)?;
        let file = rest
            .strip_prefix("  ")
            .or_else(|| rest.strip_prefix(" *"))
            .ok_or_else(malformed)?;
        let hash = parse_hex(hash).ok_or_else(malformed)?;
        if file == name {
            return Ok(Some(hash));
        }
    }
    Ok(None)
}

pub fn check_hash(actual: &Sha256, expected: &Sha256) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(unverified(format!(
            "the download's SHA-256 is {}, SHA256SUMS lists {}",
            hex(actual),
            hex(expected)
        )))
    }
}

fn parse_hex(text: &str) -> Option<Sha256> {
    let mut out = [0u8; 32];
    if text.len() != 64 {
        return None;
    }
    for (byte, pair) in out.iter_mut().zip(text.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).ok()?;
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use ed25519_dalek::{Signer as _, SigningKey};
    use sha2::Digest as _;

    use super::*;

    /// A key pair made at run time: no private key is ever committed.
    pub(crate) fn key_pair() -> (SigningKey, PublicKey) {
        let seed: String = (0..4).map(|_| crate::accounts::nonce()).collect();
        let seed: [u8; 32] = (0..32)
            .map(|i| u8::from_str_radix(&seed[2 * i..2 * i + 2], 16).unwrap())
            .collect::<Vec<u8>>()
            .try_into()
            .unwrap();
        let key = SigningKey::from_bytes(&seed);
        let public = key.verifying_key().to_bytes();
        (key, public)
    }

    pub(crate) fn sign(key: &SigningKey, tag: &str, sums: &[u8]) -> Vec<u8> {
        key.sign(&signed_message(tag, sums)).to_bytes().to_vec()
    }

    pub(crate) fn sha256(bytes: &[u8]) -> Sha256 {
        sha2::Sha256::digest(bytes).into()
    }

    pub(crate) fn sums_line(bytes: &[u8], name: &str) -> String {
        format!("{}  {name}\n", hex(&sha256(bytes)))
    }

    const SUMS: &str = "\
1111111111111111111111111111111111111111111111111111111111111111  colab-playground-aarch64-apple-darwin.zip
2222222222222222222222222222222222222222222222222222222222222222  colab-playground-x86_64-pc-windows-msvc.exe
3333333333333333333333333333333333333333333333333333333333333333 *colab-playground-x86_64-unknown-linux-gnu.tar.gz
";

    fn unverified_kind(result: Result<()>) -> bool {
        result.is_err_and(|e| e.kind == ErrorKind::UpdateUnverified)
    }

    #[test]
    fn a_signature_by_a_trusted_key_over_the_tag_and_list_verifies() {
        let (key, public) = key_pair();
        let signature = sign(&key, "v0.3.0", SUMS.as_bytes());
        let message = signed_message("v0.3.0", SUMS.as_bytes());
        check_signature(&message, &signature, &[public]).unwrap();
    }

    #[test]
    fn a_tampered_list_does_not_verify() {
        let (key, public) = key_pair();
        let signature = sign(&key, "v0.3.0", SUMS.as_bytes());
        let tampered = SUMS.replace("2222", "2223");
        assert!(unverified_kind(check_signature(
            &signed_message("v0.3.0", tampered.as_bytes()),
            &signature,
            &[public]
        )));
    }

    #[test]
    fn an_older_release_signed_list_does_not_pass_for_a_newer_tag() {
        let (key, public) = key_pair();
        let signature = sign(&key, "v0.2.0", SUMS.as_bytes());
        assert!(unverified_kind(check_signature(
            &signed_message("v0.3.0", SUMS.as_bytes()),
            &signature,
            &[public]
        )));
    }

    #[test]
    fn a_signature_by_another_key_does_not_verify() {
        let (key, _) = key_pair();
        let (_, other) = key_pair();
        assert_ne!(key.verifying_key().to_bytes(), other);
        let signature = sign(&key, "v0.3.0", SUMS.as_bytes());
        let message = signed_message("v0.3.0", SUMS.as_bytes());
        assert!(unverified_kind(check_signature(
            &message,
            &signature,
            &[other]
        )));
        assert!(
            unverified_kind(check_signature(&message, &signature, &[])),
            "no keys, nothing verifies"
        );
    }

    #[test]
    fn any_one_of_several_keys_is_enough() {
        let (old, old_public) = key_pair();
        let (new, new_public) = key_pair();
        let message = signed_message("v0.3.0", SUMS.as_bytes());
        for key in [&old, &new] {
            let signature = sign(key, "v0.3.0", SUMS.as_bytes());
            check_signature(&message, &signature, &[old_public, new_public]).unwrap();
        }
    }

    #[test]
    fn a_malformed_signature_does_not_verify() {
        let (_, public) = key_pair();
        let message = signed_message("v0.3.0", SUMS.as_bytes());
        for signature in [&b""[..], &[0u8; 63], &[0u8; 65], &[0u8; 64]] {
            assert!(unverified_kind(check_signature(
                &message,
                signature,
                &[public]
            )));
        }
    }

    /// Made with OpenSSL exactly as the release workflow signs (`openssl pkeyutl -sign -rawin`),
    /// with a key that was thrown away afterwards: proves the two agree on the format.
    #[test]
    fn a_signature_made_by_openssl_verifies() {
        const PUBLIC: &str = include_str!("../../tests/data/update-openssl/public.txt");
        const SIGNATURE: &str = include_str!("../../tests/data/update-openssl/signature.txt");
        const SUMS: &[u8] = include_bytes!("../../tests/data/update-openssl/SHA256SUMS");
        let keys = parse_keys(PUBLIC).unwrap();
        let signature = B64.decode(SIGNATURE.trim()).unwrap();
        check_signature(&signed_message("v9.8.7", SUMS), &signature, &keys).unwrap();
        assert!(unverified_kind(check_signature(
            &signed_message("v9.8.6", SUMS),
            &signature,
            &keys
        )));
    }

    #[test]
    fn the_listed_hash_is_found_by_exact_name_in_either_mode() {
        assert_eq!(
            listed_hash(
                SUMS.as_bytes(),
                "colab-playground-x86_64-pc-windows-msvc.exe"
            )
            .unwrap(),
            Some([0x22; 32])
        );
        assert_eq!(
            listed_hash(
                SUMS.as_bytes(),
                "colab-playground-x86_64-unknown-linux-gnu.tar.gz"
            )
            .unwrap(),
            Some([0x33; 32])
        );
        assert_eq!(
            listed_hash(SUMS.as_bytes(), "colab-playground-x86_64-pc-windows-msvc").unwrap(),
            None
        );
        let crlf = SUMS.replace('\n', "\r\n");
        assert_eq!(
            listed_hash(crlf.as_bytes(), "colab-playground-aarch64-apple-darwin.zip").unwrap(),
            Some([0x11; 32])
        );
    }

    #[test]
    fn a_malformed_list_is_unverified() {
        for sums in [
            "not a list\n",
            "1111  short\n",
            &format!("{}x  name\n", "1".repeat(63)),
            &format!("{} name\n", "1".repeat(64)),
        ] {
            assert_eq!(
                listed_hash(sums.as_bytes(), "name").unwrap_err().kind,
                ErrorKind::UpdateUnverified,
                "{sums:?}"
            );
        }
        assert!(listed_hash(&[0xff, 0xfe], "name").is_err());
    }

    #[test]
    fn a_hash_mismatch_is_unverified() {
        let file = b"the program";
        check_hash(&sha256(file), &sha256(file)).unwrap();
        assert!(unverified_kind(check_hash(
            &sha256(file),
            &sha256(b"tampered")
        )));
    }

    #[test]
    fn keys_parse_strictly() {
        let (_, public) = key_pair();
        let line = B64.encode(public);
        let text = format!("# comment\n\n{line}   # the first key\n");
        assert_eq!(parse_keys(&text).unwrap(), vec![public]);
        assert!(parse_keys("not-base64!\n").is_err());
        assert!(parse_keys(&B64.encode([1u8; 31])).is_err(), "31 bytes");
        assert!(parse_keys("# only comments\n").unwrap().is_empty());
    }

    #[test]
    fn this_build_trusts_at_least_one_valid_key() {
        let keys = trusted_keys().expect("update-keys.txt holds only valid keys");
        assert!(
            !keys.is_empty(),
            "update-keys.txt must list the release signing key (docs/RELEASING.md)"
        );
    }
}
