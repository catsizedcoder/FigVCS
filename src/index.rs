use std::collections::BTreeMap;
use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::repo::Repository;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub hash: String,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Index {
    pub entries: BTreeMap<String, IndexEntry>,
}

impl Index {
    pub fn load(repo: &Repository) -> Result<Self> {
        let path = repo.index_file();
        if !path.exists() {
            return Ok(Index::default());
        }
        let text = fs::read_to_string(&path).context("reading index")?;
        serde_json::from_str(&text).context("parsing index")
    }

    pub fn save(&self, repo: &Repository) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        fs::write(repo.index_file(), text).context("writing index")
    }

    pub fn tree(&self) -> BTreeMap<String, String> {
        self.entries
            .iter()
            .map(|(p, e)| (p.clone(), e.hash.clone()))
            .collect()
    }
}
