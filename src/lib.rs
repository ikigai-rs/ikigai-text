//! `ikigai-text` — Unix-like text endpoints as pure pipeline citizens.
//!
//! A standalone **ikigai module crate** (like `ikigai-fn`): a host links it in
//! and mounts [`space`], rather than the engine shipping the behaviour itself.
//! It depends only on the published `ikigai-core` kernel, uses no OS or platform
//! APIs, and compiles to `wasm32-unknown-unknown` — so the same module links
//! into a native CLI and an in-browser WebAssembly host alike.
//!
//! The engine already speaks Unix: `|` pipes one resource's representation into
//! the next, and `..` maps an endpoint over a newline-separated list. These
//! `urn:text:*` endpoints make the REPL a coreutils analog:
//!
//! ```text
//! source urn:file:log.txt | urn:text:grep pattern=ERROR | urn:text:wc
//! ```
//!
//! ## Conventions
//!
//! - **Input** is the piped value, declared as the `in` argument (the
//!   `ikigai-fn` convention — the engine routes a piped representation into the
//!   sole unnamed required input). Line-oriented tools treat the input as
//!   newline-separated lines (via [`str::lines`], so a trailing newline and
//!   `\r\n` are handled the Unix way).
//! - **Output** is always `text/plain; charset=utf-8`, and — for line tools — a
//!   newline-joined list with no trailing newline, so the result flows straight
//!   into the next stage or a `..` map.
//! - Every endpoint is a **pure function of its inputs**, so every
//!   representation is [`Representation::cacheable`]: identical input, identical
//!   output, cached under the kernel's capability-fingerprinted key.
//!
//! ## ArgSpec showcase
//!
//! Each endpoint authors a full self-description — an XSD `class` on every
//! input, `one_of` for enums, `default` where applicable — so that each becomes
//! a well-typed, MCP-projectable tool once the manifold is projected. The
//! `describe` tests assert those declarations directly, and
//! `tests/conformance.rs` runs `ikigai-conformance` over the whole space: every
//! endpoint is declared `pure` there (a pure function needs no golden thread)
//! and `cacheable` (so a dependency that silently downgraded the expiry would
//! be a red test).

use ikigai_core::{
    ArgSpec, Description, Error, Exact, FnEndpoint, Invocation, ReprType, Representation, Result,
    Verb,
};

/// The conventional `text/plain; charset=utf-8` representation type.
fn text_plain_utf8() -> ReprType {
    ReprType::new("text/plain").with_param("charset", "utf-8")
}

/// The same media type as a description-output string.
const TEXT_PLAIN_UTF8: &str = "text/plain;charset=utf-8";

/// The XSD `string` datatype IRI — the `class` of the piped input and of every
/// free-text or enum-token argument.
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
/// The XSD `integer` datatype IRI — the `class` of a line-count argument.
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
/// The XSD `boolean` datatype IRI — the `class` of a flag argument.
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";

// --- shared helpers --------------------------------------------------------

/// A cacheable `text/plain; charset=utf-8` representation of `s`.
fn text(s: String) -> Representation {
    Representation::new(text_plain_utf8(), s.into_bytes()).cacheable()
}

/// Read an optional boolean flag: absent → `false`; `"true"`/`"false"`
/// (case-insensitive, trimmed) → the value; anything else → an error, so a
/// mistyped flag can't silently read as off.
fn flag(inv: &Invocation<'_>, name: &str) -> Result<bool> {
    match inv.inline_str(name) {
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            other => Err(Error::InvalidArgument {
                name: name.to_string(),
                detail: format!("expected `true` or `false`, got {other:?}"),
            }),
        },
        // Missing (or non-UTF-8) → treat as unset.
        Err(_) => Ok(false),
    }
}

/// Read an optional non-negative line count: absent → `default`; otherwise the
/// parsed value, erroring on anything that isn't a non-negative integer.
fn count_arg(inv: &Invocation<'_>, name: &str, default: usize) -> Result<usize> {
    match inv.inline_str(name) {
        Ok(raw) => raw
            .trim()
            .parse::<usize>()
            .map_err(|_| Error::InvalidArgument {
                name: name.to_string(),
                detail: format!("expected a non-negative integer, got {:?}", raw.trim()),
            }),
        Err(_) => Ok(default),
    }
}

/// The `in` argument as UTF-8 text — the piped input every text tool reads.
fn input<'a>(inv: &'a Invocation<'_>) -> Result<&'a str> {
    inv.inline_str("in")
}

