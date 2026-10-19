//! Signed/encrypted Evolve Stage 2 CryPAK reader.
//! RSA-1024 + RSA-PSS(SHA256,salt=0) signs the DECRYPTED central directory
//! followed by its original ASCII archive basename. Seventeen OAEP-formatted
//! 128-byte secrets use raw RSA PUBLIC exponent unwrap (NOT standard RSA
//! private-key decryption). Twofish-128 CTR counters use little-endian u128.
//! This module never embeds game keys or writes to installed game folders.
use crate::archive::safe_member_name;
use cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use crc32fast::Hasher;
use flate2::read::DeflateDecoder;
use rsa::{BigUint, Pss, RsaPublicKey, pkcs1::DecodeRsaPublicKey, pkcs8::DecodePublicKey, traits::PublicKeyParts};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use twofish::Twofish;

const TRAILER_BYTES: usize = 2320;
const RSA_BYTES: usize = 128;
const MAX_ENTRIES: usize = 60000;
const MAX_ENTRY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_CDR_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SignedPakError {
    #[error("Signed archive trailer/header unsupported or inconsistent: {0}")]
    Format(String),
    #[error("RSA signature, key, or decrypted central directory validation failed: {0}")]
    Crypto(String),
    #[error("Unsafe signed member path: {0}")]
    UnsafePath(String),
    #[error("Signed file or extracted size exceeds safety limits")]
    Limits,
    #[error("Refusing output to an existing or unsafe directory")]
    UnsafeOutput,
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedEntry {
    pub path: String,
    pub method: u16,
    pub flags: u16,
    pub crc32: u32,
    pub compressed_size: u64,
    pub size: u64,
    pub encrypted_data_offset: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedPakInfo {
    pub version: u32,
    pub archive_name: String,
    pub signature_verified: bool,
    pub entries: Vec<SignedEntry>,
    pub total_expanded_bytes: u64,
}

/// Holds verified metadata and locally unwrapped encryption secrets in process
/// memory only. Never Serialize/Debug the secrets or return them in reports.
pub struct VerifiedSignedPak {
    pub info: SignedPakInfo,
    keys: [[u8; 16]; 16],
}

fn read_u16(data: &[u8], at: usize) -> Result<u16, SignedPakError> {
    let bytes = data.get(at..at + 2).ok_or_else(|| SignedPakError::Format("truncated field".into()))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], at: usize) -> Result<u32, SignedPakError> {
    let bytes = data.get(at..at + 4).ok_or_else(|| SignedPakError::Format("truncated field".into()))?;
    Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| SignedPakError::Limits)?))
}

fn parse_public_key(bytes: &[u8]) -> Result<RsaPublicKey, SignedPakError> {
    let key = RsaPublicKey::from_pkcs1_der(bytes).ok()
        .or_else(|| RsaPublicKey::from_public_key_der(bytes).ok())
        .ok_or_else(|| SignedPakError::Crypto("invalid RSA public key DER".into()))?;
    if key.size() != RSA_BYTES {
        return Err(SignedPakError::Crypto("expected 1024-bit original/custom RSA public key".into()));
    }
    Ok(key)
}

