//! `ikigai-text` — Unix-like text endpoints as pure pipeline citizens.
//!
//! A standalone **ikigai module crate** (like `ikigai-fn`): a host links it in
//! and mounts [`space`], rather than the engine shipping the behavior itself.
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
//! ## Streaming (ledger #2275)
//!
//! Each tool answers two ways. The bytes path ([`Endpoint::invoke`]) reads an
//! inline `in` whole, as it always has. The body path
//! ([`Endpoint::invoke_body`], what the kernel calls) reads `in` with
//! [`Invocation::content_body`], so `in` may also be a reference or a content id,
//! and a streamed input is answered as a stream, line by line, never held whole:
//!
//! - `head` stops after `n` lines and drops its producer, so it never reads the
//!   rest of the input;
//! - `wc`, `grep`, `nl`, `rev` and `uniq` hold one line at a time;
//! - `tail` holds at most `n` lines;
//! - `sort` needs every line, so it collects the input under the run's byte
//!   budget ([`Invocation::collect`]).
//!
//! The input is decoded incrementally with std's UTF-8 validator, one line at a
//! time (a line split across chunks is joined first, and `\n` is never inside a
//! multi-byte sequence). Invalid UTF-8 is refused (`InvalidArgument` on `in`),
//! never replaced. A line, or `tail`'s held lines, longer than what is left of
//! the run's byte budget (`urn:cap:stream:memory:<bytes>`) ends the answer with
//! [`Error::OverBudget`]; a failing input ends it with the input's error. A
//! streamed answer never ends cleanly short.
//!
//! A one-shot input gives a one-shot answer (never stored by the kernel); an
//! input that can be read again ([`Body::Replay`]) gives an answer that can be
//! read again, each reader re-reading the input — except that over a sized
//! input of at most [`COLLECT_BOUND`] bytes the answer is collected to bytes
//! (ledger #2530), so the kernel's cache admits it. A streamed answer is not
//! cached: its length is not known, and the cache's byte bound cannot admit it
//! (`urn:kernel:uncached` says `policy`). Collected, the body answer
//! equals the bytes answer for the same input. Because `head` stops reading, it
//! answers over an input whose invalid UTF-8 lies past its `n` lines, as Unix
//! `head` does; the bytes path, which is handed the input whole, refuses it.
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

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::stream::{self, StreamExt};
use ikigai_core::{
    ArgRef, ArgSpec, Body, ByteStream, Bytes, Description, Endpoint, Error, Exact, FailureClass,
    Invocation, Replay, ReprType, Representation, Result, Verb,
};

/// The conventional `text/plain; charset=utf-8` representation type.
fn text_plain_utf8() -> ReprType {
    ReprType::new("text/plain").with_param("charset", "utf-8")
}

/// The same media type as a description-output string.
const TEXT_PLAIN_UTF8: &str = "text/plain;charset=utf-8";

/// The XSD `string` datatype IRI — the `class` of the piped input and of every
/// free-text or enum-token argument.
/// The largest input, in bytes, whose answer a tool collects rather than
/// streams (ledger #2530): 1 MiB. When `in` can be read again
/// ([`Body::Replay`]) and says its length ([`Replay::len_hint`]) is at most
/// this, the answer is collected to bytes, under the run's byte budget, so the
/// kernel's cache admits it. A larger input, or one whose length is not known,
/// is answered as a stream, which the cache does not store
/// (`urn:kernel:uncached` says `policy`).
pub const COLLECT_BOUND: u64 = 1 << 20;

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
        .reads_body()
        .summary(
            "the input text (piped; inline, by reference, or by content id); treated as \
             newline-separated lines",
        )
        .class(XSD_STRING)
}

/// `in` that is not valid UTF-8, refused (never replaced), naming the byte offset.
fn not_utf8(at: u64) -> Error {
    Error::InvalidArgument {
        name: "in".to_string(),
        detail: format!("not valid UTF-8 (at byte {at})"),
    }
}

/// More held than the run's byte budget had left when the answer began.
fn over_budget(what: &str, held: usize, limit: u64) -> Error {
    Error::OverBudget {
        detail: format!(
            "{what} holds at least {held} byte(s), more than the {limit} byte(s) left of the \
             run's byte budget (a `urn:cap:stream:memory:<bytes>` grant raises it, up to the \
             host's ceiling)"
        ),
    }
}

