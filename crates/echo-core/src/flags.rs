//! Change flags: after any HEAD move, match the changed file list against
//! dependency / migration / env-template patterns.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum FlagKind {
    Deps,
    Migrations,
    Env,
}

impl FlagKind {
    pub const ALL: [FlagKind; 3] = [FlagKind::Deps, FlagKind::Migrations, FlagKind::Env];

    pub fn label(self) -> &'static str {
        match self {
            FlagKind::Deps => "Dependencies changed",
            FlagKind::Migrations => "Migrations changed",
            FlagKind::Env => "Env template changed",
        }
    }
}

pub const DEFAULT_DEPS: &[&str] = &[
    "**/package.json",
    "**/package-lock.json",
    "**/pnpm-lock.yaml",
    "**/yarn.lock",
    "**/bun.lock*",
    "**/requirements*.txt",
    "**/pyproject.toml",
    "**/poetry.lock",
    "**/uv.lock",
    "**/Gemfile.lock",
    "**/go.sum",
    "**/Cargo.lock",
];

pub const DEFAULT_MIGRATIONS: &[&str] =
    &["**/migrations/**", "**/prisma/schema.prisma", "**/drizzle/**", "**/supabase/migrations/**"];

pub const DEFAULT_ENV: &[&str] = &["**/.env.example", "**/.env.sample"];

pub fn default_patterns(kind: FlagKind) -> Vec<String> {
    let src = match kind {
        FlagKind::Deps => DEFAULT_DEPS,
        FlagKind::Migrations => DEFAULT_MIGRATIONS,
        FlagKind::Env => DEFAULT_ENV,
    };
    src.iter().map(|s| s.to_string()).collect()
}

/// Compiled matcher for the three flag kinds.
pub struct FlagMatcher {
    sets: Vec<(FlagKind, GlobSet)>,
}

impl FlagMatcher {
    /// Build from per-kind pattern lists; invalid patterns are skipped (and logged).
    pub fn new(patterns: &[(FlagKind, Vec<String>)]) -> Self {
        let sets = patterns
            .iter()
            .map(|(kind, pats)| {
                let mut b = GlobSetBuilder::new();
                for p in pats {
                    match GlobBuilder::new(p).literal_separator(true).build() {
                        Ok(g) => {
                            b.add(g);
                        }
                        Err(e) => log::warn!("invalid flag pattern {p:?}: {e}"),
                    }
                }
                (*kind, b.build().unwrap_or_else(|_| GlobSet::empty()))
            })
            .collect();
        Self { sets }
    }

    pub fn defaults() -> Self {
        Self::new(&FlagKind::ALL.map(|k| (k, default_patterns(k))))
    }

    /// Return `(kind, matching files)` for each kind with at least one match.
    pub fn detect(&self, files: &[String]) -> Vec<(FlagKind, Vec<String>)> {
        self.sets
            .iter()
            .filter_map(|(kind, set)| {
                let hits: Vec<String> = files.iter().filter(|f| set.is_match(f.as_str())).cloned().collect();
                (!hits.is_empty()).then_some((*kind, hits))
            })
            .collect()
    }
}

/// Guess the install command from the lockfiles at the repo root.
pub fn detect_install_command(repo: &Path) -> Option<String> {
    let has = |f: &str| repo.join(f).is_file();
    let cmd = if has("pnpm-lock.yaml") {
        "pnpm install"
    } else if has("bun.lockb") || has("bun.lock") {
        "bun install"
    } else if has("yarn.lock") {
        "yarn install"
    } else if has("package-lock.json") {
        "npm install"
    } else if has("uv.lock") {
        "uv sync"
    } else if has("poetry.lock") {
        "poetry install"
    } else if has("Gemfile.lock") {
        "bundle install"
    } else if has("go.sum") {
        "go mod download"
    } else if has("package.json") {
        "npm install"
    } else if has("requirements.txt") {
        "pip install -r requirements.txt"
    } else {
        return None;
    };
    Some(cmd.to_string())
}

/// A dev server command from package.json scripts (`dev`, else `start`),
/// run with the repo's package manager.
pub fn detect_dev_command(repo: &Path) -> Option<String> {
    let pkg = std::fs::read_to_string(repo.join("package.json")).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&pkg).ok()?;
    let scripts = pkg.get("scripts")?.as_object()?;
    let script = ["dev", "start"].into_iter().find(|s| scripts.contains_key(*s))?;
    let has = |f: &str| repo.join(f).is_file();
    let runner = if has("pnpm-lock.yaml") {
        "pnpm"
    } else if has("bun.lockb") || has("bun.lock") {
        "bun run"
    } else if has("yarn.lock") {
        "yarn"
    } else {
        "npm run"
    };
    Some(format!("{runner} {script}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_command_from_package_json() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_dev_command(dir.path()), None);
        std::fs::write(dir.path().join("package.json"), r#"{"scripts":{"start":"node ."}}"#).unwrap();
        assert_eq!(detect_dev_command(dir.path()).as_deref(), Some("npm run start"));
        std::fs::write(dir.path().join("package.json"), r#"{"scripts":{"start":"x","dev":"vite"}}"#).unwrap();
        std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_dev_command(dir.path()).as_deref(), Some("pnpm dev"));
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        assert_eq!(detect_dev_command(dir.path()), None);
    }

    fn files(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn detects_defaults() {
        let m = FlagMatcher::defaults();
        let got = m.detect(&files(&[
            "package.json",
            "apps/web/package.json",
            "pnpm-lock.yaml",
            "src/index.ts",
            "db/migrations/0001_init.sql",
            "prisma/schema.prisma",
            ".env.example",
            "bun.lockb",
            "requirements-dev.txt",
        ]));
        let deps = &got.iter().find(|(k, _)| *k == FlagKind::Deps).unwrap().1;
        assert_eq!(
            deps,
            &files(&["package.json", "apps/web/package.json", "pnpm-lock.yaml", "bun.lockb", "requirements-dev.txt"])
        );
        let mig = &got.iter().find(|(k, _)| *k == FlagKind::Migrations).unwrap().1;
        assert_eq!(mig, &files(&["db/migrations/0001_init.sql", "prisma/schema.prisma"]));
        assert!(got.iter().any(|(k, _)| *k == FlagKind::Env));
    }

    #[test]
    fn no_false_positives() {
        let m = FlagMatcher::defaults();
        assert!(m.detect(&files(&["src/package.json.ts", "docs/migrations.md", "README.md", ".env"])).is_empty());
    }

    #[test]
    fn custom_patterns_and_invalid_ones() {
        let m = FlagMatcher::new(&[(FlagKind::Deps, vec!["deps.lock".into(), "[".into()])]);
        assert_eq!(m.detect(&files(&["deps.lock"])).len(), 1);
        assert!(m.detect(&files(&["sub/deps.lock"])).is_empty());
    }

    #[test]
    fn install_command_detection() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(detect_install_command(d.path()), None);
        std::fs::write(d.path().join("package.json"), "{}").unwrap();
        assert_eq!(detect_install_command(d.path()).as_deref(), Some("npm install"));
        std::fs::write(d.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_install_command(d.path()).as_deref(), Some("pnpm install"));
    }
}
