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
    /// Parse an `algorithm:hex` digest string (e.g. `sha256:2cf24d…`, the form GitHub's release
    /// API publishes per asset) into a `Checksum`.
    ///
    /// Supported algorithms are `sha256` and `sha512` (case-insensitive, surrounding whitespace
    /// ignored). Any other prefix — or a string with no `:` separator — is rejected with an error
    /// naming the digest, rather than silently skipping verification.
    ///
    /// ```
    /// use self_update::Checksum;
    /// let c = Checksum::parse_digest(
    ///     "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    /// ).unwrap();
    /// assert!(matches!(c, Checksum::Sha256(_)));
    /// assert!(Checksum::parse_digest("md5:abc123").is_err());
    /// ```
    pub fn parse_digest(digest: &str) -> Result<Self> {
        let digest = digest.trim();
        let unsupported = || {
            Error::invalid_response(format!(
                "unsupported asset digest `{digest}` (expected `sha256:<hex>` or `sha512:<hex>`)"
            ))
        };
        let (algorithm, hex) = digest.split_once(':').ok_or_else(unsupported)?;
        match algorithm.trim().to_ascii_lowercase().as_str() {
            "sha256" => Ok(Checksum::Sha256(hex.to_string())),
            "sha512" => Ok(Checksum::Sha512(hex.to_string())),
            _ => Err(unsupported()),
        }
    }

    /// Resolve the expected checksum for `file_name` from the contents of a published sums file
    /// (a `SHA256SUMS` / `SHA512SUMS` asset, or a single-artifact `<name>.sha256`).
    ///
    /// The format is only loosely standardized, so all of the shapes in the wild are accepted:
    ///
    /// - `<hex>  <name>` (coreutils text mode, two spaces) and `<hex> *<name>` (binary mode);
    /// - a single space, or any other run of whitespace, between the two;
    /// - a name carrying leading path components (`dist/app.tar.gz`), matched on its last
    ///   component;
    /// - the BSD tag form, `SHA256 (<name>) = <hex>`;
    /// - blank lines and `#` comments, skipped;
    /// - a file whose entire contents are one bare digest, which is taken as the digest for
    ///   `file_name` regardless of name (the `<name>.sha256` convention, where the file name
    ///   itself carries the association).
    ///
    /// The algorithm comes from the digest's length (64 hex chars -> SHA-256, 128 -> SHA-512), so
    /// a `SHA512SUMS` file needs no separate configuration. Names are compared exactly (a checksum
    /// file is machine-generated, so case-insensitive matching would only add ambiguity).
    ///
    /// Returns [`Error::ChecksumSourceInvalid`] naming `file_name` when there is no entry for it or
    /// the entry's digest is not a supported length.
    ///
    /// ```
    /// use self_update::Checksum;
    /// let sums = "\
    /// 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  app-1.0.0.tar.gz
    /// 0000000000000000000000000000000000000000000000000000000000000000  other.tar.gz
    /// ";
    /// let c = Checksum::from_sums_file(sums, "app-1.0.0.tar.gz").unwrap();
    /// assert!(matches!(c, Checksum::Sha256(_)));
    /// assert!(Checksum::from_sums_file(sums, "missing.tar.gz").is_err());
    /// ```
    pub fn from_sums_file(sums: &str, file_name: &str) -> Result<Self> {
        let invalid = |reason: String| Error::ChecksumSourceInvalid {
            asset: file_name.to_string(),
            reason,
        };

        // A whole-file bare digest: the `<artifact>.sha256` convention, where the association with
        // the artifact is the file's own name rather than a field inside it.
        let trimmed = sums.trim();
        if is_hex_digest(trimmed) {
            return checksum_for_digest(trimmed).ok_or_else(|| {
                invalid(format!(
                    "digest `{trimmed}` is {} hex characters, expected 64 (sha256) or 128 (sha512)",
                    trimmed.len()
                ))
            });
        }

        for line in sums.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((digest, name)) = parse_sums_line(line) else {
                continue;
            };
            if base_name(name) != file_name {
                continue;
            }
            return checksum_for_digest(digest).ok_or_else(|| {
                invalid(format!(
                    "the entry for `{file_name}` has a {}-character digest, \
                     expected 64 (sha256) or 128 (sha512)",
                    digest.len()
                ))
            });
        }
        Err(invalid(format!("no entry for `{file_name}`")))
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