// --- the line tools, one line at a time ------------------------------------

/// What `wc` counts.
#[derive(Clone, Copy)]
enum Count {
    Lines,
    Words,
    Bytes,
}

/// A line tool with its arguments read: everything but `sort`, which needs the
/// whole input. Both paths run it through one [`Pass`], so they answer alike.
#[derive(Clone)]
enum LineOp {
    Wc(Count),
    Head(usize),
    Tail(usize),
    Grep {
        pattern: String,
        /// The needle, lower-cased once, when matching case-insensitively.
        lowered: Option<String>,
        invert: bool,
    },
    Uniq {
        counts: bool,
    },
    Nl,
    Rev,
}

/// One pass of a [`LineOp`] over the input's lines: fed a line at a time, it
/// appends its output to `out` (newline-joined, no trailing newline).
struct Pass {
    op: LineOp,
    /// The most bytes the pass may hold (`tail`'s lines).
    limit: u64,
    /// Lines (or words) counted, lines kept by `head`, or `nl`'s last number.
    seen: u64,
    /// `tail`'s last lines, and their total length.
    kept: VecDeque<String>,
    kept_bytes: usize,
    /// `uniq`'s current run of equal lines.
    run: Option<(String, u64)>,
    /// Output not yet handed out.
    out: String,
    /// Whether anything has been written, so the next line needs a separator.
    started: bool,
}

impl Pass {
    fn new(op: LineOp, limit: u64) -> Self {
        Pass {
            op,
            limit,
            seen: 0,
            kept: VecDeque::new(),
            kept_bytes: 0,
            run: None,
            out: String::new(),
            started: false,
        }
    }

    /// Whether another line could change the answer: `false` once `head` has
    /// its `n` lines, so the reader stops and drops its producer.
    fn wants_more(&self) -> bool {
        match self.op {
            LineOp::Head(n) => self.seen < n as u64,
            _ => true,
        }
    }

    /// Write one output line.
    fn emit(&mut self, line: &str) {
        if self.started {
            self.out.push('\n');
        }
        self.started = true;
        self.out.push_str(line);
    }

    /// Feed the next input line.
    fn line(&mut self, line: &str) -> Result<()> {
        match &self.op {
            LineOp::Wc(Count::Lines) => self.seen += 1,
            LineOp::Wc(Count::Words) => self.seen += line.split_whitespace().count() as u64,
            LineOp::Wc(Count::Bytes) => {}
            LineOp::Head(_) => {
                self.seen += 1;
                self.emit(line);
            }
            LineOp::Tail(n) => {
                let n = *n;
                if n == 0 {
                    return Ok(());
                }
                if self.kept.len() == n {
                    if let Some(dropped) = self.kept.pop_front() {
                        self.kept_bytes -= dropped.len();
                    }
                }
                self.kept_bytes += line.len();
                if self.kept_bytes as u64 > self.limit {
                    return Err(over_budget("tail", self.kept_bytes, self.limit));
                }
                self.kept.push_back(line.to_string());
            }
            LineOp::Grep {
                pattern,
                lowered,
                invert,
            } => {
                let hit = match lowered {
                    Some(needle) => line.to_lowercase().contains(needle.as_str()),
                    None => line.contains(pattern.as_str()),
                };
                if hit != *invert {
                    self.emit(line);
                }
            }
            LineOp::Uniq { counts } => {
                let counts = *counts;
                match &mut self.run {
                    Some((prev, n)) if prev == line => *n += 1,
                    _ => {
                        if let Some((prev, n)) = self.run.take() {
                            self.emit(&emit_uniq(&prev, n, counts));
                        }
                        self.run = Some((line.to_string(), 1));
                    }
                }
            }
            LineOp::Nl => {
                self.seen += 1;
                let numbered = format!("{:>6}\t{line}", self.seen);
                self.emit(&numbered);
            }
            LineOp::Rev => {
                let reversed: String = line.chars().rev().collect();
                self.emit(&reversed);
            }
        }
        Ok(())
    }

    /// The input has ended after `total` bytes: write what was held back.
    fn finish(&mut self, total: u64) {
        match self.op {
            LineOp::Wc(Count::Bytes) => self.out = total.to_string(),
            LineOp::Wc(_) => self.out = self.seen.to_string(),
            LineOp::Tail(_) => {
                for line in std::mem::take(&mut self.kept) {
                    self.emit(&line);
                }
                self.kept_bytes = 0;
            }
            LineOp::Uniq { counts } => {
                if let Some((prev, n)) = self.run.take() {
                    self.emit(&emit_uniq(&prev, n, counts));
                }
            }
            _ => {}
        }
    }

