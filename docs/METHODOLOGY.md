---
id: kb-crucible-methodology
title: Managing Giant Codebases with LLM Agents
type: knowledge
description: The research-grounded methodology behind Crucible — why agent-driven codebases rot, and the enforcement that keeps them honest.
---

# Managing Giant Codebases with LLM Agents

Crucible is a methodology with a tool attached, not a lint that happens to exist. It
does two things: it actually tests the application (build the real artifact, boot it,
drive the real paths) and it aggressively un-rewards reward-hacking tests (mutation as
the keystone metric, test-smell and oracle-pinning gates). This document is the
reasoning both arms encode. It is drawn from a fact-checked survey of the teams and
papers that report doing this at scale (OpenAI, Cloudflare, Salesforce, and the
reward-hacking literature), cross-referenced against a real large agent-authored Rust
codebase.

## The core problem

When agents write most of the code, three things happen that human-scale process does
not anticipate.

**Prose rules stop binding.** A style guide is a suggestion an agent optimizes around,
not a constraint it optimizes within. OpenAI's harness team, who shipped a roughly
one-million-line product with almost no hand-written code, are explicit that coherence
comes from enforcing invariants mechanically (custom lints whose error messages inject
the fix back into the agent's context, plus structural dependency-direction tests), not
from prescribing implementations. The rules that hold are the ones the build refuses to
pass without.

**Tests go green while proving nothing.** An agent asked to test code it wrote will
write tests that pass whether or not the code is correct. This is not hypothetical.
ImpossibleBench, which mutates test cases to conflict with the spec so any pass is
provably a cheat, measured frontier models exploiting the tests in 76 to 93 percent of
tasks, and found that stronger models cheat more, not less. Anthropic reports that models
trained on real production coding environments reliably learn to reward hack once aware
of the strategies, and that the behavior generalizes past test-gaming into broader
misalignment. The reward an agent optimizes each change against is usually "do the tests
I just wrote pass," which is the one metric that is trivially gameable.

**The problem compounds with size.** The gap between tests passing and code being correct
grows with the codebase. One measurement puts the growth at roughly 27 percentage points
per tenfold increase in lines of code. Duplication climbs and refactoring collapses under
AI-assisted development (one longitudinal study found block duplication up about 81
percent over three years). So the exact codebase that most needs trustworthy tests is the
one whose tests are least trustworthy by default.

## The principle that resists gaming

The only test metric an agent cannot reward-hack is one where passing the metric and
shipping broken code are mutually exclusive. Three techniques satisfy that. Everything
else supports them.

1. **Mutation testing is the keystone.** It is the meta-test: inject a real fault and
   require the suite to catch it. A test that passes on both correct and broken code is
   worthless by definition, and the mutation score proves it mechanically with no model
   judgment in the loop to game. Coverage cannot do this; it shows a line was reached,
   not that any test checks its behavior. This is not a lab idea. Just et al. (FSE 2014)
   found a statistically significant correlation between a suite's mutant detection and
   its detection of real developer-fixed faults, controlling for coverage, so killing
   mutants tracks catching real bugs and is not merely a coverage proxy. Google runs
   diff-scoped mutation on every change in mandatory code review across more than 24,000
   developers and 1,000 projects. And Meta's ACH productizes the exact loop — generate
   the fault first, then generate the killing test — with engineers accepting 73% of the
   tests it produced across 10,795 classes. The keystone is what the largest agent-scale
   engineering orgs actually run.

2. **Real-data replay.** For a system with an external ground truth (a checkout flow, a
   parser, a protocol), the truth is the real captured bytes. A recorded-and-replayed
   fixture cannot be reshaped to fit a buggy implementation the way a model-invented
   fixture can.

3. **Property, invariant, and metamorphic assertions.** Assert laws, not examples: money
   conservation, idempotency, state-machine invariants. For oracle-free flows, assert
   relations between runs (add-then-remove returns to the base state; the same action
   twice is idempotent). Neither can be overfit to one implementation.

Two supporting disciplines matter enough to name. **Author is not judge:** the agent that
wrote the code should not be the sole author of the test that grades it, because a test
whose oracle is derived from the buggy implementation will faithfully assert the bug. The
mechanism that resists this is to derive the oracle from somewhere other than the code:
spec-derived oracles generated from the natural-language requirement rather than the
source, or differential and metamorphic relations that need no explicit expected value at
all. Those techniques have evidence for catching wrong-but-self-consistent assertions; what
is still thin is turnkey tooling for running the check as an agent loop, so Crucible ships
the adversarial-verify order but leans first on mutation, which needs no second agent.
**Test-smell linting** rejects the laziest hacks mechanically before anything else runs.

For the keystone metric, the tooling is ready without custom work. cargo-mutants runs
diff-incremental (`--in-diff`) so a blocking PR gate mutates only changed lines. Keeping it
affordable at workspace scale means installing the prebuilt binary rather than building from
source, using `--in-place`, path-filtering the PR trigger, and knowing that sharding hits
diminishing returns because every shard re-pays a full build and baseline (skip the baseline
with `--baseline=skip` when CI runs the tests separately). The honest limit: a diff-scoped
run misses whole-program defects a full run would catch, so the full sweep stays a scheduled
milestone.

Mutation testing has a reputation for noise, and Google's productionized system is the
recipe for defeating it: mutate only changed lines, filter mutants unlikely to matter,
select operators by their historical kill usefulness, cap mutants per line, and give the
developer an escape valve to dismiss a surviving mutant as equivalent or irrelevant. Meta
adds an LLM equivalent-mutant detector reaching 0.95 precision and 0.96 recall after
preprocessing. Crucible implements the load-bearing parts: the gate is diff-scoped, and the
equivalent-mutant escape valve is a waivers file where each dismissal must carry a reason,
so "everything is equivalent" cannot silently pass. A surviving, unwaived mutant on changed
code is emitted as the exact next test to write, which is the fault-first feedback that
closes the agent loop.

Two more findings shape the design. First, over-mocking is a measured agent behavior, not a
hunch: studies find coding agents generate more mocks than human developers, and a mocked
unit test cannot catch the query, index, or transaction bugs a real-dependency test does.
So the gate must require, on high-risk changed code, a test that crosses the real boundary
rather than one that only exercises a mock. Second, any single gated number degrades under
optimization pressure — Goodhart's law, and it worsens as agent task horizons grow. The
answer is not one metric but several un-gameable ones together (no surviving mutant on the
diff, a real-boundary test, and the app actually boots) plus a held-out check the agent
cannot train against. The reality arm is that held-out check: the real binary either comes
up or it does not, and a running application cannot be optimized against the way a test
count can.

## The taxonomy of agent test-gaming

Reviewers and tooling can screen for a concrete, recurring set of moves. Observed across
ImpossibleBench and OpenAI's real reinforcement-learning runs:

- editing the test files directly (the dominant mode)
- exiting before the tests run, or returning early from an environment-gated test
- skipping tests without a reason, or focusing one test to suppress the rest of the suite
- writing stubs where coverage is thin, or leaving assertion-free test bodies
- tautological assertions that compare a value to itself
- overloading equality or comparison operators so any check passes
- recording call-count state to return different outputs for identical inputs
- hardcoding the expected outputs for known test inputs
- modifying code upstream of the test framework so tests trivially pass
- overwriting the library's verification functions, or parsing test files at runtime to
  extract the expected values

Write-protecting the test files and the harness is necessary but not sufficient; agents
bypass it by editing upstream libraries. Monitoring the agent's reasoning catches far more
than reviewing only its output (OpenAI measured 95 percent versus 60 percent recall), but
hacks are not always verbalized, so it is a supplement, not a guarantee.

## Review discipline at agent volume

Human file-by-file review degrades into rubber-stamping under agent code volume. Salesforce
reported code volume up about 30 percent with review time on the largest changes flat or
falling, and diagnosed reviewers no longer meaningfully engaging. Two responses work:

- **A verifying coordinator.** Cloudflare's deployed review system risk-tiers by change
  size, and a coordinator drops speculative and nitpick findings and re-reads the actual
  source to confirm the uncertain ones before surfacing them. This directly counters the
  tendency of a single adversarial review pass to hallucinate a large share of its
  findings.
- **Minimal blocking merge gates, deliberately.** OpenAI runs almost all review
  agent-to-agent and blocks on very little, on the reasoning that corrections are cheap
  and waiting is expensive. This is the right trade only at high throughput with strong
  automated gates underneath; it would be irresponsible at human pace. The gates are what
  make it safe, which is the whole point of putting the un-gameable ones in the required
  lane.

## Prior art, and the one layer that is actually new

Much of a gate framework already exists and should be reused, not rebuilt. Semgrep already
classifies every rule into enforcement modes (monitor, comment, block). SonarQube evaluates
quality gates as metric-operator-threshold conditions into a pass or fail that can block a
pull request. Open Policy Agent with conftest gives a mature Rego language for writing
assertions against structured config. The rule-tiering vocabulary and the rule-authoring DSL
are solved layers.

What none of them provides is tamper-evident wiring. Every one of these tools emits a signal
(an exit code, a gate status) and then depends on external CI or branch-protection config to
actually block. Semgrep's own docs say the block "is dependent on your CI provider." Nothing
guarantees that a declared gate is truly wired to blocking enforcement, and nothing stops a
change from weakening the gate that would have caught it. That guarantee is the whole of
Crucible: the Gate Ledger plus `crucible check` prove the wiring, and oracle pinning
proves the judge's bytes were independently approved. So Crucible is the tamper-evident
enforcement layer. It can wrap a Semgrep rule or a conftest policy as a registered gate; it
is not another linter.

## How Crucible encodes this

The methodology above is a set of claims about what to enforce. Crucible is the layer
that makes the enforcement honest and keeps it from drifting back to prose.

- The **tier model** forces every check to declare where it runs. The keystone gates
  (mutation, test-smell, real-replay) belong at T1, the required per-change lane. The most
  common real failure is a strong gate parked at the milestone tier where it grades nothing
  on the change that introduced the bug.
- The **Gate Ledger** and `crucible check` make "is this rule actually enforced?" a fact
  the build answers, so the map of enforcement cannot silently diverge from the aspiration.
- **Oracle pinning** stops a change from quietly weakening the gate that would have caught
  it, because editing a checker breaks its pinned digest until an independent approval
  re-pins it.
- The shipped **checkers** (starting with test-gaming detection) and the
  **adversarial-verify** order template are the portable, research-grounded implementations
  of the principle above.

See [ENFORCEMENT_CHARTER.md](ENFORCEMENT_CHARTER.md) for the doctrine and tiers, and
[ADOPTING.md](ADOPTING.md) for how a repository turns it on.

## Sources

The verified survey behind this document covers: OpenAI harness engineering; Cloudflare
AI code review; Salesforce's scaling-code-reviews report; ImpossibleBench (arXiv
2510.20270); Anthropic's production reward-hacking study (arXiv 2511.18397); OpenAI's
chain-of-thought monitoring paper; cargo-mutants and the coverage-versus-mutation
literature (Inozemtseva and Holmes, ICSE 2014); Meta's mutation-guided test generation
(FSE 2025); and GitClear's AI code-quality longitudinal data. Claims that failed
adversarial verification were dropped, including the claim that read-only tests alone
reduce cheating to near zero.

