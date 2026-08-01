use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::repo::Repository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Branch(String),
    Detached(String),
}

pub fn head(repo: &Repository) -> Result<Head> {
    let text = fs::read_to_string(repo.head_file()).context("reading HEAD")?;
    let text = text.trim();
    if let Some(reference) = text.strip_prefix("refs/heads/") {
        Ok(Head::Branch(reference.to_string()))
    } else {
        Ok(Head::Detached(text.to_string()))
    }
}

pub fn set_head_branch(repo: &Repository, name: &str) -> Result<()> {
    fs::write(repo.head_file(), format!("refs/heads/{name}")).context("writing HEAD")
}

pub fn set_head_detached(repo: &Repository, hash: &str) -> Result<()> {
    fs::write(repo.head_file(), hash).context("writing HEAD")
}

fn read_ref(path: &Path) -> Result<String> {
    Ok(fs::read_to_string(path)?.trim().to_string())
}

pub fn branch_commit(repo: &Repository, name: &str) -> Result<Option<String>> {
    let path = repo.heads().join(name);
    if path.exists() {
        Ok(Some(read_ref(&path)?))
    } else {
        Ok(None)
    }
}

pub fn update_branch(repo: &Repository, name: &str, hash: &str) -> Result<()> {
    fs::write(repo.heads().join(name), hash).with_context(|| format!("updating branch {name}"))
}

pub fn delete_branch(repo: &Repository, name: &str) -> Result<()> {
    let path = repo.heads().join(name);
    if !path.exists() {
        bail!("branch '{name}' does not exist");
    }
    fs::remove_file(path).with_context(|| format!("deleting branch {name}"))
}

pub fn list_branches(repo: &Repository) -> Result<Vec<String>> {
    list_ref_dir(&repo.heads())
}

pub fn list_tags(repo: &Repository) -> Result<Vec<String>> {
    list_ref_dir(&repo.tags())
}

fn list_ref_dir(dir: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(dir)? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names)
}

pub fn create_tag(repo: &Repository, name: &str, hash: &str) -> Result<()> {
    let path = repo.tags().join(name);
    if path.exists() {
        bail!("tag '{name}' already exists");
    }
    fs::write(path, hash).with_context(|| format!("creating tag {name}"))
}

pub fn current_commit(repo: &Repository) -> Result<Option<String>> {
    match head(repo)? {
        Head::Branch(name) => branch_commit(repo, &name),
        Head::Detached(hash) => Ok(Some(hash)),
    }
}

pub fn resolve(repo: &Repository, rev: &str) -> Result<String> {
    if let Some(hash) = branch_commit(repo, rev)? {
        return Ok(hash);
    }
    let tag_path = repo.tags().join(rev);
    if tag_path.exists() {
        return read_ref(&tag_path);
    }
    if rev.len() == 64 && repo.commits().join(format!("{rev}.json")).exists() {
        return Ok(rev.to_string());
    }
    if rev.len() >= 4 && rev.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut matches = Vec::new();
        for entry in fs::read_dir(repo.commits())? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if let Some(hash) = name.strip_suffix(".json") {
                if hash.starts_with(rev) {
                    matches.push(hash.to_string());
                }
            }
        }
        match matches.len() {
            1 => return Ok(matches.pop().unwrap()),
            0 => {}
            _ => bail!("ambiguous revision '{rev}'"),
        }
    }
    bail!("unknown revision '{rev}'")
}