    /// The output written since the last take.
    fn take(&mut self) -> String {
        std::mem::take(&mut self.out)
    }

    /// The whole answer over an input already in memory: the bytes path.
    fn over_str(op: LineOp, text: &str) -> Result<String> {
        let mut pass = Pass::new(op, u64::MAX);
        for line in text.lines() {
            if !pass.wants_more() {
                break;
            }
            pass.line(line)?;
        }
        if pass.wants_more() {
            pass.finish(text.len() as u64);
        }
        Ok(pass.take())
    }
}

/// Splits a stream of chunks into lines exactly as [`str::lines`] splits a
/// whole string, validating each line with std's UTF-8 validator.
struct Splitter {
    /// The start of a line whose end has not arrived yet.
    pending: Vec<u8>,
    /// The input offset `pending` starts at, for the error's offset.
    start: u64,
    /// The longest line it may hold.
    limit: u64,
}

impl Splitter {
    fn new(limit: u64) -> Self {
        Splitter {
            pending: Vec::new(),
            start: 0,
            limit,
        }
    }

    /// One line's bytes, decoded.
    fn decode(bytes: &[u8], at: u64) -> Result<&str> {
        std::str::from_utf8(bytes).map_err(|e| not_utf8(at + e.valid_up_to() as u64))
    }

    /// Feed `chunk`, handing each complete line to `pass` while it wants more.
    fn feed(&mut self, mut chunk: &[u8], pass: &mut Pass) -> Result<()> {
        while pass.wants_more() {
            let Some(end) = chunk.iter().position(|&b| b == b'\n') else {
                self.pending.extend_from_slice(chunk);
                if self.pending.len() as u64 > self.limit {
                    return Err(over_budget("a line", self.pending.len(), self.limit));
                }
                return Ok(());
            };
            let (segment, rest) = (&chunk[..end], &chunk[end + 1..]);
            chunk = rest;
            // A line is refused past the limit even when it arrived whole: a
            // tool copies it (`rev`, `tail`, `uniq`), and the answer must not
            // depend on where the producer cut its chunks.
            let len = self.pending.len() + segment.len();
            if len as u64 > self.limit {
                return Err(over_budget("a line", len, self.limit));
            }
            let at = self.start;
            self.start += (segment.len() + self.pending.len() + 1) as u64;
            // `str::lines` drops a `\r` before the `\n`.
            let strip = |line: &[u8]| -> usize { line.len() - usize::from(line.ends_with(b"\r")) };
            if self.pending.is_empty() {
                pass.line(Self::decode(&segment[..strip(segment)], at)?)?;
            } else {
                self.pending.extend_from_slice(segment);
                let line = std::mem::take(&mut self.pending);
                pass.line(Self::decode(&line[..strip(&line)], at)?)?;
                // Keep the allocation for the next long line.
                self.pending = line;
                self.pending.clear();
            }
        }
        Ok(())
    }

    /// The input has ended: the last line, when it had no newline.
    fn finish(&mut self, pass: &mut Pass) -> Result<()> {
        let rest = std::mem::take(&mut self.pending);
        for line in Self::decode(&rest, self.start)?.lines() {
            if !pass.wants_more() {
                break;
            }
            pass.line(line)?;
        }
        Ok(())
    }
}

/// Reads a streamed input for one [`Pass`].
struct Reader {
    /// The producer, dropped as soon as nothing more is wanted from it.
    input: Option<ByteStream>,
    split: Splitter,
    pass: Pass,
    /// Bytes read from the input, for `wc count=bytes`.
    total: u64,
    done: bool,
}

