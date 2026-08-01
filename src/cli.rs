use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;

use crate::commit::Commit;
use crate::diff;
use crate::ignore::Ignore;
use crate::index::{Index, IndexEntry};
use crate::object;
use crate::refs::{self, Head};
use crate::repo::Repository;
use crate::status::{self, Status};

#[derive(Parser)]
#[command(
    name = "fvcs",
    version,
    about = "FigVCS | version control for Figura avatars"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Create a new FigVCS repository")]
    Init { path: Option<PathBuf> },
    #[command(about = "Stage files for the next commit")]
    Add { paths: Vec<PathBuf> },
    #[command(about = "Show staged, unstaged and untracked changes")]
    Status,
    #[command(about = "Record staged changes as a new version")]
    Commit {
        #[arg(short, long)]
        message: String,
    },
    #[command(about = "Show commit history")]
    Log {
        #[arg(long)]
        oneline: bool,
        #[arg(short = 'n', help = "Limit the number of commits shown")]
        count: Option<usize>,
    },
    #[command(about = "Show differences (workdir vs index, --cached, or between commits)")]
    Diff {
        a: Option<String>,
        b: Option<String>,
        #[arg(long, help = "Compare the index against HEAD instead of the workdir")]
        cached: bool,
    },
    #[command(about = "Switch to a branch or commit")]
    Checkout {
        target: String,
        #[arg(long, help = "Discard uncommitted changes")]
        force: bool,
    },
    #[command(about = "Unstage files (--staged) or discard workdir changes")]
    Restore {
        paths: Vec<PathBuf>,
        #[arg(long)]
        staged: bool,
    },
    #[command(about = "List, create or delete branches")]
    Branch {
        name: Option<String>,
        #[arg(short, long)]
        delete: bool,
    },
    #[command(about = "List or create tags")]
    Tag {
        name: Option<String>,
        commit: Option<String>,
    },
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { path } => cmd_init(path),
        Command::Add { paths } => cmd_add(paths),
        Command::Status => cmd_status(),
        Command::Commit { message } => cmd_commit(&message),
        Command::Log { oneline, count } => cmd_log(oneline, count),
        Command::Diff { a, b, cached } => cmd_diff(a, b, cached),
        Command::Checkout { target, force } => cmd_checkout(&target, force),
        Command::Restore { paths, staged } => cmd_restore(paths, staged),
        Command::Branch { name, delete } => cmd_branch(name, delete),
        Command::Tag { name, commit } => cmd_tag(name, commit),
    }
}

fn open_repo() -> Result<Repository> {
    Repository::discover(&std::env::current_dir()?)
}

fn cmd_init(path: Option<PathBuf>) -> Result<()> {
    let path = path.unwrap_or_else(|| PathBuf::from("."));
    let repo = Repository::init(&path)?;
    println!(
        "Initialized empty FigVCS repository in {}",
        repo.fvcs().display()
    );
    let avatar = repo.root.join("avatar.json");
    if avatar.exists() {
        let text = fs::read_to_string(&avatar)?;
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(name) = json.get("name").and_then(|n| n.as_str()) {
                println!("Detected Figura avatar '{name}'");
            }
        }
    }
    Ok(())
}

