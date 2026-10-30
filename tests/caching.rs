//! A text tool over a piped file is cached (ledger #2530). A file reaches a tool
//! as a [`Body::Replay`] that knows its length; at or under
//! [`ikigai_text::COLLECT_BOUND`] the tool collects its answer to bytes, which
//! the kernel's byte-bounded cache admits. Above the bound, or with no length
//! known, the answer streams and is not cached (`urn:kernel:uncached` says
//! `policy`).

use futures::executor::block_on;
use ikigai_core::{
    ArgRef, Body, ByteStream, Bytes, Capability, Endpoint, Exact, Invocation, Iri, Kernel, Replay,
    ReprType, Representation, Request, Result, TraceEvent, Tracer, Verb, UNCACHED_NOTE,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

/// Where the test file is bound.
const FILE: &str = "urn:test:file";

/// A file: `bytes` read in 4 KiB chunks, its length known (or not, when
/// `sized` is false), counting how often it is opened.
#[derive(Clone)]
struct File {
    bytes: Bytes,
    sized: bool,
    opened: Arc<AtomicUsize>,
}

impl Replay for File {
    fn open(&self) -> ByteStream {
        self.opened.fetch_add(1, Ordering::SeqCst);
        let chunks: Vec<Result<Bytes>> = self
            .bytes
            .chunks(4096)
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect();
        Box::pin(futures::stream::iter(chunks))
    }

    fn len_hint(&self) -> Option<u64> {
        self.sized.then_some(self.bytes.len() as u64)
    }
}

#[async_trait]
impl Endpoint for File {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        let body = self.invoke_body(inv).await?;
        body.collect(usize::MAX).await
    }

    async fn invoke_body(&self, _inv: &Invocation<'_>) -> Result<Representation<Body>> {
        Ok(
            Representation::from_body(ReprType::new("text/plain"), Body::replay(self.clone()))
                .cacheable(),
        )
    }
}

/// `lines` lines of about 70 bytes, like a GeoJSON feature per line.
fn lines(lines: usize) -> Bytes {
    let mut out = String::new();
    for i in 0..lines {
        out.push_str(&format!(
            "{{\"type\":\"Feature\",\"id\":\"ev{i:06}\",\"mag\":{}.{},\"place\":\"x\"}}\n",
            i % 7,
            i % 10
        ));
    }
    Bytes::from(out)
}

fn kernel_over(bytes: Bytes, sized: bool) -> (Kernel, Arc<AtomicUsize>) {
    let opened = Arc::new(AtomicUsize::new(0));
    let file = File {
        bytes,
        sized,
        opened: Arc::clone(&opened),
    };
    let kernel = Kernel::new(Arc::new(ikigai_text::space().bind(Exact::new(FILE), file)));
    (kernel, opened)
}