impl Reader {
    /// The next output chunk; `None` once the answer is complete. An error item
    /// is the last item.
    async fn next_chunk(&mut self) -> Option<Result<Bytes>> {
        loop {
            if self.done {
                return None;
            }
            let Some(input) = self.input.as_mut() else {
                self.done = true;
                let out = self.pass.take();
                return (!out.is_empty()).then(|| Ok(Bytes::from(out)));
            };
            if !self.pass.wants_more() {
                // `head` has its lines: drop the producer without reading on.
                self.input = None;
                continue;
            }
            let step = match input.next().await {
                Some(Ok(chunk)) => {
                    self.total += chunk.len() as u64;
                    self.split.feed(&chunk, &mut self.pass)
                }
                Some(Err(error)) => Err(error),
                None => {
                    self.input = None;
                    let ended = self.split.finish(&mut self.pass);
                    if ended.is_ok() && self.pass.wants_more() {
                        self.pass.finish(self.total);
                    }
                    ended
                }
            };
            if let Err(error) = step {
                self.input = None;
                self.done = true;
                return Some(Err(error));
            }
            if !self.pass.wants_more() {
                self.input = None;
            }
            let out = self.pass.take();
            if !out.is_empty() {
                return Some(Ok(Bytes::from(out)));
            }
        }
    }
}

/// The answer of `op` over a streamed `input`, as a stream.
fn lines_stream(input: ByteStream, op: LineOp, limit: u64) -> ByteStream {
    let reader = Reader {
        input: Some(input),
        split: Splitter::new(limit),
        pass: Pass::new(op, limit),
        total: 0,
        done: false,
    };
    Box::pin(stream::unfold(reader, |mut reader| async move {
        let item = reader.next_chunk().await?;
        Some((item, reader))
    }))
}

/// The answer of `op` over an input that can be read again: each reader
/// re-reads the input.
struct LinesReplay {
    input: Arc<dyn Replay>,
    op: LineOp,
    limit: u64,
}

impl Replay for LinesReplay {
    fn open(&self) -> ByteStream {
        lines_stream(self.input.open(), self.op.clone(), self.limit)
    }
}

// --- the endpoints ---------------------------------------------------------

/// Which text tool a [`TextTool`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Wc,
    Head,
    Tail,
    Grep,
    Sort,
    Uniq,
    Nl,
    Rev,
}

/// One `urn:text:*` endpoint (what [`wc`], [`head`], … build): answers the
/// bytes path from an inline `in`, and the body path from `in` in any form,
/// streaming it (see the crate documentation).
pub struct TextTool {
    tool: Tool,
}

impl TextTool {
    /// The tool's arguments read, but for `in`: a [`LineOp`], or `None` for `sort`.
    fn line_op(&self, inv: &Invocation<'_>) -> Result<Option<LineOp>> {
        Ok(Some(match self.tool {
            Tool::Wc => LineOp::Wc(match inv.inline_str("count").unwrap_or("lines") {
                "lines" => Count::Lines,
                "words" => Count::Words,
                "bytes" => Count::Bytes,
                other => {
                    return Err(Error::InvalidArgument {
                        name: "count".to_string(),
                        detail: format!("expected one of lines|words|bytes, got {other:?}"),
                    })
                }
            }),
            Tool::Head => LineOp::Head(count_arg(inv, "n", 10)?),
            Tool::Tail => LineOp::Tail(count_arg(inv, "n", 10)?),
            Tool::Grep => {
                let pattern = inv.inline_str("pattern")?.to_string();
                let case_insensitive = flag(inv, "i")?;
                let invert = flag(inv, "v")?;
                LineOp::Grep {
                    lowered: case_insensitive.then(|| pattern.to_lowercase()),
                    pattern,
                    invert,
                }
            }
            Tool::Uniq => LineOp::Uniq {
                counts: flag(inv, "c")?,
            },
            Tool::Nl => LineOp::Nl,
            Tool::Rev => LineOp::Rev,
            Tool::Sort => return Ok(None),
        }))
    }

    /// The answer over the whole input `text`, once `in` is known valid.
    fn over_str(&self, inv: &Invocation<'_>, text_in: &str) -> Result<Representation> {
        match self.line_op(inv)? {
            Some(op) => Pass::over_str(op, text_in).map(text),
            None => sort_str(inv, text_in),
        }
    }
}