/// Raw RSA public-exponent unwrap followed by OAEP-SHA256 unmasking.
/// Used only for the observed CryPAK custom private-wrap/public-unwrap scheme.
fn unwrap_key(wrapped: &[u8], pubkey: &RsaPublicKey) -> Result<[u8; 16], SignedPakError> {
    if wrapped.len() != RSA_BYTES {
        return Err(SignedPakError::Crypto("invalid wrapped key size".into()));
    }
    let ciphertext = BigUint::from_bytes_be(wrapped);
    if &ciphertext >= pubkey.n() {
        return Err(SignedPakError::Crypto("RSA representative outside modulus".into()));
    }
    let encoded = ciphertext.modpow(pubkey.e(), pubkey.n()).to_bytes_be();
    if encoded.len() > RSA_BYTES {
        return Err(SignedPakError::Crypto("invalid RSA output length".into()));
    }
    let mut em = [0u8; RSA_BYTES];
    em[RSA_BYTES - encoded.len()..].copy_from_slice(&encoded);
    if em[0] != 0 {
        return Err(SignedPakError::Crypto("invalid OAEP leading byte".into()));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&em[1..33]);
    let db = &em[33..];
    let mask = mgf1(db, 32);
    for (value, byte) in seed.iter_mut().zip(mask) { *value ^= byte; }
    let mask = mgf1(&seed, db.len());
    let clear = db.iter().zip(mask).map(|(a, b)| a ^ b).collect::<Vec<_>>();
    if clear[..32] != Sha256::digest([])[..] {
        return Err(SignedPakError::Crypto("wrong OAEP label hash".into()));
    }
    let padding = &clear[32..];
    let position = padding.iter().position(|b| *b == 1)
        .ok_or_else(|| SignedPakError::Crypto("OAEP separator missing".into()))?;
    if padding[..position].iter().any(|b| *b != 0) || padding.len() - position - 1 != 16 {
        return Err(SignedPakError::Crypto("OAEP padding or secret length invalid".into()));
    }
    let mut key = [0u8; 16];
    key.copy_from_slice(&padding[position + 1..]);
    Ok(key)
}

fn mgf1(seed: &[u8], length: usize) -> Vec<u8> {
    let mut result = Vec::with_capacity(length);
    for counter in 0..length.div_ceil(32) {
        let mut hash = Sha256::new();
        hash.update(seed);
        hash.update((counter as u32).to_be_bytes());
        result.extend_from_slice(&hash.finalize());
    }
    result.truncate(length);
    result
}

struct CtrRead<R: Read> {
    reader: R,
    cipher: Twofish,
    counter: u128,
    keystream: [u8; 16],
    used: usize,
}
impl<R: Read> CtrRead<R> {
    fn new(reader: R, key: &[u8; 16], iv: [u8; 16]) -> Self {
        Self {
            reader,
            cipher: Twofish::new(GenericArray::from_slice(key)),
            counter: u128::from_le_bytes(iv),
            keystream: [0u8; 16],
            used: 16,
        }
    }
}
impl<R: Read> Read for CtrRead<R> {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        let size = self.reader.read(target)?;
        for byte in &mut target[..size] {
            if self.used == 16 {
                let mut block = GenericArray::clone_from_slice(&self.counter.to_le_bytes());
                self.cipher.encrypt_block(&mut block);
                self.keystream.copy_from_slice(&block);
                self.counter = self.counter.wrapping_add(1);
                self.used = 0;
            }
            *byte ^= self.keystream[self.used];
            self.used += 1;
        }
        Ok(size)
    }
}

fn crypt_bytes(data: &[u8], key: &[u8; 16], iv: [u8; 16]) -> Result<Vec<u8>, SignedPakError> {
    let mut out = Vec::with_capacity(data.len());
    CtrRead::new(data, key, iv).read_to_end(&mut out)?;
    Ok(out)
}

