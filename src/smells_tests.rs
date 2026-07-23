use super::*;

fn any(v: &[String], needle: &str) -> bool {
    v.iter().any(|s| s.contains(needle))
}

#[test]
fn declared_assertion_helper_suppresses_the_flag() {
    // A test that asserts only through a named helper is flagged by default (we cannot
    // see into the helper) but not once the repo declares the helper.
    let src = "#[test]\nfn t() { recovers_after(200, body); }\n";
    assert!(
        any(&inspect_rust("a.rs", src), "has no assertion"),
        "flagged without the helper declared"
    );
    let m = helper_matcher(&["recovers_after".to_string()]);
    assert!(
        inspect_rust_with("a.rs", src, m.as_ref()).is_empty(),
        "cleared once declared"
    );
}

#[test]
fn real_world_assertion_styles_are_not_flagged() {
    // Patterns found on a real large codebase that used to false-positive.
    assert!(
        inspect_rust(
            "a.rs",
            "proptest! {\n #[test]\n fn t(x in 0u32..9) { prop_assert!(x < 9); }\n}\n"
        )
        .is_empty()
    );
    assert!(
        inspect_rust(
            "a.rs",
            "#[test]\nfn t() { assert_exact::<Task>(json!({})); }\n"
        )
        .is_empty()
    );
    assert!(inspect_ts("a.test.ts", "test('x', () => { assert.equal(f(), 1); });\n").is_empty());
    assert!(
        inspect_ts(
            "a.test.ts",
            "it('x', () => { if (!ok()) throw new Error('no'); });\n"
        )
        .is_empty()
    );
    // A custom test-runner helper's own definition is not a test call.
    assert!(
        inspect_ts(
            "a.test.ts",
            "function test(name, fn) { try { fn(); } catch (e) { log(e); } }\n"
        )
        .is_empty()
    );
}

// --- Rust: clean cases must not fire ---------------------------------------

#[test]
fn rust_test_with_real_assertion_is_clean() {
    assert!(
        inspect_rust(
            "a.rs",
            "#[test]\nfn adds() {\n  assert_eq!(add(2, 2), 4);\n}\n"
        )
        .is_empty()
    );
}

#[test]
fn rust_question_mark_returning_test_asserts() {
    let src =
        "#[tokio::test]\nasync fn loads() -> Result<()> {\n  let v = load().await?;\n  Ok(())\n}\n";
    assert!(inspect_rust("a.rs", src).is_empty());
}

#[test]
fn rust_ignore_with_reason_is_clean() {
    let src = "#[test]\n#[ignore = \"live network\"]\nfn hits_prod() {\n  assert!(ping());\n}\n";
    assert!(inspect_rust("a.rs", src).is_empty());
}

#[test]
fn rust_should_panic_with_no_assertion_is_clean() {
    let src = "#[test]\n#[should_panic]\nfn rejects() {\n  parse(\"bad\");\n}\n";
    assert!(inspect_rust("a.rs", src).is_empty());
}

// --- Rust: planted smells must fire ----------------------------------------

#[test]
fn rust_assertion_free_test_is_caught() {
    let f = inspect_rust("a.rs", "#[test]\nfn nothing() {\n  let x = compute();\n}\n");
    assert_eq!(f.len(), 1);
    assert!(f[0].contains("has no assertion"));
}

#[test]
fn rust_ignore_without_reason_is_caught() {
    let src = "#[test]\n#[ignore]\nfn skipped() {\n  assert!(x());\n}\n";
    assert!(any(
        &inspect_rust("a.rs", src),
        "#[ignore] without a reason"
    ));
}

#[test]
fn rust_assert_true_is_tautological() {
    assert!(any(
        &inspect_rust("a.rs", "#[test]\nfn t() {\n  assert!(true);\n}\n"),
        "tautological"
    ));
}

#[test]
fn rust_assert_eq_self_is_tautological() {
    let src = "#[test]\nfn t() {\n  let x = f();\n  assert_eq!(x, x);\n}\n";
    assert!(any(&inspect_rust("a.rs", src), "compares x to itself"));
}

