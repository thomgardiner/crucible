//! SHA-256 helpers. Digests are the trusted-computing-base of the gate arm: a
//! checker's bytes, a pinned file, the folded gate fingerprint. Hex output matches
//! the Node reference (`crypto.createHash('sha256')...digest('hex')`) byte for byte
//! so approvals minted by either implementation verify against the other.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
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

/// Stream a whole file into SHA-256 without buffering it all in memory.
pub fn sha256_hex_stream_file(path: &Path) -> Result<String> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .with_context(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(to_hex(&h.finalize()))
}

/// Content digest for receipt fingerprints. Full stream when the file is at most
/// `full_max` bytes; larger files use length + head + tail so Stop cannot hang on a
/// multi-GB dirty build product while still seeing most rewrites.
pub fn content_fingerprint(path: &Path, full_max: u64) -> Result<String> {
    let meta = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let len = meta.len();
    if len <= full_max {
        return sha256_hex_stream_file(path);
    }
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut h = Sha256::new();
    h.update(len.to_le_bytes());
    let chunk = 64 * 1024usize;
    let mut buf = vec![0u8; chunk];
    let n = f
        .read(&mut buf)
        .with_context(|| format!("reading head of {}", path.display()))?;
    h.update(&buf[..n]);
    if len > chunk as u64 {
        let back = len.saturating_sub(chunk as u64);
        f.seek(SeekFrom::Start(back))
            .with_context(|| format!("seeking {}", path.display()))?;
        let n = f
            .read(&mut buf)
            .with_context(|| format!("reading tail of {}", path.display()))?;
        h.update(&buf[..n]);
    }
    Ok(to_hex(&h.finalize()))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_fingerprint_matches_full_stream_under_cap() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.rs");
        std::fs::write(&p, b"fn a() { let x = 1; }\n").unwrap();
        let a = content_fingerprint(&p, 1024).unwrap();
        let b = sha256_hex_stream_file(&p).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn content_fingerprint_large_mode_changes_when_tail_changes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.bin");
        // Cap forces head+tail mode while keeping the fixture small for tests.
        let mut body = vec![b'a'; 200];
        body.extend(std::iter::repeat_n(b'b', 200));
        std::fs::write(&p, &body).unwrap();
        let before = content_fingerprint(&p, 50).unwrap();
        let last = body.len() - 1;
        body[last] = b'Z';
        std::fs::write(&p, &body).unwrap();
        let after = content_fingerprint(&p, 50).unwrap();
        assert_ne!(before, after, "tail rewrite must change large-mode digest");
    }
}
