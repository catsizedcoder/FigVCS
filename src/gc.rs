use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::Result;

use crate::commit::Commit;
use crate::repo::Repository;

#[derive(Debug, Default)]
pub struct GcStats {
    pub repos_scanned: usize,
    pub commits_removed: usize,
    pub objects_removed: usize,
    pub objects_kept: usize,
    pub bytes_freed: u64,
}

pub fn collect(server_dir: &Path, grace: Duration) -> Result<GcStats> {
    let mut stats = GcStats::default();
    let mut marked = BTreeSet::new();
    let now = SystemTime::now();

    for entry in fs::read_dir(server_dir)? {
        let path = entry?.path();
        if !path.is_dir() || !path.join(".fvcs").is_dir() {
            continue;
        }
        stats.repos_scanned += 1;
        let repo = Repository::discover(&path)?;
        let reachable = reachable_commits(&repo)?;
        for commit_file in fs::read_dir(repo.commits())? {
            let commit_file = commit_file?.path();
            let hash = commit_file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
                .trim_end_matches(".json")
                .to_string();
            if reachable.contains(&hash) {
                if let Ok(commit) = Commit::load(&repo, &hash) {
                    marked.extend(commit.tree.into_values());
                }
            } else {
                fs::remove_file(&commit_file)?;
                stats.commits_removed += 1;
            }
        }
    }

    let pool = server_dir.join("objects-pool");
    if pool.is_dir() {
        for prefix in fs::read_dir(&pool)? {
            let prefix = prefix?.path();
            if !prefix.is_dir() {
                continue;
            }
            for object in fs::read_dir(&prefix)? {
                let object = object?.path();
                let hash = format!(
                    "{}{}",
                    prefix.file_name().unwrap_or_default().to_string_lossy(),
                    object.file_name().unwrap_or_default().to_string_lossy()
                );
                if marked.contains(&hash) {
                    stats.objects_kept += 1;
                    continue;
                }
                let young = object
                    .metadata()
                    .and_then(|m| m.modified())
                    .map(|mtime| now.duration_since(mtime).unwrap_or_default() < grace)
                    .unwrap_or(false);
                if young {
                    stats.objects_kept += 1;
                    continue;
                }
                stats.bytes_freed += object.metadata().map(|m| m.len()).unwrap_or(0);
                fs::remove_file(&object)?;
                stats.objects_removed += 1;
            }
            if fs::read_dir(&prefix)?.next().is_none() {
                fs::remove_dir(&prefix)?;
            }
        }
    }
    Ok(stats)
}

pub fn repo_size(repo: &Repository, tip: &str) -> Result<u64> {
    let mut bytes = 0u64;
    let mut seen = BTreeSet::new();
    let mut objects = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::from([tip.to_string()]);
    while let Some(hash) = queue.pop_front() {
        if !seen.insert(hash.clone()) {
            continue;
        }
        let path = repo.commits().join(format!("{hash}.json"));
        bytes += fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if let Ok(commit) = Commit::load(repo, &hash) {
            objects.extend(commit.tree.into_values());
            if let Some(parent) = commit.parent {
                queue.push_back(parent);
            }
        }
    }
    for hash in objects {
        let path = repo.objects().join(&hash[..2]).join(&hash[2..]);
        bytes += fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    }
    Ok(bytes)
}

fn reachable_commits(repo: &Repository) -> Result<BTreeSet<String>> {
    let mut tips = Vec::new();
    for branch in crate::refs::list_branches(repo)? {
        if let Some(hash) = crate::refs::branch_commit(repo, &branch)? {
            tips.push(hash);
        }
    }
    let mut seen = BTreeSet::new();
    let mut queue: VecDeque<String> = tips.into();
    while let Some(hash) = queue.pop_front() {
        if !seen.insert(hash.clone()) {
            continue;
        }
        if let Some(parent) = Commit::load(repo, &hash)?.parent {
            queue.push_back(parent);
        }
    }
    Ok(seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn commit_with(repo: &Repository, files: &[(&str, &[u8])], parent: Option<String>) -> String {
        let mut tree = BTreeMap::new();
        for (path, data) in files {
            let hash = crate::object::write(repo, data).unwrap();
            tree.insert(path.to_string(), hash);
        }
        Commit::new(tree, parent, "c".into(), "t".into())
            .store(repo)
            .unwrap()
    }

    #[test]
    fn gc_removes_only_unreachable() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = tmp.path().join("objects-pool");

        let repo_a = Repository::with_shared_objects(tmp.path().join("a"), pool.clone()).unwrap();
        let keep = commit_with(&repo_a, &[("keep.lua", b"keep me")], None);
        let orphan_parent = commit_with(&repo_a, &[("old.lua", b"old stuff")], None);
        let keep_child = commit_with(
            &repo_a,
            &[("keep2.lua", b"keep me too")],
            Some(keep.clone()),
        );
        crate::refs::update_branch(&repo_a, "main", &keep_child).unwrap();

        let repo_b = Repository::with_shared_objects(tmp.path().join("b"), pool.clone()).unwrap();
        commit_with(&repo_b, &[("shared.lua", b"shared")], None);
        crate::object::write(&repo_b, b"never committed").unwrap();

        let stats = collect(tmp.path(), Duration::from_secs(0)).unwrap();
        assert_eq!(stats.repos_scanned, 2);
        assert_eq!(stats.commits_removed, 2);
        assert_eq!(stats.objects_removed, 3);
        assert_eq!(stats.objects_kept, 2);

        let data = crate::object::read(&repo_a, &crate::object::hash_bytes(b"keep me")).unwrap();
        assert_eq!(data, b"keep me");
        assert!(Commit::load(&repo_a, &orphan_parent).is_err());
    }
}
