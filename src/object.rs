use std::fs;
use std::io::{Read, Write};

use anyhow::{Context, Result};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::repo::Repository;

pub fn hash_bytes(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn object_path(repo: &Repository, hash: &str) -> std::path::PathBuf {
    repo.objects().join(&hash[..2]).join(&hash[2..])
}

pub fn write(repo: &Repository, data: &[u8]) -> Result<String> {
    let hash = hash_bytes(data);
    let path = object_path(repo, &hash);
    if path.exists() {
        return Ok(hash);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    let compressed = encoder.finish()?;
    fs::write(&path, compressed).with_context(|| format!("writing object {hash}"))?;
    Ok(hash)
}

pub fn read(repo: &Repository, hash: &str) -> Result<Vec<u8>> {
    let path = object_path(repo, hash);
    let compressed =
        fs::read(&path).with_context(|| format!("reading object {hash} (missing?)"))?;
    let mut decoder = ZlibDecoder::new(&compressed[..]);
    let mut data = Vec::new();
    decoder.read_to_end(&mut data)?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_dedup() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::init(tmp.path()).unwrap();
        let h1 = write(&repo, b"hello figura").unwrap();
        let h2 = write(&repo, b"hello figura").unwrap();
        assert_eq!(h1, h2);
        assert_eq!(read(&repo, &h1).unwrap(), b"hello figura");
        let mut count = 0;
        for entry in walkdir::WalkDir::new(repo.objects()) {
            if entry.unwrap().file_type().is_file() {
                count += 1;
            }
        }
        assert_eq!(count, 1);
    }
}
