use std::collections::{BTreeSet, VecDeque};
use std::fs;

use anyhow::{bail, Context, Result};

use crate::commit::Commit;
use crate::http_store::HttpStore;
use crate::object;
use crate::repo::Repository;

pub trait Store {
    fn head_branch(&self) -> Result<Option<String>>;
    fn branch_commit(&self, branch: &str) -> Result<Option<String>>;
    fn update_branch(&self, branch: &str, hash: &str) -> Result<()>;
    fn has_commit(&self, hash: &str) -> Result<bool>;
    fn read_commit(&self, hash: &str) -> Result<Commit>;
    fn write_commit(&self, commit: &Commit) -> Result<String>;
    fn has_object(&self, hash: &str) -> Result<bool>;
    fn read_object_wire(&self, hash: &str) -> Result<Vec<u8>>;
    fn write_object_wire(&self, hash: &str, compressed: &[u8]) -> Result<()>;
    fn write_readme(&self, content: &str) -> Result<()>;

    fn read_object(&self, hash: &str) -> Result<Vec<u8>> {
        object::decompress(&self.read_object_wire(hash)?)
    }
    fn write_object(&self, raw: &[u8]) -> Result<String> {
        let hash = object::hash_bytes(raw);
        if !self.has_object(&hash)? {
            self.write_object_wire(&hash, &object::compress(raw)?)?;
        }
        Ok(hash)
    }

    fn set_visibility(&self, _visibility: &str) -> Result<()> {
        bail!("visibility only applies to server remotes")
    }
    fn share(&self, _username: &str, _remove: bool) -> Result<()> {
        bail!("sharing only applies to server remotes")
    }
    fn repo_name(&self) -> Option<String> {
        None
    }
    fn delete_repo(&self) -> Result<()> {
        bail!("deleting repos only applies to server remotes")
    }
}

impl Store for Repository {
    fn head_branch(&self) -> Result<Option<String>> {
        match crate::refs::head(self)? {
            crate::refs::Head::Branch(name) => Ok(Some(name)),
            crate::refs::Head::Detached(_) => Ok(None),
        }
    }
    fn branch_commit(&self, branch: &str) -> Result<Option<String>> {
        crate::refs::branch_commit(self, branch)
    }
    fn update_branch(&self, branch: &str, hash: &str) -> Result<()> {
        crate::refs::update_branch(self, branch, hash)
    }
    fn has_commit(&self, hash: &str) -> Result<bool> {
        Ok(self.commits().join(format!("{hash}.json")).exists())
    }
    fn read_commit(&self, hash: &str) -> Result<Commit> {
        Commit::load(self, hash)
    }
    fn write_commit(&self, commit: &Commit) -> Result<String> {
        commit.store(self)
    }
    fn has_object(&self, hash: &str) -> Result<bool> {
        Ok(object::exists(self, hash))
    }
    fn read_object_wire(&self, hash: &str) -> Result<Vec<u8>> {
        object::read_compressed(self, hash)
    }
    fn write_object_wire(&self, hash: &str, compressed: &[u8]) -> Result<()> {
        object::write_compressed(self, hash, compressed)
    }
    fn write_readme(&self, content: &str) -> Result<()> {
        fs::write(self.root.join("README.md"), content).context("writing README.md")
    }
}

pub fn open_store(url: &str, token: Option<&str>) -> Result<Box<dyn Store>> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(Box::new(HttpStore::new(url, token)?))
    } else {
        Ok(Box::new(open_or_init_remote(url)?))
    }
}

pub fn open_store_readonly(url: &str, token: Option<&str>) -> Result<Box<dyn Store>> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(Box::new(HttpStore::new(url, token)?))
    } else {
        Ok(Box::new(Repository::discover(std::path::Path::new(url))?))
    }
}

pub fn reachable(store: &dyn Store, tip: &str) -> Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([tip.to_string()]);
    while let Some(hash) = queue.pop_front() {
        if !seen.insert(hash.clone()) {
            continue;
        }
        if let Some(parent) = store.read_commit(&hash)?.parent {
            queue.push_back(parent);
        }
    }
    Ok(seen.into_iter().collect())
}

pub fn is_ancestor(store: &dyn Store, ancestor: &str, descendant: &str) -> Result<bool> {
    Ok(reachable(store, descendant)?.iter().any(|h| h == ancestor))
}

pub fn copy_history(from: &dyn Store, to: &dyn Store, commits: &[String]) -> Result<usize> {
    let mut copied = 0;
    for hash in commits {
        let commit = from.read_commit(hash)?;
        if !to.has_commit(hash)? {
            to.write_commit(&commit)?;
            copied += 1;
        }
        for blob in commit.tree.values() {
            if !to.has_object(blob)? {
                let wire = from
                    .read_object_wire(blob)
                    .with_context(|| format!("copying object {blob}"))?;
                to.write_object_wire(blob, &wire)?;
            }
        }
    }
    Ok(copied)
}

pub fn open_or_init_remote(path: &str) -> Result<Repository> {
    let dir = std::path::Path::new(path);
    match Repository::discover(dir) {
        Ok(repo) => Ok(repo),
        Err(_) => {
            if dir.exists() && fs::read_dir(dir)?.next().is_some() {
                bail!("'{path}' exists but is not a FigVCS repository");
            }
            Repository::init(dir)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refs;
    use std::collections::BTreeMap;

    fn commit_chain(repo: &Repository, count: usize) -> Vec<String> {
        let mut hashes = Vec::new();
        for i in 0..count {
            let mut tree = BTreeMap::new();
            tree.insert(format!("f{i}.lua"), format!("hash{i}"));
            let parent = hashes.last().cloned();
            let c = Commit::new(tree, parent, format!("c{i}"), "t".into());
            hashes.push(c.store(repo).unwrap());
        }
        hashes
    }

    #[test]
    fn reachable_and_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Repository::init(tmp.path()).unwrap();
        let chain = commit_chain(&repo, 3);
        assert_eq!(reachable(&repo, &chain[2]).unwrap().len(), 3);
        assert!(is_ancestor(&repo, &chain[0], &chain[2]).unwrap());
        assert!(!is_ancestor(&repo, &chain[2], &chain[0]).unwrap());
    }

    #[test]
    fn open_or_init_creates_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = tmp.path().join("hub");
        let repo = open_or_init_remote(remote.to_str().unwrap()).unwrap();
        assert!(repo.fvcs().is_dir());
        assert!(refs::current_commit(&repo).unwrap().is_none());
    }
}