fn mtime_of(path: &Path) -> i64 {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn cmd_add(paths: Vec<PathBuf>) -> Result<()> {
    if paths.is_empty() {
        bail!("nothing to add | specify paths or `fvcs add .`");
    }
    let repo = open_repo()?;
    let cwd = std::env::current_dir()?.canonicalize()?;
    let ignore = Ignore::load(&repo.root);
    let mut index = Index::load(&repo)?;
    let mut added = 0usize;

    for input in &paths {
        let abs = cwd
            .join(input)
            .canonicalize()
            .with_context(|| format!("path '{}' does not exist", input.display()))?;
        let prefix = if abs.is_dir() {
            Some(repo.rel(&abs)?)
        } else {
            None
        };
        let rel_file = if abs.is_file() {
            repo.rel(&abs)?
        } else {
            String::new()
        };
        let mut files = Vec::new();
        if abs.is_dir() {
            for entry in walkdir::WalkDir::new(&abs) {
                let entry = entry?;
                if entry.file_type().is_file() {
                    files.push(entry.path().to_path_buf());
                }
            }
        } else {
            files.push(abs);
        }
        let vanished: Vec<String> = index
            .entries
            .keys()
            .filter(|p| match &prefix {
                Some(dir) if !dir.is_empty() => p.starts_with(&format!("{dir}/")),
                Some(_) => true,
                None => *p == &rel_file,
            })
            .filter(|p| !repo.abs(p).exists())
            .cloned()
            .collect();
        for p in vanished {
            index.entries.remove(&p);
        }
        for file in files {
            let rel = repo.rel(&file)?;
            if crate::repo::is_internal(&rel) {
                continue;
            }
            if ignore.is_ignored(&rel) {
                continue;
            }
            let data = fs::read(&file)?;
            let hash = object::write(&repo, &data)?;
            index.entries.insert(
                rel,
                IndexEntry {
                    hash,
                    size: data.len() as u64,
                    mtime: mtime_of(&file),
                },
            );
            added += 1;
        }
    }
    index.save(&repo)?;
    println!("staged {added} file(s)");
    Ok(())
}

fn cmd_status() -> Result<()> {
    let repo = open_repo()?;
    match refs::head(&repo)? {
        Head::Branch(name) => println!("On branch {}", name.bold()),
        Head::Detached(hash) => println!("HEAD detached at {}", hash[..8].yellow()),
    }
    let s = status::compute(&repo)?;
    if s.is_clean() && s.untracked.is_empty() {
        println!("nothing to commit, working tree clean");
        return Ok(());
    }
    if !s.staged_new.is_empty() || !s.staged_modified.is_empty() || !s.staged_deleted.is_empty() {
        println!("Changes to be committed:");
        for p in &s.staged_new {
            println!("  {}", format!("new file:   {p}").green());
        }
        for p in &s.staged_modified {
            println!("  {}", format!("modified:   {p}").green());
        }
        for p in &s.staged_deleted {
            println!("  {}", format!("deleted:    {p}").green());
        }
    }
    if !s.unstaged_modified.is_empty() || !s.unstaged_deleted.is_empty() {
        println!("Changes not staged for commit:");
        for p in &s.unstaged_modified {
            println!("  {}", format!("modified:   {p}").red());
        }
        for p in &s.unstaged_deleted {
            println!("  {}", format!("deleted:    {p}").red());
        }
    }
    if !s.untracked.is_empty() {
        println!("Untracked files:");
        for p in &s.untracked {
            println!("  {}", p.red());
        }
    }
    Ok(())
}

fn cmd_commit(message: &str) -> Result<()> {
    let repo = open_repo()?;
    let index = Index::load(&repo)?;
    if index.entries.is_empty() {
        bail!("nothing to commit (stage files with `fvcs add` first)");
    }
    let parent = refs::current_commit(&repo)?;
    let tree = index.tree();
    if let Some(parent_hash) = &parent {
        if Commit::load(&repo, parent_hash)?.tree == tree {
            bail!("nothing to commit, tree matches HEAD");
        }
    }
    let author = std::env::var("FVCS_AUTHOR")
        .or_else(|_| std::env::var("USERNAME"))
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".into());
    let commit = Commit::new(tree, parent.clone(), message.to_string(), author);
    let hash = commit.store(&repo)?;
    match refs::head(&repo)? {
        Head::Branch(name) => refs::update_branch(&repo, &name, &hash)?,
        Head::Detached(_) => refs::set_head_detached(&repo, &hash)?,
    }
    let branch = match refs::head(&repo)? {
        Head::Branch(name) => name,
        Head::Detached(_) => "detached".to_string(),
    };
    println!("[{branch} {}] {message}", &hash[..8]);
    Ok(())
}

fn cmd_log(oneline: bool, count: Option<usize>) -> Result<()> {
    let repo = open_repo()?;
    let mut current = refs::current_commit(&repo)?;
    let mut shown = 0usize;
    while let Some(hash) = current {
        if let Some(limit) = count {
            if shown >= limit {
                break;
            }
        }
        let commit = Commit::load(&repo, &hash)?;
        if oneline {
            println!("{} {}", hash[..8].yellow(), commit.message);
        } else {
            println!("{} {}", "commit".yellow(), hash.yellow());
            println!("Author: {}", commit.author);
            println!("Date:   {}", commit.timestamp);
            println!();
            for line in commit.message.lines() {
                println!("    {line}");
            }
            println!();
        }
        shown += 1;
        current = commit.parent;
    }
    if shown == 0 {
        println!("no commits yet");
    }
    Ok(())
}

type FileMap = BTreeMap<String, Vec<u8>>;

fn tree_file_map(repo: &Repository, rev: &str) -> Result<FileMap> {
    let hash = refs::resolve(repo, rev)?;
    let commit = Commit::load(repo, &hash)?;
    let mut map = BTreeMap::new();
    for (path, blob) in &commit.tree {
        map.insert(path.clone(), object::read(repo, blob)?);
    }
    Ok(map)
}

fn index_file_map(repo: &Repository) -> Result<FileMap> {
    let index = Index::load(repo)?;
    let mut map = BTreeMap::new();
    for (path, entry) in &index.entries {
        map.insert(path.clone(), object::read(repo, &entry.hash)?);
    }
    Ok(map)
}

fn workdir_file_map(repo: &Repository) -> Result<FileMap> {
    let workdir = status::collect_workdir(repo)?;
    let mut map = BTreeMap::new();
    for (path, _) in workdir {
        map.insert(path.clone(), fs::read(repo.abs(&path))?);
    }
    Ok(map)
}

fn head_file_map(repo: &Repository) -> Result<FileMap> {
    let mut map = BTreeMap::new();
    for (path, blob) in status::head_tree(repo)? {
        map.insert(path, object::read(repo, &blob)?);
    }
    Ok(map)
}

fn cmd_diff(a: Option<String>, b: Option<String>, cached: bool) -> Result<()> {
    let repo = open_repo()?;
    match (a, b) {
        (None, None) if cached => {
            diff::print_maps(
                "HEAD",
                &head_file_map(&repo)?,
                "index",
                &index_file_map(&repo)?,
            );
        }
        (None, None) => {
            diff::print_maps(
                "index",
                &index_file_map(&repo)?,
                "workdir",
                &workdir_file_map(&repo)?,
            );
        }
        (Some(rev), None) => {
            diff::print_maps(
                &rev,
                &tree_file_map(&repo, &rev)?,
                "workdir",
                &workdir_file_map(&repo)?,
            );
        }
        (Some(ra), Some(rb)) => {
            diff::print_maps(
                &ra,
                &tree_file_map(&repo, &ra)?,
                &rb,
                &tree_file_map(&repo, &rb)?,
            );
        }
        (None, Some(_)) => bail!("usage: fvcs diff <a> <b>"),
    }
    Ok(())
}

fn ensure_clean(s: &Status) -> Result<()> {
    if !s.is_clean() {
        bail!("you have uncommitted changes | commit them or use --force to discard");
    }
    Ok(())
}

fn cmd_checkout(target: &str, force: bool) -> Result<()> {
    let repo = open_repo()?;
    let s = status::compute(&repo)?;
    if !force {
        ensure_clean(&s)?;
    }
    let branch_exists = refs::branch_commit(&repo, target)?.is_some();
    let hash = refs::resolve(&repo, target)?;
    let commit = Commit::load(&repo, &hash)?;

    let tracked_now = status::head_tree(&repo)?;
    let index = Index::load(&repo)?;
    let mut tracked: Vec<String> = tracked_now
        .keys()
        .chain(index.entries.keys())
        .cloned()
        .collect();
    tracked.sort();
    tracked.dedup();
    for path in tracked {
        if !commit.tree.contains_key(&path) {
            let abs = repo.abs(&path);
            if abs.exists() {
                fs::remove_file(&abs).with_context(|| format!("removing {}", abs.display()))?;
            }
        }
    }
    let mut new_index = Index::default();
    for (path, blob) in &commit.tree {
        let data = object::read(&repo, blob)?;
        let abs = repo.abs(path);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&abs, &data).with_context(|| format!("writing {}", abs.display()))?;
        new_index.entries.insert(
            path.clone(),
            IndexEntry {
                hash: blob.clone(),
                size: data.len() as u64,
                mtime: mtime_of(&abs),
            },
        );
    }
    new_index.save(&repo)?;

    if branch_exists {
        refs::set_head_branch(&repo, target)?;
        println!("Switched to branch '{target}'");
    } else {
        refs::set_head_detached(&repo, &hash)?;
        println!("HEAD is now at {} {}", &hash[..8], commit.message);
    }
    Ok(())
}

