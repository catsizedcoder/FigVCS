use std::collections::BTreeMap;
use std::fs;

use anyhow::Result;
use walkdir::WalkDir;

use crate::commit::Commit;
use crate::ignore::Ignore;
use crate::index::{Index, IndexEntry};
use crate::object;
use crate::refs;
use crate::repo::Repository;

#[derive(Debug, Clone)]
pub struct WorkFile {
    pub hash: String,
}

#[derive(Debug, Default)]
pub struct Status {
    pub staged_new: Vec<String>,
    pub staged_modified: Vec<String>,
    pub staged_deleted: Vec<String>,
    pub unstaged_modified: Vec<String>,
    pub unstaged_deleted: Vec<String>,
    pub untracked: Vec<String>,
}

impl Status {
    pub fn is_clean(&self) -> bool {
        self.staged_new.is_empty()
            && self.staged_modified.is_empty()
            && self.staged_deleted.is_empty()
            && self.unstaged_modified.is_empty()
            && self.unstaged_deleted.is_empty()
    }
}

pub fn head_tree(repo: &Repository) -> Result<BTreeMap<String, String>> {
    match refs::current_commit(repo)? {
        Some(hash) => Ok(Commit::load(repo, &hash)?.tree),
        None => Ok(BTreeMap::new()),
    }
}

pub fn collect_workdir(repo: &Repository) -> Result<BTreeMap<String, WorkFile>> {
    let ignore = Ignore::load(&repo.root);
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(&repo.root).min_depth(1) {
        let entry = entry?;
        let path = entry.path();
        let rel = repo.rel(path)?;
        if crate::repo::is_internal(&rel) {
            continue;
        }
        if entry.file_type().is_dir() {
            continue;
        }
        if ignore.is_ignored(&rel) {
            continue;
        }
        let data = fs::read(path)?;
        files.insert(
            rel,
            WorkFile {
                hash: object::hash_bytes(&data),
            },
        );
    }
    Ok(files)
}

pub fn compute(repo: &Repository) -> Result<Status> {
    let head = head_tree(repo)?;
    let index = Index::load(repo)?;
    let workdir = collect_workdir(repo)?;
    Ok(classify(&head, &index.entries, &workdir))
}

pub fn classify(
    head: &BTreeMap<String, String>,
    index: &BTreeMap<String, IndexEntry>,
    workdir: &BTreeMap<String, WorkFile>,
) -> Status {
    let mut status = Status::default();

    for (path, entry) in index {
        match head.get(path) {
            None => status.staged_new.push(path.clone()),
            Some(h) if h != &entry.hash => status.staged_modified.push(path.clone()),
            _ => {}
        }
        match workdir.get(path) {
            None => status.unstaged_deleted.push(path.clone()),
            Some(w) if w.hash != entry.hash => status.unstaged_modified.push(path.clone()),
            _ => {}
        }
    }
    for path in head.keys() {
        if !index.contains_key(path) {
            status.staged_deleted.push(path.clone());
        }
    }
    for path in workdir.keys() {
        if !index.contains_key(path) {
            status.untracked.push(path.clone());
        }
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(hash: &str) -> IndexEntry {
        IndexEntry {
            hash: hash.into(),
            size: 1,
            mtime: 0,
        }
    }
    fn work(hash: &str) -> WorkFile {
        WorkFile { hash: hash.into() }
    }

    #[test]
    fn classify_all_buckets() {
        let mut head = BTreeMap::new();
        head.insert("same.lua".to_string(), "a".to_string());
        head.insert("tweak.lua".to_string(), "a".to_string());
        head.insert("gone.lua".to_string(), "a".to_string());
        head.insert("dirty.lua".to_string(), "d".to_string());
        head.insert("deleted.lua".to_string(), "e".to_string());

        let mut index = BTreeMap::new();
        index.insert("same.lua".to_string(), entry("a"));
        index.insert("tweak.lua".to_string(), entry("b"));
        index.insert("fresh.lua".to_string(), entry("c"));
        index.insert("dirty.lua".to_string(), entry("d"));
        index.insert("deleted.lua".to_string(), entry("e"));

        let mut wd = BTreeMap::new();
        wd.insert("same.lua".to_string(), work("a"));
        wd.insert("tweak.lua".to_string(), work("b"));
        wd.insert("fresh.lua".to_string(), work("c"));
        wd.insert("dirty.lua".to_string(), work("X"));
        wd.insert("loose.lua".to_string(), work("u"));

        let s = classify(&head, &index, &wd);
        assert_eq!(s.staged_new, vec!["fresh.lua"]);
        assert_eq!(s.staged_modified, vec!["tweak.lua"]);
        assert_eq!(s.staged_deleted, vec!["gone.lua"]);
        assert_eq!(s.unstaged_modified, vec!["dirty.lua"]);
        assert_eq!(s.unstaged_deleted, vec!["deleted.lua"]);
        assert_eq!(s.untracked, vec!["loose.lua"]);
        assert!(!s.is_clean());
    }
}
