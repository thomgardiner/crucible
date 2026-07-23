use super::*;

const LCOV: &str = "\
SF:/repo/src/pay.rs
FN:10,charge
FNDA:0,charge
FN:20,refund
FNDA:5,refund
DA:11,0
DA:12,0
DA:21,5
end_of_record
SF:/repo/src/util.rs
FN:3,helper
FNDA:2,helper
DA:3,2
end_of_record
";

fn changed(paths: &[&str]) -> std::collections::HashSet<String> {
    paths.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_lcov_extracts_functions_hits_and_uncovered_lines() {
    let files = parse_lcov(LCOV);
    assert_eq!(files.len(), 2);
    let pay = &files[0];
    assert_eq!(pay.file, "/repo/src/pay.rs");
    assert_eq!(
        pay.functions,
        vec![
            FnCov {
                name: "charge".into(),
                line: 10,
                hits: 0
            },
            FnCov {
                name: "refund".into(),
                line: 20,
                hits: 5
            },
        ]
    );
    assert_eq!(pay.uncovered_lines, vec![11, 12]);
    assert_eq!(files[1].functions[0].hits, 2);
}

#[test]
fn cover_flags_a_never_called_function_in_a_changed_file() {
    let files = parse_lcov(LCOV);
    // The lcov path is absolute; the git-diff path is relative. They still match.
    let r = cover(&files, &changed(&["src/pay.rs"]), &["pay".to_string()]);
    assert_eq!(r.untested.len(), 1, "{:?}", r.untested);
    assert_eq!(r.untested[0].name, "charge");
    assert!(r.untested[0].high_risk);
    assert_eq!(
        r.verdict, "block",
        "a never-called function in a high-risk unit blocks"
    );
    assert_eq!(r.uncovered_line_count, 2);
}

#[test]
fn cover_is_advisory_outside_high_risk_and_passes_when_covered() {
    let files = parse_lcov(LCOV);
    let advisory = cover(&files, &changed(&["src/pay.rs"]), &[]);
    assert_eq!(advisory.verdict, "advisory");
    let clean = cover(&files, &changed(&["src/util.rs"]), &["pay".to_string()]);
    assert!(clean.untested.is_empty());
    assert_eq!(clean.verdict, "pass");
}

#[test]
fn windows_lcov_paths_match_forward_slash_diff_paths() {
    // Codex round 3: `SF:C:\repo\src\pay.rs` never matched a changed `src/pay.rs`, so the
    // whole diff fell out of scope and cover passed vacuously.
    let lcov = "SF:C:\\repo\\src\\pay.rs\nFN:10,charge\nFNDA:0,charge\nend_of_record\n";
    let r = cover(&parse_lcov(lcov), &changed(&["src/pay.rs"]), &[]);
    assert_eq!(r.untested.len(), 1, "{:?}", r.untested);
    assert_eq!(r.untested[0].name, "charge");
}

#[test]
fn a_changed_source_file_with_no_lcov_record_is_reported_not_skipped() {
    // Codex round 3: a brand-new source file no test compiles has no record at all; the
    // intersection silently skipped it, certifying exactly the least-tested code.
    let files = parse_lcov(LCOV);
    let r = cover(&files, &changed(&["src/pay.rs", "src/brand_new.rs"]), &[]);
    assert_eq!(r.unmatched_changed, vec!["src/brand_new.rs".to_string()]);
    // A changed file whose extension coverage never reports (docs) stays out of scope.
    let docs = cover(&files, &changed(&["README.md"]), &[]);
    assert!(docs.unmatched_changed.is_empty());
}

#[test]
fn matched_records_without_function_data_are_not_function_coverage() {
    // Codex round 3: `SF:` + `DA:` with no `FN:` records made "every changed function is
    // exercised" vacuously true. The report exposes the evidence counts so the CLI can
    // refuse to certify.
    let lcov = "SF:src/pay.rs\nDA:1,0\nend_of_record\n";
    let r = cover(&parse_lcov(lcov), &changed(&["src/pay.rs"]), &[]);
    assert_eq!(r.fn_less_matched, vec!["src/pay.rs".to_string()]);
    assert!(
        r.untested.is_empty(),
        "no FN data → no untested claim either"
    );
}

#[test]
fn a_file_with_fn_data_does_not_mask_a_matched_file_without_it() {
    // Codex round 4: the FN-less check is per-file — src/a.rs having function records
    // must not certify src/b.rs, whose matched record carries none.
    let lcov = "SF:src/a.rs\nFN:1,a\nFNDA:1,a\nend_of_record\nSF:src/b.rs\nDA:1,1\nend_of_record\n";
    let r = cover(&parse_lcov(lcov), &changed(&["src/a.rs", "src/b.rs"]), &[]);
    assert_eq!(r.fn_less_matched, vec!["src/b.rs".to_string()]);
}

#[test]
fn extension_filtering_is_case_insensitive() {
    // `src/Foo.RS` must not slip past the unmatched-changed screen because of case.
    let files = parse_lcov(LCOV);
    let r = cover(&files, &changed(&["src/Foo.RS"]), &[]);
    assert_eq!(r.unmatched_changed, vec!["src/Foo.RS".to_string()]);
}

#[test]
fn parse_lcov_demangles_rust_symbols() {
    let sym = "_RNvMNtCsjWoOq7o8dkT_5proxy5leaseNtB2_10ProxyLease3new";
    let lcov = format!("SF:src/lease.rs\nFN:19,{sym}\nFNDA:0,{sym}\nend_of_record\n");
    let files = parse_lcov(&lcov);
    let name = &files[0].functions[0].name;
    assert!(
        name.contains("ProxyLease") && name.contains("new"),
        "demangled: {name}"
    );
    assert!(!name.contains("_RNv"), "still mangled: {name}");
}

#[test]
fn cover_dedups_codegen_duplicate_symbols() {
    // The same function reported twice (two codegen instances) collapses to one.
    let lcov = "\
SF:src/pay.rs
FN:10,charge
FNDA:0,charge
FN:10,charge
FNDA:0,charge
end_of_record
";
    let r = cover(&parse_lcov(lcov), &changed(&["src/pay.rs"]), &[]);
    assert_eq!(
        r.untested.len(),
        1,
        "one function, not two: {:?}",
        r.untested
    );
}

#[test]
fn parse_and_cover_flag_never_taken_branches() {
    // Line 5 has one arm taken (3) and one never taken (-); line 9 has an arm taken 0.
    let lcov = "\
SF:src/pay.rs
FN:1,f
FNDA:2,f
BRDA:5,0,0,3
BRDA:5,0,1,-
BRDA:9,0,0,0
end_of_record
";
    let files = parse_lcov(lcov);
    assert_eq!(files[0].untested_branches, vec![5, 9]);
    let r = cover(&files, &changed(&["src/pay.rs"]), &[]);
    assert_eq!(
        r.untested_branch_lines,
        vec![("src/pay.rs".to_string(), 5), ("src/pay.rs".to_string(), 9)]
    );
    // Branches are advisory info; a covered function means no block.
    assert_eq!(r.verdict, "pass");
}

#[test]
fn notfoo_does_not_match_a_changed_foo() {
    // Suffix matching must respect path-component boundaries (Codex P1 #9).
    let lcov = "SF:src/notfoo.rs\nFN:1,g\nFNDA:0,g\nend_of_record\n";
    let r = cover(&parse_lcov(lcov), &changed(&["src/foo.rs"]), &[]);
    assert!(
        r.untested.is_empty(),
        "notfoo.rs must not match changed foo.rs"
    );
    assert_eq!(r.verdict, "pass");
}

#[test]
fn trailing_record_without_end_of_record_is_kept() {
    // A truncated LCOV must not silently drop its last file (Codex P1 #9).
    let lcov = "SF:src/pay.rs\nFN:1,charge\nFNDA:0,charge\n";
    let files = parse_lcov(lcov);
    assert_eq!(files.len(), 1, "the trailing record must survive");
    let r = cover(&files, &changed(&["src/pay.rs"]), &[]);
    assert_eq!(r.untested.len(), 1);
}

#[test]
fn cover_ignores_files_the_change_did_not_touch() {
    let files = parse_lcov(LCOV);
    let r = cover(&files, &changed(&["src/other.rs"]), &["pay".to_string()]);
    assert!(r.untested.is_empty());
    assert_eq!(r.verdict, "pass");
}

// ---- mutation-run kill-tests (self-audit) ----------------------------------

#[test]
fn a_branch_with_only_taken_arms_is_not_flagged() {
    // BRDA taken counts > 0 are healthy arms; only "-" and "0" mark untested ones.
    let lcov = "SF:src/pay.rs\nFN:1,f\nFNDA:2,f\nBRDA:12,0,0,7\nBRDA:12,0,1,3\nend_of_record\n";
    let files = parse_lcov(lcov);
    assert!(
        files[0].untested_branches.is_empty(),
        "{:?}",
        files[0].untested_branches
    );
}

#[test]
fn ends_with_component_handles_identity_and_boundaries() {
    assert!(ends_with_component("src/foo.rs", "src/foo.rs"), "identity");
    assert!(ends_with_component("/repo/src/foo.rs", "src/foo.rs"));
    assert!(!ends_with_component("src/notfoo.rs", "foo.rs"));
}

#[test]
fn high_risk_unit_matches_by_path_component_not_bare_substring() {
    // The reward-hack a bare `contains` allowed: near-name files count as
    // high-risk, and a rename silently escapes the gate.
    assert!(path_hits_unit("src/pay.rs", "pay"), "module by file stem");
    assert!(
        !path_hits_unit("src/payload.rs", "pay"),
        "near-name must NOT match"
    );
    assert!(!path_hits_unit("src/repayment.rs", "pay"));
    assert!(
        path_hits_unit("src/payments/api.rs", "payments"),
        "dir segment"
    );
    assert!(
        path_hits_unit("src/payments/api.rs", "src/payments"),
        "path prefix"
    );
    assert!(
        path_hits_unit("/abs/repo/src/pay.rs", "pay"),
        "absolute lcov path"
    );
    assert!(
        !path_hits_unit("src/pay.rs", ""),
        "empty unit matches nothing"
    );
    // Windows separators normalize.
    assert!(path_hits_unit("src\\payments\\api.rs", "payments"));
}
