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
//! No fixtures (the minimal inputs the ArgSpecs admit are valid calls), no
//! opt-outs, no module namespace (there is no RDF face).

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
        .fold(Suite::new(), |suite, id| suite.pure(*id).cacheable(*id));
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
}
