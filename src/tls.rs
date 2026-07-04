/*!
Custom TLS root CA certificates for the HTTP clients the crate builds.

A [`Certificate`] is an opaque, format-tagged byte buffer. Construction
([`Certificate::from_pem`] / [`Certificate::from_der`]) is infallible and just stores the bytes;
the bytes are only parsed/validated when the certificate is applied to a client (at `build()` time
on a backend builder, or when a `Download` materializes its client). A malformed certificate
therefore surfaces as a config-time error from `build()` rather than panicking at construction.
*/

/// The wire encoding of the stored certificate bytes. Private: the public surface is the two
/// `from_*` constructors plus the crate-internal [`Certificate::is_pem`] discriminator.
#[derive(Clone)]
enum CertFormat {
    Pem,
    Der,
}

/// An opaque TLS root CA certificate.
///
/// Construct with [`Certificate::from_pem`] or [`Certificate::from_der`].
/// The bytes are validated at `build()` time, not here.
#[derive(Clone)]
pub struct Certificate {
    format: CertFormat,
    bytes: Vec<u8>,
}

impl Certificate {
    /// Create a certificate from PEM-encoded bytes. The bytes are stored as-is;
    /// parsing is deferred to when the certificate is applied.
    pub fn from_pem(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            format: CertFormat::Pem,
            bytes: bytes.into(),
        }
    }

    /// Create a certificate from DER-encoded bytes. The bytes are stored as-is;
    /// parsing is deferred to when the certificate is applied.
    pub fn from_der(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            format: CertFormat::Der,
            bytes: bytes.into(),
        }
    }

    /// The stored certificate bytes, exactly as supplied to the constructor.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// `true` for a [`from_pem`](Self::from_pem) certificate, `false` for
    /// [`from_der`](Self::from_der). Used by the client builders to pick the right decoder.
    pub(crate) fn is_pem(&self) -> bool {
        matches!(self.format, CertFormat::Pem)
    }
}

impl std::fmt::Debug for Certificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Do not dump raw certificate bytes; show only the format tag and byte count.
        f.debug_struct("Certificate")
            .field("format", &if self.is_pem() { "pem" } else { "der" })
            .field("bytes", &format_args!("<{} bytes>", self.bytes.len()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::Certificate;

    #[test]
    fn from_pem_stores_bytes_and_marks_pem() {
        // `from_pem` is infallible and stores the bytes verbatim (no parse here); `is_pem()` reports
        // the PEM tag so the client builders route it to the PEM decoder.
        let cert = Certificate::from_pem(b"data".to_vec());
        assert!(cert.is_pem(), "from_pem must mark the cert as PEM");
        assert_eq!(cert.bytes(), b"data", "from_pem must store the bytes as-is");
    }

    #[test]
    fn from_der_stores_bytes_and_marks_der() {
        // Mirror of the PEM case: DER bytes are stored verbatim and `is_pem()` is false so the
        // builders route it to the DER decoder.
        let cert = Certificate::from_der(b"data".to_vec());
        assert!(!cert.is_pem(), "from_der must NOT be marked as PEM");
        assert_eq!(cert.bytes(), b"data", "from_der must store the bytes as-is");
    }
}
