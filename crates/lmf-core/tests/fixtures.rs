//! Fixture-driven conformance tests.
//!
//! Every `*.lmf` file under `tests/fixtures/` has a sibling `*.expect` file listing the
//! diagnostic codes `check` must produce, one per line (empty file = no diagnostics).

use std::fs;
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read fixtures dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().map(|e| e == "lmf").unwrap_or(false) {
            out.push(path);
        }
    }
}

fn fixtures() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    assert!(files.len() >= 20, "expected the fixture corpus, found {}", files.len());
    files
}

#[test]
fn fixtures_produce_exactly_the_expected_diagnostics() {
    let mut failures = Vec::new();
    for path in fixtures() {
        let source = fs::read(&path).expect("read fixture");
        let expect_path = path.with_extension("expect");
        let expected_text = fs::read_to_string(&expect_path)
            .unwrap_or_else(|_| panic!("missing {}", expect_path.display()));
        let mut expected: Vec<String> = expected_text
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        expected.sort();

        let doc = lmf_core::parse_bytes(&source);
        let mut actual: Vec<String> = lmf_core::check(&doc)
            .iter()
            .map(|d| d.code.to_string())
            .collect();
        actual.sort();

        if actual != expected {
            failures.push(format!(
                "{}: expected {:?}, got {:?}",
                path.display(),
                expected,
                actual
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn every_fixture_round_trips_byte_for_byte() {
    for path in fixtures() {
        let source = fs::read_to_string(&path).expect("read fixture");
        let doc = lmf_core::parse(&source);
        assert_eq!(doc.to_source(), source, "{}", path.display());
    }
}

#[test]
fn every_fixture_exports_without_panicking_and_reports_losses_for_extensions() {
    use lmf_core::export::{to_markdown, ExportOptions};
    for path in fixtures() {
        let source = fs::read_to_string(&path).expect("read fixture");
        let doc = lmf_core::parse(&source);
        let conversion = to_markdown(&doc, &ExportOptions::default());
        let has_directive = !doc.directives().is_empty();
        if has_directive {
            assert!(
                !conversion.losses.is_empty(),
                "{} has directives but reported no losses",
                path.display()
            );
        }
    }
}

#[test]
fn spans_always_slice_cleanly() {
    for path in fixtures() {
        let source = fs::read_to_string(&path).expect("read fixture");
        let doc = lmf_core::parse(&source);
        for d in lmf_core::check(&doc) {
            assert!(d.span.start <= d.span.end, "{}: {:?}", path.display(), d);
            assert!(d.span.end <= source.len(), "{}: {:?}", path.display(), d);
        }
        for dir in doc.directives() {
            assert!(
                !doc.text(dir.span).is_empty(),
                "{}: empty directive span",
                path.display()
            );
        }
    }
}
