use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;

use crate::commit::Commit;
use crate::config::{Config, Lib};
use crate::diff;
use crate::global_config::GlobalConfig;
use crate::ignore::Ignore;
use crate::index::{Index, IndexEntry};
use crate::object;
use crate::refs::{self, Head};
use crate::remote;
use crate::repo::Repository;
use crate::status::{self, Status};

#[derive(Parser)]
#[command(
    name = "fvcs",
    version,
    about = "FigVCS is a version control system built specifically for Figura avatars"
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
    #[command(about = "Manage remotes (folders, drives or server mounts to sync with)")]
    Remote {
        #[command(subcommand)]
        action: Option<RemoteAction>,
    },
    #[command(about = "Upload commits to a remote")]
    Push {
        remote: Option<String>,
        #[arg(
            long,
            help = "Don't generate a README from avatar.json, now or on future pushes"
        )]
        no_readme: bool,
    },
    #[command(about = "Copy a repository into a new folder")]
    Clone {
        source: String,
        dir: Option<PathBuf>,
    },
    #[command(about = "Fetch from the remote and update libraries")]
    Pull { remote: Option<String> },
    #[command(about = "Fetch the central registry and update libraries from it")]
    Sync {
        #[arg(long, help = "Overwrite libraries even if you edited them locally")]
        force: bool,
    },
    #[command(about = "Build a registry.json from a folder of library repos (for registry hosts)")]
    Registry { dir: PathBuf },
    #[command(about = "Set or show the central registry URL")]
    RegistryUrl { url: Option<String> },
    #[command(about = "Log in (or register) on a FigVCS server")]
    Login {
        server: String,
        #[arg(short, long)]
        username: Option<String>,
        #[arg(short, long)]
        password: Option<String>,
        #[arg(long, help = "Create a new account instead of logging in")]
        register: bool,
    },
    #[command(about = "Get or set identity settings (user.name, user.email)")]
    Config {
        key: Option<String>,
        value: Option<String>,
    },
    #[command(about = "Grant (or revoke) push access to your repo for another user")]
    Share {
        username: String,
        remote: Option<String>,
        #[arg(long)]
        remove: bool,
    },
    #[command(about = "Link external libraries that update on `fvcs pull`")]
    Lib {
        #[command(subcommand)]
        action: Option<LibAction>,
    },
}

