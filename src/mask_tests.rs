use super::*;

#[test]
fn masks_line_comments_but_keeps_code() {
    let m = mask_comments_and_strings("let x = 1; // assert_eq!(a, a)\nlet y = 2;");
    assert!(m.contains("let x = 1;"));
    assert!(
        !m.contains("assert_eq"),
        "comment content must be blanked: {m:?}"
    );
    assert!(m.contains("let y = 2;"));
}

#[test]
fn masks_block_comments() {
    let m = mask_comments_and_strings("a /* panic!() */ b");
    assert!(!m.contains("panic"));
    assert!(m.contains("a "));
    assert!(m.contains(" b"));
}

#[test]
fn masks_string_contents_including_escaped_quote() {
    let m = mask_comments_and_strings(r#"let s = "he said \"assert\" ok"; let z = 9;"#);
    assert!(!m.contains("assert"), "string body blanked: {m:?}");
    assert!(
        m.contains("let z = 9;"),
        "code after the string survives: {m:?}"
    );
}

#[test]
fn masks_rust_raw_string_with_embedded_quotes() {
    let m = mask_comments_and_strings(r####"let s = r#"a "b" panic!()"#; let k = 1;"####);
    assert!(!m.contains("panic"));
    assert!(m.contains("let k = 1;"));
}

#[test]
fn output_is_byte_length_preserving() {
    let src = "fn f() { let s = \"π ✓ multi\"; /* café */ }\nnext";
    let m = mask_comments_and_strings(src);
    assert_eq!(
        m.len(),
        src.len(),
        "mask must preserve byte length for offset math"
    );
}

#[test]
fn line_of_counts_newlines() {
    let src = "a\nb\nc";
    assert_eq!(line_of(src, 0), 1);
    assert_eq!(line_of(src, 2), 2);
    assert_eq!(line_of(src, 4), 3);
    assert_eq!(line_of(src, 999), 3, "past end clamps");
}

#[test]
fn block_span_finds_balanced_braces() {
    let src = "fn f() { if x { y } }";
    let (start, end) = block_span(src, 0).unwrap();
    assert_eq!(&src[start..end], "{ if x { y } }");
}

#[test]
fn block_span_none_when_unbalanced() {
    assert!(block_span("fn f() { oops", 0).is_none());
}

#[test]
fn template_interpolation_stays_code() {
    // `${…}` is executable; only the literal text around it is masked.
    let m = mask_comments_and_strings("`before ${call(x)} after`;");
    assert!(m.contains("${call(x)}"), "{m}");
    assert!(!m.contains("before"), "{m}");
    assert!(!m.contains("after"), "{m}");
}

#[test]
fn nested_braces_inside_interpolation_do_not_end_it_early() {
    let m = mask_comments_and_strings("`x ${f({ a: 1 })} y`;");
    assert!(m.contains("f({ a: 1 })"), "{m}");
}

#[test]
fn a_quoted_brace_inside_interpolation_does_not_end_it() {
    // `${f("}")}`: the `}` inside the string must not close the interpolation and hide
    // the code after it (Codex round 7).
    let m = mask_comments_and_strings("`${f(\"}\") && g(x)}rest`;");
    assert!(m.contains("&& g(x)"), "{m}");
    assert!(!m.contains("rest"), "template text stays masked: {m}");
}

// ---- mutation-run kill-tests (self-audit) ----------------------------------

#[test]
fn raw_strings_mask_contents_and_keep_structure() {
    // Embedded quotes do not end a raw string; content is blanked, code after stays.
    let m = mask_comments_and_strings("let s = r#\"a \"b\" c\"#; assert!(x);");
    assert!(m.contains("assert!(x)"), "{m}");
    assert!(!m.contains("a \"b\" c"), "{m}");
    assert!(m.contains("r#\""), "the raw-string frame survives: {m}");
}

#[test]
fn multi_hash_raw_strings_close_only_on_the_full_delimiter() {
    // `"#` inside an r##"…"## body is content, not the closer.
    let m = mask_comments_and_strings("let s = r##\"has \"# inside\"##; assert!(y);");
    assert!(m.contains("assert!(y)"), "{m}");
    assert!(!m.contains("inside"), "{m}");
}

#[test]
fn an_identifier_ending_in_r_is_not_a_raw_string() {
    // `war"x"` is ident + normal string, not a raw string.
    let m = mask_comments_and_strings("war\"x\"; assert!(z);");
    assert!(m.contains("war\""), "{m}");
    assert!(m.contains("assert!(z)"), "{m}");
    assert!(!m.contains('x'), "string content masked: {m}");
}

#[test]
fn an_unterminated_raw_string_masks_to_eof() {
    let m = mask_comments_and_strings("let s = r#\"never closed; assert!(q);");
    assert!(
        !m.contains("assert!(q)"),
        "content after opener is string: {m}"
    );
    assert_eq!(m.len(), "let s = r#\"never closed; assert!(q);".len());
}

#[test]
fn masking_preserves_length_and_newline_positions_for_every_tricky_shape() {
    // The module contract: same byte length, newlines kept, so line/offset math
    // survives. One structural oracle over every construct the masker handles.
    let samples = [
        "let s = r#\"multi\nline \"quoted\" raw\"#;\nassert!(a);",
        "let s = r##\"has \"# inside\nand newline\"##; assert!(b);",
        "/* block with * and / inside\nsecond line */ assert!(c);",
        "\"escaped \\\" quote\" + 'x' + `tpl ${call(1)} end`;\nassert!(d);",
        "`${f(\"}\")} after`; // trailing comment\nassert!(e);",
        "let s = r#\"never closed\nwith newline; assert!(f);",
        "\"dollar ${not_interp} in normal string\"; assert!(g);",
        "`${g(\"a\nb\")} tail`;\nassert!(h);",
    ];
    for src in samples {
        let m = mask_comments_and_strings(src);
        assert_eq!(m.len(), src.len(), "length changed for: {src}");
        for (i, (a, b)) in src.bytes().zip(m.bytes()).enumerate() {
            assert_eq!(
                a == b'\n',
                b == b'\n',
                "newline mismatch at byte {i} for: {src}"
            );
        }
    }
}

#[test]
fn a_block_comment_is_only_ended_by_the_full_terminator() {
    // A lone `*` or `/` inside the block must not end it.
    let m = mask_comments_and_strings("/* alpha * beta / gamma */ assert!(k);");
    assert!(m.contains("assert!(k)"), "{m}");
    assert!(
        !m.contains("alpha") && !m.contains("beta") && !m.contains("gamma"),
        "{m}"
    );
}

#[test]
fn dollar_brace_in_a_normal_string_is_not_an_interpolation() {
    // Only template literals interpolate; `"${…}"` in quotes is literal text.
    let m = mask_comments_and_strings("\"a ${secret}\"; assert!(w);");
    assert!(!m.contains("secret"), "{m}");
    assert!(m.contains("assert!(w)"), "{m}");
}

#[test]
fn a_raw_prefix_requires_a_non_ident_boundary() {
    // `ber"alpha\"beta"` is ident + NORMAL string: the escape keeps it open to the
    // second quote. Misreading it as a raw string would close at the escaped quote and
    // let string content leak as code.
    let m = mask_comments_and_strings("ber\"alpha\\\"beta\"; assert!(v);");
    assert!(m.contains("assert!(v)"), "{m}");
    assert!(
        !m.contains("alpha") && !m.contains("beta"),
        "content masked: {m}"
    );
}

#[test]
fn a_raw_opener_at_eof_does_not_panic() {
    assert_eq!(mask_comments_and_strings("r").len(), 1);
    assert_eq!(mask_comments_and_strings("r#").len(), 2);
    assert_eq!(mask_comments_and_strings("let x = r#\"").len(), 11);
}

#[test]
fn interp_depth_unwinds_back_to_template_masking() {
    // After the nested `}` unwinds, the remaining template text is prose again.
    let m = mask_comments_and_strings("`x ${f({ a: 1 })} y`;");
    assert!(m.contains("f({ a: 1 })"), "{m}");
    assert!(
        !m.contains('y'),
        "template text after the interp is masked: {m}"
    );
    assert!(
        !m.contains('x'),
        "template text before the interp is masked: {m}"
    );
}

#[test]
fn exact_output_for_a_simple_string() {
    // Byte-exact: any stray structural byte (e.g. a spurious raw-string 'r') fails.
    assert_eq!(
        mask_comments_and_strings("a = \"bc\"; ok"),
        "a = \"  \"; ok"
    );
}

#[test]
fn exact_output_for_a_quoted_brace_interpolation() {
    assert_eq!(
        mask_comments_and_strings("`${f(\"}\")}end`;"),
        "`${f(\" \")}   `;"
    );
}

#[test]
fn an_escape_inside_an_interpolation_string_masks_exactly() {
    // `${f("a\"b")}x`; — the escaped quote must not close the inner string.
    assert_eq!(
        mask_comments_and_strings("`${f(\"a\\\"b\")}x`;"),
        "`${f(\"    \")} `;"
    );
}

#[test]
fn an_unterminated_interpolation_string_at_eof_does_not_panic() {
    let src = "`${f(\"unterminated";
    let m = mask_comments_and_strings(src);
    assert_eq!(m.len(), src.len());
}

#[test]
fn a_lone_slash_or_star_operator_does_not_start_a_block_comment() {
    // `c == '/' && next == '*'` must be AND: a division or multiplication operator is
    // code, not a comment opener. (kills the `&&`->`||` mutant)
    assert_eq!(
        mask_comments_and_strings("let q = a / b * c;"),
        "let q = a / b * c;"
    );
}

#[test]
fn a_single_quote_string_inside_interpolation_is_masked_inline() {
    // The interpolation inner-string detector must cover `'` and backtick, not just `"`:
    // otherwise a `}` inside a single-quoted string ends the interpolation early and
    // hides the code after it. (kills the second `||`->`&&` mutant on line 133)
    // With the mutant, the `'` is not detected inline, so the `}` inside `'}'` ends the
    // interpolation early and `&& real(x)` is masked as template text.
    let m = mask_comments_and_strings("`${ok('}') && real(x)}`;");
    assert!(m.contains("&& real(x)"), "{m}");
}