/// The `in` ArgSpec — the piped, line-oriented input shared by every endpoint.
fn in_lines() -> ArgSpec {
    ArgSpec::new("in")
        .summary("the input text (piped); treated as newline-separated lines")
        .class(XSD_STRING)
}

// --- wc --------------------------------------------------------------------

fn wc_impl(inv: &Invocation<'_>) -> Result<Representation> {
    let text_in = input(inv)?;
    let count = match inv.inline_str("count").unwrap_or("lines") {
        "lines" => text_in.lines().count(),
        "words" => text_in.split_whitespace().count(),
        "bytes" => text_in.len(),
        other => {
            return Err(Error::InvalidArgument {
                name: "count".to_string(),
                detail: format!("expected one of lines|words|bytes, got {other:?}"),
            })
        }
    };
    Ok(text(count.to_string()))
}

/// `wc`: counts lines (default), words, or bytes of the input — the single
/// count, as a number. Lines are logical lines ([`str::lines`]); words are
/// whitespace-separated runs; bytes is the UTF-8 byte length.
pub fn wc() -> FnEndpoint {
    FnEndpoint::new("wc", wc_impl).with_description(
        Description::new("wc")
            .title("Word/line/byte count")
            .summary(
                "Counts the input's lines (default), words, or bytes and returns the single \
                 count as a number. `count=words` counts whitespace-separated runs; \
                 `count=bytes` is the UTF-8 byte length.",
            )
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .input(in_lines())
            .input(
                ArgSpec::new("count")
                    .summary("what to count: lines, words, or bytes")
                    .class(XSD_STRING)
                    .one_of(["lines", "words", "bytes"])
                    .default_value("lines"),
            )
            .output(TEXT_PLAIN_UTF8),
    )
}

// --- head ------------------------------------------------------------------

fn head_impl(inv: &Invocation<'_>) -> Result<Representation> {
    let n = count_arg(inv, "n", 10)?;
    let out = input(inv)?.lines().take(n).collect::<Vec<_>>().join("\n");
    Ok(text(out))
}

/// `head`: the first `n` lines of the input (default 10).
pub fn head() -> FnEndpoint {
    FnEndpoint::new("head", head_impl).with_description(
        Description::new("head")
            .title("First lines")
            .summary("Returns the first `n` lines of the input (default 10).")
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .input(in_lines())
            .input(
                ArgSpec::new("n")
                    .summary("how many leading lines to keep")
                    .class(XSD_INTEGER)
                    .default_value("10"),
            )
            .output(TEXT_PLAIN_UTF8),
    )
}

// --- tail ------------------------------------------------------------------

fn tail_impl(inv: &Invocation<'_>) -> Result<Representation> {
    let n = count_arg(inv, "n", 10)?;
    let all: Vec<&str> = input(inv)?.lines().collect();
    let start = all.len().saturating_sub(n);
    Ok(text(all[start..].join("\n")))
}

/// `tail`: the last `n` lines of the input (default 10).
pub fn tail() -> FnEndpoint {
    FnEndpoint::new("tail", tail_impl).with_description(
        Description::new("tail")
            .title("Last lines")
            .summary("Returns the last `n` lines of the input (default 10).")
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .input(in_lines())
            .input(
                ArgSpec::new("n")
                    .summary("how many trailing lines to keep")
                    .class(XSD_INTEGER)
                    .default_value("10"),
            )
            .output(TEXT_PLAIN_UTF8),
    )
}

// --- grep ------------------------------------------------------------------

