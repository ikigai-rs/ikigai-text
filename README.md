# ikigai-text

Unix-like **text endpoints** for [ikigai](https://github.com/ikigai-rs) — a
module crate of `urn:text:*` resources that turn the REPL into a coreutils
analog.

The engine already speaks Unix: `|` pipes one resource's representation into the
next, and `..` maps an endpoint over a newline-separated list. These endpoints
are the coreutils to match:

```text
source urn:file:log.txt | urn:text:grep pattern=ERROR | urn:text:wc
```

Like `ikigai-fn`, this is a standalone **module crate**: a host links it in and
mounts [`space`], rather than the engine shipping the behavior itself. It
depends only on the published `ikigai-core` kernel, uses no OS or platform APIs,
and compiles to `wasm32-unknown-unknown` — so the same module links into a
native CLI and an in-browser WebAssembly host alike.

## Endpoints

Every endpoint is a **single-verb `Source`**, reads its piped input from the
`in` argument, produces `text/plain; charset=utf-8`, and — being a pure function
of its inputs — is **cacheable**. Line tools split the input into lines the Unix
way (`str::lines`: a trailing newline and `\r\n` are handled), and join their
output with `\n` and no trailing newline, so a result flows straight into the
next stage or a `..` map.

| IRI | args | behavior |
|-----|------|-----------|
| `urn:text:wc` | `count=` `lines`\|`words`\|`bytes` (default `lines`) | the single count, as a number |
| `urn:text:head` | `n=` integer (default `10`) | the first `n` lines |
| `urn:text:tail` | `n=` integer (default `10`) | the last `n` lines |
| `urn:text:grep` | `pattern=` (required); `i=` bool; `v=` bool | lines containing `pattern` (literal substring, **not** regex); `i` = case-insensitive, `v` = invert |
| `urn:text:sort` | `r=` bool; `n=` bool; `u=` bool | sorted lines; `r` reverse, `n` numeric (non-numeric → 0), `u` unique (global) |
| `urn:text:uniq` | `c=` bool | collapse **adjacent** duplicate lines; `c` prefixes run counts |
| `urn:text:nl` | — | number every line |
| `urn:text:rev` | — | reverse each line's characters |

### Streaming

The kernel calls each tool's body path, which reads `in` with
`Invocation::content_body`: inline, by reference, or by content id. A streamed
input is answered as a stream, line by line, and never held whole:

- `head` stops after `n` lines and drops its producer, so
  `source urn:file:big.log | urn:text:head` never reads the rest of the file;
- `wc`, `grep`, `nl`, `rev` and `uniq` hold one line at a time;
- `tail` holds at most `n` lines;
- `sort` needs every line, so it collects the input under the run's byte budget.

All eight tools declare `reads_body` on their piped `in` argument (the argument
read through `content_body`). Inline bytes, a `b3:` content id, and a live
attached body produce the same text. Engine pipes can therefore stream into
these tools without collecting the input first; `sort` still collects under
the run's byte budget. This requires `ikigai-core` 0.2.3 and changes all eight
tools' contract IRIs: consumers pinned to the previous contracts must update.

UTF-8 is decoded incrementally with std's validator (a line split across chunks
is joined first); invalid UTF-8 is refused, never replaced. A line, `tail`'s
lines or `sort`'s input past the run's byte budget
(`urn:cap:stream:memory:<bytes>`) ends the answer with `OverBudget`, and a
failing input ends it with the input's error: never a well-formed shorter
answer. Collected, the body answer equals the bytes answer for the same input.
An inline `in` is answered exactly as before.

Booleans take `true`/`false`; an absent flag is `false`, and any other value is
an error (a mistyped flag never silently reads as off). `uniq` collapses only
*adjacent* duplicates (Unix semantics) — pipe through `sort` first for a global
dedup.

Each endpoint authors a full self-description — an XSD `class` on every input,
`one_of` for the enums, `default` where applicable — so that each projects to a
well-typed, MCP-legible tool once the action manifold is projected.

## Conformance

The module **passes
[`ikigai-conformance`](https://github.com/ikigai-rs/ikigai-conformance)** with
no opt-outs: `tests/conformance.rs` walks every `urn:text:*` endpoint and runs
every check (ArgSpec completeness, declared = enforced, cacheability, pipeline
citizenship, naming). Every endpoint is declared `pure` there — it reads nothing
but its inline arguments, so its cacheable result rightly has no golden thread —
and `cacheable`, so a future dependency that silently downgraded the effective
expiry would fail the test rather than slow every read.

## Usage

Mount the whole library at its conventional `urn:text:*` IRIs and chain your own
bindings on top (`EndpointSpace::bind` is a builder):

```rust
use ikigai_core::{Exact, Kernel};
use std::sync::Arc;

let space = ikigai_text::space()
    .bind(Exact::new("urn:file:log.txt"), my_file_endpoint());
let kernel = Kernel::new(Arc::new(space));
```

`space()` names itself `urn:iki:space:text`, exported as `ikigai_text::SPACE_ID`
(binding more doors on top, as above, drops the name: the host names its extension).

Or pull the individual constructors (`ikigai_text::grep()`, …) and bind them at
IRIs of your choosing — binding authority is a host concern.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at
your option.
