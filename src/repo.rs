use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

pub const DIR: &str = ".fvcs";
pub const DEFAULT_BRANCH: &str = "main";

pub struct Repository {
    pub root: PathBuf,
}

impl Repository {
    pub fn fvcs(&self) -> PathBuf {
        self.root.join(DIR)
    }
    pub fn objects(&self) -> PathBuf {
        self.fvcs().join("objects")
    }
    pub fn commits(&self) -> PathBuf {
        self.fvcs().join("commits")
    }
    pub fn heads(&self) -> PathBuf {
        self.fvcs().join("refs").join("heads")
    }
    pub fn tags(&self) -> PathBuf {
        self.fvcs().join("refs").join("tags")
    }
    pub fn head_file(&self) -> PathBuf {
        self.fvcs().join("HEAD")
    }
    pub fn index_file(&self) -> PathBuf {
        self.fvcs().join("index")
    }

    pub fn init(path: &Path) -> Result<Self> {
        fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))?;
        let root = path
            .canonicalize()
            .with_context(|| format!("canonicalizing {}", path.display()))?;
        let repo = Repository { root };
        if repo.fvcs().exists() {
            bail!("{} is already a FigVCS repository", repo.root.display());
        }
        fs::create_dir_all(repo.objects())?;
        fs::create_dir_all(repo.commits())?;
        fs::create_dir_all(repo.heads())?;
        fs::create_dir_all(repo.tags())?;
        fs::write(repo.head_file(), format!("refs/heads/{DEFAULT_BRANCH}"))?;
        Ok(repo)
    }

    pub fn discover(start: &Path) -> Result<Self> {
        let mut cur = start
            .canonicalize()
            .with_context(|| format!("canonicalizing {}", start.display()))?;
        loop {
            if cur.join(DIR).is_dir() {
                return Ok(Repository { root: cur });
            }
            if !cur.pop() {
                bail!("not a FigVCS repository (or any parent directory)");
            }
        }
    }

    pub fn rel(&self, abs: &Path) -> Result<String> {
        let rel = abs
            .strip_prefix(&self.root)
            .with_context(|| format!("{} is outside the repository", abs.display()))?;
        Ok(rel.to_string_lossy().replace('\\', "/"))
    }

    pub fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
}

pub fn is_internal(rel: &str) -> bool {
    rel == DIR || rel.starts_with(&format!("{DIR}/"))
}