#[test]
fn rust_comment_mentioning_assert_does_not_rescue_empty_test() {
    let src = "#[test]\nfn t() {\n  // assert_eq! would go here\n  let x = f();\n}\n";
    assert!(any(&inspect_rust("a.rs", src), "has no assertion"));
}

#[test]
fn rust_raw_string_containing_assert_does_not_rescue_empty_test() {
    let src = "#[test]\nfn t() {\n  let s = r#\"assert_eq!(a, b)\"#;\n}\n";
    assert!(any(&inspect_rust("a.rs", src), "has no assertion"));
}

// --- TS: clean and planted -------------------------------------------------

#[test]
fn ts_test_with_expect_is_clean() {
    assert!(
        inspect_ts(
            "a.test.ts",
            "it('adds', () => {\n  expect(add(2, 2)).toBe(4);\n});\n"
        )
        .is_empty()
    );
}

#[test]
fn ts_it_skip_is_caught() {
    assert!(any(
        &inspect_ts(
            "a.test.ts",
            "it.skip('later', () => { expect(x()).toBe(1); });\n"
        ),
        "skipped/todo test"
    ));
}

#[test]
fn ts_it_only_is_caught() {
    assert!(any(
        &inspect_ts(
            "a.test.ts",
            "it.only('just this', () => { expect(x()).toBe(1); });\n"
        ),
        "focused test"
    ));
}

#[test]
fn ts_assertion_free_test_is_caught() {
    assert!(any(
        &inspect_ts(
            "a.test.ts",
            "test('nothing', () => {\n  const y = compute();\n});\n"
        ),
        "has no assertion"
    ));
}

#[test]
fn ts_expect_self_is_tautological() {
    assert!(any(
        &inspect_ts(
            "a.test.ts",
            "it('t', () => { const x = f(); expect(x).toBe(x); });\n"
        ),
        "compared to itself"
    ));
}

#[test]
fn ts_process_exit_in_test_file_is_caught() {
    let src = "test('t', () => { expect(1).toBe(1 + 0); });\nprocess.exit(0);\n";
    assert!(any(&inspect_ts("a.test.ts", src), "process.exit(0)"));
}

// --- regressions for the review findings -----------------------------------

#[test]
fn ts_options_object_is_not_mistaken_for_the_callback_body() {
    assert!(
        inspect_ts(
            "a.test.ts",
            "it('x', { timeout: 100 }, () => { expect(f()).toBe(1); });\n"
        )
        .is_empty()
    );
}

#[test]
fn ts_expression_bodied_arrow_with_no_assertion_is_caught() {
    assert!(any(
        &inspect_ts("a.test.ts", "test('nothing', () => compute());\n"),
        "has no assertion"
    ));
}

#[test]
fn ts_expression_bodied_arrow_with_assertion_is_clean() {
    assert!(inspect_ts("a.test.ts", "it('t', () => expect(f()).toBe(1));\n").is_empty());
}

#[test]
fn ts_chained_modifiers_are_caught() {
    assert!(any(
        &inspect_ts(
            "a.test.ts",
            "test.concurrent.only('x', () => { expect(x()).toBe(1); });\n"
        ),
        "focused test"
    ));
    assert!(any(
        &inspect_ts(
            "a.test.ts",
            "it.concurrent.skip('x', () => { expect(x()).toBe(1); });\n"
        ),
        "skipped/todo test"
    ));
}

// ---- silent self-skip (Rust) ------------------------------------------------

#[test]
fn rust_return_before_the_first_assertion_is_a_silent_self_skip() {
    let src = "#[test]\nfn t() {\n    if std::env::var(\"CI\").is_err() { return; }\n    assert!(f());\n}\n";
    assert!(any(&inspect_rust("a.rs", src), "silent self-skip"));
}

#[test]
fn rust_return_after_assertions_is_fine() {
    let src = "#[test]\nfn t() {\n    assert!(pre());\n    if done() { return; }\n    assert!(post());\n}\n";
    assert!(inspect_rust("a.rs", src).is_empty());
}

// ---- swallowed assertions (TS) ---------------------------------------------

#[test]
fn ts_empty_catch_swallowing_an_expect_is_flagged() {
    let src = "it('x', async () => { try { expect(await f()).toBe(1); } catch (e) {} });\n";
    assert!(any(&inspect_ts("a.test.ts", src), "swallowed"));
}