#[derive(Subcommand)]
enum RemoteAction {
    #[command(about = "Link a remote path or server URL")]
    Add {
        name: String,
        path: String,
        #[arg(long, help = "Bearer token for servers that require auth")]
        token: Option<String>,
    },
    #[command(about = "Unlink a remote")]
    Remove { name: String },
    #[command(about = "Make the repo on this remote public or private (owner only)")]
    Visibility { name: String, visibility: String },
    #[command(about = "Delete the repo on the server (owner only, cannot be undone)")]
    Delete {
        name: String,
        #[arg(long, help = "Skip the type-the-name confirmation prompt")]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum LibAction {
    #[command(about = "Link a library from another repository")]
    Add {
        name: String,
        source: String,
        #[arg(long, help = "Only take this subfolder of the source repo")]
        subdir: Option<String>,
    },
    #[command(about = "Unlink a library")]
    Remove { name: String },
    #[command(about = "Update libraries now (also runs on `fvcs pull`)")]
    Update,
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
        Command::Remote { action } => cmd_remote(action),
        Command::Push { remote, no_readme } => cmd_push(remote, no_readme),
        Command::Clone { source, dir } => cmd_clone(&source, dir),
        Command::Pull { remote } => cmd_pull(remote),
        Command::Sync { force } => cmd_sync(force),
        Command::Registry { dir } => {
            let registry = crate::registry::build(&dir)?;
            println!(
                "registry.json built | {} libraries, updated {}",
                registry.libs.len(),
                registry.updated
            );
            Ok(())
        }
        Command::RegistryUrl { url } => {
            let repo = open_repo()?;
            let mut config = Config::load(&repo)?;
            match url {
                Some(url) => {
                    config.registry = Some(url.clone());
                    config.save(&repo)?;
                    println!("Registry set to {url}");
                }
                None => match &config.registry {
                    Some(url) => println!("{url}"),
                    None => println!("no registry configured"),
                },
            }
            Ok(())
        }
        Command::Lib { action } => cmd_lib(action),
        Command::Login {
            server,
            username,
            password,
            register,
        } => cmd_login(&server, username, password, register),
        Command::Config { key, value } => cmd_config(key, value),
        Command::Share {
            username,
            remote,
            remove,
        } => cmd_share(&username, remote, remove),
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
    let author = GlobalConfig::load().ok().and_then(|g| g.author());
    let author = match author {
        Some(author) => author,
        None => {
            eprintln!("hint: set your identity with `fvcs config user.name \"...\"` and `fvcs config user.email \"...\"`");
            std::env::var("FVCS_AUTHOR")
                .or_else(|_| std::env::var("USERNAME"))
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_else(|_| "unknown".into())
        }
    };
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

fn apply_tree(repo: &Repository, commit: &Commit) -> Result<()> {
    let tracked_now = status::head_tree(repo)?;
    let index = Index::load(repo)?;
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
        let data = object::read(repo, blob)?;
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
    new_index.save(repo)
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
    apply_tree(&repo, &commit)?;

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

fn stored_token(url: &str) -> Option<String> {
    GlobalConfig::load()
        .ok()
        .and_then(|g| g.token_for(url).map(|t| t.to_string()))
}

fn pick_remote(
    repo: &Repository,
    name: Option<String>,
) -> Result<(String, Box<dyn remote::Store>)> {
    let config = Config::load(repo)?;
    let name = match name {
        Some(n) => n,
        None if config.remotes.len() == 1 => config.remotes.keys().next().unwrap().clone(),
        None => "origin".to_string(),
    };
    let entry = config
        .remotes
        .get(&name)
        .with_context(|| format!("no remote named '{name}' (add one with `fvcs remote add`)"))?;
    let token = entry
        .token()
        .map(|t| t.to_string())
        .or_else(|| stored_token(entry.url()));
    let store = remote::open_store(entry.url(), token.as_deref())?;
    Ok((name, store))
}

fn current_branch(repo: &Repository) -> Result<String> {
    match refs::head(repo)? {
        Head::Branch(name) => Ok(name),
        Head::Detached(_) => bail!("you are on a detached HEAD | switch to a branch first"),
    }
}

fn cmd_remote(action: Option<RemoteAction>) -> Result<()> {
    let repo = open_repo()?;
    let mut config = Config::load(&repo)?;
    match action {
        None => {
            if config.remotes.is_empty() {
                println!("no remotes yet | add one with `fvcs remote add <name> <path>`");
            }
            for (name, entry) in &config.remotes {
                let lock = if entry.token().is_some() {
                    " (token set)"
                } else {
                    ""
                };
                println!("{name}  {}{lock}", entry.url());
            }
        }
        Some(RemoteAction::Add { name, path, token }) => {
            let entry = match token {
                Some(token) => crate::config::Remote::Full {
                    url: path.clone(),
                    token: Some(token),
                },
                None => crate::config::Remote::Plain(path.clone()),
            };
            config.remotes.insert(name.clone(), entry);
            config.save(&repo)?;
            println!("Remote '{name}' -> {path}");
        }
        Some(RemoteAction::Remove { name }) => {
            if config.remotes.remove(&name).is_none() {
                bail!("no remote named '{name}'");
            }
            config.save(&repo)?;
            println!("Removed remote '{name}'");
        }
        Some(RemoteAction::Visibility { name, visibility }) => {
            if visibility != "public" && visibility != "private" {
                bail!("visibility is either 'public' or 'private'");
            }
            let (_, store) = pick_remote(&repo, Some(name.clone()))?;
            store.set_visibility(&visibility)?;
            println!("'{name}' is now {visibility}");
        }
        Some(RemoteAction::Delete { name, yes }) => {
            let (_, store) = pick_remote(&repo, Some(name.clone()))?;
            let repo_name = store
                .repo_name()
                .context("deleting repos only applies to server remotes")?;
            if !yes {
                eprintln!(
                    "This deletes '{repo_name}' on the server for everyone | it cannot be undone."
                );
                eprint!("Type the repo name to confirm: ");
                use std::io::Write;
                std::io::stderr().flush()?;
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                if line.trim() != repo_name {
                    bail!("aborted | name did not match");
                }
            }
            store.delete_repo()?;
            println!("Deleted '{repo_name}' on remote '{name}'");
            println!("The remote is still linked locally | `fvcs remote remove {name}` unlinks it");
        }
    }
    Ok(())
}

fn readme_for_head(repo: &Repository) -> Option<String> {
    let tree = status::head_tree(repo).ok()?;
    let blob = tree.get("avatar.json")?;
    let data = object::read(repo, blob).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&data).ok()?;
    let name = json.get("name")?.as_str()?;
    let mut out = format!("# {name}\n\n");
    if let Some(desc) = json.get("description").and_then(|d| d.as_str()) {
        if !desc.is_empty() {
            out.push_str(&format!("{desc}\n\n"));
        }
    }
    if let Some(authors) = json.get("authors").and_then(|a| a.as_array()) {
        let names: Vec<&str> = authors.iter().filter_map(|a| a.as_str()).collect();
        if !names.is_empty() {
            out.push_str(&format!("by {}\n\n", names.join(", ")));
        }
    }
    out.push_str("*README generated by [FigVCS](https://github.com/catsizedcoder/FigVCS) from avatar.json*\n");
    Some(out)
}

fn cmd_push(name: Option<String>, no_readme: bool) -> Result<()> {
    let repo = open_repo()?;
    let branch = current_branch(&repo)?;
    let tip = refs::current_commit(&repo)?.context("nothing to push yet")?;
    let mut config = Config::load(&repo)?;
    if no_readme && !config.no_readme {
        config.no_readme = true;
        config.save(&repo)?;
        println!("Noted | no README will be generated on pushes from now on");
    }
    let (name, store) = pick_remote(&repo, name)?;

    if let Some(remote_tip) = store.branch_commit(&branch)? {
        let we_have_it = repo.commits().join(format!("{remote_tip}.json")).exists();
        if !we_have_it || !remote::is_ancestor(&repo, &remote_tip, &tip)? {
            bail!("remote has commits you don't | run `fvcs pull` first");
        }
    }

    let all = remote::reachable(&repo, &tip)?;
    let copied = remote::copy_history(&repo, store.as_ref(), &all)?;
    store.update_branch(&branch, &tip)?;
    println!(
        "Pushed '{branch}' to '{name}' ({} commits checked, {copied} new)",
        all.len()
    );
    if !config.no_readme {
        if let Some(readme) = readme_for_head(&repo) {
            store.write_readme(&readme)?;
            println!("README.md generated from avatar.json on the remote");
        }
    }
    Ok(())
}

fn cmd_clone(source: &str, dir: Option<PathBuf>) -> Result<()> {
    let store = remote::open_store_readonly(source, stored_token(source).as_deref())
        .with_context(|| format!("'{source}' is not a FigVCS repository"))?;
    let dir = match dir {
        Some(d) => d,
        None => {
            let name = Path::new(source)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "clone".to_string());
            PathBuf::from(name)
        }
    };
    let repo = Repository::init(&dir)?;

    let branch = store
        .head_branch()?
        .unwrap_or_else(|| crate::repo::DEFAULT_BRANCH.to_string());
    let tip = store
        .branch_commit(&branch)?
        .context("source repository has no commits yet")?;

    let all = remote::reachable(store.as_ref(), &tip)?;
    remote::copy_history(store.as_ref(), &repo, &all)?;
    refs::set_head_branch(&repo, &branch)?;
    refs::update_branch(&repo, &branch, &tip)?;
    let commit = Commit::load(&repo, &tip)?;
    apply_tree(&repo, &commit)?;

    let mut config = Config::default();
    let origin_url = if source.starts_with("http://") || source.starts_with("https://") {
        source.to_string()
    } else {
        std::path::Path::new(source)
            .canonicalize()?
            .to_string_lossy()
            .into_owned()
    };
    config.remotes.insert(
        "origin".to_string(),
        crate::config::Remote::Plain(origin_url),
    );
    config.save(&repo)?;
    println!("Cloned '{source}' into {}", repo.root.display());
    Ok(())
}

fn cmd_pull(name: Option<String>) -> Result<()> {
    let repo = open_repo()?;
    let branch = current_branch(&repo)?;
    let config = Config::load(&repo)?;

    if config.remotes.is_empty() {
        println!("no remote configured | updating libraries only");
    } else {
        let (name, store) = pick_remote(&repo, name)?;

        if let Some(remote_tip) = store.branch_commit(&branch)? {
            let local = refs::current_commit(&repo)?;
            match local {
                Some(local_tip) if local_tip == remote_tip => {
                    println!("Already up to date.");
                }
                Some(local_tip) => {
                    let all = remote::reachable(store.as_ref(), &remote_tip)?;
                    remote::copy_history(store.as_ref(), &repo, &all)?;
                    if !remote::is_ancestor(&repo, &local_tip, &remote_tip)? {
                        bail!("local and remote have diverged | merging is not supported yet");
                    }
                    let s = status::compute(&repo)?;
                    ensure_clean(&s)?;
                    refs::update_branch(&repo, &branch, &remote_tip)?;
                    let commit = Commit::load(&repo, &remote_tip)?;
                    apply_tree(&repo, &commit)?;
                    println!("Updated to {}", &remote_tip[..8]);
                }
                None => {
                    let all = remote::reachable(store.as_ref(), &remote_tip)?;
                    remote::copy_history(store.as_ref(), &repo, &all)?;
                    refs::update_branch(&repo, &branch, &remote_tip)?;
                    let commit = Commit::load(&repo, &remote_tip)?;
                    apply_tree(&repo, &commit)?;
                    println!("Updated to {}", &remote_tip[..8]);
                }
            }
        } else {
            println!("Remote '{name}' has no branch '{branch}' yet.");
        }
    }

    update_libs(&repo)?;
    Ok(())
}

fn cmd_lib(action: Option<LibAction>) -> Result<()> {
    let repo = open_repo()?;
    let mut libs = crate::config::load_libs(&repo)?;
    match action {
        None => {
            if libs.is_empty() {
                println!("no libraries linked | add one with `fvcs lib add <name> <source>`");
            }
            for lib in &libs {
                match &lib.subdir {
                    Some(sub) => println!("{}  {} (subfolder {sub})", lib.name, lib.source),
                    None => println!("{}  {}", lib.name, lib.source),
                }
            }
        }
        Some(LibAction::Add {
            name,
            source,
            subdir,
        }) => {
            if libs.iter().any(|l| l.name == name) {
                bail!("a library named '{name}' is already linked");
            }
            let abs = std::env::current_dir()?.join(&source);
            let path = if abs.exists() {
                abs
            } else {
                PathBuf::from(&source)
            };
            Repository::discover(&path)
                .with_context(|| format!("'{source}' is not inside a FigVCS repository"))?;
            libs.push(Lib {
                name: name.clone(),
                source: path.to_string_lossy().into_owned(),
                subdir,
                token: None,
            });
            crate::config::save_libs(&repo, &libs)?;
            println!("Linked library '{name}' | run `fvcs lib update` to fetch it");
        }
        Some(LibAction::Remove { name }) => {
            let before = libs.len();
            libs.retain(|l| l.name != name);
            if libs.len() == before {
                bail!("no library named '{name}'");
            }
            crate::config::save_libs(&repo, &libs)?;
            println!("Unlinked library '{name}' (its files stay in place)");
        }
        Some(LibAction::Update) => {
            update_libs(&repo)?;
        }
    }
    Ok(())
}

fn update_libs(repo: &Repository) -> Result<()> {
    let libs = crate::config::load_libs(repo)?;
    if libs.is_empty() {
        return Ok(());
    }
    let mut index = Index::load(repo)?;
    for lib in &libs {
        let token = lib.token.clone().or_else(|| stored_token(&lib.source));
        let store =
            remote::open_store_readonly(&lib.source, token.as_deref()).with_context(|| {
                format!("library source '{}' is not a FigVCS repository", lib.source)
            })?;
        let branch = store
            .head_branch()?
            .unwrap_or_else(|| crate::repo::DEFAULT_BRANCH.to_string());
        let tip = store
            .branch_commit(&branch)?
            .with_context(|| format!("library '{}' source has no commits yet", lib.name))?;
        let count = fetch_lib_files(repo, &mut index, lib, store.as_ref(), &tip)?;
        println!(
            "Library '{}' updated ({} files from {})",
            lib.name,
            count,
            &tip[..8]
        );
    }
    index.save(repo)?;
    Ok(())
}

fn fetch_lib_files(
    repo: &Repository,
    index: &mut Index,
    lib: &Lib,
    store: &dyn remote::Store,
    tip: &str,
) -> Result<usize> {
    let commit = store.read_commit(tip)?;
    let mut count = 0;
    for (path, blob) in &commit.tree {
        let inner = match &lib.subdir {
            Some(sub) => match path.strip_prefix(&format!("{sub}/")) {
                Some(rest) => rest.to_string(),
                None => continue,
            },
            None => path.clone(),
        };
        let target = format!("{}/{inner}", lib.name);
        let data = store.read_object(blob)?;
        let hash = object::write(repo, &data)?;
        let abs = repo.abs(&target);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&abs, &data).with_context(|| format!("writing {}", abs.display()))?;
        index.entries.insert(
            target,
            IndexEntry {
                hash,
                size: data.len() as u64,
                mtime: mtime_of(&abs),
            },
        );
        count += 1;
    }
    Ok(count)
}

