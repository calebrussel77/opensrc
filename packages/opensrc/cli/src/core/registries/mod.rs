pub mod crates;
pub mod npm;
pub mod pypi;
pub mod repo;

use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Registry {
    Npm,
    #[serde(rename = "pypi")]
    PyPI,
    Crates,
}

impl std::fmt::Display for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Registry::Npm => write!(f, "npm"),
            Registry::PyPI => write!(f, "pypi"),
            Registry::Crates => write!(f, "crates"),
        }
    }
}

impl Registry {
    pub fn label(&self) -> &'static str {
        match self {
            Registry::Npm => "npm",
            Registry::PyPI => "PyPI",
            Registry::Crates => "crates.io",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedPackage {
    pub registry: Registry,
    pub name: String,
    pub version: String,
    pub repo_url: String,
    pub repo_directory: Option<String>,
    pub git_tag: String,
}

#[derive(Debug, Clone)]
pub struct PackageSpec {
    pub registry: Registry,
    pub name: String,
    pub version: Option<String>,
}

pub struct DetectedRegistry {
    pub registry: Registry,
    pub clean_spec: String,
}

const REGISTRY_PREFIXES: &[(&str, Registry)] = &[
    ("npm:", Registry::Npm),
    ("pypi:", Registry::PyPI),
    ("pip:", Registry::PyPI),
    ("python:", Registry::PyPI),
    ("crates:", Registry::Crates),
    ("cargo:", Registry::Crates),
    ("rust:", Registry::Crates),
];

pub fn detect_registry(spec: &str) -> DetectedRegistry {
    let trimmed = spec.trim();
    let lower = trimmed.to_lowercase();

    for &(prefix, registry) in REGISTRY_PREFIXES {
        if lower.starts_with(prefix) {
            return DetectedRegistry {
                registry,
                clean_spec: trimmed[prefix.len()..].to_string(),
            };
        }
    }

    DetectedRegistry {
        registry: Registry::Npm,
        clean_spec: trimmed.to_string(),
    }
}

pub fn parse_package_spec(spec: &str) -> PackageSpec {
    let detected = detect_registry(spec);

    let (name, version) = match detected.registry {
        Registry::Npm => npm::parse_npm_spec(&detected.clean_spec),
        Registry::PyPI => pypi::parse_pypi_spec(&detected.clean_spec),
        Registry::Crates => crates::parse_crates_spec(&detected.clean_spec),
    };

    PackageSpec {
        registry: detected.registry,
        name,
        version,
    }
}

pub fn resolve_package(spec: &PackageSpec) -> super::error::Result<ResolvedPackage> {
    match spec.registry {
        Registry::Npm => npm::resolve_npm_package(&spec.name, spec.version.as_deref()),
        Registry::PyPI => pypi::resolve_pypi_package(&spec.name, spec.version.as_deref()),
        Registry::Crates => crates::resolve_crate(&spec.name, spec.version.as_deref()),
    }
}

pub(crate) fn is_git_repo_url(url: &str) -> bool {
    url.contains("github.com") || url.contains("gitlab.com") || url.contains("bitbucket.org")
}

pub(crate) fn normalize_repo_url(url: &str) -> String {
    url.trim_end_matches('/')
        .trim_end_matches(".git")
        .split("/tree/")
        .next()
        .unwrap_or(url)
        .split("/blob/")
        .next()
        .unwrap_or(url)
        .to_string()
}

pub(crate) fn http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .user_agent("opensrc-cli (https://github.com/vercel-labs/opensrc)")
        .build()
        .expect("failed to build HTTP client")
}

pub(crate) fn github_token() -> Option<String> {
    github_token_from_env().or_else(github_token_from_gh)
}

fn first_non_empty_env(vars: &[&str]) -> Option<String> {
    vars.iter().find_map(|var| {
        std::env::var(var).ok().and_then(|value| {
            let token = value.trim();
            (!token.is_empty()).then(|| token.to_string())
        })
    })
}

fn github_token_from_env() -> Option<String> {
    first_non_empty_env(&["GITHUB_TOKEN", "GH_TOKEN"])
}

fn parse_gh_token_output(status_success: bool, stdout: &[u8]) -> Option<String> {
    if !status_success {
        return None;
    }

    let token = String::from_utf8_lossy(stdout).trim().to_string();
    (!token.is_empty()).then_some(token)
}

fn github_token_from_gh() -> Option<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    parse_gh_token_output(output.status.success(), &output.stdout)
}

pub(crate) fn gitlab_token() -> Option<String> {
    gitlab_token_from_env().or_else(gitlab_token_from_glab)
}

fn gitlab_token_from_env() -> Option<String> {
    first_non_empty_env(&["GITLAB_TOKEN", "GL_TOKEN"])
}

