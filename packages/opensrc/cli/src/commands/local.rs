use std::fs;
use std::path::{Path, PathBuf};

use crate::core::error::{Error, Result};
use crate::core::fetcher::ensure_cached;

const LOCAL_DIR: &str = "opensrc";

fn copy_dir_all(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        let target_path = target.join(entry.file_name());

        if entry_type.is_dir() {
            copy_dir_all(&entry.path(), &target_path)?;
        } else if entry_type.is_file() {
            fs::copy(entry.path(), target_path)?;
        }
    }

    Ok(())
}

fn sanitize_path_segment(segment: &str) -> String {
    let sanitized: String = segment
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            ch if ch.is_control() => '-',
            ch => ch,
        })
        .collect();

    let sanitized = sanitized.trim_matches([' ', '.']).trim_start_matches('@');

    if sanitized.is_empty() {
        "source".to_string()
    } else {
        sanitized.to_string()
    }
}

fn local_dir_name(name: &str) -> String {
    let leaf = name.rsplit('/').next().unwrap_or(name);
    sanitize_path_segment(leaf)
}

fn local_target(name: &str, cwd: &str) -> PathBuf {
    PathBuf::from(cwd)
        .join(LOCAL_DIR)
        .join(local_dir_name(name))
}

pub fn run(specs: &[String], cwd: Option<&str>) -> Result<()> {
    let cwd = cwd.unwrap_or(".");
    let mut had_errors = false;

    for spec in specs {
        match ensure_cached(spec, cwd, true) {
            Ok(outcome) => {
                let target = local_target(&outcome.name, cwd);

                if target.exists() {
                    if target.is_dir() {
                        println!("  ✓ {} already exists ({})", outcome.name, target.display());
                        continue;
                    }

                    had_errors = true;
                    eprintln!(
                        "  ✗ {spec}: target exists and is not a directory ({})",
                        target.display()
                    );
                    continue;
                }

                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }

                if let Err(e) = copy_dir_all(&outcome.path, &target) {
                    had_errors = true;
                    let _ = fs::remove_dir_all(&target);
                    eprintln!("  ✗ {spec}: failed to copy source: {e}");
                    continue;
                }

                println!(
                    "  ✓ Saved {}@{} to {}",
                    outcome.name,
                    outcome.version,
                    target.display()
                );
            }
            Err(e) => {
                had_errors = true;
                eprintln!("  ✗ {spec}: {e}");
            }
        }
    }

    if had_errors {
        return Err(Error::Other(
            "Some sources could not be saved locally".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_dir_name_uses_repo_leaf() {
        assert_eq!(
            local_dir_name("github.com/calebrussel77/nfluenzo"),
            "nfluenzo"
        );
    }

    #[test]
    fn test_local_dir_name_sanitizes_scoped_package() {
        assert_eq!(local_dir_name("@vercel/ai"), "ai");
    }

    #[test]
    fn test_sanitize_path_segment_replaces_windows_separators() {
        assert_eq!(sanitize_path_segment("bad:name*"), "bad-name-");
    }
}
