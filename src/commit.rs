use std::collections::BTreeMap;
use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::object;
use crate::repo::Repository;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub tree: BTreeMap<String, String>,
    pub parent: Option<String>,
    pub message: String,
    pub author: String,
    pub timestamp: String,
}

impl Commit {
    pub fn new(
        tree: BTreeMap<String, String>,
        parent: Option<String>,
        message: String,
        author: String,
    ) -> Self {
        Commit {
            tree,
            parent,
            message,
            author,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn store(&self, repo: &Repository) -> Result<String> {
        let json = self.to_json()?;
        let hash = object::hash_bytes(&json);
        let path = repo.commits().join(format!("{hash}.json"));
        if !path.exists() {
            fs::write(&path, &json).with_context(|| format!("writing commit {hash}"))?;
        }
        Ok(hash)
    }

    pub fn to_json(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    pub fn from_json(data: &[u8]) -> Result<Self> {
        serde_json::from_slice(data).context("parsing commit")
    }

    pub fn load(repo: &Repository, hash: &str) -> Result<Self> {
        let path = repo.commits().join(format!("{hash}.json"));
        let text = fs::read_to_string(&path).with_context(|| format!("reading commit {hash}"))?;
        serde_json::from_str(&text).with_context(|| format!("parsing commit {hash}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::init(tmp.path()).unwrap();
        let mut tree = BTreeMap::new();
        tree.insert("Init.lua".to_string(), "abc123".to_string());
        let commit = Commit::new(tree, None, "initial".into(), "tester".into());
        let hash = commit.store(&repo).unwrap();
        let loaded = Commit::load(&repo, &hash).unwrap();
        assert_eq!(loaded.message, "initial");
        assert_eq!(loaded.tree["Init.lua"], "abc123");
        assert!(loaded.parent.is_none());
    }
}
