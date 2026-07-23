use super::*;

// Env reads are process-global; serialize the tests that touch CRUCIBLE_MAX_CONCURRENCY.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn one_slot_serializes_a_second_acquire() {
    let dir = tempfile::tempdir().unwrap();
    let held = acquire_in(dir.path(), 1, Duration::from_millis(50)).unwrap();
    // With the only slot held, a second acquire cannot get in and times out.
    let err = acquire_in(dir.path(), 1, Duration::from_millis(200)).unwrap_err();
    assert!(err.contains("already active"), "{err}");
    // Releasing the first frees the machine-wide slot for the next waiter.
    drop(held);
    assert!(acquire_in(dir.path(), 1, Duration::from_millis(200)).is_ok());
}

#[test]
fn concurrency_of_n_admits_exactly_n() {
    let dir = tempfile::tempdir().unwrap();
    let a = acquire_in(dir.path(), 2, Duration::from_millis(50)).unwrap();
    let b = acquire_in(dir.path(), 2, Duration::from_millis(50)).unwrap();
    // Both slots are taken; the third must wait and fail.
    assert!(acquire_in(dir.path(), 2, Duration::from_millis(150)).is_err());
    drop(a);
    // One freed → one admitted.
    let _c = acquire_in(dir.path(), 2, Duration::from_millis(200)).unwrap();
    drop(b);
}

#[test]
fn a_dropped_slot_is_reusable() {
    let dir = tempfile::tempdir().unwrap();
    for _ in 0..5 {
        let slot = acquire_in(dir.path(), 1, Duration::from_millis(200)).unwrap();
        drop(slot);
    }
    // Material check: after the cycle the slot is free, and a held slot still serializes.
    let held = acquire_in(dir.path(), 1, Duration::from_millis(200)).unwrap();
    assert!(
        acquire_in(dir.path(), 1, Duration::from_millis(100)).is_err(),
        "held slot must still serialize"
    );
    drop(held);
    assert!(
        acquire_in(dir.path(), 1, Duration::from_millis(200)).is_ok(),
        "drop must free the slot for a later waiter"
    );
}

#[test]
fn max_concurrency_defaults_to_one_and_reads_the_env() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Point at an empty config dir so a real machine config on the dev box cannot
    // change what "no file" resolves to.
    let empty = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("CRUCIBLE_CONFIG_DIR", empty.path()) };
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    unsafe { std::env::remove_var("CRUCIBLE_MAX_CONCURRENCY") };
    assert_eq!(max_concurrency(), 1, "default is fully serialized");
    unsafe { std::env::set_var("CRUCIBLE_MAX_CONCURRENCY", "4") };
    assert_eq!(
        max_concurrency(),
        4.min(cores),
        "reads env, capped at cores"
    );
    // An absurd value cannot exceed the machine's parallelism — the gate never disables.
    unsafe { std::env::set_var("CRUCIBLE_MAX_CONCURRENCY", "4294967295") };
    assert_eq!(max_concurrency(), cores.max(1), "capped at cores");
    // A garbage or zero value falls back to the safe default, never 0 (which admits none).
    unsafe { std::env::set_var("CRUCIBLE_MAX_CONCURRENCY", "0") };
    assert_eq!(max_concurrency(), 1);
    unsafe { std::env::set_var("CRUCIBLE_MAX_CONCURRENCY", "nonsense") };
    assert_eq!(max_concurrency(), 1);
    unsafe { std::env::remove_var("CRUCIBLE_MAX_CONCURRENCY") };
    unsafe { std::env::remove_var("CRUCIBLE_CONFIG_DIR") };
}

#[test]
fn env_beats_file_beats_default_and_all_are_capped_at_cores() {
    assert_eq!(resolve_max(Some(2), Some(4), 8), (2, Source::Env));
    assert_eq!(resolve_max(None, Some(4), 8), (4, Source::ConfigFile));
    assert_eq!(resolve_max(None, None, 8), (1, Source::Default));
    // The cap applies whichever layer won — the source still names the loser honestly.
    assert_eq!(resolve_max(Some(16), None, 8), (8, Source::Env));
    assert_eq!(resolve_max(None, Some(16), 8), (8, Source::ConfigFile));
    // A zero core count (unreadable parallelism) never produces a zero-slot gate.
    assert_eq!(resolve_max(None, Some(4), 0), (1, Source::ConfigFile));
}

#[test]
fn file_max_reads_a_valid_value_and_ignores_a_broken_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    assert_eq!(file_max(&path), None, "absent file");
    std::fs::write(&path, "{\"maxConcurrency\": 3}").unwrap();
    assert_eq!(file_max(&path), Some(3));
    // A zero would admit nothing; a malformed file could be anything. Both fall back to
    // the default rather than granting concurrency a valid file did not.
    std::fs::write(&path, "{\"maxConcurrency\": 0}").unwrap();
    assert_eq!(file_max(&path), None);
    std::fs::write(&path, "not json").unwrap();
    assert_eq!(file_max(&path), None);
}

#[test]
fn set_max_roundtrips_through_the_file_read() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("CRUCIBLE_CONFIG_DIR", dir.path()) };
    let path = set_max(3).unwrap();
    assert_eq!(path, dir.path().join("config.json"));
    assert_eq!(file_max(&path), Some(3));
    // A second write replaces the value, not appends beside it.
    set_max(2).unwrap();
    assert_eq!(file_max(&path), Some(2));
    unsafe { std::env::remove_var("CRUCIBLE_CONFIG_DIR") };
}
