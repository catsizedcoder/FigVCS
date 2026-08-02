use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct User {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default)]
    pub user: User,
    #[serde(default)]
    pub servers: BTreeMap<String, String>,
}

fn home_dir() -> Result<PathBuf> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .context("could not find your home directory")?;
    Ok(PathBuf::from(home))
}

pub fn server_key(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(split) if split > "https://".len() => trimmed[..split].to_string(),
        _ => trimmed.to_string(),
    }
}

impl GlobalConfig {
    pub fn path() -> Result<PathBuf> {
        Ok(home_dir()?.join(".fvcsconfig"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(GlobalConfig::default());
        }
        let text = fs::read_to_string(&path).context("reading ~/.fvcsconfig")?;
        serde_json::from_str(&text).context("parsing ~/.fvcsconfig")
    }

    pub fn save(&self) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        fs::write(Self::path()?, text).context("writing ~/.fvcsconfig")
    }

    pub fn token_for(&self, url: &str) -> Option<&str> {
        self.servers.get(&server_key(url)).map(|t| t.as_str())
    }

    pub fn author(&self) -> Option<String> {
        match (&self.user.name, &self.user.email) {
            (Some(name), Some(email)) => Some(format!("{name} <{email}>")),
            (Some(name), None) => Some(name.clone()),
            _ => None,
        }
    }
}