/// Whether `s` is a bare hex digest of a length this crate can verify.
fn is_hex_digest(s: &str) -> bool {
    matches!(s.len(), 64 | 128) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The `Checksum` variant a digest's length implies, or `None` for a length neither algorithm
/// produces (or a non-hex string). Callers turn the `None` into an error naming the entry.
fn checksum_for_digest(digest: &str) -> Option<Checksum> {
    if !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match digest.len() {
        64 => Some(Checksum::Sha256(digest.to_string())),
        128 => Some(Checksum::Sha512(digest.to_string())),
        _ => None,
    }
}

/// Split one line of a sums file into `(digest, name)`, or `None` when it is not an entry.
///
/// Only leading whitespace and the coreutils binary marker `*` are stripped from the name, so a
/// name containing spaces survives intact.
fn parse_sums_line(line: &str) -> Option<(&str, &str)> {
    // BSD tag form: `SHA256 (name) = hex`. Checked first because it also contains whitespace and
    // would otherwise be mis-split by the coreutils branch below.
    if let Some((head, digest)) = line.rsplit_once(" = ")
        && let Some(open) = head.find(" (")
        && let Some(name) = head[open + 2..].strip_suffix(')')
    {
        return Some((digest.trim(), name));
    }

    let split = line.find(char::is_whitespace)?;
    let (digest, rest) = line.split_at(split);
    let rest = rest.trim_start();
    let name = rest.strip_prefix('*').unwrap_or(rest);
    if name.is_empty() {
        return None;
    }
    Some((digest, name))
}

/// The last path component of a name listed in a sums file, so an entry written as
/// `dist/app.tar.gz` still matches the asset `app.tar.gz`. Both separators are honored: a sums file
/// generated on windows can carry `\`.
fn base_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
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

    // `parse_digest` maps the `algorithm:hex` forge form onto the matching variant, tolerating
    // case and surrounding whitespace in the algorithm, and the parsed checksum verifies files.
    #[test]
    fn parse_digest_supports_sha256_and_sha512() {
        let (_dir, path) = write_tmp(b"hello");
        let sha256 = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let parsed = Checksum::parse_digest(&format!("sha256:{sha256}")).unwrap();
        assert!(matches!(parsed, Checksum::Sha256(_)));
        parsed.verify(&path).unwrap();

        let sha512 = "9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043";
        let parsed = Checksum::parse_digest(&format!("SHA512:{sha512}")).unwrap();
        assert!(matches!(parsed, Checksum::Sha512(_)));
        parsed.verify(&path).unwrap();

        Checksum::parse_digest(&format!("  sha256:{sha256}  "))
            .unwrap()
            .verify(&path)
            .unwrap();
    }

    // An unknown algorithm or a string without the `:` separator is rejected with
    // `Error::InvalidResponse`, and the message names the offending digest.
    #[test]
    fn parse_digest_rejects_unsupported_or_malformed() {
        for bad in ["md5:abc123", "2cf24dba", "sha256"] {
            let err = Checksum::parse_digest(bad).unwrap_err();
            assert!(
                matches!(err, crate::errors::Error::InvalidResponse { .. }),
                "expected Error::InvalidResponse for {bad:?}, got {err:?}"
            );
            assert!(
                err.to_string().contains(bad),
                "the error must name the digest, got: {}",
                err
            );
        }
    }

    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    const HELLO_SHA512: &str = "9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043";

    // Every shape a published sums file turns up in resolves to the entry for the requested name,
    // and the resolved checksum really verifies the file it names.
    #[test]
    fn from_sums_file_accepts_the_formats_in_the_wild() {
        let (_dir, path) = write_tmp(b"hello");
        let other = "0".repeat(64);
        let cases: Vec<(&str, String)> = vec![
            // coreutils text mode (two spaces) with a decoy entry either side.
            (
                "two-space",
                format!(
                    "{other}  before.tar.gz\n{HELLO_SHA256}  app.tar.gz\n{other}  after.tar.gz\n"
                ),
            ),
            // A single space, which plenty of hand-rolled release scripts emit.
            ("one-space", format!("{HELLO_SHA256} app.tar.gz\n")),
            // coreutils binary mode.
            ("binary-marker", format!("{HELLO_SHA256} *app.tar.gz\n")),
            // A leading path component, matched on the last component.
            (
                "leading-path",
                format!("{HELLO_SHA256}  dist/linux/app.tar.gz\n"),
            ),
            // A windows-generated separator in the listed path.
            (
                "windows-path",
                format!("{HELLO_SHA256}  dist\\app.tar.gz\n"),
            ),
            // BSD tag form.
            ("bsd-tag", format!("SHA256 (app.tar.gz) = {HELLO_SHA256}\n")),
            // Comments and blank lines are skipped.
            (
                "comments",
                format!("# generated by release.sh\n\n{HELLO_SHA256}  app.tar.gz\n"),
            ),
            // Trailing whitespace / CRLF line endings.
            ("crlf", format!("{HELLO_SHA256}  app.tar.gz\r\n")),
        ];
        for (label, sums) in cases {
            let checksum = Checksum::from_sums_file(&sums, "app.tar.gz")
                .unwrap_or_else(|e| panic!("{label}: {e}"));
            assert!(
                matches!(checksum, Checksum::Sha256(_)),
                "{label}: a 64-character digest must resolve to sha256"
            );
            checksum.verify(&path).unwrap_or_else(|e| {
                panic!("{label}: the resolved digest must verify the file: {e}")
            });
        }
    }

    // The algorithm comes from the digest length, so a SHA512SUMS asset needs no configuration.
    #[test]
    fn from_sums_file_picks_the_algorithm_from_the_digest_length() {
        let (_dir, path) = write_tmp(b"hello");
        let sums = format!("{HELLO_SHA512}  app.tar.gz\n");
        let checksum = Checksum::from_sums_file(&sums, "app.tar.gz").unwrap();
        assert!(matches!(checksum, Checksum::Sha512(_)));
        checksum.verify(&path).unwrap();
    }

    // The `<artifact>.sha256` convention: the whole file is one bare digest, with the association
    // carried by the file's own name rather than by a field inside it.
    #[test]
    fn from_sums_file_accepts_a_bare_digest_file() {
        let (_dir, path) = write_tmp(b"hello");
        Checksum::from_sums_file(&format!("  {HELLO_SHA256}\n"), "app.tar.gz")
            .unwrap()
            .verify(&path)
            .unwrap();
    }

    // A name that is not listed is an error naming that name, never a silently skipped check: the
    // caller asked for sums verification and must not get a pass instead.
    #[test]
    fn from_sums_file_rejects_a_missing_entry() {
        let sums = format!("{HELLO_SHA256}  other.tar.gz\n");
        let err = Checksum::from_sums_file(&sums, "app.tar.gz").unwrap_err();
        match &err {
            crate::errors::Error::ChecksumSourceInvalid { asset, .. } => {
                assert_eq!(asset, "app.tar.gz")
            }
            other => panic!("expected ChecksumSourceInvalid, got {other:?}"),
        }
        assert!(
            err.to_string().contains("no entry for `app.tar.gz`"),
            "the error must say what was missing, got: {err}"
        );
    }

    // Name matching is on the full last path component, so a listed `myapp.tar.gz` does not satisfy
    // a request for `app.tar.gz`.
    #[test]
    fn from_sums_file_does_not_match_a_name_suffix() {
        let sums = format!("{HELLO_SHA256}  myapp.tar.gz\n");
        assert!(Checksum::from_sums_file(&sums, "app.tar.gz").is_err());
    }

    // A matching entry whose digest is neither a sha256 nor a sha512 length is a hard error, not a
    // fallthrough to "no entry": the difference matters when diagnosing a release's sums file.
    #[test]
    fn from_sums_file_rejects_an_unusable_digest_length() {
        let sums = "abc123  app.tar.gz\n";
        let err = Checksum::from_sums_file(sums, "app.tar.gz").unwrap_err();
        let shown = err.to_string();
        assert!(
            shown.contains("6-character digest"),
            "the error must name the offending length, got: {shown}"
        );
    }

    // A name containing spaces survives: only the leading whitespace and the binary marker are
    // stripped from the remainder of the line.
    #[test]
    fn from_sums_file_keeps_spaces_inside_a_name() {
        let sums = format!("{HELLO_SHA256}  my app.tar.gz\n");
        assert!(Checksum::from_sums_file(&sums, "my app.tar.gz").is_ok());
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
