use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::object;
use crate::repo::Repository;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryLib {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub source: String,
    #[serde(default)]
    pub subdir: Option<String>,
    #[serde(default)]
    pub hashes: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub updated: String,
    #[serde(default)]
    pub libs: Vec<RegistryLib>,
}

pub fn lib_hash(tree: &BTreeMap<String, String>) -> String {
    let mut text = String::new();
    for (path, hash) in tree {
        text.push_str(&format!("{path}:{hash}\n"));
    }
    object::hash_bytes(text.as_bytes())
}

impl Registry {
    pub fn fetch(url: &str) -> Result<Self> {
        let url = format!("{}/registry.json", url.trim_end_matches('/'));
        let response = reqwest::blocking::get(&url)
            .with_context(|| format!("fetching registry from {url}"))?;
        response.error_for_status_ref()?;
        Ok(response.json()?)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).context("parsing registry")
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        fs::write(path, text).context("writing registry")
    }

    pub fn cache_path(repo: &Repository) -> std::path::PathBuf {
        repo.fvcs().join("registry-cache.json")
    }
}

/// Build or refresh a registry from a directory of fvcs library repos.
/// Existing hashes are kept so clients can tell "known old version" apart from "user edits".
pub fn build(dir: &Path) -> Result<Registry> {
    let registry_file = dir.join("registry.json");
    let mut registry = if registry_file.exists() {
        Registry::load(&registry_file)?
    } else {
        Registry::default()
    };
    registry.updated = chrono::Utc::now().format("%Y-%m-%d").to_string();

    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(repo) = Repository::discover(&path) else {
            continue;
        };
        if repo.root != path.canonicalize().unwrap_or(path.clone()) {
            continue;
        }
        let Some(tip) = crate::refs::current_commit(&repo)? else {
            continue;
        };
        let commit = crate::commit::Commit::load(&repo, &tip)?;
        let hash = lib_hash(&commit.tree);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let description = read_avatar_description(&repo, &commit);

        match registry.libs.iter_mut().find(|l| l.name == name) {
            Some(existing) => {
                if existing.hashes.first() != Some(&hash) {
                    existing.hashes.insert(0, hash);
                    existing.hashes.truncate(50);
                }
                if description.is_some() {
                    existing.description = description;
                }
            }
            None => registry.libs.push(RegistryLib {
                name,
                description,
                source: String::new(),
                subdir: None,
                hashes: vec![hash],
            }),
        }
    }
    registry.save(&registry_file)?;
    Ok(registry)
}

fn read_avatar_description(repo: &Repository, commit: &crate::commit::Commit) -> Option<String> {
    let blob = commit.tree.get("avatar.json")?;
    let data = crate::object::read(repo, blob).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&data).ok()?;
    json.get("description")?.as_str().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lib_hash_stable_and_order_independent() {
        let mut a = BTreeMap::new();
        a.insert("x.lua".to_string(), "1".to_string());
        a.insert("y.lua".to_string(), "2".to_string());
        let mut b = BTreeMap::new();
        b.insert("y.lua".to_string(), "2".to_string());
        b.insert("x.lua".to_string(), "1".to_string());
        assert_eq!(lib_hash(&a), lib_hash(&b));
        b.insert("z.lua".to_string(), "3".to_string());
        assert_ne!(lib_hash(&a), lib_hash(&b));
    }

    #[test]
    fn build_collects_and_preserves_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("CoolLib");
        std::fs::create_dir_all(&lib).unwrap();
        let repo = Repository::init(&lib).unwrap();
        std::fs::write(lib.join("cool.lua"), "v1").unwrap();
        let hash1 = crate::object::write(&repo, b"v1").unwrap();
        let mut tree = BTreeMap::new();
        tree.insert("cool.lua".to_string(), hash1);
        let c1 = crate::commit::Commit::new(tree, None, "c1".into(), "t".into());
        let tip1 = c1.store(&repo).unwrap();
        crate::refs::update_branch(&repo, "main", &tip1).unwrap();

        let reg = build(tmp.path()).unwrap();
        assert_eq!(reg.libs.len(), 1);
        assert_eq!(reg.libs[0].hashes.len(), 1);

        let hash2 = crate::object::write(&repo, b"v2").unwrap();
        let mut tree2 = BTreeMap::new();
        tree2.insert("cool.lua".to_string(), hash2);
        let c2 = crate::commit::Commit::new(tree2, Some(tip1), "c2".into(), "t".into());
        let tip2 = c2.store(&repo).unwrap();
        crate::refs::update_branch(&repo, "main", &tip2).unwrap();

        let reg = build(tmp.path()).unwrap();
        assert_eq!(reg.libs[0].hashes.len(), 2);
        assert_eq!(reg.libs[0].hashes[0], lib_hash(&c2.tree));
        assert_eq!(reg.libs[0].hashes[1], lib_hash(&c1.tree));
    }
}
