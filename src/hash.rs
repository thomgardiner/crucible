//! SHA-256 helpers. Digests are the trusted-computing-base of the gate arm: a
//! checker's bytes, a pinned file, the folded gate fingerprint. Hex output matches
//! the Node reference (`crypto.createHash('sha256')...digest('hex')`) byte for byte
//! so approvals minted by either implementation verify against the other.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    to_hex(&h.finalize())
}

pub fn sha256_hex_of_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}
