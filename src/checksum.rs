/*!
Checksum verification of downloaded release artifacts.

Enabled by the `checksums` feature. Unlike [`signatures`](crate#features) (zipsign / ed25519),
this verifies a plain content hash you already know — e.g. one published in a `SHA256SUMS`
file — against the downloaded file before it is installed.
*/
#![cfg(feature = "checksums")]

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256, Sha512};

use crate::errors::*;

/// An expected checksum for a downloaded release artifact, tagged with its hash algorithm.
///
/// The variant selects the algorithm; the contained `String` is the expected digest, hex
/// encoded (case-insensitive, surrounding whitespace ignored). Pass one to
/// `Update::configure().verify_checksum(..)`; the download is rejected before installation
/// if it does not match.
///
/// ```
/// use self_update::Checksum;
/// let _sha256 = Checksum::Sha256("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string());
/// let _sha512 = Checksum::Sha512("9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043".to_string());
/// ```
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Checksum {
    /// Expected SHA-256 digest, hex encoded.
    Sha256(String),
    /// Expected SHA-512 digest, hex encoded.
    Sha512(String),
}

/// Expected hex-digest lengths (nibbles = bytes * 2).
const SHA256_HEX_LEN: usize = 64; // 32 bytes * 2
const SHA512_HEX_LEN: usize = 128; // 64 bytes * 2

impl Checksum {
    /// Construct a `Checksum::Sha256` after validating that `hex` is a lowercase or
    /// uppercase hex string of exactly 64 characters (32 bytes). Returns
    /// `Err(Error::Config(_))` if the length or encoding is wrong, so a typo is a
    /// config error that surfaces immediately rather than a runtime mismatch.
    ///
    /// # Example
    ///
    /// ```
    /// # #[cfg(feature = "checksums")] {
    /// use self_update::Checksum;
    /// let c = Checksum::sha256("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824").unwrap();
    /// // Wrong length => config error.
    /// assert!(Checksum::sha256("abc").is_err());
    /// # }
    /// ```
    pub fn sha256(hex: impl Into<String>) -> Result<Self> {
        let s = hex.into();
        validate_hex_digest(&s, SHA256_HEX_LEN, "sha256")?;
        Ok(Checksum::Sha256(s))
    }

    /// Construct a `Checksum::Sha512` after validating that `hex` is a lowercase or
    /// uppercase hex string of exactly 128 characters (64 bytes). Returns
    /// `Err(Error::Config(_))` if the length or encoding is wrong.
    ///
    /// # Example
    ///
    /// ```
    /// # #[cfg(feature = "checksums")] {
    /// use self_update::Checksum;
    /// let digest = "9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043";
    /// let c = Checksum::sha512(digest).unwrap();
    /// assert!(Checksum::sha512("tooshort").is_err());
    /// # }
    /// ```
    pub fn sha512(hex: impl Into<String>) -> Result<Self> {
        let s = hex.into();
        validate_hex_digest(&s, SHA512_HEX_LEN, "sha512")?;
        Ok(Checksum::Sha512(s))
    }

    /// The expected digest, hex encoded.
    fn expected(&self) -> &str {
        match self {
            Checksum::Sha256(hex) | Checksum::Sha512(hex) => hex,
        }
    }

    /// Hash the file at `path` with this checksum's algorithm and return the hex digest.
    fn hash_file(&self, path: &Path) -> Result<String> {
        match self {
            Checksum::Sha256(_) => hash_file::<Sha256>(path),
            Checksum::Sha512(_) => hash_file::<Sha512>(path),
        }
    }

    /// Verify that the file at `path` matches this checksum, returning an error on mismatch.
    pub(crate) fn verify(&self, path: &Path) -> Result<()> {
        let expected = self.expected().trim().to_lowercase();
        let actual = self.hash_file(path)?;
        if actual.eq_ignore_ascii_case(&expected) {
            Ok(())
        } else {
            Err(Error::ChecksumMismatch {
                expected,
                computed: actual,
            })
        }
    }
}