fn entry_iv(entry: &SignedEntry) -> [u8; 16] {
    let compressed = entry.compressed_size as u32;
    let expanded = entry.size as u32;
    let crc = entry.crc32;
    let values = [
        expanded ^ compressed.wrapping_shl(12),
        u32::from(compressed == 0),
        crc ^ compressed.wrapping_shl(12),
        u32::from(expanded == 0) ^ compressed,
    ];
    let mut bytes = [0u8; 16];
    for (index, value) in values.iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn parse_cdr(data: &[u8], archive_name: &str, cdr_position: u64, count: usize) -> Result<SignedPakInfo, SignedPakError> {
    let mut at = 0usize;
    let mut paths = HashSet::new();
    let mut spans = Vec::new();
    let mut entries = Vec::with_capacity(count);
    let mut total = 0u64;
    for _ in 0..count {
        if data.get(at..at + 4) != Some(b"PK\x01\x02") {
            return Err(SignedPakError::Format("invalid central directory signature".into()));
        }
        let name_length = read_u16(data, at + 28)? as usize;
        let extra_length = read_u16(data, at + 30)? as usize;
        let comment_length = read_u16(data, at + 32)? as usize;
        let next = at.checked_add(46)
            .and_then(|v| v.checked_add(name_length))
            .and_then(|v| v.checked_add(extra_length))
            .and_then(|v| v.checked_add(comment_length)).ok_or(SignedPakError::Limits)?;
        let raw = data.get(at + 46..at + 46 + name_length)
            .ok_or_else(|| SignedPakError::Format("truncated signed member name".into()))?;
        let name = std::str::from_utf8(raw).map_err(|_| SignedPakError::Format("non-UTF8 signed path".into()))?;
        let path = safe_member_name(name).map_err(|_| SignedPakError::UnsafePath(name.to_string()))?;
        if !paths.insert(path.to_lowercase()) {
            return Err(SignedPakError::UnsafePath("duplicate signed member path".into()));
        }
        if data.get(at..next).is_none() {
            return Err(SignedPakError::Format("truncated central directory entry".into()));
        }
        let flags = read_u16(data, at + 8)?;
        let method = read_u16(data, at + 10)?;
        let crc32 = read_u32(data, at + 16)?;
        let compressed_size = read_u32(data, at + 20)? as u64;
        let size = read_u32(data, at + 24)? as u64;
        let local_header_offset = read_u32(data, at + 42)? as u64;
        if !matches!(method, 13 | 14) || extra_length != 0
            || (method == 13 && compressed_size != size) || size > MAX_ENTRY_BYTES
        {
            return Err(SignedPakError::Format("unsupported signed member method or sizes".into()));
        }
        let data_offset = local_header_offset.checked_add(30)
            .and_then(|v| v.checked_add(name_length as u64)).ok_or(SignedPakError::Limits)?;
        let end = data_offset.checked_add(compressed_size).ok_or(SignedPakError::Limits)?;
        if end > cdr_position || data_offset <= local_header_offset {
            return Err(SignedPakError::Format("encrypted local payload outside signed bounds".into()));
        }
        spans.push((local_header_offset, end));
        total = total.checked_add(size).ok_or(SignedPakError::Limits)?;
        if total > MAX_TOTAL_BYTES {
            return Err(SignedPakError::Limits);
        }
        entries.push(SignedEntry {
            path, method, flags, crc32, compressed_size, size,
            encrypted_data_offset: data_offset,
        });
        at = next;
    }
    if at != data.len() {
        return Err(SignedPakError::Format("unexpected signed directory trailing bytes".into()));
    }
    spans.sort_unstable();
    for pair in spans.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(SignedPakError::Format("overlapping encrypted entry ranges".into()));
        }
    }
    Ok(SignedPakInfo {
        version: 1,
        archive_name: archive_name.to_string(),
        signature_verified: true,
        total_expanded_bytes: total,
        entries,
    })
}