fn local_lib_hash(repo: &Repository, lib: &Lib) -> Result<Option<String>> {
    let workdir = status::collect_workdir(repo)?;
    let prefix = format!("{}/", lib.name);
    let tree: BTreeMap<String, String> = workdir
        .into_iter()
        .filter(|(path, _)| path.starts_with(&prefix))
        .map(|(path, f)| {
            let inner = path.strip_prefix(&prefix).unwrap().to_string();
            let key = match &lib.subdir {
                Some(sub) => format!("{sub}/{inner}"),
                None => inner,
            };
            (key, f.hash)
        })
        .collect();
    if tree.is_empty() {
        return Ok(None);
    }
    Ok(Some(crate::registry::lib_hash(&tree)))
}

fn cmd_sync(force: bool) -> Result<()> {
    let repo = open_repo()?;
    let config = Config::load(&repo)?;
    let registry_url = config
        .registry
        .clone()
        .context("no registry configured | set one with `fvcs registry-url <url>`")?;
    let registry = crate::registry::Registry::fetch(&registry_url)?;
    registry.save(&crate::registry::Registry::cache_path(&repo))?;
    println!(
        "Registry fetched ({} libraries, updated {})",
        registry.libs.len(),
        registry.updated
    );

    let mut libs = crate::config::load_libs(&repo)?;
    let mut index = Index::load(&repo)?;
    let mut changed = false;

    for entry in &registry.libs {
        if entry.source.is_empty() {
            println!(
                "Skipping '{}' | the registry doesn't say where to get it",
                entry.name
            );
            continue;
        }
        let linked = libs.iter().position(|l| l.name == entry.name);
        if linked.is_none() {
            libs.push(Lib {
                name: entry.name.clone(),
                source: entry.source.clone(),
                subdir: entry.subdir.clone(),
                token: None,
            });
            changed = true;
            println!("Linked new library '{}'", entry.name);
        }
        let lib = libs.iter().find(|l| l.name == entry.name).unwrap().clone();

        if let Some(local) = local_lib_hash(&repo, &lib)? {
            if entry.hashes.first() == Some(&local) {
                println!("'{}' is already current", entry.name);
                continue;
            }
            if !entry.hashes.contains(&local) && !force {
                println!(
                    "Keeping your modified '{}' | it differs from every registry version (use `fvcs sync --force` to overwrite)",
                    entry.name
                );
                continue;
            }
            if !entry.hashes.contains(&local) && force {
                println!("Overwriting your modified '{}' (--force)", entry.name);
            }
        }

        let token = lib.token.clone().or_else(|| stored_token(&lib.source));
        let store = remote::open_store_readonly(&lib.source, token.as_deref())
            .with_context(|| format!("can't reach library source '{}'", lib.source))?;
        let branch = store
            .head_branch()?
            .unwrap_or_else(|| crate::repo::DEFAULT_BRANCH.to_string());
        let Some(tip) = store.branch_commit(&branch)? else {
            println!("Skipping '{}' | its source has no commits yet", entry.name);
            continue;
        };
        let count = fetch_lib_files(&repo, &mut index, &lib, store.as_ref(), &tip)?;
        println!(
            "'{}' synced ({} files from {})",
            entry.name,
            count,
            &tip[..8]
        );
    }

    if changed {
        crate::config::save_libs(&repo, &libs)?;
    }
    index.save(&repo)?;
    Ok(())
}