fn cmd_restore(paths: Vec<PathBuf>, staged: bool) -> Result<()> {
    if paths.is_empty() {
        bail!("specify paths to restore, e.g. `fvcs restore .`");
    }
    let repo = open_repo()?;
    let cwd = std::env::current_dir()?.canonicalize()?;
    let head = status::head_tree(&repo)?;
    let mut index = Index::load(&repo)?;

    let mut targets: Vec<String> = Vec::new();
    for input in &paths {
        let abs = cwd.join(input);
        let rel = repo.rel(&abs).unwrap_or_default();
        if input == &PathBuf::from(".") {
            targets.extend(index.entries.keys().cloned());
            targets.extend(head.keys().cloned());
        } else {
            for path in index.entries.keys().chain(head.keys()) {
                if path == &rel || path.starts_with(&format!("{rel}/")) {
                    targets.push(path.clone());
                }
            }
        }
    }
    targets.sort();
    targets.dedup();
    if targets.is_empty() {
        bail!("no tracked files match");
    }

    for path in &targets {
        if staged {
            match head.get(path) {
                Some(hash) => {
                    let size = object::read(&repo, hash)?.len() as u64;
                    index.entries.insert(
                        path.clone(),
                        IndexEntry {
                            hash: hash.clone(),
                            size,
                            mtime: 0,
                        },
                    );
                }
                None => {
                    index.entries.remove(path);
                }
            }
        } else {
            let entry = index
                .entries
                .get(path)
                .with_context(|| format!("'{path}' is not staged"))?;
            let data = object::read(&repo, &entry.hash)?;
            let abs = repo.abs(path);
            if let Some(parent) = abs.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&abs, &data)?;
        }
    }
    index.save(&repo)?;
    println!("restored {} file(s)", targets.len());
    Ok(())
}

