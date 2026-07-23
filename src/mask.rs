//! Blank out `//` line comments, `/* block */` comments, and string/char/raw-string
//! contents so a checker pattern cannot false-positive on prose or literals.
//! Operates on bytes and replaces every masked byte with a space (newlines kept), so
//! the result is the same byte length as the input and line/offset math survives.
//! Masked regions are pure ASCII and code regions are copied verbatim, so the output
//! is always valid UTF-8.

#[derive(PartialEq)]
enum Mode {
    Code,
    Line,
    Block,
    // Inside a string literal; the byte holds the closing quote (" ' or `).
    Str(u8),
    // Inside a template literal's `${ … }` interpolation: executable code, kept visible
    // (an assertion or promise chain in there is real code, not prose — Codex round 6).
    // The depth tracks nested braces; at 0 the closing `}` returns to the template.
    Interp(u32),
}

pub fn mask_comments_and_strings(src: &str) -> String {
    let b = src.as_bytes();
    let n = b.len();
    let mut out: Vec<u8> = Vec::with_capacity(n);
    let mut i = 0usize;
    let mut mode = Mode::Code;
    while i < n {
        let c = b[i];
        let next = if i + 1 < n { b[i + 1] } else { 0 };
        match mode {
            Mode::Code => {
                if c == b'/' && next == b'/' {
                    mode = Mode::Line;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    continue;
                }
                if c == b'/' && next == b'*' {
                    mode = Mode::Block;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    continue;
                }
                // Rust raw string r"..." / r#"..."# — embedded quotes do not end it.
                if c == b'r' && (i == 0 || !is_ident_byte(b[i - 1])) {
                    let mut j = i + 1;
                    let mut hashes = 0usize;
                    while j < n && b[j] == b'#' {
                        hashes += 1;
                        j += 1;
                    }
                    if j < n && b[j] == b'"' {
                        out.push(b'r');
                        out.extend(std::iter::repeat_n(b'#', hashes));
                        out.push(b'"');
                        let content_start = j + 1;
                        let close: Vec<u8> = std::iter::once(b'"')
                            .chain(std::iter::repeat_n(b'#', hashes))
                            .collect();
                        match find(b, content_start, &close) {
                            Some(end) => {
                                for &byte in &b[content_start..end] {
                                    out.push(if byte == b'\n' { b'\n' } else { b' ' });
                                }
                                out.extend_from_slice(&close);
                                i = end + close.len();
                            }
                            None => {
                                for &byte in &b[content_start..n] {
                                    out.push(if byte == b'\n' { b'\n' } else { b' ' });
                                }
                                i = n;
                            }
                        }
                        continue;
                    }
                }
                if c == b'"' || c == b'\'' || c == b'`' {
                    mode = Mode::Str(c);
                    out.push(c);
                    i += 1;
                    continue;
                }
                out.push(c);
                i += 1;
            }
            Mode::Line => {
                if c == b'\n' {
                    mode = Mode::Code;
                    out.push(b'\n');
                } else {
                    out.push(b' ');
                }
                i += 1;
            }
            Mode::Block => {
                if c == b'*' && next == b'/' {
                    mode = Mode::Code;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    continue;
                }
                out.push(if c == b'\n' { b'\n' } else { b' ' });
                i += 1;
            }
            Mode::Str(q) => {
                if c == b'\\' {
                    // Skip the escaped byte so `\"` does not close the string. A
                    // multibyte escaped char keeps its continuation bytes in string
                    // mode, where they are spaced out individually.
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                } else if q == b'`' && c == b'$' && next == b'{' {
                    // `${…}` in a template literal is executable code, not prose.
                    mode = Mode::Interp(0);
                    out.push(b'$');
                    out.push(b'{');
                    i += 2;
                } else if c == q {
                    mode = Mode::Code;
                    out.push(c);
                    i += 1;
                } else {
                    out.push(if c == b'\n' { b'\n' } else { b' ' });
                    i += 1;
                }
            }
            Mode::Interp(depth) => {
                if c == b'"' || c == b'\'' || c == b'`' {
                    // A string inside the interpolation: mask its contents inline
                    // WITHOUT leaving Interp, so a `}` in the string cannot end the
                    // interpolation early and hide the code after it (Codex round 7).
                    // A nested backtick template is masked whole — its own `${}` is
                    // lost, which errs toward masking too little elsewhere, never
                    // toward hiding code outside it.
                    out.push(c);
                    let mut j = i + 1;
                    while j < n {
                        if b[j] == b'\\' {
                            out.push(b' ');
                            out.push(b' ');
                            j += 2;
                        } else if b[j] == c {
                            out.push(c);
                            j += 1;
                            break;
                        } else {
                            out.push(if b[j] == b'\n' { b'\n' } else { b' ' });
                            j += 1;
                        }
                    }
                    i = j;
                    continue;
                }
                if c == b'{' {
                    mode = Mode::Interp(depth + 1);
                } else if c == b'}' {
                    mode = if depth == 0 {
                        Mode::Str(b'`')
                    } else {
                        Mode::Interp(depth - 1)
                    };
                }
                out.push(c);
                i += 1;
            }
        }
    }
    // Safe: masked bytes are ASCII, code bytes are copied contiguously from valid UTF-8.
    String::from_utf8(out).expect("mask preserves UTF-8")
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn find(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// Given a source string and a byte index at or before an opening brace, return the
/// `[start, end)` byte span of the balanced `{ ... }` block, or None if unbalanced.
pub fn block_span(src: &str, from_index: usize) -> Option<(usize, usize)> {
    let b = src.as_bytes();
    let start = find(b, from_index, b"{")?;
    let mut depth = 0i32;
    let mut i = start;
    while i < b.len() {
        if b[i] == b'{' {
            depth += 1;
        } else if b[i] == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some((start, i + 1));
            }
        }
        i += 1;
    }
    None
}

/// 1-based line number of a byte index (counts newlines before it).
pub fn line_of(src: &str, index: usize) -> usize {
    let b = src.as_bytes();
    let end = index.min(b.len());
    1 + b[..end].iter().filter(|&&c| c == b'\n').count()
}

#[cfg(test)]
#[path = "mask_tests.rs"]
mod tests;
