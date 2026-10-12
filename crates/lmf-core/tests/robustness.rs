//! Robustness tests: the parser must never panic and must never lose input.

use lmf_core::export::{to_markdown, ExportOptions};
use lmf_core::import::{from_markdown, ImportOptions};

/// Tiny deterministic generator so the tests need no external crates.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[(self.next() as usize) % items.len()]
    }
}

const TOKENS: &[&str] = &[
    ":::", "::::", ":::lmf:callout", ":::lmf:tabs", ":::lmf:tab", ":::vendor:x", ":::note",
    " {kind=\"tip\"}", " {title=\"a b\"}", " {", "}", "{", "=", "\"", "\\", "true", "-1.5",
    "\n", "\n", "\n", "\r\n", " ", "    ", "\t", "---", "...", "key: value", "- item",
    "tags: [a, b", "```", "~~~", "```rust", "> [!NOTE]", "> ", "# Heading", "é", "日本語",
    "\u{FEFF}", "\u{0}", "😀", ":", "lmf_version: 1", "lmf_version: 99",
];

fn random_doc(rng: &mut Lcg) -> String {
    let n = 1 + (rng.next() % 40) as usize;
    let mut s = String::new();
    for _ in 0..n {
        s.push_str(rng.pick(TOKENS));
    }
    s
}

#[test]
fn random_inputs_never_panic_and_round_trip() {
    let mut rng = Lcg(0x5EED);
    for _ in 0..3000 {
        let src = random_doc(&mut rng);
        let doc = lmf_core::parse(&src);
        assert_eq!(doc.to_source(), src, "round trip failed for {:?}", src);

        // All spans must be valid slices of the source.
        for d in lmf_core::check(&doc) {
            assert!(d.span.start <= d.span.end && d.span.end <= src.len(), "{:?} in {:?}", d, src);
        }
        for dir in doc.directives() {
            assert!(dir.span.start <= dir.span.end && dir.span.end <= src.len(), "{:?}", src);
        }

        // Converters must not panic on arbitrary input either.
        let _ = to_markdown(&doc, &ExportOptions::default());
        let _ = from_markdown(&src, &ImportOptions { stamp_version: true });
    }
}

#[test]
fn deep_nesting_is_handled() {
    let depth = 500;
    let mut src = String::new();
    for _ in 0..depth {
        src.push_str(":::vendor:x\n");
    }
    src.push_str("deep\n");
    for _ in 0..depth {
        src.push_str(":::\n");
    }
    let doc = lmf_core::parse(&src);
    assert_eq!(doc.directives().len(), depth);
    assert!(doc.directives().iter().all(|d| d.closed));
}

#[test]
fn large_document_parses_quickly_enough_to_finish() {
    let mut src = String::new();
    for i in 0..20_000 {
        src.push_str(&format!("# Heading {}\n\nParagraph with *text*.\n\n", i));
        if i % 50 == 0 {
            src.push_str(":::lmf:callout {kind=\"note\"}\nbody\n:::\n\n");
        }
    }
    let doc = lmf_core::parse(&src);
    assert_eq!(doc.directives().len(), 400);
    assert_eq!(doc.to_source().len(), src.len());
}

#[test]
fn malformed_extension_does_not_destroy_surrounding_content() {
    let src = "before\n\n:::lmf:callout {kind=\n\nafter\n";
    let doc = lmf_core::parse(src);
    let conversion = to_markdown(&doc, &ExportOptions::default());
    assert!(conversion.output.contains("before"));
    assert!(conversion.output.contains("after"));
}

#[test]
fn invalid_utf8_is_diagnosed_not_fatal() {
    let doc = lmf_core::parse_bytes(b"# ok\n\xC0\xAF\n:::lmf:callout\nx\n:::\n");
    assert!(lmf_core::check(&doc).iter().any(|d| d.code == "LMF001"));
    assert_eq!(doc.directives().len(), 1);
}
