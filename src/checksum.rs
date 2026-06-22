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

impl Checksum {
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
