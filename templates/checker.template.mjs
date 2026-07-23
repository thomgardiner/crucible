#!/usr/bin/env node
// Checker template. Copy this to scripts/check-<thing>.mjs, fill in the invariant,
// and ship a scripts/check-<thing>.test.mjs alongside it that proves the checker
// fires on a planted violation. The shape below is the load-bearing convention:
// mask comments and strings so matches are code-only, accumulate itemized
// failures, and exit non-zero when any are found.

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

// Blank out // line comments, /* block */ comments, and "string"/'string'/`tpl`
// contents so a pattern cannot false-positive on prose or string literals.
export function maskCommentsAndStrings(src) {
  let out = '';
  let i = 0;
  const n = src.length;
  let mode = 'code';
  while (i < n) {
    const c = src[i];
    const next = src[i + 1];
    if (mode === 'code') {
      if (c === '/' && next === '/') { mode = 'line'; out += '  '; i += 2; continue; }
      if (c === '/' && next === '*') { mode = 'block'; out += '  '; i += 2; continue; }
      if (c === '"' || c === "'" || c === '`') { mode = c; out += c; i += 1; continue; }
      out += c; i += 1; continue;
    }
    if (mode === 'line') {
      if (c === '\n') { mode = 'code'; out += '\n'; } else out += ' ';
      i += 1; continue;
    }
    if (mode === 'block') {
      if (c === '*' && next === '/') { mode = 'code'; out += '  '; i += 2; continue; }
      out += c === '\n' ? '\n' : ' '; i += 1; continue;
    }
    // inside a string; mode holds the closing quote char
    if (c === '\\') { out += '  '; i += 2; continue; }
    if (c === mode) { mode = 'code'; out += c; i += 1; continue; }
    out += c === '\n' ? '\n' : ' '; i += 1; continue;
  }
  return out;
}

export function walk(dir, test) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const s = statSync(p);
    if (s.isDirectory()) out.push(...walk(p, test));
    else if (test(p)) out.push(p);
  }
  return out;
}

// Replace with the real invariant. Return an array of human-readable failures.
export function inspect(/* files */) {
  const failures = [];
  // e.g. for each file, mask, match a forbidden pattern, push a failure with path.
  return failures;
}

function main() {
  const failures = inspect(/* walk(root, (p) => p.endsWith('.rs')) */);
  if (failures.length) {
    console.error(`check-<thing>: ${failures.length} violation(s):`);
    for (const f of failures) console.error(`  ✗ ${f}`);
    process.exit(1);
  }
}

// Only run when invoked directly, so tests can import inspect()/mask without
// exiting. pathToFileURL handles paths with spaces and Windows separators, which
// a raw `file://${argv[1]}` comparison silently gets wrong (the checker would
// then no-op).
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