fn cmd_login(
    server: &str,
    username: Option<String>,
    password: Option<String>,
    register: bool,
) -> Result<()> {
    let username = match username {
        Some(u) => u,
        None => {
            print!("username: ");
            use std::io::Write;
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            input.trim().to_string()
        }
    };
    let password = match password {
        Some(p) => p,
        None => rpassword::prompt_password("password: ")?,
    };
    let token = if register {
        crate::http_store::HttpStore::register(server, &username, &password)?
    } else {
        crate::http_store::HttpStore::login(server, &username, &password)?
    };
    let mut global = GlobalConfig::load()?;
    global
        .servers
        .insert(crate::global_config::server_key(server), token);
    global.save()?;
    let action = if register {
        "Registered and logged in"
    } else {
        "Logged in"
    };
    println!("{action} as '{username}' on {server}");
    Ok(())
}

fn cmd_config(key: Option<String>, value: Option<String>) -> Result<()> {
    let mut global = GlobalConfig::load()?;
    match (key.as_deref(), value) {
        (None, _) => {
            match &global.user.name {
                Some(name) => println!("user.name={name}"),
                None => println!("user.name is not set"),
            }
            match &global.user.email {
                Some(email) => println!("user.email={email}"),
                None => println!("user.email is not set"),
            }
        }
        (Some("user.name"), Some(value)) => {
            global.user.name = Some(value);
            global.save()?;
        }
        (Some("user.email"), Some(value)) => {
            global.user.email = Some(value);
            global.save()?;
        }
        (Some(key @ ("user.name" | "user.email")), None) => {
            let current = if key == "user.name" {
                &global.user.name
            } else {
                &global.user.email
            };
            match current {
                Some(value) => println!("{value}"),
                None => println!("{key} is not set"),
            }
        }
        (Some(other), _) => bail!("unknown setting '{other}' | try user.name or user.email"),
    }
    Ok(())
}

fn cmd_share(username: &str, remote: Option<String>, remove: bool) -> Result<()> {
    let repo = open_repo()?;
    let (_, store) = pick_remote(&repo, remote)?;
    store.share(username, remove)?;
    if remove {
        println!("'{username}' can no longer push to your repo");
    } else {
        println!("'{username}' can now push to your repo");
    }
    Ok(())
}