#[async_trait]
impl Endpoint for TextTool {
    /// The bytes path: an inline `in` read whole, as before bodies existed. A
    /// referenced `in` is answered through the body path and collected under
    /// the run's byte budget.
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        if !matches!(inv.request.args.get("in"), None | Some(ArgRef::Inline(_))) {
            let body = self.invoke_body(inv).await?;
            return match inv.memory_budget() {
                Some(budget) => body.collect_within(budget).await,
                None => {
                    body.collect(ikigai_core::DEFAULT_STREAM_MEMORY as usize)
                        .await
                }
            };
        }
        self.over_str(inv, input(inv)?)
    }

    /// The body path: `in` read with [`Invocation::content_body`], a streamed one
    /// answered as a stream (see the crate documentation).
    async fn invoke_body(&self, inv: &Invocation<'_>) -> Result<Representation<Body>> {
        if matches!(inv.request.args.get("in"), None | Some(ArgRef::Inline(_))) {
            return self.invoke(inv).await.map(Representation::into_body);
        }
        let op = self.line_op(inv)?;
        let body = inv.content_body("in").await?;
        let limit = inv
            .memory_budget()
            .map_or(ikigai_core::DEFAULT_STREAM_MEMORY, |budget| {
                budget.remaining()
            });
        let answer = match (op, body) {
            (_, Body::Bytes(bytes)) => {
                let text_in = Splitter::decode(&bytes, 0)?;
                return self.over_str(inv, text_in).map(Representation::into_body);
            }
            (None, body) => {
                let bytes = inv.collect(body).await?;
                let text_in = Splitter::decode(&bytes, 0)?;
                return sort_str(inv, text_in).map(Representation::into_body);
            }
            (Some(op), Body::Replay(input))
                if input.len_hint().is_some_and(|len| len <= COLLECT_BOUND) =>
            {
                // Small and sized: collect the streamed answer, so it is cached.
                let answer = Body::replay(LinesReplay { input, op, limit });
                Body::Bytes(inv.collect(answer).await?)
            }
            (Some(op), Body::Replay(input)) => Body::replay(LinesReplay { input, op, limit }),
            (Some(op), Body::Once(input)) => Body::Once(lines_stream(input, op, limit)),
        };
        Ok(Representation::from_body(text_plain_utf8(), answer).cacheable())
    }

    fn name(&self) -> &str {
        match self.tool {
            Tool::Wc => "wc",
            Tool::Head => "head",
            Tool::Tail => "tail",
            Tool::Grep => "grep",
            Tool::Sort => "sort",
            Tool::Uniq => "uniq",
            Tool::Nl => "nl",
            Tool::Rev => "rev",
        }
    }

    fn describe(&self) -> Description {
        let description = match self.tool {
            Tool::Wc => wc_description(),
            Tool::Head => head_description(),
            Tool::Tail => tail_description(),
            Tool::Grep => grep_description(),
            Tool::Sort => sort_description(),
            Tool::Uniq => uniq_description(),
            Tool::Nl => nl_description(),
            Tool::Rev => rev_description(),
        };
        // A streamed `in` past the run's byte budget (a line too long, `tail`'s
        // lines, `sort`'s whole input) is refused, never truncated.
        description.fails(FailureClass::OverBudget)
    }
}

// --- wc --------------------------------------------------------------------

fn wc_description() -> Description {
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
        .output(TEXT_PLAIN_UTF8)
}

/// `wc`: counts lines (default), words, or bytes of the input — the single
/// count, as a number. Lines are logical lines ([`str::lines`]); words are
/// whitespace-separated runs; bytes is the UTF-8 byte length. Streamed, it
/// holds one line at a time.
pub fn wc() -> TextTool {
    TextTool { tool: Tool::Wc }
}

// --- head ------------------------------------------------------------------

fn head_description() -> Description {
    Description::new("head")
        .title("First lines")
        .summary(
            "Returns the first `n` lines of the input (default 10). Streamed, it stops \
             reading after them.",
        )
        .verb(Verb::Source)
        .verb(Verb::Meta)
        .input(in_lines())
        .input(
            ArgSpec::new("n")
                .summary("how many leading lines to keep")
                .class(XSD_INTEGER)
                .default_value("10"),
        )
        .output(TEXT_PLAIN_UTF8)
}

/// `head`: the first `n` lines of the input (default 10). Streamed, it drops
/// its producer once it has them.
pub fn head() -> TextTool {
    TextTool { tool: Tool::Head }
}

// --- tail ------------------------------------------------------------------

fn tail_description() -> Description {
    Description::new("tail")
        .title("Last lines")
        .summary(
            "Returns the last `n` lines of the input (default 10). Streamed, it holds at \
             most `n` lines.",
        )
        .verb(Verb::Source)
        .verb(Verb::Meta)
        .input(in_lines())
        .input(
            ArgSpec::new("n")
                .summary("how many trailing lines to keep")
                .class(XSD_INTEGER)
                .default_value("10"),
        )
        .output(TEXT_PLAIN_UTF8)
}

