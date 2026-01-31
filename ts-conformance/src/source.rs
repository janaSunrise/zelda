//! Test source discovery.

use std::path::{Path, PathBuf};

pub struct TestSource {
    pub root: PathBuf,
}

impl TestSource {
    pub fn typescript() -> Self {
        Self {
            root: PathBuf::from("_submodules/typescript/tests/cases"),
        }
    }

    pub fn custom(path: PathBuf) -> Self {
        Self { root: path }
    }

    pub fn discover(&self, filter: Option<&str>) -> Vec<PathBuf> {
        if !self.root.exists() {
            return Vec::new();
        }

        let mut files = Vec::new();
        Self::walk(&self.root, &mut files);
        files.sort();

        if let Some(pattern) = filter {
            files.retain(|p| {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                glob_match(pattern, name)
            });
        }

        files
    }

    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::walk(&path, files);
            } else if let Some(ext) = path.extension() {
                if ext == "ts" || ext == "tsx" {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !name.ends_with(".d.ts") {
                        files.push(path);
                    }
                }
            }
        }
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();

    let mut p = 0;
    let mut t = 0;
    let mut star_p = usize::MAX;
    let mut star_t = usize::MAX;

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star_p = p;
            star_t = t;
            p += 1;
        } else if star_p != usize::MAX {
            p = star_p + 1;
            star_t += 1;
            t = star_t;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }

    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_match() {
        assert!(glob_match("*.ts", "foo.ts"));
        assert!(glob_match("union*", "unionTypes.ts"));
        assert!(!glob_match("foo*", "bar.ts"));
    }
}