fn grep_impl(inv: &Invocation<'_>) -> Result<Representation> {
    let text_in = input(inv)?;
    let pattern = inv.inline_str("pattern")?;
    let case_insensitive = flag(inv, "i")?;
    let invert = flag(inv, "v")?;
    // Lower-case the needle once when case-insensitive; otherwise match directly.
    let lowered = case_insensitive.then(|| pattern.to_lowercase());
    let out = text_in
        .lines()
        .filter(|&line| {
            let hit = match &lowered {
                Some(needle) => line.to_lowercase().contains(needle.as_str()),
                None => line.contains(pattern),
            };
            hit != invert
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(text(out))
}

/// `grep`: the lines that contain `pattern` (a **literal substring**, not a
/// regex in v1). `i=true` matches case-insensitively; `v=true` inverts the test
/// (keep the lines that do *not* contain it).
pub fn grep() -> FnEndpoint {
    FnEndpoint::new("grep", grep_impl).with_description(
        Description::new("grep")
            .title("Filter lines by substring")
            .summary(
                "Keeps the input lines that contain `pattern` (a literal substring, not a regex). \
                 `i=true` matches case-insensitively; `v=true` inverts the match (keep \
                 non-matching lines).",
            )
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .input(in_lines())
            .input(
                ArgSpec::new("pattern")
                    .summary("the literal substring to search for")
                    .class(XSD_STRING),
            )
            .input(
                ArgSpec::new("i")
                    .summary("match case-insensitively")
                    .class(XSD_BOOLEAN)
                    .one_of(["true", "false"])
                    .optional(),
            )
            .input(
                ArgSpec::new("v")
                    .summary("invert the match — keep lines NOT containing the pattern")
                    .class(XSD_BOOLEAN)
                    .one_of(["true", "false"])
                    .optional(),
            )
            .output(TEXT_PLAIN_UTF8),
    )
}

// --- sort ------------------------------------------------------------------

/// A line's numeric key for `sort n=true`: the leading value, or 0 when the
/// line does not parse as a number (the coreutils `sort -n` convention).
fn numeric_key(line: &str) -> f64 {
    line.trim().parse::<f64>().unwrap_or(0.0)
}

fn sort_impl(inv: &Invocation<'_>) -> Result<Representation> {
    let numeric = flag(inv, "n")?;
    let reverse = flag(inv, "r")?;
    let unique = flag(inv, "u")?;
    let mut lines: Vec<&str> = input(inv)?.lines().collect();
    if numeric {
        lines.sort_by(|a, b| numeric_key(a).total_cmp(&numeric_key(b)));
    } else {
        lines.sort_unstable();
    }
    if unique {
        // Sorted, so adjacent-dedup is a global dedup.
        lines.dedup();
    }
    if reverse {
        lines.reverse();
    }
    Ok(text(lines.join("\n")))
}

/// `sort`: sorts the input lines. `r=true` reverses, `n=true` compares
/// numerically (non-numeric lines sort as 0), `u=true` drops duplicates. With
/// `u`, dedup runs after the sort, so it is global (Unix `sort -u`); ordering is
/// sort → unique → reverse.
pub fn sort() -> FnEndpoint {
    FnEndpoint::new("sort", sort_impl).with_description(
        Description::new("sort")
            .title("Sort lines")
            .summary(
                "Sorts the input lines lexically (or numerically with `n=true`, where \
                 non-numeric lines sort as 0). `r=true` reverses the result; `u=true` drops \
                 duplicates globally (dedup after sort). Order of operations: sort, unique, \
                 reverse.",
            )
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .input(in_lines())
            .input(
                ArgSpec::new("r")
                    .summary("reverse the sort order")
                    .class(XSD_BOOLEAN)
                    .one_of(["true", "false"])
                    .optional(),
            )
            .input(
                ArgSpec::new("n")
                    .summary("compare numerically instead of lexically")
                    .class(XSD_BOOLEAN)
                    .one_of(["true", "false"])
                    .optional(),
            )
            .input(
                ArgSpec::new("u")
                    .summary("drop duplicate lines (global, after sorting)")
                    .class(XSD_BOOLEAN)
                    .one_of(["true", "false"])
                    .optional(),
            )
            .output(TEXT_PLAIN_UTF8),
    )
}

// --- uniq ------------------------------------------------------------------

fn uniq_impl(inv: &Invocation<'_>) -> Result<Representation> {
    let with_counts = flag(inv, "c")?;
    let mut out: Vec<String> = Vec::new();
    let mut run: Option<(&str, usize)> = None;
    for line in input(inv)?.lines() {
        match run {
            Some((prev, n)) if prev == line => run = Some((prev, n + 1)),
            Some((prev, n)) => {
                out.push(emit_uniq(prev, n, with_counts));
                run = Some((line, 1));
            }
            None => run = Some((line, 1)),
        }
    }
    if let Some((prev, n)) = run {
        out.push(emit_uniq(prev, n, with_counts));
    }
    Ok(text(out.join("\n")))
}

/// One `uniq` output line: the line itself, optionally prefixed by its
/// adjacent-run count and a space.
fn emit_uniq(line: &str, count: usize, with_counts: bool) -> String {
    if with_counts {
        format!("{count} {line}")
    } else {
        line.to_string()
    }
}

/// `uniq`: collapses each run of **adjacent** duplicate lines to one (Unix
/// semantics — sort first for a global dedup). `c=true` prefixes each line with
/// its run count and a space.
pub fn uniq() -> FnEndpoint {
    FnEndpoint::new("uniq", uniq_impl).with_description(
        Description::new("uniq")
            .title("Collapse adjacent duplicates")
            .summary(
                "Collapses each run of ADJACENT duplicate lines to a single line (Unix \
                 semantics — pipe through `sort` first for a global dedup). `c=true` prefixes \
                 each line with its run count and a space.",
            )
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .input(in_lines())
            .input(
                ArgSpec::new("c")
                    .summary("prefix each line with its adjacent-run count")
                    .class(XSD_BOOLEAN)
                    .one_of(["true", "false"])
                    .optional(),
            )
            .output(TEXT_PLAIN_UTF8),
    )
}

// --- nl --------------------------------------------------------------------

fn nl_impl(inv: &Invocation<'_>) -> Result<Representation> {
    let out = input(inv)?
        .lines()
        .enumerate()
        .map(|(i, line)| format!("{:>6}\t{line}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(text(out))
}

/// `nl`: numbers every line, from 1 — a right-aligned (6-wide) number, a tab,
/// then the line.
pub fn nl() -> FnEndpoint {
    FnEndpoint::new("nl", nl_impl).with_description(
        Description::new("nl")
            .title("Number lines")
            .summary(
                "Numbers every input line from 1: a right-aligned (6-wide) number, a tab, then \
                 the line.",
            )
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .input(in_lines())
            .output(TEXT_PLAIN_UTF8),
    )
}

// --- rev -------------------------------------------------------------------

fn rev_impl(inv: &Invocation<'_>) -> Result<Representation> {
    let out = input(inv)?
        .lines()
        .map(|line| line.chars().rev().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(text(out))
}

/// `rev`: reverses the characters of each line (by Unicode scalar), line order
/// unchanged.
pub fn rev() -> FnEndpoint {
    FnEndpoint::new("rev", rev_impl).with_description(
        Description::new("rev")
            .title("Reverse characters")
            .summary(
                "Reverses the characters (Unicode scalars) of each line; the order of lines is \
                 unchanged.",
            )
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .input(in_lines())
            .output(TEXT_PLAIN_UTF8),
    )
}

// --- the library as a mountable space --------------------------------------

/// The name [`space`] claims: `urn:iki:space:text`.
pub const SPACE_ID: &str = "urn:iki:space:text";

/// The text-tool library as a mountable [`EndpointSpace`](ikigai_core::EndpointSpace), binding every
/// endpoint at its conventional `urn:text:*` IRI. A host mounts this and chains
/// its own bindings on top (`EndpointSpace::bind` is a builder):
///
/// ```ignore
/// let space = ikigai_text::space()
///     .bind(Exact::new("urn:file:log.txt"), my_file());
/// ```
///
/// Hosts that want different IRIs (binding authority is a host concern) can
/// instead pull the individual constructors and bind them as they like.
///
/// The space is configuration-free, so it names itself [`SPACE_ID`]. A host that
/// binds more doors on top gets an anonymous space (the extended space no longer
/// holds the same doors) and names it itself if it wants a name.
pub fn space() -> ikigai_core::EndpointSpace {
    ikigai_core::EndpointSpace::new()
        .bind(Exact::new("urn:text:wc"), wc())
        .bind(Exact::new("urn:text:head"), head())
        .bind(Exact::new("urn:text:tail"), tail())
        .bind(Exact::new("urn:text:grep"), grep())
        .bind(Exact::new("urn:text:sort"), sort())
        .bind(Exact::new("urn:text:uniq"), uniq())
        .bind(Exact::new("urn:text:nl"), nl())
        .bind(Exact::new("urn:text:rev"), rev())
        .named(ikigai_core::space_iri("text"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    // `Endpoint` brings the `describe` method into scope for the contract tests.
    use ikigai_core::{ArgRef, Capability, Endpoint, Iri, Kernel, Request};
    use std::sync::Arc;

    fn kernel() -> Kernel {
        Kernel::new(Arc::new(space()))
    }

    /// Source `iri` with the given inline args and return the UTF-8 body.
    fn run(iri: &str, args: &[(&str, &str)]) -> String {
        let mut request = Request::new(Verb::Source, Iri::parse(iri).unwrap());
        for (name, value) in args {
            request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
        }
        let rep = block_on(kernel().issue(request, &Capability::root())).unwrap();
        String::from_utf8(rep.bytes).unwrap()
    }

    /// Source `iri`, returning the whole `Result` for error-path assertions.
    fn try_run(iri: &str, args: &[(&str, &str)]) -> Result<Representation> {
        let mut request = Request::new(Verb::Source, Iri::parse(iri).unwrap());
        for (name, value) in args {
            request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
        }
        block_on(kernel().issue(request, &Capability::root()))
    }

    /// The `Description` an endpoint reports through a `Meta` request's routing —
    /// here read straight off the constructor for the declaration assertions.
    fn describe(ep: &FnEndpoint) -> Description {
        ep.describe()
    }

    fn arg<'a>(d: &'a Description, name: &str) -> &'a ArgSpec {
        d.inputs
            .iter()
            .find(|a| a.name == name)
            .unwrap_or_else(|| panic!("no arg `{name}`"))
    }

    // ---- wc ----------------------------------------------------------------

    #[test]
    fn wc_counts_lines_by_default() {
        assert_eq!(run("urn:text:wc", &[("in", "a\nb\nc")]), "3");
    }

    #[test]
    fn wc_counts_words_and_bytes() {
        assert_eq!(
            run("urn:text:wc", &[("in", "a b  c"), ("count", "words")]),
            "3"
        );
        assert_eq!(
            run("urn:text:wc", &[("in", "abc"), ("count", "bytes")]),
            "3"
        );
    }

    #[test]
    fn wc_of_empty_input_is_zero() {
        assert_eq!(run("urn:text:wc", &[("in", "")]), "0");
        assert_eq!(run("urn:text:wc", &[("in", ""), ("count", "words")]), "0");
    }

    #[test]
    fn wc_counts_a_trailing_newline_the_unix_way() {
        // `str::lines()` does not count a trailing empty line.
        assert_eq!(run("urn:text:wc", &[("in", "a\nb\n")]), "2");
    }

    #[test]
    fn wc_rejects_an_unknown_count() {
        assert!(try_run("urn:text:wc", &[("in", "x"), ("count", "chars")]).is_err());
    }

    // ---- head / tail -------------------------------------------------------

    #[test]
    fn head_defaults_to_ten_lines() {
        let input: String = (1..=20)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            run("urn:text:head", &[("in", &input)]),
            "1\n2\n3\n4\n5\n6\n7\n8\n9\n10"
        );
    }

    #[test]
    fn head_takes_n_lines() {
        assert_eq!(
            run("urn:text:head", &[("in", "a\nb\nc\nd"), ("n", "2")]),
            "a\nb"
        );
    }

    #[test]
    fn head_of_fewer_than_n_lines_returns_all() {
        assert_eq!(run("urn:text:head", &[("in", "a\nb"), ("n", "5")]), "a\nb");
    }

    #[test]
    fn tail_takes_the_last_n_lines() {
        assert_eq!(
            run("urn:text:tail", &[("in", "a\nb\nc\nd"), ("n", "2")]),
            "c\nd"
        );
    }

    #[test]
    fn tail_of_fewer_than_n_lines_returns_all() {
        assert_eq!(run("urn:text:tail", &[("in", "a\nb"), ("n", "5")]), "a\nb");
    }

    #[test]
    fn head_rejects_a_non_integer_n() {
        assert!(try_run("urn:text:head", &[("in", "a"), ("n", "two")]).is_err());
    }

    // ---- grep --------------------------------------------------------------

    #[test]
    fn grep_keeps_matching_lines() {
        assert_eq!(
            run(
                "urn:text:grep",
                &[("in", "error: x\nok\nerror: y"), ("pattern", "error")]
            ),
            "error: x\nerror: y"
        );
    }

    #[test]
    fn grep_is_case_sensitive_by_default() {
        assert_eq!(
            run(
                "urn:text:grep",
                &[("in", "Error\nerror"), ("pattern", "error")]
            ),
            "error"
        );
    }

    #[test]
    fn grep_case_insensitive_flag() {
        assert_eq!(
            run(
                "urn:text:grep",
                &[
                    ("in", "Error\nerror\nOK"),
                    ("pattern", "error"),
                    ("i", "true")
                ]
            ),
            "Error\nerror"
        );
    }

    #[test]
    fn grep_invert_keeps_non_matching_lines() {
        assert_eq!(
            run(
                "urn:text:grep",
                &[("in", "a\nbad\nc"), ("pattern", "bad"), ("v", "true")]
            ),
            "a\nc"
        );
    }

    #[test]
    fn grep_no_match_is_empty() {
        assert_eq!(
            run("urn:text:grep", &[("in", "a\nb"), ("pattern", "z")]),
            ""
        );
    }

    #[test]
    fn grep_rejects_a_non_boolean_flag() {
        assert!(try_run(
            "urn:text:grep",
            &[("in", "a"), ("pattern", "a"), ("i", "yes")]
        )
        .is_err());
    }

    // ---- sort --------------------------------------------------------------

    #[test]
    fn sort_orders_lines_lexically() {
        assert_eq!(
            run("urn:text:sort", &[("in", "banana\napple\ncherry")]),
            "apple\nbanana\ncherry"
        );
    }

    #[test]
    fn sort_reverse() {
        assert_eq!(
            run("urn:text:sort", &[("in", "a\nc\nb"), ("r", "true")]),
            "c\nb\na"
        );
    }

    #[test]
    fn sort_numeric_beats_lexical() {
        // Lexically "10" < "9"; numerically 9 < 10.
        assert_eq!(
            run("urn:text:sort", &[("in", "10\n9\n100\n2")]),
            "10\n100\n2\n9"
        );
        assert_eq!(
            run("urn:text:sort", &[("in", "10\n9\n100\n2"), ("n", "true")]),
            "2\n9\n10\n100"
        );
    }

    #[test]
    fn sort_numeric_treats_non_numeric_as_zero() {
        // "foo" -> 0, so it sorts alongside "0" ahead of the positive values.
        assert_eq!(
            run("urn:text:sort", &[("in", "3\nfoo\n1"), ("n", "true")]),
            "foo\n1\n3"
        );
    }

    #[test]
    fn sort_unique_is_global_after_sorting() {
        assert_eq!(
            run("urn:text:sort", &[("in", "b\na\nb\na\nc"), ("u", "true")]),
            "a\nb\nc"
        );
    }

    #[test]
    fn sort_unique_then_reverse() {
        assert_eq!(
            run(
                "urn:text:sort",
                &[("in", "b\na\nb\nc\na"), ("u", "true"), ("r", "true")]
            ),
            "c\nb\na"
        );
    }

    // ---- uniq --------------------------------------------------------------

    #[test]
    fn uniq_collapses_adjacent_only() {
        // The non-adjacent "a" at the end is NOT collapsed — Unix semantics.
        assert_eq!(run("urn:text:uniq", &[("in", "a\na\nb\na")]), "a\nb\na");
    }

    #[test]
    fn uniq_with_counts() {
        assert_eq!(
            run("urn:text:uniq", &[("in", "a\na\na\nb\nb"), ("c", "true")]),
            "3 a\n2 b"
        );
    }

    #[test]
    fn uniq_of_empty_input_is_empty() {
        assert_eq!(run("urn:text:uniq", &[("in", "")]), "");
    }

    // ---- nl / rev ----------------------------------------------------------

    #[test]
    fn nl_numbers_every_line() {
        assert_eq!(
            run("urn:text:nl", &[("in", "a\nb")]),
            "     1\ta\n     2\tb"
        );
    }

    #[test]
    fn rev_reverses_each_line() {
        assert_eq!(run("urn:text:rev", &[("in", "abc\nhello")]), "cba\nolleh");
    }

    #[test]
    fn rev_reverses_by_unicode_scalar() {
        assert_eq!(run("urn:text:rev", &[("in", "áé")]), "éá");
    }

    // ---- pipeline citizenship: cacheable & mountable -----------------------

    #[test]
    fn representations_are_cacheable() {
        use ikigai_core::Expiry;
        let request = Request::new(Verb::Source, Iri::parse("urn:text:wc").unwrap())
            .with_arg("in", ArgRef::Inline(b"a\nb".to_vec()));
        let rep = block_on(kernel().issue(request, &Capability::root())).unwrap();
        assert_eq!(
            rep.expiry,
            Expiry::Never,
            "a pure text function is cacheable"
        );
    }

    #[test]
    fn output_is_text_plain_utf8() {
        let request = Request::new(Verb::Source, Iri::parse("urn:text:head").unwrap())
            .with_arg("in", ArgRef::Inline(b"a".to_vec()));
        let rep = block_on(kernel().issue(request, &Capability::root())).unwrap();
        assert_eq!(rep.repr_type.media_type, "text/plain");
        assert_eq!(
            rep.repr_type.params.get("charset").map(String::as_str),
            Some("utf-8")
        );
    }

    #[test]
    fn space_binds_all_eight_endpoints() {
        for iri in [
            "urn:text:wc",
            "urn:text:head",
            "urn:text:tail",
            "urn:text:grep",
            "urn:text:sort",
            "urn:text:uniq",
            "urn:text:nl",
            "urn:text:rev",
        ] {
            let request = Request::new(Verb::Source, Iri::parse(iri).unwrap())
                .with_arg("in", ArgRef::Inline(b"x".to_vec()))
                .with_arg("pattern", ArgRef::Inline(b"x".to_vec()));
            assert!(
                block_on(kernel().issue(request, &Capability::root())).is_ok(),
                "{iri} should resolve"
            );
        }
    }

    // ---- describe: the ArgSpec contract ------------------------------------

    #[test]
    fn wc_describes_its_count_enum_and_default() {
        let d = describe(&wc());
        let count = arg(&d, "count");
        assert_eq!(count.class.as_deref(), Some(XSD_STRING));
        assert_eq!(count.one_of, vec!["lines", "words", "bytes"]);
        assert_eq!(count.default.as_deref(), Some("lines"));
        assert!(!count.required, "an argument with a default is optional");
        // Every ArgSpec carries a summary.
        assert!(!count.summary.is_empty());
        assert!(!arg(&d, "in").summary.is_empty());
    }

    #[test]
    fn head_and_tail_declare_an_integer_n_defaulting_to_ten() {
        for ep in [head(), tail()] {
            let d = describe(&ep);
            let n = arg(&d, "n");
            assert_eq!(n.class.as_deref(), Some(XSD_INTEGER));
            assert_eq!(n.default.as_deref(), Some("10"));
            assert!(!n.required);
        }
    }

    #[test]
    fn grep_declares_a_required_pattern_and_boolean_flags() {
        let d = describe(&grep());
        assert!(arg(&d, "pattern").required, "pattern is required");
        assert_eq!(arg(&d, "pattern").class.as_deref(), Some(XSD_STRING));
        for name in ["i", "v"] {
            let f = arg(&d, name);
            assert_eq!(f.class.as_deref(), Some(XSD_BOOLEAN));
            assert_eq!(f.one_of, vec!["true", "false"]);
            assert!(!f.required, "flags are optional");
        }
    }

    #[test]
    fn sort_declares_three_boolean_flags() {
        let d = describe(&sort());
        for name in ["r", "n", "u"] {
            let f = arg(&d, name);
            assert_eq!(
                f.class.as_deref(),
                Some(XSD_BOOLEAN),
                "{name} is xsd:boolean"
            );
            assert_eq!(f.one_of, vec!["true", "false"]);
            assert!(!f.required);
        }
    }

    #[test]
    fn uniq_declares_a_boolean_count_flag() {
        let d = describe(&uniq());
        let c = arg(&d, "c");
        assert_eq!(c.class.as_deref(), Some(XSD_BOOLEAN));
        assert_eq!(c.one_of, vec!["true", "false"]);
        assert!(!c.required);
    }

    #[test]
    fn every_endpoint_answers_source_and_meta_with_a_summary() {
        for ep in [wc(), head(), tail(), grep(), sort(), uniq(), nl(), rev()] {
            let d = describe(&ep);
            assert!(d.verbs.contains(&Verb::Source), "{} sources", d.id);
            assert!(d.verbs.contains(&Verb::Meta), "{} self-describes", d.id);
            assert!(!d.summary.is_empty(), "{} has a summary", d.id);
            // The piped input is typed text on every endpoint.
            assert_eq!(
                arg(&d, "in").class.as_deref(),
                Some(XSD_STRING),
                "{} types its piped input",
                d.id
            );
            // Every input carries a class — the ARGSPECS rule the conformance
            // suite mechanizes, pinned here per endpoint as well.
            for input in &d.inputs {
                assert!(input.class.is_some(), "{}.{} has a class", d.id, input.name);
            }
            assert_eq!(
                d.outputs,
                vec![TEXT_PLAIN_UTF8.to_string()],
                "{} outputs plain text",
                d.id
            );
        }
    }
}
