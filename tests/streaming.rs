//! The body path (ledger #2275): `in` read with `content_body`, a streamed input
//! answered as a stream. Every test goes through the kernel: `in` is a reference
//! to a test producer bound beside `ikigai_text::space()`.

use async_trait::async_trait;
use futures::executor::block_on;
use futures::StreamExt;
use ikigai_core::{
    ArgRef, Body, ByteStream, Bytes, Capability, Endpoint, Error, Exact, Invocation, Iri, Kernel,
    Replay, ReprType, Representation, Request, Result, Verb,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// Where a test producer is bound.
const INPUT: &str = "urn:test:in";

/// What a producer yields: its chunks, then (maybe) an error.
#[derive(Clone)]
struct Chunks {
    chunks: Vec<Bytes>,
    fail: bool,
}

impl Chunks {
    /// `bytes` cut into chunks of `size`.
    fn of(bytes: &[u8], size: usize) -> Self {
        Chunks {
            chunks: bytes
                .chunks(size.max(1))
                .map(Bytes::copy_from_slice)
                .collect(),
            fail: false,
        }
    }

    fn stream(&self) -> ByteStream {
        let mut items: Vec<Result<Bytes>> = self.chunks.iter().cloned().map(Ok).collect();
        if self.fail {
            items.push(Err(Error::Unavailable("the producer broke".into())));
        }
        Box::pin(futures::stream::iter(items))
    }
}

impl Replay for Chunks {
    fn open(&self) -> ByteStream {
        self.stream()
    }
}

/// A producer of `chunks`, one-shot or replayable.
struct Producer {
    chunks: Chunks,
    replay: bool,
}

#[async_trait]
impl Endpoint for Producer {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        self.invoke_body(inv).await?.collect(usize::MAX).await
    }

    async fn invoke_body(&self, _inv: &Invocation<'_>) -> Result<Representation<Body>> {
        let body = if self.replay {
            Body::replay(self.chunks.clone())
        } else {
            Body::Once(self.chunks.stream())
        };
        Ok(Representation::from_body(ReprType::new("text/plain"), body).cacheable())
    }
}

/// A one-shot producer of `lines` numbered lines, one per chunk, counting the
/// chunks polled from it and noting when it is dropped.
struct Counted {
    lines: u64,
    polled: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

/// Sets its flag when the stream holding it is dropped.
struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl Endpoint for Counted {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        self.invoke_body(inv).await?.collect(usize::MAX).await
    }

    async fn invoke_body(&self, _inv: &Invocation<'_>) -> Result<Representation<Body>> {
        let polled = Arc::clone(&self.polled);
        let flag = DropFlag(Arc::clone(&self.dropped));
        let chunks = (0..self.lines).map(move |i| {
            let _held = &flag;
            polled.fetch_add(1, Ordering::SeqCst);
            Ok(Bytes::from(format!("line {i}\n")))
        });
        Ok(Representation::from_body(
            ReprType::new("text/plain"),
            Body::once(futures::stream::iter(chunks)),
        ))
    }
}

fn kernel_with(input: impl Endpoint + 'static) -> Kernel {
    Kernel::new(Arc::new(
        ikigai_text::space().bind(Exact::new(INPUT), input),
    ))
}

