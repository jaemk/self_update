use std::convert::TryInto;
use std::fs::File;
use std::io::{copy, Read, Seek, SeekFrom};
use std::path::Path;

use ed25519_dalek::{Digest, Sha512, Signature, VerifyingKey, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH};

use crate::errors::Error;
use crate::{detect_archive, ArchiveKind};

const MAGIC_HEADER: &[u8; 14] = b"\x0c\x04\x01ed25519ph\x00\x00";
const HEADER_SIZE: usize = 16;
type SignatureCountLeInt = u16;

pub(crate) fn verify(archive_path: &Path, keys: &[[u8; PUBLIC_KEY_LENGTH]]) -> crate::Result<()> {
    if keys.is_empty() {
        return Ok(());
    }

    println!("Verifying downloaded file...");

    let keys = keys
        .into_iter()
        .map(VerifyingKey::from_bytes)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error::NoValidSignature)?;
    let file_name = archive_path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.as_bytes())
        .ok_or(Error::NoValidSignature)?;
    let archive_kind = detect_archive(&archive_path)?;

    let mut exe = File::open(&archive_path)?;

    match archive_kind {
        ArchiveKind::Plain(_) => {
            unimplemented!("Can only check signatures for .zip and .tar* files.")
        }
        #[cfg(feature = "archive-tar")]
        ArchiveKind::Tar(_) => do_verify(&mut exe, &keys, file_name, true),
        #[cfg(feature = "archive-zip")]
        ArchiveKind::Zip => do_verify(&mut exe, &keys, file_name, false),
    }
}

fn do_verify(
    exe: &mut File,
    keys: &[VerifyingKey],
    context: &[u8],
    signature_at_eof: bool,
) -> Result<(), Error> {
    if signature_at_eof {
        exe.seek(SeekFrom::End(-(HEADER_SIZE as i64)))?;
    }

    let mut header = [0; HEADER_SIZE];
    exe.read_exact(&mut header)?;
    if header[..MAGIC_HEADER.len()] != MAGIC_HEADER[..] {
        println!("Signature header was not found.");
        return Err(Error::NoValidSignature);
    }
    let signature_count = header[MAGIC_HEADER.len()..].try_into().unwrap();
    let signature_count = SignatureCountLeInt::from_le_bytes(signature_count) as usize;
    let signature_size = signature_count * SIGNATURE_LENGTH;

    let content_size = match signature_at_eof {
        false => 0,
        true => exe.seek(SeekFrom::End(-((HEADER_SIZE + signature_size) as i64)))?
    };

    let mut signatures = vec![0; signature_size];
    exe.read_exact(&mut signatures)?;
    let signatures = signatures
        .chunks_exact(SIGNATURE_LENGTH)
        .map(Signature::from_slice)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error::NoValidSignature)?;

    let mut prehashed_message = Sha512::new();
    if signature_at_eof {
        exe.seek(SeekFrom::Start(0))?;
        copy(&mut exe.take(content_size), &mut prehashed_message)?;
    } else {
        copy(exe, &mut prehashed_message)?;
    }

    for key in keys {
        for signature in &signatures {
            if key
                .verify_prehashed_strict(prehashed_message.clone(), Some(context), signature)
                .is_ok()
            {
                println!("OK");
                return Ok(());
            }
        }
    }
    Err(Error::NoValidSignature)
}