/// Parse+verify signed metadata and unwrap encryption secrets using a public
/// key supplied by the local user. Refuses archives with invalid signatures.
pub fn open_verified_signed_pak(source: &Path, der_public_key: &Path, signed_basename: &str) -> Result<VerifiedSignedPak, SignedPakError> {
    if signed_basename.is_empty() || !signed_basename.is_ascii() || signed_basename.contains(['/', '\\'])
        || !signed_basename.to_ascii_lowercase().ends_with(".pak")
    {
        return Err(SignedPakError::Format("expected original signed .pak basename".into()));
    }
    let meta = fs::symlink_metadata(source)?;
    let pub_meta = fs::symlink_metadata(der_public_key)?;
    if !meta.file_type().is_file() || !pub_meta.file_type().is_file() || pub_meta.len() > 8 * 1024 {
        return Err(SignedPakError::Format("archive/key must be local non-linked regular files".into()));
    }
    let pubkey = parse_public_key(&fs::read(der_public_key)?)?;
    let mut file = File::open(source)?;
    let file_len = meta.len();
    if file_len < TRAILER_BYTES as u64 + 22 {
        return Err(SignedPakError::Format("archive too small".into()));
    }
    let tail_size = file_len.min(65557);
    file.seek(SeekFrom::Start(file_len - tail_size))?;
    let mut tail = vec![0u8; tail_size as usize];
    file.read_exact(&mut tail)?;
    let (at, count, cd_size, cd_pos) = (0..=tail.len() - 22)
        .rev()
        .find_map(|index| {
            if tail.get(index..index + 4) != Some(b"PK\x05\x06") { return None; }
            let disks = (read_u16(&tail, index + 4).ok()?, read_u16(&tail, index + 6).ok()?);
            let local_count = read_u16(&tail, index + 8).ok()?;
            let count = read_u16(&tail, index + 10).ok()?;
            let cd_size = read_u32(&tail, index + 12).ok()? as usize;
            let cd_pos = read_u32(&tail, index + 16).ok()? as u64;
            let comment = read_u16(&tail, index + 20).ok()? as usize;
            if disks != (0, 0) || count != local_count || comment != TRAILER_BYTES
                || index + 22 + comment != tail.len() { return None; }
            Some((index, count as usize, cd_size, cd_pos))
        })
        .ok_or_else(|| SignedPakError::Format("missing terminated signed EOCD".into()))?;
    if count == 0 || count > MAX_ENTRIES || cd_size == 0 || cd_size > MAX_CDR_BYTES {
        return Err(SignedPakError::Limits);
    }
    let eocd_at = file_len - tail_size + at as u64;
    let computed_cdr = eocd_at.checked_sub(cd_size as u64).ok_or(SignedPakError::Limits)?;
    if computed_cdr != cd_pos {
        return Err(SignedPakError::Format("signed central directory offset mismatch".into()));
    }
    let comment = &tail[at + 22..];
    if comment.get(..6) != Some(b"\x06\x00\x00\x00\x01\x03")
        || read_u32(comment, 6)? != 133 || read_u32(comment, 139)? != 2181
    {
        return Err(SignedPakError::Format("unsupported signed comment structure".into()));
    }
    let iv = unwrap_key(&comment[143..271], &pubkey)?;
    let mut keys = [[0u8; 16]; 16];
    for (i, slot) in keys.iter_mut().enumerate() {
        *slot = unwrap_key(&comment[272 + i * RSA_BYTES..272 + (i + 1) * RSA_BYTES], &pubkey)?;
    }
    file.seek(SeekFrom::Start(cd_pos))?;
    let mut ciphertext = vec![0u8; cd_size];
    file.read_exact(&mut ciphertext)?;
    let cdr = crypt_bytes(&ciphertext, &keys[0], iv)?;
    let mut signed_message = Vec::with_capacity(cdr.len() + signed_basename.len());
    signed_message.extend_from_slice(&cdr);
    signed_message.extend_from_slice(signed_basename.as_bytes());
    let digest = Sha256::digest(&signed_message);
    pubkey.verify(Pss::new_with_salt::<Sha256>(0), &digest, &comment[11..139])
        .map_err(|_| SignedPakError::Crypto("RSA-PSS SHA256 signature incorrect".into()))?;
    let info = parse_cdr(&cdr, signed_basename, cd_pos, count)?;
    Ok(VerifiedSignedPak { info, keys })
}

pub fn inspect_signed_pak(source: &Path, der_public_key: &Path, signed_basename: &str) -> Result<SignedPakInfo, SignedPakError> {
    Ok(open_verified_signed_pak(source, der_public_key, signed_basename)?.info)
}

