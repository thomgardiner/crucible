use super::*;
use std::fs;
use std::process::Command;

// An explicit recipe `memoryMb` resolves to exactly that many bytes — never dropped to the
// uncapped `None` path. Pins the branch a mutation survivor flipped to `Ok(None)`, which
// would silently run every heavy tree without a ceiling.
#[test]
fn resolve_memory_limit_honors_an_explicit_recipe_limit() {
    let resolved = resolve_memory_limit(Some(512)).expect("valid limit");
    assert_eq!(resolved, Some(512 * 1024 * 1024));
}

// A zero `memoryMb` is rejected (it would kill every command instantly), not silently
// treated as "no limit".
#[test]
fn resolve_memory_limit_rejects_a_zero_limit() {
    assert!(resolve_memory_limit(Some(0)).is_err());
}

// With no explicit limit, the machine-aware default applies: on any host that can read its
// RAM the ceiling is a real positive value, not None. (If RAM is unreadable it is None and
// the caller warns — both are acceptable, so this only asserts the positive case holds.)
#[test]
fn resolve_memory_limit_defaults_to_a_positive_ceiling_when_ram_is_readable() {
    if let Some(bytes) = resolve_memory_limit(None).expect("default path") {
        assert!(bytes > 0);
    }
}

#[test]
fn changed_files_scopes_to_adoption_root_inside_a_monorepo() {
    // A nested --repo must not see sibling dirt: otherwise high-risk scoping goes
    // advisory whenever anything else in the monorepo is dirty.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .unwrap()
                .success(),
            "{args:?}"
        );
    };
    git(&["init", "-q"]);
    let _ = Command::new("git")
        .args(["config", "user.email", "t@t"])
        .current_dir(root)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.name", "t"])
        .current_dir(root)
        .status();
    fs::create_dir_all(root.join("pkg/app")).unwrap();
    fs::create_dir_all(root.join("other")).unwrap();
    fs::write(root.join("pkg/app/core.ts"), "export const a = 1;\n").unwrap();
    fs::write(root.join("other/noise.rs"), "fn n() {}\n").unwrap();
    git(&["add", "-A"]);
    git(&["-c", "commit.gpgsign=false", "commit", "-qm", "seed"]);

    // Dirt only outside the adoption root (untracked, candidate=HEAD).
    fs::write(root.join("other/noise.rs"), "fn n() { /* dirty */ }\n").unwrap();
    let scoped = changed_files(&root.join("pkg"), "HEAD", "HEAD").unwrap();
    assert!(
        scoped.is_empty(),
        "sibling monorepo dirt must not enter pkg scope: {scoped:?}"
    );

    // Dirt under the adoption root is visible, re-rooted (no pkg/ prefix).
    fs::write(root.join("pkg/app/core.ts"), "export const a = 2;\n").unwrap();
    let scoped = changed_files(&root.join("pkg"), "HEAD", "HEAD").unwrap();
    assert!(
        scoped.contains("app/core.ts"),
        "adoption-root change must be visible as relative path: {scoped:?}"
    );
    assert!(
        !scoped.iter().any(|p| p.contains("noise")),
        "still must not include sibling dirt: {scoped:?}"
    );
}