/// `tail`: the last `n` lines of the input (default 10). Streamed, it holds at
/// most `n` lines, under the run's byte budget.
pub fn tail() -> TextTool {
    TextTool { tool: Tool::Tail }
}

// --- grep ------------------------------------------------------------------

fn grep_description() -> Description {
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
        .output(TEXT_PLAIN_UTF8)
}

/// `grep`: the lines that contain `pattern` (a **literal substring**, not a
/// regex in v1). `i=true` matches case-insensitively; `v=true` inverts the test
/// (keep the lines that do *not* contain it).
pub fn grep() -> TextTool {
    TextTool { tool: Tool::Grep }
}

// --- sort ------------------------------------------------------------------

/// A line's numeric key for `sort n=true`: the leading value, or 0 when the
/// line does not parse as a number (the coreutils `sort -n` convention).
fn numeric_key(line: &str) -> f64 {
    line.trim().parse::<f64>().unwrap_or(0.0)
}

fn sort_str(inv: &Invocation<'_>, text_in: &str) -> Result<Representation> {
    let numeric = flag(inv, "n")?;
    let reverse = flag(inv, "r")?;
    let unique = flag(inv, "u")?;
    let mut lines: Vec<&str> = text_in.lines().collect();
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

fn sort_description() -> Description {
    Description::new("sort")
        .title("Sort lines")
        .summary(
            "Sorts the input lines lexically (or numerically with `n=true`, where \
             non-numeric lines sort as 0). `r=true` reverses the result; `u=true` drops \
             duplicates globally (dedup after sort). Order of operations: sort, unique, \
             reverse. A streamed input is collected under the run's byte budget.",
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
        .output(TEXT_PLAIN_UTF8)
}

/// `sort`: sorts the input lines. `r=true` reverses, `n=true` compares
/// numerically (non-numeric lines sort as 0), `u=true` drops duplicates. With
/// `u`, dedup runs after the sort, so it is global (Unix `sort -u`); ordering is
/// sort → unique → reverse. It needs every line, so a streamed input is
/// collected under the run's byte budget.
pub fn sort() -> TextTool {
    TextTool { tool: Tool::Sort }
}

// --- uniq ------------------------------------------------------------------

/// One `uniq` output line: the line itself, optionally prefixed by its
/// adjacent-run count and a space.
fn emit_uniq(line: &str, count: u64, with_counts: bool) -> String {
    if with_counts {
        format!("{count} {line}")
    } else {
        line.to_string()
    }
}

fn uniq_description() -> Description {
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
        .output(TEXT_PLAIN_UTF8)
}

/// `uniq`: collapses each run of **adjacent** duplicate lines to one (Unix
/// semantics — sort first for a global dedup). `c=true` prefixes each line with
/// its run count and a space.
pub fn uniq() -> TextTool {
    TextTool { tool: Tool::Uniq }
}

// --- nl --------------------------------------------------------------------

fn nl_description() -> Description {
    Description::new("nl")
        .title("Number lines")
        .summary(
            "Numbers every input line from 1: a right-aligned (6-wide) number, a tab, then \
             the line.",
        )
        .verb(Verb::Source)
        .verb(Verb::Meta)
        .input(in_lines())
        .output(TEXT_PLAIN_UTF8)
}

/// `nl`: numbers every line, from 1 — a right-aligned (6-wide) number, a tab,
/// then the line.
pub fn nl() -> TextTool {
    TextTool { tool: Tool::Nl }
}

// --- rev -------------------------------------------------------------------

fn rev_description() -> Description {
    Description::new("rev")
        .title("Reverse characters")
        .summary(
            "Reverses the characters (Unicode scalars) of each line; the order of lines is \
             unchanged.",
        )
        .verb(Verb::Source)
        .verb(Verb::Meta)
        .input(in_lines())
        .output(TEXT_PLAIN_UTF8)
}

/// `rev`: reverses the characters of each line (by Unicode scalar), line order
/// unchanged.
pub fn rev() -> TextTool {
    TextTool { tool: Tool::Rev }
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
    fn describe(ep: &TextTool) -> Description {
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
