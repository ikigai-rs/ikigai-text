//! Constant memory, measured (ledger #2275): a counting allocator records the
//! peak bytes allocated while the line tools read 64 MiB through the kernel.
//! Its own test binary, with one test, so nothing else allocates beside it.

use async_trait::async_trait;
use futures::executor::block_on;
use futures::StreamExt;
use ikigai_core::{
    ArgRef, Body, Bytes, Capability, Endpoint, Exact, Invocation, Iri, Kernel, ReprType,
    Representation, Request, Result, Verb,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// The system allocator, counting the bytes live now and the most live since
/// the last [`reset_peak`].
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(live, Ordering::SeqCst);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Start measuring: the peak from here is relative to what is live now.
fn reset_peak() -> usize {
    let live = LIVE.load(Ordering::SeqCst);
    PEAK.store(live, Ordering::SeqCst);
    live
}

/// 64 MiB of 64-byte lines, generated 4 KiB at a time as it is read.
struct Generated;

const LINES: usize = 1 << 20;

#[async_trait]
impl Endpoint for Generated {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        self.invoke_body(inv).await?.collect(usize::MAX).await
    }

    async fn invoke_body(&self, _inv: &Invocation<'_>) -> Result<Representation<Body>> {
        let chunks = (0..LINES).step_by(64).map(|first| {
            let mut chunk = Vec::with_capacity(64 * 64);
            for i in first..first + 64 {
                chunk.extend_from_slice(format!("{i:063}\n").as_bytes());
            }
            Ok(Bytes::from(chunk))
        });
        Ok(Representation::from_body(
            ReprType::new("text/plain"),
            Body::once(futures::stream::iter(chunks)),
        ))
    }
}

#[test]
fn line_tools_read_64_mib_in_constant_memory() {
    let kernel = Kernel::new(Arc::new(
        ikigai_text::space().bind(Exact::new("urn:test:big"), Generated),
    ));
    let cases: &[(&str, &[(&str, &str)])] = &[
        ("urn:text:wc", &[]),
        ("urn:text:grep", &[("pattern", "77777")]),
        ("urn:text:tail", &[("n", "10")]),
        ("urn:text:uniq", &[]),
        ("urn:text:nl", &[]),
        ("urn:text:rev", &[]),
        ("urn:text:head", &[("n", "1000000")]),
    ];
    for (tool, args) in cases {
        let mut request = Request::new(Verb::Source, Iri::parse(*tool).unwrap())
            .with_arg("in", ArgRef::Reference(Iri::parse("urn:test:big").unwrap()));
        for (name, value) in *args {
            request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
        }
        let before = reset_peak();
        let answer = block_on(kernel.issue_body(request, &Capability::root())).unwrap();
        // Read the answer as a stream, keeping only its length.
        let read = block_on(
            answer
                .bytes
                .into_stream()
                .fold(0usize, |n, chunk| async move { n + chunk.unwrap().len() }),
        );
        let peak = PEAK.load(Ordering::SeqCst) - before;
        assert!(read > 0, "{tool} answered");
        assert!(
            peak < 1 << 20,
            "{tool} over 64 MiB peaked at {peak} bytes allocated (answer {read} bytes)"
        );
    }
}
