//! The module recipe as one test: `ikigai-conformance` walks every endpoint
//! `ikigai_text::space()` binds and reports every violation at once.
//!
//! Two declarations, both stated once per endpoint so a newcomer cannot slip in
//! unlisted:
//!
//! - `pure` — every text tool reads nothing but its inline arguments (no file,
//!   network, clock or platform read), so its cacheable result rightly carries
//!   an empty golden-thread set. Nothing ever needs to cut it.
//! - `cacheable` — every text tool marks its result `.cacheable()`. Holding the
//!   suite to that turns a future dependency that silently downgraded the
//!   effective expiry into a red test instead of a ~2000× slowdown.
//!
//! REQUIRED (0.8.0) drops each required input in turn (`in` on all eight, and
//! grep's `pattern`) and holds the call to a typed `MissingArgument`; every
//! tool reads them with `inline_str(…)?`, which refuses exactly that way.
//!
//! DATATYPES and FAILURES (on by default since 0.11.0) run with no waiver:
//! there is no Turtle face to type a literal in, and no action states failure
//! classes (the report says FAILURES compared nothing), so both are clean.
//!
//! No fixtures (the minimal inputs the ArgSpecs admit are valid calls), no
//! opt-outs, no module namespace (there is no RDF face). One space declaration:
//! `space()` is self-named `urn:iki:space:text`.

use ikigai_conformance::Suite;
use ikigai_core::Kernel;
use std::sync::Arc;

/// Every endpoint `space()` binds, by description id.
const ENDPOINTS: [&str; 8] = ["wc", "head", "tail", "grep", "sort", "uniq", "nl", "rev"];

#[test]
fn conforms() {
    let kernel = Kernel::new(Arc::new(ikigai_text::space()));
    let suite = ENDPOINTS
        .iter()
        .fold(Suite::new(), |suite, id| suite.pure(*id).cacheable(*id))
        // `space()` is configuration-free, so it names itself; the suite calls
        // it twice and holds both calls to the same name over the same doors.
        .self_named_space("text", ikigai_text::space);
    let report = suite.run_blocking(&kernel);
    assert!(report.is_clean(), "{report}");
    // The walk saw exactly the endpoints declared above. A ninth tool bound
    // without a `pure`/`cacheable` line would be held to a weaker standard
    // (the suite cannot know which endpoints it was not told about); a
    // declared id that binds nothing is a stale list. Both change this count
    // or fail the checks above.
    assert_eq!(
        report.endpoints,
        ENDPOINTS.len(),
        "every urn:text:* binding is declared: {report}"
    );
    assert_eq!(
        report.actions,
        ENDPOINTS.len(),
        "one Source action per endpoint: {report}"
    );
    assert_eq!(
        ikigai_core::space_iri("text").as_str(),
        ikigai_text::SPACE_ID
    );
}