/// Decrypt/DEFLATE the whole verified PAK using capped streaming, CRC checks,
/// and an uncommitted temporary directory. Refuses a preexisting output folder.
/// Encrypted local headers are intentionally NOT interpreted as plaintext ZIP.
pub fn extract_signed_pak(source: &Path, der_public_key: &Path, signed_basename: &str, destination: &Path) -> Result<SignedPakInfo, SignedPakError> {
    let verified = open_verified_signed_pak(source, der_public_key, signed_basename)?;
    if destination.exists() || fs::symlink_metadata(destination).is_ok() {
        return Err(SignedPakError::UnsafeOutput);
    }
    let parent = destination.parent().ok_or(SignedPakError::UnsafeOutput)?;
    if !parent.is_dir() || fs::symlink_metadata(parent)?.file_type().is_symlink() {
        return Err(SignedPakError::UnsafeOutput);
    }
    let stage = tempfile::Builder::new().prefix(".signed-pak-").tempdir_in(parent)?;
    let mut source = File::open(source)?;
    for entry in &verified.info.entries {
        source.seek(SeekFrom::Start(entry.encrypted_data_offset))?;
        let slot = (!(entry.crc32 >> 2) & 15) as usize;
        let ctr = CtrRead::new((&mut source).take(entry.compressed_size), &verified.keys[slot], entry_iv(entry));
        let target = stage.path().join(PathBuf::from(&entry.path));
        if let Some(folder) = target.parent() {
            fs::create_dir_all(folder)?;
        }
        let mut output = fs::OpenOptions::new().create_new(true).write(true).open(target)?;
        let mut hasher = Hasher::new();
        let mut written = 0u64;
        let mut stream: Box<dyn Read + '_> = if entry.method == 14 {
            Box::new(DeflateDecoder::new(ctr))
        } else {
            Box::new(ctr)
        };
        let mut buffer = [0u8; 128 * 1024];
        loop {
            let n = stream.read(&mut buffer)?;
            if n == 0 { break; }
            written = written.checked_add(n as u64).ok_or(SignedPakError::Limits)?;
            if written > entry.size {
                return Err(SignedPakError::Format("expanded signed entry exceeded verified length".into()));
            }
            hasher.update(&buffer[..n]);
            output.write_all(&buffer[..n])?;
        }
        if written != entry.size || hasher.finalize() != entry.crc32 {
            return Err(SignedPakError::Crypto("decrypted entry length or CRC mismatch".into()));
        }
        output.sync_all()?;
    }
    if destination.exists() || fs::symlink_metadata(destination).is_ok() {
        return Err(SignedPakError::UnsafeOutput);
    }
    fs::rename(stage.path(), destination)?;
    Ok(verified.info)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ctr_is_symmetric_across_arbitrary_reader_boundaries() {
        let key = [7u8; 16];
        let iv = [29u8; 16];
        let bytes = (0..32789).map(|n| (n % 251) as u8).collect::<Vec<_>>();
        let encrypted = crypt_bytes(&bytes, &key, iv).unwrap();
        let fragmented = encrypted.chunks(7).flat_map(|c| c.iter().copied()).collect::<Vec<u8>>();
        let mut reader = CtrRead::new(fragmented.as_slice(), &key, iv);
        let mut result = Vec::new();
        reader.read_to_end(&mut result).unwrap();
        assert_eq!(result, bytes);
    }
    #[test]
    fn signed_cdr_rejects_duplicate_and_outside_payload() {
        let mut cdr = vec![0u8; 46 + 11];
        cdr[..4].copy_from_slice(b"PK\x01\x02");
        cdr[10..12].copy_from_slice(&13u16.to_le_bytes());
        cdr[28..30].copy_from_slice(&11u16.to_le_bytes());
        cdr[46..].copy_from_slice(b"file01.caf!");
        assert!(parse_cdr(&cdr, "Game.pak", 100, 1).is_err());
    }
    #[test]
    fn validates_user_basename_before_opening_archive() {
        let path = Path::new("does-not-exist.pak");
        assert!(open_verified_signed_pak(path, path, "bad/name.pak").is_err());
    }
}