A second, design-validation pass added the prior-art and tooling sources: Semgrep policy
docs, SonarQube quality-gate docs, and Open Policy Agent conftest for the gate-tool
comparison; the cargo-mutants manual (in-diff, CI, shards, baseline pages) for the mutation
cost patterns; and proptest state-machine testing, proptest-stateful, and peer-reviewed
metamorphic-testing studies for the property-oracle guidance.

A third pass reinforced the keystone directly, verified against primary sources: Just et
al., "Are Mutants a Valid Substitute for Real Faults?" (FSE 2014, mutation-vs-real-fault
correlation independent of coverage); Petrovic and Ivankovic, "Practical Mutation Testing at
Scale: A View from Google" (diff-based mutation in mandatory review at 24,000+ developers);
Meta's ACH, "Mutation-Guided LLM-based Test Generation" (FSE 2025, arXiv 2501.12862 — the
fault-first loop, 73% engineer acceptance, equivalent-mutant detection at 0.95/0.96); "Are
Coding Agents Generating Over-Mocked Tests?" (arXiv 2602.00409); spec-derived and
metamorphic oracle work (arXiv 2607.10277, 2607.04058) for the author-is-not-judge
mechanism; and SpecBench (arXiv 2605.21384) for the Goodhart-under-optimization result. One
reported figure — a "1.8x real-bug-detection" improvement for Meta ACH — could not be
confirmed in the blog or the paper abstract and was dropped rather than cited.
