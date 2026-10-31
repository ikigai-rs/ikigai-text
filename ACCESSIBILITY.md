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
| Last reviewed | 2026-10-31 |

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
- Input is refused, never altered: invalid UTF-8 is not replaced, and the refusal names the byte
  where it starts.
- An answer past the byte budget ends with an error that says how much was held, how much was
  left, and which grant raises it; it never ends as a shorter answer that looks complete.
- A mistyped boolean flag is refused rather than read as off.
- No color, no terminal control codes and no markup are written.

## How it is tested

- `tests/streaming.rs`: `invalid_utf8_is_refused_never_replaced` (refused as an invalid `in`),
  `a_line_longer_than_the_budget_is_refused_not_truncated`,
  `tail_holding_more_than_the_budget_is_refused` (each ends with `OverBudget`, not a clean
  end), `line_tools_stream_an_input_far_larger_than_the_budget` (a `sort` past the budget is
  refused), and `a_bad_argument_is_refused_before_the_input_is_read`.
- `src/lib.rs`: `head_rejects_a_non_integer_n` and `grep_rejects_a_non_boolean_flag`.

These pin the kind of refusal and the argument it names, not the wording. **No automated
accessibility checker, browser or assistive technology has been used on this output.**

## Known limitations

Found by reading the source on 2026-10-31, not by testing:

- `sort n=true` sorts a line that is not a number as 0, with no note that it did.
- A flag or count whose value cannot be read as inline UTF-8 text is treated as absent (off, or
  the default), with no refusal; only a readable value that is wrong is refused.
- The two `tail` errors (`seekable tail input ended before its stated length`, `tail input
  stopped supporting seeking`) name no input and do not say what to do.

## Contributing

Read this file before changing any output listed above: a tool's answer, a refusal's words or a
description. Keep meaning in words, name the argument a refusal is about, and keep a refusal
from ever looking like a shorter answer. Say in the pull request what was tested, with what, and
what was not.

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
