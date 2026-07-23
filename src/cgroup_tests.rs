use super::*;

#[test]
fn wrap_builds_a_bounded_scope_invocation() {
    let inner = vec!["sh".to_string(), "-c".to_string(), "cargo test".to_string()];
    let four_gib: u64 = 4 * 1024 * 1024 * 1024;
    let argv = wrap("crucible-1-0.scope", Some(four_gib), &inner);

    // The payload is run inside a named, garbage-collected user scope.
    assert_eq!(argv[0], "systemd-run");
    assert!(argv.contains(&"--user".to_string()));
    assert!(argv.contains(&"--scope".to_string()));
    assert!(argv.contains(&"--collect".to_string()));
    assert!(argv.contains(&"--unit=crucible-1-0.scope".to_string()));
    // Kernel-enforced ceilings.
    assert!(argv.contains(&format!("--property=MemoryMax={four_gib}")));
    assert!(argv.contains(&"--property=MemorySwapMax=0".to_string()));
    assert!(argv.iter().any(|a| a.starts_with("--property=TasksMax=")));
    // The real command follows the `--` separator, unchanged.
    let sep = argv.iter().position(|a| a == "--").unwrap();
    assert_eq!(&argv[sep + 1..], inner.as_slice());
}

#[test]
fn wrap_without_a_memory_limit_omits_memory_max() {
    let inner = vec!["sh".to_string(), "-c".to_string(), "true".to_string()];
    let argv = wrap("u.scope", None, &inner);
    assert!(!argv.iter().any(|a| a.starts_with("--property=MemoryMax=")));
    // TasksMax and swap are still capped even without an explicit memory limit.
    assert!(argv.iter().any(|a| a.starts_with("--property=TasksMax=")));
    assert!(argv.contains(&"--property=MemorySwapMax=0".to_string()));
}

#[test]
fn unit_names_are_unique_per_spawn() {
    let a = unit_name();
    let b = unit_name();
    assert_ne!(a, b, "sequential spawns must not collide");
    assert!(a.starts_with("crucible-") && a.ends_with(".scope"), "{a}");
    assert!(b.ends_with(".scope"), "{b}");
}

#[cfg(not(target_os = "linux"))]
#[test]
fn containment_is_unavailable_off_linux() {
    // Off Linux there is no cgroup path, so the caller falls back to process-group
    // containment — the behaviour every non-Linux test in the suite relies on.
    assert!(!available());
}
