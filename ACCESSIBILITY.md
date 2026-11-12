# Accessibility

What `ikigai-text` emits that people read, the rules that output follows, how it is tested, and
how to report a barrier. It is a library of `urn:text:*` tools (a coreutils analog): it answers
with plain text, and a host decides how a person sees it. It is not a conformance claim. Adapted
from the [ACCESSIBILITY.md template](https://github.com/mgifford/ACCESSIBILITY.md) (MIT).

| Field | Value |
|---|---|
| Target | WCAG 2.2 Level AA, for any page that presents this output; this repository writes text only, no HTML or SVG |
| Status | Evaluation in progress. No conformance claim. |
| Owner | the maintainers of this repository |
| Report a barrier | [a GitHub issue](https://github.com/ikigai-rs/ikigai-text/issues/new?labels=accessibility) with the `accessibility` label; anything security-sensitive through [private vulnerability reporting](https://github.com/ikigai-rs/ikigai-text/security/advisories/new) |
| Last reviewed | 2026-11-12 |

## What it emits that people read

All of it is in `src/lib.rs`.

- **The tools' answers** (`text/plain; charset=utf-8`): `wc`'s count as a number, the lines
  `head`, `tail`, `grep`, `sort`, `uniq` and `rev` keep or reorder, `uniq c=true`'s count before
  each line, and `nl`'s line numbers (right-aligned in six columns, then a tab). Lines are joined
  with `\n` and no trailing newline.
- **Refusals**: an argument that is not what it should be (``expected `true` or `false`, got
  "yes"``, `expected a non-negative integer, got "x"`, `expected one of lines|words|bytes, got
  …`), input that is not UTF-8 (`not valid UTF-8 (at byte N)`), an answer past the run's byte
  budget, and two `tail` errors when a seekable input stops behaving as one.
- **The tools' self-descriptions**: each endpoint's summary and its arguments' summaries, which a
  host shows in its catalog, help or tool list.

Not this repository's: the words around an error (`Error`'s `Display`, and a missing required
argument such as `grep`'s `pattern`, from `ikigai-core`), the text the tools are handed, and any
page, terminal or tool that shows the answer. This repository has no binary, so it writes nothing
to a terminal.

## The rules that output follows

- A refusal of an argument names the argument and quotes the value it got, and says what it
  expected.
- Invalid UTF-8 in consumed input is refused, never replaced. The streamed decoder names
  the byte where it starts. Streamed `head` can stop before invalid bytes after its selected
  lines; seekable `tail` can skip invalid bytes before its selected lines. The inline path
  validates the whole input.
- An answer past the byte budget ends with an error that says how much was held, how much was
  left, and which grant raises it; it never ends as a shorter answer that looks complete.
- A mistyped boolean flag is refused rather than read as off.
- The tools add no color, terminal control sequences or markup. Input text can contain
  those characters and the tools can preserve them; a host controls their presentation.

## How it is tested

Every named check has a result: Pass, Fail or Blocked, with evidence, following the
[reporting format adopted in gonk-starter PR 91](https://github.com/ikigai-rs/ikigai-gonk-starter/pull/91).
A Pass covers only the stated check, not overall accessibility or WCAG conformance.

The automated checks below were run with `cargo test` on 2026-11-12, before this documentation
change, on main commit
[`8824a5b230eeec98225fa8eea029f01e5621a6dc`](https://github.com/ikigai-rs/ikigai-text/commit/8824a5b230eeec98225fa8eea029f01e5621a6dc),
using cargo 1.99.0 (5f94df478 2026-08-27), rustc 1.99.0 (b940084d7 2026-09-28),
and resolved dependency `ikigai-core` 0.3.5. CI repeats the Rust tests through
[`.github/workflows/ci.yml`](.github/workflows/ci.yml); it does not evaluate a host's presentation.
A historical run that CI does not repeat must be recorded as Blocked with its date until rerun.

| Check | Result | Evidence |
|---|---|---|
| Invalid consumed UTF-8 is refused | Pass | `cargo test`: `tests/streaming.rs::invalid_utf8_is_refused_never_replaced` checks invalid `in` across chunk boundaries and an inline case. It checks refusal, not the byte-offset wording. |
| Optional arguments with invalid UTF-8 are refused | Pass | `cargo test`: `tests/streaming.rs::non_utf8_options_are_refused_instead_of_defaulted` checks every optional argument through inline and streamed input and asserts the argument name. |
| Invalid line counts and boolean flags are refused | Pass | `cargo test`: `src/lib.rs` tests `head_rejects_a_non_integer_n`, `head_and_tail_enforce_nonnegative_counts_without_preallocating`, `grep_rejects_a_non_boolean_flag`; `tests/streaming.rs::a_bad_argument_is_refused_before_the_input_is_read`. Checks rejection and selected argument names, not exact wording. |
| Invalid count choice is refused | Pass | `cargo test`: `src/lib.rs::tests::wc_rejects_an_unknown_count` checks the invalid `count` argument. |
| Over-budget output fails instead of ending as a complete shorter answer | Pass | `cargo test`: `tests/streaming.rs` tests `a_line_longer_than_the_budget_is_refused_not_truncated`, `tail_holding_more_than_the_budget_is_refused`, `line_tools_stream_an_input_far_larger_than_the_budget` (including collecting `sort`), and `seekable_tail_enforces_the_retained_lines_budget`. Checks `OverBudget`, not its wording. |
| Selected-line UTF-8 limits | Pass | `cargo test`: `tests/conformance.rs::conforms` exercises the declared `head` exception; `tests/streaming.rs::seekable_tail_validates_only_selected_lines` checks the seekable `tail` exception. Neither promises validation of skipped bytes. |
| Plain-text media type | Pass | `cargo test`: `src/lib.rs::tests::output_is_text_plain_utf8` checks `text/plain; charset=utf-8` on a `head` answer; it does not check rendering or sanitize input. |
| Endpoint and argument descriptions | Pass | `cargo test`: the description tests in `src/lib.rs` check summaries and argument declarations, including `every_endpoint_answers_source_and_meta_with_a_summary`. They do not establish readability in a host's catalog. |
| Refusal wording, byte offsets and budget recovery instructions | Blocked | Existing automated assertions above do not comprehensively check wording, offsets or recovery instructions. No complete wording evaluation was performed for this review. |
| Numeric sort treats nonnumeric lines as zero | Pass | `cargo test`: `src/lib.rs::tests::sort_numeric_treats_non_numeric_as_zero` checks the result. It does not evaluate whether a person understands the behavior. |
| Seekable-tail errors explain recovery | Fail | Source inspection on 2026-11-12 at the main commit above: `seek_tail` in `src/lib.rs` emits `seekable tail input ended before its stated length` and `tail input stopped supporting seeking`. Neither message tells the person what to do. `tests/streaming.rs::seekable_tail_propagates_errors_and_refuses_short_reads` checks the error kind only. This is a message limitation, not a WCAG determination. |
| Automated accessibility checker | Blocked | No axe or other automated accessibility checker was run. This library has no browser interface; no host presentation was configured for evaluation. Rust tests are not an accessibility checker. |
| Browser rendering, contrast, zoom and reflow | Blocked | No browser presentation was evaluated. Rendering depends on the host; no host/browser session was available for this review. |
| Manual keyboard | Blocked | No person performed a keyboard pass on a host presenting this output; no interactive host session was available for this review. |
| Screen reader | Blocked | No screen reader was used on this output; no screen reader session was available for this review. |
| Screen magnifier | Blocked | No screen magnifier was used on this output; no magnifier session was available for this review. |
| Speech input | Blocked | No speech-input software was used on this output; no speech-input session was available for this review. |
| Switch access | Blocked | No switch-access software was used on this output; no switch-access session was available for this review. |
| Braille display | Blocked | No braille display was used on this output; no braille display was available for this review. |

## Known limitations

- `sort n=true` sorts a line that is not a number as 0, without a per-line warning. The tool's
  summary describes this behavior. The numeric-sort row above records the functional check
  and its limits.
- The two seekable-tail errors lack recovery instructions, as recorded in the Fail row above.
- Input control characters can reach the host, and streamed tools may validate only selected
  lines. Plain-text output alone does not establish accessible presentation.

## Contributing

Read this file before changing any output listed above: a tool's answer, a refusal's words or a
description. Keep meaning in words, name the argument a refusal is about, and keep a refusal
from ever looking like a shorter answer. In the pull request's accessibility section, give
each check a row in a Check | Result | Evidence table, using Pass, Fail or Blocked. Include
the test or command, tool and version, or why it could not run, and state the check's limits.
Keep unrun checks as Blocked, including assistive technologies nobody used.

## Reporting a barrier

[Open an issue](https://github.com/ikigai-rs/ikigai-text/issues/new?labels=accessibility) with
the `accessibility` label (the link sets it when GitHub lets you add labels; otherwise a
maintainer adds it): what you were doing, what happened, and any assistive technology you use,
if you are comfortable sharing it. Anything security-sensitive goes through
[private vulnerability reporting](https://github.com/ikigai-rs/ikigai-text/security/advisories/new).
**We acknowledge a report within 7 days**, and promise nothing further.

## Change history

| Date | Change |
|---|---|
| 2026-10-31 | First version. |
| 2026-11-11 | Optional arguments with invalid UTF-8 are refused instead of silently defaulted; tested through the kernel with inline and streamed input. |
| 2026-11-12 | Record current main checks as Pass, Fail or Blocked, identify unrun presentation and assistive-technology checks, and clarify output limits. |
