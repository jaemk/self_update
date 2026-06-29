# Embedded Key Verification

Status: pending

Compile-time public key embedding and key rotation for signature verification.

## Summary

Document and demonstrate compile-time embedding of ed25519ph public keys and the
key rotation workflow. No new library API is required; the work is a documented
`const` pattern, a `signatures` example, and rotation-specific tests.

See `ref-signatures-and-checksums.md` for the current signature verification behavior.

## Research findings

### Key type

zipsign uses ed25519ph exclusively. `VerifyingKey` in self_update is a type alias
for `[u8; zipsign_api::PUBLIC_KEY_LENGTH]`, which is 32 raw bytes (the standard
ed25519 public key size). The zipsign tooling produces and consumes raw 32-byte key
files, not PEM or any other encoding.

### Compile-time embedding

Because `VerifyingKey` is `[u8; 32]`, `include_bytes!` works directly with a
dereference:

```rust
const MY_KEY: self_update::VerifyingKey = *include_bytes!("my_key.pub");
```

This is valid stable Rust when the key file is exactly 32 bytes. No macro or
build-rs helper is needed. The implementation work is documenting this pattern in
the crate docs and adding an example that uses it.

### Multi-key / any-of semantics (confirmed)

`zipsign_api::verify::find_match` (`zipsign-api-0.1.5/src/verify/mod.rs:57`)
iterates all (key, signature) pairs and returns on the first match. Both
`verify_tar` and `verify_zip` call `find_match` identically. An archive can carry
multiple signatures; the header encodes a `SignatureCountLeInt` count followed by
the signature block.

Both `.tar.gz` (via `verify_tar`) and `.zip` (via `verify_zip`) use this same
path. Note: only `.tar.gz` is supported for tar; bare `.tar` and other
compressions yield `Error::NoSignatures`.

### Key rotation (no library changes needed)

When a key is rotated, the publisher dual-signs each release with both the old and
new keys (two signatures in the archive). Old binaries (embedding only the old key)
pass because the old key's signature is present. New binaries (embedding only the
new key) pass because the new key's signature is present. No binary needs to know
both keys simultaneously. After the transition window, future releases can be
signed with the new key only.

## EMBEDD-1: Documented embedding pattern

Add a `const`-based embedding example to the crate docs under the `signatures`
feature, showing:

```rust
const VERIFYING_KEY: self_update::VerifyingKey = *include_bytes!("my_key.pub");

// in the update builder:
.verify_keys(vec![VERIFYING_KEY])
```

Note that the file at `my_key.pub` must be exactly 32 raw bytes. If the size does
not match, the `include_bytes!` dereference is a compile error.

## EMBEDD-2: Key rotation documentation

Document the rotation protocol in the crate docs alongside the embedding pattern:

1. When rotating, sign new releases with both the old key and the new key.
2. Release the new binary (embedding the new key). Old binaries can still verify
   and install it because the old key's signature is in the archive.
3. After users have updated, stop dual-signing.

## EMBEDD-3: Example

Add or extend an example under `examples/` (alongside the existing `github`
example) that demonstrates:

- Declaring a compile-time `VerifyingKey` constant via `include_bytes!`.
- Passing it to `verify_keys()` in a GitHub backend builder.

## Implementation notes

- No changes to `src/` are required. The feature is entirely documentation and
  example coverage.
- The `signatures` feature must be enabled for the example to compile
  (`#[cfg(feature = "signatures")]` or a dedicated example with the feature in
  `Cargo.toml`).
- `verify_keys` already accepts `impl Into<Vec<VerifyingKey>>`, so a single
  constant or a `vec!` of multiple constants both work without any API change.

## Tests

- Static `const` key accepted: pass a compile-time constant through `verify_keys`
  against a test archive signed with the matching key; verify `Ok`.
- Rotation: create a test archive dual-signed with key A and key B; verify that
  passing only key A succeeds and passing only key B also succeeds.
- Wrong key: verify that passing a key that did not sign the archive yields
  `Err(Error::Signature(...))`.

## Related

- `ref-signatures-and-checksums.md`
- `checksum-verification.md`