/// Stream the file through digest `D` and return its lowercase hex digest.
fn hash_file<D: Digest>(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = D::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

/// Validate that `s` (after trimming) is a hex string of exactly `expected_len` nibbles.
/// Returns `Err(Error::Config(_))` on length or non-hex character violations.
fn validate_hex_digest(s: &str, expected_len: usize, algo: &str) -> Result<()> {
    let trimmed = s.trim();
    if trimmed.len() != expected_len {
        return Err(Error::Config(format!(
            "invalid {} digest: expected {} hex characters (got {})",
            algo,
            expected_len,
            trimmed.len()
        )));
    }
    if !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Config(format!(
            "invalid {} digest: contains non-hex characters",
            algo
        )));
    }
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{:02x}", byte);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::Checksum;

    fn write_tmp(contents: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact");
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    #[test]
    fn sha256_matches_known_digest() {
        let (_dir, path) = write_tmp(b"hello");
        // `printf hello | sha256sum`
        let digest = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        Checksum::Sha256(digest.to_string()).verify(&path).unwrap();
        // Upper-case and surrounding whitespace are tolerated.
        Checksum::Sha256(format!("  {}  ", digest.to_uppercase()))
            .verify(&path)
            .unwrap();
    }

    #[test]
    fn sha512_matches_known_digest() {
        let (_dir, path) = write_tmp(b"hello");
        // `printf hello | sha512sum`
        let digest = "9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043";
        Checksum::Sha512(digest.to_string()).verify(&path).unwrap();
    }

    #[test]
    fn mismatch_is_rejected() {
        let (_dir, path) = write_tmp(b"hello");
        let err = Checksum::Sha256("00".repeat(32)).verify(&path);
        assert!(err.is_err());
        // A SHA-256 digest is not a valid SHA-512 digest for the same content.
        let sha256 = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(Checksum::Sha512(sha256.to_string()).verify(&path).is_err());
    }

    // A checksum mismatch through the verification path yields `Error::ChecksumMismatch` (not
    // `Error::Update`). The variant must carry the expected and computed digests as fields, and
    // its Display must contain "checksum mismatch".
    #[test]
    fn mismatch_yields_checksum_mismatch_variant() {
        let (_dir, path) = write_tmp(b"hello");
        let wrong_digest = "00".repeat(32);
        let real_digest = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

        let err = Checksum::Sha256(wrong_digest.clone())
            .verify(&path)
            .unwrap_err();

        assert!(
            matches!(err, crate::errors::Error::ChecksumMismatch { .. }),
            "a digest mismatch must produce Error::ChecksumMismatch, got {:?}",
            err
        );
        if let crate::errors::Error::ChecksumMismatch { expected, computed } = err {
            assert_eq!(
                expected, wrong_digest,
                "expected field must hold the configured digest (lowercased/trimmed)"
            );
            assert_eq!(
                computed, real_digest,
                "computed field must hold the actual file digest"
            );
        }
    }

    // A5: `Checksum::sha256` / `Checksum::sha512` validating constructors.

    #[test]
    fn sha256_constructor_accepts_valid_hex() {
        let digest = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        Checksum::sha256(digest).expect("valid 64-char hex must be accepted by sha256()");
        // Upper-case should also be accepted.
        Checksum::sha256(digest.to_uppercase()).expect("uppercase hex must be accepted");
    }

    #[test]
    fn sha256_constructor_rejects_wrong_length() {
        let err = Checksum::sha256("abc").expect_err("too-short hex must be rejected");
        assert!(
            matches!(err, crate::errors::Error::Config(_)),
            "wrong length must produce Error::Config, got {:?}",
            err
        );
        // Too long is also rejected.
        let too_long = "a".repeat(65);
        let err2 = Checksum::sha256(too_long).expect_err("too-long hex must be rejected");
        assert!(matches!(err2, crate::errors::Error::Config(_)));
    }

    #[test]
    fn sha256_constructor_rejects_non_hex() {
        // 64 chars but contains 'g' which is not a hex digit.
        let non_hex = "g".repeat(64);
        let err = Checksum::sha256(non_hex).expect_err("non-hex chars must be rejected");
        assert!(
            matches!(err, crate::errors::Error::Config(_)),
            "non-hex chars must produce Error::Config, got {:?}",
            err
        );
    }

    #[test]
    fn sha512_constructor_accepts_valid_hex() {
        let digest = "9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043";
        Checksum::sha512(digest).expect("valid 128-char hex must be accepted by sha512()");
    }

    #[test]
    fn sha512_constructor_rejects_wrong_length() {
        // A SHA-256 digest (64 chars) is too short for SHA-512 (128 chars).
        let sha256_len = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let err =
            Checksum::sha512(sha256_len).expect_err("SHA-256 length must fail SHA-512 constructor");
        assert!(
            matches!(err, crate::errors::Error::Config(_)),
            "wrong length must produce Error::Config, got {:?}",
            err
        );
    }

    #[test]
    fn sha512_constructor_rejects_non_hex() {
        let non_hex = "z".repeat(128);
        let err = Checksum::sha512(non_hex).expect_err("non-hex chars must be rejected");
        assert!(
            matches!(err, crate::errors::Error::Config(_)),
            "non-hex chars must produce Error::Config, got {:?}",
            err
        );
    }

    // The Display of ChecksumMismatch embeds the expected and computed digests.
    #[test]
    fn mismatch_display_contains_expected_and_computed() {
        let (_dir, path) = write_tmp(b"hello");
        let wrong_digest = "00".repeat(32);
        let err = Checksum::Sha256(wrong_digest.clone())
            .verify(&path)
            .unwrap_err();
        let shown = err.to_string();
        assert!(
            shown.starts_with("ChecksumMismatchError:"),
            "Display must start with 'ChecksumMismatchError:', got: {}",
            shown
        );
        assert!(
            shown.contains(&wrong_digest),
            "Display must contain the expected digest, got: {}",
            shown
        );
        assert!(
            shown.contains("2cf24dba"),
            "Display must contain the computed digest, got: {}",
            shown
        );
    }
}