#[test]
fn ts_empty_catch_around_cleanup_is_not_flagged() {
    let src =
        "it('x', async () => { expect(await f()).toBe(1); try { await rm(tmp); } catch {} });\n";
    assert!(inspect_ts("a.test.ts", src).is_empty());
}

#[test]
fn ts_catch_asserting_on_the_error_is_not_flagged() {
    let src =
        "it('x', async () => { try { await f(); } catch (e) { expect(e.code).toBe('E'); } });\n";
    assert!(inspect_ts("a.test.ts", src).is_empty());
}

#[test]
fn ts_dot_catch_empty_on_an_assertion_chain_is_flagged() {
    let src = "it('x', () => { f().then(r => expect(r).toBe(1)).catch(() => {}); });\n";
    assert!(any(&inspect_ts("a.test.ts", src), ".catch(() => {})"));
}

// ---- fire-and-forget async assertions (TS) ---------------------------------

#[test]
fn ts_unawaited_then_with_an_assertion_is_flagged() {
    let src = "it('x', () => { f().then(r => { expect(r).toBe(1); }); });\n";
    assert!(any(
        &inspect_ts("a.test.ts", src),
        "unawaited promise chain"
    ));
}

#[test]
fn ts_returned_and_awaited_chains_are_not_flagged() {
    let ret = "it('x', () => { return f().then(r => expect(r).toBe(1)); });\n";
    assert!(inspect_ts("a.test.ts", ret).is_empty());
    let awaited = "it('x', async () => { await f().then(r => expect(r).toBe(1)); });\n";
    assert!(inspect_ts("a.test.ts", awaited).is_empty());
}

#[test]
fn ts_assigned_chain_is_not_flagged() {
    let src = "it('x', async () => { const p = f().then(r => expect(r).toBe(1)); await p; });\n";
    assert!(inspect_ts("a.test.ts", src).is_empty());
}

#[test]
fn ts_then_without_an_assertion_is_not_flagged() {
    // Only assertion-carrying chains are the false green; a data chain is ordinary code.
    let src = "it('x', async () => { f().then(log); expect(await g()).toBe(1); });\n";
    assert!(inspect_ts("a.test.ts", src).is_empty());
}

#[test]
fn rust_poll_loop_return_on_success_is_not_a_self_skip() {
    // The poll-until idiom: return inside a loop is the SUCCESS path; the fall-through
    // panics. Poll-loop return-on-success must stay clean.
    let src = "#[test]\nfn t() {\n    for _ in 0..50 {\n        if probe.is_finished() { return; }\n        sleep(ms(10));\n    }\n    panic!(\"never finished\");\n}\n";
    assert!(inspect_rust("a.rs", src).is_empty());
}

#[test]
fn ts_expect_true_is_reported_once_not_twice() {
    let f = inspect_ts(
        "a.test.ts",
        "it('x', () => { expect(true).toBe(true); });\n",
    );
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].contains("expect(true) is tautological"));
}

// ---- Codex round 5 refinements ---------------------------------------------

#[test]
fn rust_return_ok_unit_before_assertion_is_a_self_skip() {
    let src = "#[test]\nfn t() -> Result<(), E> {\n    if skip() { return Ok(()); }\n    assert!(f());\n    Ok(())\n}\n";
    assert!(any(&inspect_rust("a.rs", src), "silent self-skip"));
}

#[test]
fn rust_process_exit_zero_in_a_test_is_flagged() {
    let src = "#[test]\nfn t() {\n    std::process::exit(0);\n    assert!(f());\n}\n";
    assert!(any(&inspect_rust("a.rs", src), "process::exit(0)"));
    // In production code (no #[test]) it is not a smell.
    assert!(inspect_rust("a.rs", "fn main() { std::process::exit(0); }\n").is_empty());
}

#[test]
fn ts_comparison_operators_do_not_suppress_fire_and_forget() {
    // `p !== null` is not an assignment; the chain is still unawaited.
    let src = "it('x', () => { if (p !== null) p.then(r => { expect(r).toBe(1); }); });\n";
    assert!(any(
        &inspect_ts("a.test.ts", src),
        "unawaited promise chain"
    ));
}