/// `tool` with `in` by reference to [`INPUT`], and `args` inline.
fn request(tool: &str, args: &[(&str, &str)]) -> Request {
    let mut request = Request::new(Verb::Source, Iri::parse(tool).unwrap())
        .with_arg("in", ArgRef::Reference(Iri::parse(INPUT).unwrap()));
    for (name, value) in args {
        request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    request
}

/// `tool` with `in` inline: the bytes path.
fn inline(tool: &str, input: &[u8], args: &[(&str, &str)]) -> Result<Vec<u8>> {
    let mut request = Request::new(Verb::Source, Iri::parse(tool).unwrap())
        .with_arg("in", ArgRef::Inline(input.to_vec()));
    for (name, value) in args {
        request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    let kernel = Kernel::new(Arc::new(ikigai_text::space()));
    block_on(kernel.issue(request, &Capability::root())).map(|r| r.bytes)
}

/// Read a body to its end, every item kept (an error item included).
fn items(body: Body) -> Vec<Result<Bytes>> {
    block_on(body.into_stream().collect::<Vec<_>>())
}

/// Every tool with the arguments worth trying.
const CASES: &[(&str, &[(&str, &str)])] = &[
    ("urn:text:wc", &[]),
    ("urn:text:wc", &[("count", "words")]),
    ("urn:text:wc", &[("count", "bytes")]),
    ("urn:text:head", &[]),
    ("urn:text:head", &[("n", "0")]),
    ("urn:text:head", &[("n", "2")]),
    ("urn:text:tail", &[]),
    ("urn:text:tail", &[("n", "0")]),
    ("urn:text:tail", &[("n", "2")]),
    ("urn:text:grep", &[("pattern", "é")]),
    ("urn:text:grep", &[("pattern", "A"), ("i", "true")]),
    ("urn:text:grep", &[("pattern", "a"), ("v", "true")]),
    ("urn:text:sort", &[]),
    (
        "urn:text:sort",
        &[("n", "true"), ("u", "true"), ("r", "true")],
    ),
    ("urn:text:uniq", &[]),
    ("urn:text:uniq", &[("c", "true")]),
    ("urn:text:nl", &[]),
    ("urn:text:rev", &[]),
];

/// Inputs that exercise the line rules: `\r\n`, a trailing newline or not,
/// blank lines, a lone `\r`, multi-byte characters (cut across chunks below).
const INPUTS: &[&str] = &[
    "",
    "\n",
    "a",
    "a\n",
    "a\r\nb\r\n",
    "a\r",
    "x\r\ny\n\n\nz",
    "b\na\na\né\nÉté\n10\n9\na\n",
    "日本語\n🦀 crab\n🦀 crab\nAé\n  spaced   words \t here\n",
];

#[test]
fn the_body_answer_collected_equals_the_bytes_answer() {
    for input in INPUTS {
        for (tool, args) in CASES {
            let expected = inline(tool, input.as_bytes(), args).unwrap();
            // Every chunk size from one byte (a character split across chunks) up.
            for size in 1..=input.len().max(1) {
                for replay in [false, true] {
                    let kernel = kernel_with(Producer {
                        chunks: Chunks::of(input.as_bytes(), size),
                        replay,
                    });
                    let answer =
                        block_on(kernel.issue_body(request(tool, args), &Capability::root()))
                            .unwrap();
                    let got = block_on(answer.bytes.collect(usize::MAX)).unwrap();
                    assert_eq!(
                        String::from_utf8_lossy(&got),
                        String::from_utf8_lossy(&expected),
                        "{tool} {args:?} over {input:?} in {size}-byte chunks (replay: {replay})"
                    );
                    // The bytes API over the same reference agrees too.
                    let bytes = block_on(kernel.issue(request(tool, args), &Capability::root()))
                        .unwrap()
                        .bytes;
                    assert_eq!(bytes, expected, "{tool} {args:?} via issue over {input:?}");
                }
            }
        }
    }
}

#[test]
fn head_drops_its_producer_after_n_lines() {
    let polled = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let kernel = kernel_with(Counted {
        lines: 1_000_000,
        polled: Arc::clone(&polled),
        dropped: Arc::clone(&dropped),
    });
    let answer =
        block_on(kernel.issue_body(request("urn:text:head", &[("n", "3")]), &Capability::root()))
            .unwrap();
    assert!(
        answer.bytes.is_one_shot(),
        "a one-shot input, a one-shot answer"
    );
    let got = items(answer.bytes);
    let text: Vec<u8> = got
        .into_iter()
        .flat_map(|item| item.unwrap().to_vec())
        .collect();
    assert_eq!(text, b"line 0\nline 1\nline 2");
    assert_eq!(
        polled.load(Ordering::SeqCst),
        3,
        "three chunks of a million were read"
    );
    assert!(
        dropped.load(Ordering::SeqCst),
        "the producer was dropped once head had its lines"
    );
}

#[test]
fn head_of_zero_lines_reads_nothing() {
    let polled = Arc::new(AtomicUsize::new(0));
    let kernel = kernel_with(Counted {
        lines: 1_000,
        polled: Arc::clone(&polled),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let answer =
        block_on(kernel.issue(request("urn:text:head", &[("n", "0")]), &Capability::root()))
            .unwrap();
    assert_eq!(answer.bytes, b"");
    assert_eq!(polled.load(Ordering::SeqCst), 0);
}

/// A one-shot input of `lines` lines of `width` bytes each, generated as it is
/// read: never in memory whole.
struct Generated {
    lines: usize,
    width: usize,
}

#[async_trait]
impl Endpoint for Generated {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        self.invoke_body(inv).await?.collect(usize::MAX).await
    }

    async fn invoke_body(&self, _inv: &Invocation<'_>) -> Result<Representation<Body>> {
        let width = self.width;
        // 64 lines per chunk, so lines also arrive whole and many to a chunk.
        let chunks = (0..self.lines).step_by(64).map(move |first| {
            let mut chunk = Vec::with_capacity(64 * (width + 1));
            for i in first..first + 64 {
                let tag = if i % 10_000 == 0 { 'E' } else { '.' };
                chunk.extend(std::iter::repeat_n(tag as u8, width));
                chunk.push(b'\n');
            }
            Ok(Bytes::from(chunk))
        });
        Ok(Representation::from_body(
            ReprType::new("text/plain"),
            Body::once(futures::stream::iter(chunks)),
        ))
    }
}

/// 64 MiB of input against a 64 KiB budget.
fn big_kernel() -> Kernel {
    kernel_with(Generated {
        lines: 1 << 20,
        width: 63,
    })
    .with_stream_memory(64 * 1024, 64 * 1024)
}

#[test]
fn line_tools_stream_an_input_far_larger_than_the_budget() {
    let kernel = big_kernel();
    let read = |tool: &str, args: &[(&str, &str)]| {
        block_on(kernel.issue(request(tool, args), &Capability::root()))
            .map(|r| String::from_utf8(r.bytes).unwrap())
    };
    assert_eq!(read("urn:text:wc", &[]).unwrap(), (1u64 << 20).to_string());
    assert_eq!(
        read("urn:text:wc", &[("count", "bytes")]).unwrap(),
        (64u64 << 20).to_string()
    );
    // One line in ten thousand matches: 105 of 1 048 576.
    assert_eq!(
        read("urn:text:grep", &[("pattern", "E")])
            .unwrap()
            .lines()
            .count(),
        105
    );
    assert_eq!(
        read("urn:text:tail", &[("n", "2")]).unwrap(),
        [".".repeat(63), ".".repeat(63)].join("\n")
    );
    assert_eq!(
        read("urn:text:uniq", &[("c", "true")])
            .unwrap()
            .lines()
            .count(),
        2 * 105
    );
    // Collecting is what the budget refuses: `sort` needs the whole input.
    assert!(matches!(
        read("urn:text:sort", &[]),
        Err(Error::OverBudget { .. })
    ));
    // `nl` and `rev` answer as much as they read, so the answer is read as a
    // stream: every chunk is small, and the whole is never held.
    for tool in ["urn:text:nl", "urn:text:rev"] {
        let answer = block_on(kernel.issue_body(request(tool, &[]), &Capability::root())).unwrap();
        let (mut total, mut largest) = (0usize, 0usize);
        block_on(answer.bytes.into_stream().for_each(|chunk| {
            let chunk = chunk.unwrap();
            total += chunk.len();
            largest = largest.max(chunk.len());
            async {}
        }));
        assert!(total >= (64 << 20) - 1, "{tool} answered {total} bytes");
        assert!(largest <= 16 * 1024, "{tool} held {largest} bytes at once");
    }
}

#[test]
fn a_line_longer_than_the_budget_is_refused_not_truncated() {
    let kernel = kernel_with(Generated {
        lines: 64,
        width: 128 * 1024,
    })
    .with_stream_memory(64 * 1024, 64 * 1024);
    for tool in [
        "urn:text:wc",
        "urn:text:head",
        "urn:text:grep",
        "urn:text:rev",
    ] {
        let args: &[(&str, &str)] = if tool == "urn:text:grep" {
            &[("pattern", "E")]
        } else {
            &[]
        };
        let answer = block_on(kernel.issue_body(request(tool, args), &Capability::root())).unwrap();
        let got = items(answer.bytes);
        assert!(
            matches!(got.last(), Some(Err(Error::OverBudget { .. }))),
            "{tool} ends with OverBudget, not a clean end"
        );
        assert!(
            got.iter().rev().skip(1).all(Result::is_ok),
            "{tool}: one error, last"
        );
    }
}

#[test]
fn tail_holding_more_than_the_budget_is_refused() {
    let kernel = kernel_with(Generated {
        lines: 4096,
        width: 63,
    })
    .with_stream_memory(64 * 1024, 64 * 1024);
    // 4096 lines of 63 bytes is 252 KiB: more than the budget.
    let all = block_on(kernel.issue(
        request("urn:text:tail", &[("n", "100000")]),
        &Capability::root(),
    ));
    assert!(matches!(all, Err(Error::OverBudget { .. })), "{all:?}");
    // The last hundred lines fit.
    let some = block_on(kernel.issue(
        request("urn:text:tail", &[("n", "100")]),
        &Capability::root(),
    ))
    .unwrap();
    assert_eq!(some.bytes.split(|&b| b == b'\n').count(), 100);
}

#[test]
fn a_failing_input_ends_the_answer_with_its_error() {
    let kernel = kernel_with(Producer {
        chunks: Chunks {
            fail: true,
            ..Chunks::of(b"a\nb\nc", 2)
        },
        replay: false,
    });
    for tool in [
        "urn:text:wc",
        "urn:text:nl",
        "urn:text:tail",
        "urn:text:uniq",
    ] {
        let answer = block_on(kernel.issue_body(request(tool, &[]), &Capability::root())).unwrap();
        let got = items(answer.bytes);
        assert!(
            matches!(got.last(), Some(Err(Error::Unavailable(_)))),
            "{tool} ends with the input's error"
        );
        let collected = block_on(kernel.issue(request(tool, &[]), &Capability::root()));
        assert!(
            matches!(collected, Err(Error::Unavailable(_))),
            "{tool}: {collected:?}"
        );
    }
}

#[test]
fn invalid_utf8_is_refused_never_replaced() {
    // "é" is C3 A9: a lone C3 before the newline is invalid, wherever chunks cut.
    let input = b"ok\nbad \xC3\nok\n";
    for size in 1..=input.len() {
        let kernel = kernel_with(Producer {
            chunks: Chunks::of(input, size),
            replay: false,
        });
        for tool in ["urn:text:wc", "urn:text:rev", "urn:text:sort"] {
            let answer = block_on(kernel.issue(request(tool, &[]), &Capability::root()));
            assert!(
                matches!(&answer, Err(Error::InvalidArgument { name, .. }) if name == "in"),
                "{tool} in {size}-byte chunks: {answer:?}"
            );
        }
    }
    // A sequence cut off by the end of the input is invalid too.
    let kernel = kernel_with(Producer {
        chunks: Chunks::of(b"ok\n\xE6\x97", 1),
        replay: false,
    });
    let answer = block_on(kernel.issue(request("urn:text:wc", &[]), &Capability::root()));
    assert!(
        matches!(answer, Err(Error::InvalidArgument { .. })),
        "{answer:?}"
    );
    // The bytes path refuses the same input.
    assert!(inline("urn:text:wc", input, &[]).is_err());
}

#[test]
fn a_replayable_input_gives_an_answer_that_can_be_read_again() {
    let kernel = kernel_with(Producer {
        chunks: Chunks::of(b"b\na\nb\n", 1),
        replay: true,
    });
    let answer =
        block_on(kernel.issue_body(request("urn:text:nl", &[]), &Capability::root())).unwrap();
    assert!(!answer.bytes.is_one_shot());
    let Body::Replay(replay) = &answer.bytes else {
        panic!("a replayable input, a replayable answer");
    };
    for _ in 0..2 {
        let read = block_on(Body::Replay(Arc::clone(replay)).collect(1024)).unwrap();
        assert_eq!(read, &b"     1\tb\n     2\ta\n     3\tb"[..]);
    }
}

#[test]
fn a_bad_argument_is_refused_before_the_input_is_read() {
    let polled = Arc::new(AtomicUsize::new(0));
    let kernel = kernel_with(Counted {
        lines: 10,
        polled: Arc::clone(&polled),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let answer = block_on(kernel.issue_body(
        request("urn:text:head", &[("n", "two")]),
        &Capability::root(),
    ));
    assert!(matches!(answer, Err(Error::InvalidArgument { .. })));
    assert_eq!(polled.load(Ordering::SeqCst), 0);
}

/// A pipe must hand head the live producer, even when collecting it would
/// exceed the run's budget. Only the requested prefix should be polled.
#[test]
fn attached_head_streams_without_collecting_the_input() {
    let polled = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let producer = Counted {
        lines: 1_000_000,
        polled: Arc::clone(&polled),
        dropped: Arc::clone(&dropped),
    };
    let kernel = kernel_with(producer).with_stream_memory(1024, 1024);
    let upstream = block_on(kernel.issue_body(
        Request::new(Verb::Source, Iri::parse(INPUT).unwrap()),
        &Capability::root(),
    ))
    .unwrap();
    let read = Request::new(Verb::Source, Iri::parse("urn:text:head").unwrap())
        .with_arg("n", ArgRef::Inline(b"3".to_vec()));
    let answer = block_on(kernel.issue_with(
        read,
        &Capability::root(),
        ikigai_core::BodyIssue::new().attach("in", upstream.bytes),
    ))
    .unwrap();
    assert_eq!(answer.bytes, b"line 0\nline 1\nline 2");
    assert_eq!(polled.load(Ordering::SeqCst), 3);
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn every_tool_declares_only_its_piped_input_reads_body() {
    for tool in [
        ikigai_text::wc(),
        ikigai_text::head(),
        ikigai_text::tail(),
        ikigai_text::grep(),
        ikigai_text::sort(),
        ikigai_text::uniq(),
        ikigai_text::nl(),
        ikigai_text::rev(),
    ] {
        let description = tool.describe();
        for arg in description.inputs {
            assert_eq!(
                arg.reads_body,
                arg.name == "in",
                "{}.{}",
                description.id,
                arg.name
            );
        }
    }
}

#[test]
fn inline_content_id_and_attached_body_answer_alike() {
    for input in INPUTS {
        for (tool, args) in CASES {
            let expected = inline(tool, input.as_bytes(), args).unwrap();
            let store = Arc::new(ikigai_core::MemoryContentStore::new(1024));
            let held = store.put(Bytes::copy_from_slice(input.as_bytes())).unwrap();
            let kernel = Kernel::new(Arc::new(ikigai_text::space())).with_content_store(store);
            let mut read = Request::new(Verb::Source, Iri::parse(*tool).unwrap());
            for (name, value) in *args {
                read = read.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
            }
            let content = block_on(kernel.issue(
                read.clone().with_arg("in", ArgRef::Content(held.id())),
                &Capability::root(),
            ))
            .unwrap();
            assert_eq!(
                content.bytes, expected,
                "{tool} {args:?} content id over {input:?}"
            );
            for replay in [false, true] {
                let chunks = Chunks::of(input.as_bytes(), 1);
                let body = if replay {
                    Body::replay(chunks)
                } else {
                    Body::Once(chunks.stream())
                };
                let attached = block_on(kernel.issue_with(
                    read.clone(),
                    &Capability::root(),
                    ikigai_core::BodyIssue::new().attach("in", body),
                ))
                .unwrap();
                assert_eq!(
                    attached.bytes, expected,
                    "{tool} {args:?} attached over {input:?} (replay: {replay})"
                );
                assert_eq!(attached.repr_type, content.repr_type);
            }
        }
    }
}
