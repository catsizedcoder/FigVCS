use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::repo::Repository;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Remote {
    Plain(String),
    Full { url: String, token: Option<String> },
}

impl Remote {
    pub fn url(&self) -> &str {
        match self {
            Remote::Plain(url) => url,
            Remote::Full { url, .. } => url,
        }
    }
    pub fn token(&self) -> Option<&str> {
        match self {
            Remote::Plain(_) => None,
            Remote::Full { token, .. } => token.as_deref(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub remotes: BTreeMap<String, Remote>,
    #[serde(default)]
    pub no_readme: bool,
    #[serde(default)]
    pub registry: Option<String>,
}

impl Config {
    pub fn load(repo: &Repository) -> Result<Self> {
        let path = repo.fvcs().join("config.json");
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = fs::read_to_string(&path).context("reading config")?;
        serde_json::from_str(&text).context("parsing config")
    }

    pub fn save(&self, repo: &Repository) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        fs::write(repo.fvcs().join("config.json"), text).context("writing config")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lib {
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub subdir: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

pub fn libs_path(root: &Path) -> std::path::PathBuf {
    root.join(".fvcslibs")
}

pub fn load_libs(repo: &Repository) -> Result<Vec<Lib>> {
    let path = libs_path(&repo.root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path).context("reading .fvcslibs")?;
    serde_json::from_str(&text).context("parsing .fvcslibs")
}

pub fn save_libs(repo: &Repository, libs: &[Lib]) -> Result<()> {
    let text = serde_json::to_string_pretty(libs)?;
    fs::write(libs_path(&repo.root), text).context("writing .fvcslibs")
}