#[test]
fn ts_awaited_chain_with_object_literal_args_is_not_flagged() {
    // The `{}` inside make({}) must not cut the statement head and lose the `await`.
    let src = "it('x', async () => { await make({}).then(r => expect(r).toBe(1)); });\n";
    assert!(inspect_ts("a.test.ts", src).is_empty());
}

#[test]
fn ts_expression_bodied_catch_swallow_is_flagged() {
    let src = "it('x', async () => { await f().then(r => expect(r).toBe(1)).catch(() => undefined); });\n";
    assert!(any(&inspect_ts("a.test.ts", src), ".catch"));
}

#[test]
fn ts_multiline_chain_catch_swallow_is_flagged() {
    // The then-callback's `}` must not cut the statement so the assertion goes unseen.
    let src = "it('x', async () => {\n  await f()\n    .then(r => { expect(r).toBe(1); })\n    .catch(() => {});\n});\n";
    assert!(any(&inspect_ts("a.test.ts", src), ".catch"));
}

#[test]
fn ts_chained_thens_report_once() {
    let src = "it('x', () => { f().then(a).then(r => { expect(r).toBe(1); }); });\n";
    let flags: Vec<String> = inspect_ts("a.test.ts", src)
        .into_iter()
        .filter(|f| f.contains("unawaited"))
        .collect();
    assert_eq!(flags.len(), 1, "{flags:?}");
}

// ---- Codex round 6 refinements ---------------------------------------------

#[test]
fn ts_unbraced_for_header_is_not_an_assignment() {
    // `for (let i = 0; …)` without braces put its `=` in the head and suppressed the
    // finding; parenthesized groups are stripped before the assignment check.
    let src =
        "it('x', () => { for (let i = 0; i < 1; i++) p.then(r => { expect(r).toBe(1); }); });\n";
    assert!(any(
        &inspect_ts("a.test.ts", src),
        "unawaited promise chain"
    ));
}

#[test]
fn ts_template_interpolation_is_code_not_prose() {
    // `${…}` executes; a chain hidden inside it must still be seen.
    let src =
        "it('x', () => { expect(real()).toBe(1); `${p.then(r => { expect(r).toBe(2); })}`; });\n";
    assert!(any(
        &inspect_ts("a.test.ts", src),
        "unawaited promise chain"
    ));
}

#[test]
fn rust_a_deep_return_does_not_shadow_a_later_guard_return() {
    // The first pre-assertion return was the only one examined; a depth-3 return must
    // not hide a later depth-2 self-skip guard.
    let src = "#[test]\nfn t() -> Result<(), E> {\n    if a() {\n        if b() { return Ok(()); }\n    }\n    if skip() { return Ok(()); }\n    assert!(work());\n    Ok(())\n}\n";
    assert!(any(&inspect_rust("a.rs", src), "silent self-skip"));
}

#[test]
fn ts_a_parenthesized_assignment_of_the_chain_suppresses() {
    // `while ((p = f().then(...)))` assigns the chain; only balanced groups are
    // stripped, so the unclosed group keeps its `=` (Codex round 7).
    let src = "it('x', async () => { while ((p = f().then(r => { expect(r).toBe(1); }))) break; await p; });\n";
    assert!(
        !inspect_ts("a.test.ts", src)
            .iter()
            .any(|f| f.contains("unawaited")),
        "an assigned chain is not fire-and-forget"
    );
}

// ---- scanner kill-tests (mutation self-audit) --------------------------------

#[test]
fn statement_start_respects_every_boundary_and_balance() {
    // Plain boundaries.
    assert_eq!(statement_start("a; b.then(", 3), 2);
    assert_eq!(statement_start("{ b.then(", 2), 1);
    assert_eq!(statement_start("} b.then(", 2), 1);
    // Separators inside balanced groups are not boundaries.
    assert_eq!(statement_start("f(a;b) c.then(", 7), 0);
    assert_eq!(statement_start("a[i;j] c.then(", 7), 0);
    // A completed block before the statement is a boundary …
    assert_eq!(statement_start("if (x) { A } p.then(", 13), 12);
    // … but an object literal inside the statement's own args is not.
    assert_eq!(statement_start("await make({}).then(", 14), 0);
    assert_eq!(statement_start("await make([{}]).then(", 16), 0);
}

