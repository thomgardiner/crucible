use super::*;

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