fn parse_labeled_token_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let label_end = trimmed.find(':')?;
    let label = trimmed[..label_end].trim().to_lowercase();

    if !label.ends_with("token") {
        return None;
    }

    let token = trimmed[label_end + 1..].trim();
    (!token.is_empty()).then(|| token.to_string())
}

fn parse_glab_auth_status_output(status_success: bool, output: &[u8]) -> Option<String> {
    if !status_success {
        return None;
    }

    let output = String::from_utf8_lossy(output);
    output.lines().find_map(parse_labeled_token_line)
}

fn gitlab_token_from_glab() -> Option<String> {
    let output = Command::new("glab")
        .args(["auth", "status", "--hostname", "gitlab.com", "--show-token"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;

    let mut combined = output.stdout;
    combined.extend_from_slice(&output.stderr);
    parse_glab_auth_status_output(output.status.success(), &combined)
}

pub(crate) fn bitbucket_token() -> Option<String> {
    std::env::var("BITBUCKET_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
}

/// Rewrites an HTTPS clone URL to embed auth credentials when a token is available.
pub fn authenticated_clone_url(url: &str) -> String {
    if let Some(token) = github_token() {
        if url.contains("github.com") {
            return url.replacen(
                "https://github.com",
                &format!("https://x-access-token:{token}@github.com"),
                1,
            );
        }
    }
    if let Some(token) = gitlab_token() {
        if url.contains("gitlab.com") {
            return url.replacen(
                "https://gitlab.com",
                &format!("https://oauth2:{token}@gitlab.com"),
                1,
            );
        }
    }
    if let Some(token) = bitbucket_token() {
        if url.contains("bitbucket.org") {
            return url.replacen(
                "https://bitbucket.org",
                &format!("https://x-token-auth:{token}@bitbucket.org"),
                1,
            );
        }
    }
    url.to_string()
}

pub fn detect_input_type(spec: &str) -> &'static str {
    let trimmed = spec.trim();
    let lower = trimmed.to_lowercase();

    for &(prefix, _) in REGISTRY_PREFIXES {
        if lower.starts_with(prefix) {
            return "package";
        }
    }

    if repo::is_repo_spec(trimmed) {
        return "repo";
    }

    "package"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_git_repo_url_github() {
        assert!(is_git_repo_url("https://github.com/owner/repo"));
    }

    #[test]
    fn test_is_git_repo_url_gitlab() {
        assert!(is_git_repo_url("https://gitlab.com/owner/repo"));
    }

    #[test]
    fn test_is_git_repo_url_bitbucket() {
        assert!(is_git_repo_url("https://bitbucket.org/owner/repo"));
    }

    #[test]
    fn test_is_git_repo_url_other() {
        assert!(!is_git_repo_url("https://example.com/owner/repo"));
    }

    #[test]
    fn test_normalize_repo_url_trailing_slash() {
        assert_eq!(
            normalize_repo_url("https://github.com/owner/repo/"),
            "https://github.com/owner/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_dot_git() {
        assert_eq!(
            normalize_repo_url("https://github.com/owner/repo.git"),
            "https://github.com/owner/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_tree_ref() {
        assert_eq!(
            normalize_repo_url("https://github.com/owner/repo/tree/main/src"),
            "https://github.com/owner/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_blob_ref() {
        assert_eq!(
            normalize_repo_url("https://github.com/owner/repo/blob/main/file.rs"),
            "https://github.com/owner/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_clean() {
        assert_eq!(
            normalize_repo_url("https://github.com/owner/repo"),
            "https://github.com/owner/repo"
        );
    }

    #[test]
    fn test_parse_gh_token_output_trims_token() {
        assert_eq!(
            parse_gh_token_output(true, b"gho_example\n"),
            Some("gho_example".to_string())
        );
    }

    #[test]
    fn test_parse_gh_token_output_ignores_empty_success() {
        assert_eq!(parse_gh_token_output(true, b"\n"), None);
    }

    #[test]
    fn test_parse_gh_token_output_ignores_failure() {
        assert_eq!(parse_gh_token_output(false, b"gho_example\n"), None);
    }

    #[test]
    fn test_parse_glab_auth_status_output_reads_token_line() {
        assert_eq!(
            parse_glab_auth_status_output(true, b"gitlab.com\n  Token: glpat-example\n"),
            Some("glpat-example".to_string())
        );
    }

    #[test]
    fn test_parse_glab_auth_status_output_ignores_failure() {
        assert_eq!(
            parse_glab_auth_status_output(false, b"  Token: glpat-example\n"),
            None
        );
    }

    #[test]
    fn test_parse_labeled_token_line_reads_oauth_token() {
        assert_eq!(
            parse_labeled_token_line("  OAuth token: gloas-example"),
            Some("gloas-example".to_string())
        );
    }
}