#[test]
fn statement_end_respects_every_boundary_and_balance() {
    assert_eq!(statement_end("p.then(a); tail", 0), 9);
    // Separators inside balanced groups are skipped.
    assert_eq!(statement_end("f(a;b); tail", 0), 6);
    assert_eq!(statement_end("a[i;j]; tail", 0), 6);
    assert_eq!(statement_end("g({ a: 1; }); t", 0), 12);
    // The enclosing block's close ends the statement.
    assert_eq!(statement_end("p.then(x) } rest", 0), 10);
    // No boundary at all: EOF.
    assert_eq!(statement_end("p.then(x)", 0), 9);
}

#[test]
fn callback_body_extracts_exactly_the_callback() {
    // Block-bodied arrow.
    let src = "it('x', () => { expect(1).toBe(1); })";
    assert_eq!(callback_body(src, 0), Some("{ expect(1).toBe(1); }"));
    // Expression-bodied arrow: up to the call's closing paren.
    let src = "it('x', () => expect(2))";
    assert_eq!(callback_body(src, 0).map(str::trim), Some("expect(2)"));
    // An options object before the callback is not the body.
    let src = "it('x', { timeout: 9 }, () => { expect(3); })";
    assert_eq!(callback_body(src, 0), Some("{ expect(3); }"));
    // function-keyword callback.
    let src = "it('x', function () { expect(4); })";
    assert_eq!(callback_body(src, 0), Some("{ expect(4); }"));
}

#[test]
fn callback_body_works_at_a_nonzero_call_index() {
    let src = "pre(); it('x', () => { expect(9); })";
    let idx = src.find("it(").unwrap();
    assert_eq!(callback_body(src, idx), Some("{ expect(9); }"));
}

#[test]
fn scanner_depth_guards_are_exact() {
    // Unmatched openers before the anchor must not corrupt depth accounting: the
    // boundary `;` at true depth 0 wins in each case.
    assert_eq!(statement_start("x; g(p.then(", 5), 2, "unclosed paren");
    assert_eq!(statement_start("x; a[p.then(", 5), 2, "unclosed bracket");
    assert_eq!(statement_start("x; a[i] p.then(", 8), 2, "balanced bracket");
    assert_eq!(
        statement_start("q; m({}).then(", 8),
        2,
        "balanced brace in args"
    );

    assert_eq!(statement_end("a); b", 0), 2, "stray close paren is skipped");
    assert_eq!(
        statement_end("a]; b", 0),
        2,
        "stray close bracket is skipped"
    );
    assert_eq!(statement_end("f[1]; t", 0), 4, "balanced bracket");
}

#[test]
fn is_test_file_is_exact() {
    assert!(is_test_file("src/a.rs"));
    assert!(is_test_file("x/b.test.ts"));
    assert!(is_test_file("x/b.spec.mjs"));
    assert!(!is_test_file("x/plain.ts"), "non-test TS is out of scope");
    assert!(!is_test_file("notes.md"));
}

#[test]
fn walk_collects_only_files_never_directories() {
    // A DIRECTORY named like a test file must not be scanned as one.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("fake.rs")).unwrap();
    std::fs::write(
        dir.path().join("real.test.ts"),
        "it('x', () => { expect(f()).toBe(1); });\n",
    )
    .unwrap();
    let scan = inspect_paths(&[dir.path().to_string_lossy().to_string()], &[]);
    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert_eq!(scan.files_scanned, 1);
    assert!(scan.failures.is_empty(), "{:?}", scan.failures);
}

#[test]
fn statement_start_keeps_brace_depth_inside_brackets() {
    // A balanced `{…}` object literal inside `arr[{…}]` is not a statement boundary: the
    // whole `q = arr[{…}]` is one expression, so the scan runs to the start. (A behavior
    // regression, not a mutation kill — the `{` here takes the brace>0 arm, so line 317's
    // guard is unreachable for well-formed input; see the equivalent-mutant table.)
    assert_eq!(statement_start("q = arr[{k: 1}].then(", 15), 0);
}