fn cmd_branch(name: Option<String>, delete: bool) -> Result<()> {
    let repo = open_repo()?;
    match name {
        None => {
            let current = refs::head(&repo)?;
            for branch in refs::list_branches(&repo)? {
                if current == Head::Branch(branch.clone()) {
                    println!("{} {}", "*".green(), branch.green());
                } else {
                    println!("  {branch}");
                }
            }
        }
        Some(name) if delete => {
            if refs::head(&repo)? == Head::Branch(name.clone()) {
                bail!("cannot delete the branch you are on");
            }
            refs::delete_branch(&repo, &name)?;
            println!("Deleted branch '{name}'");
        }
        Some(name) => {
            let hash = refs::current_commit(&repo)?
                .context("cannot create a branch before the first commit")?;
            if refs::branch_commit(&repo, &name)?.is_some() {
                bail!("branch '{name}' already exists");
            }
            refs::update_branch(&repo, &name, &hash)?;
            println!("Created branch '{name}' at {}", &hash[..8]);
        }
    }
    Ok(())
}

fn cmd_tag(name: Option<String>, commit: Option<String>) -> Result<()> {
    let repo = open_repo()?;
    match name {
        None => {
            for tag in refs::list_tags(&repo)? {
                let hash = fs::read_to_string(repo.tags().join(&tag))?;
                println!("{tag}  {}", hash.trim()[..8].yellow());
            }
        }
        Some(name) => {
            let hash = match commit {
                Some(rev) => refs::resolve(&repo, &rev)?,
                None => {
                    refs::current_commit(&repo)?.context("cannot tag before the first commit")?
                }
            };
            refs::create_tag(&repo, &name, &hash)?;
            println!("Tagged {} as '{name}'", &hash[..8]);
        }
    }
    Ok(())
}
