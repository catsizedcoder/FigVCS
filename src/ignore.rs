use std::fs;
use std::path::Path;

pub struct Ignore {
    patterns: Vec<String>,
}

impl Ignore {
    pub fn load(root: &Path) -> Self {
        let mut patterns = Vec::new();
        if let Ok(text) = fs::read_to_string(root.join(".fvcsignore")) {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                patterns.push(line.trim_start_matches('/').to_string());
            }
        }
        Ignore { patterns }
    }

    pub fn is_ignored(&self, rel: &str) -> bool {
        self.patterns.iter().any(|p| matches(p, rel))
    }
}

fn matches(pattern: &str, rel: &str) -> bool {
    if let Some(dir) = pattern.strip_suffix('/') {
        if dir.contains('/') {
            return rel == dir || rel.starts_with(&format!("{dir}/"));
        }
        return rel.split('/').any(|seg| seg == dir);
    }
    if pattern.contains('/') {
        wildcard(pattern, rel)
    } else {
        rel.split('/').any(|seg| wildcard(pattern, seg))
    }
}

fn wildcard(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut i, mut j) = (0usize, 0usize);
    let (mut star, mut star_j) = (usize::MAX, 0usize);
    while j < t.len() {
        if i < p.len() && (p[i] == '?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = i;
            star_j = j;
            i += 1;
        } else if star != usize::MAX {
            i = star + 1;
            star_j += 1;
            j = star_j;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_basics() {
        assert!(wildcard("*.lua", "init.lua"));
        assert!(!wildcard("*.lua", "texture.png"));
        assert!(wildcard("cache/*", "cache/a.bin"));
        assert!(wildcard("foo?", "foob"));
    }

    #[test]
    fn segment_matching() {
        let ig = Ignore {
            patterns: vec!["*.tmp".into(), "build".into()],
        };
        assert!(ig.is_ignored("src/x.tmp"));
        assert!(ig.is_ignored("deep/nested/build/file.rs"));
        assert!(!ig.is_ignored("src/main.rs"));
    }
}