/// `tool` with `in` piped from [`FILE`], and `args` inline.
fn piped(tool: &str, args: &[(&str, &str)]) -> Request {
    let mut request = Request::new(Verb::Source, Iri::parse(tool).unwrap())
        .with_arg("in", ArgRef::Reference(Iri::parse(FILE).unwrap()));
    for (name, value) in args {
        request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    request
}

#[derive(Default)]
struct Collect(Mutex<Vec<TraceEvent>>);

impl Tracer for Collect {
    fn record(&self, event: TraceEvent) {
        self.0.lock().unwrap().push(event);
    }
}

/// Issue `request` traced: its answer, whether the tool's step was a cache hit,
/// and the tool's `urn:kernel:uncached` note, if any.
fn issue(kernel: &Kernel, request: &Request) -> (Vec<u8>, bool, Option<String>) {
    let collect = Arc::new(Collect::default());
    let answer =
        block_on(kernel.issue_traced(request.clone(), &Capability::root(), collect.clone()))
            .unwrap();
    let target = request.target.to_string();
    let events = collect.0.lock().unwrap();
    let event = events
        .iter()
        .rev()
        .find(|e| e.target == target)
        .expect("the tool's step is traced");
    let note = event
        .notes
        .iter()
        .find(|(k, _)| k == UNCACHED_NOTE)
        .map(|(_, v)| v.clone());
    (answer.bytes, event.cache_hit, note)
}

/// Every line tool, and `sort`, with arguments.
const TOOLS: &[(&str, &[(&str, &str)])] = &[
    ("urn:text:wc", &[]),
    ("urn:text:wc", &[("count", "bytes")]),
    ("urn:text:head", &[("n", "3")]),
    ("urn:text:tail", &[("n", "3")]),
    ("urn:text:grep", &[("pattern", "mag\":3")]),
    ("urn:text:sort", &[("r", "true")]),
    ("urn:text:uniq", &[("c", "true")]),
    ("urn:text:nl", &[]),
    ("urn:text:rev", &[]),
];

#[test]
fn a_tool_over_a_small_file_is_cached_on_the_second_read() {
    // About 125 KB, the item's `quakes-2024-01-01.geojson`.
    let file = lines(1780);
    assert!(file.len() > 100_000 && file.len() as u64 <= ikigai_text::COLLECT_BOUND);
    for (tool, args) in TOOLS {
        let (kernel, _) = kernel_over(file.clone(), true);
        let request = piped(tool, args);
        let (first, hit, note) = issue(&kernel, &request);
        assert!(!hit, "{tool}: the first read computes");
        assert_eq!(note, None, "{tool}: nothing in urn:kernel:uncached");
        assert!(kernel.is_cached(&request, &Capability::root()), "{tool}");
        let (second, hit, note) = issue(&kernel, &request);
        assert!(hit, "{tool}: the second read is [cached]");
        assert_eq!(note, None, "{tool}");
        assert_eq!(first, second, "{tool}");
    }
}

#[test]
fn wc_over_the_items_file_counts_and_caches() {
    let (kernel, opened) = kernel_over(lines(178), true);
    let request = piped("urn:text:wc", &[]);
    let (answer, _, _) = issue(&kernel, &request);
    assert_eq!(answer, b"178");
    let reads = opened.load(Ordering::SeqCst);
    let (answer, hit, _) = issue(&kernel, &request);
    assert_eq!(answer, b"178");
    assert!(hit);
    assert_eq!(
        opened.load(Ordering::SeqCst),
        reads,
        "the file is not read again"
    );
}

#[test]
fn the_collected_answer_is_bytes_and_equals_the_streamed_one() {
    let file = lines(500);
    for (tool, args) in TOOLS {
        let (small, _) = kernel_over(file.clone(), true);
        let (unsized_, _) = kernel_over(file.clone(), false);
        let collected = block_on(small.issue_body(piped(tool, args), &Capability::root())).unwrap();
        assert!(
            matches!(collected.bytes, Body::Bytes(_)),
            "{tool}: a small sized input gives a bytes answer"
        );
        let streamed =
            block_on(unsized_.issue_body(piped(tool, args), &Capability::root())).unwrap();
        assert_eq!(
            block_on(collected.bytes.collect(usize::MAX)).unwrap(),
            block_on(streamed.bytes.collect(usize::MAX)).unwrap(),
            "{tool}"
        );
    }
}

#[test]
fn a_file_over_the_bound_still_streams_and_is_not_cached() {
    let file = lines(ikigai_text::COLLECT_BOUND as usize / 50);
    assert!(file.len() as u64 > ikigai_text::COLLECT_BOUND);
    let (kernel, _) = kernel_over(file, true);
    let request = piped("urn:text:wc", &[]);
    let answer = block_on(kernel.issue_body(request.clone(), &Capability::root())).unwrap();
    assert!(
        matches!(answer.bytes, Body::Replay(_)),
        "over the bound, the answer streams"
    );
    drop(answer);
    let (_, hit, note) = issue(&kernel, &request);
    assert!(!hit);
    assert_eq!(note.as_deref(), Some("policy"));
    assert!(!kernel.is_cached(&request, &Capability::root()));
}

#[test]
fn a_file_of_unknown_length_still_streams_and_is_not_cached() {
    let (kernel, _) = kernel_over(lines(10), false);
    let request = piped("urn:text:grep", &[("pattern", "ev")]);
    let answer = block_on(kernel.issue_body(request.clone(), &Capability::root())).unwrap();
    assert!(matches!(answer.bytes, Body::Replay(_)));
    drop(answer);
    let (_, hit, note) = issue(&kernel, &request);
    assert!(!hit);
    assert_eq!(note.as_deref(), Some("policy"));
}
