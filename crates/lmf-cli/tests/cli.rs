//! End-to-end tests that run the compiled `lmf` binary.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lmf"))
}

fn temp_file(name: &str, content: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lmf-cli-tests-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn check_ok_exits_zero() {
    let p = temp_file("ok.lmf", "# Hi\n\n:::lmf:callout {kind=\"tip\"}\nx\n:::\n");
    let out = bin().arg("check").arg(&p).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains(": ok"));
}

#[test]
fn check_error_exits_one_and_names_the_code() {
    let p = temp_file("bad.lmf", ":::lmf:callout\nnever closed\n");
    let out = bin().arg("check").arg(&p).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("LMF007"), "{}", stdout);
    assert!(stdout.contains(":1:1:"), "{}", stdout);
}

#[test]
fn strict_turns_warnings_into_failure() {
    let p = temp_file("warn.lmf", ":::lmf:calout\nx\n:::\n");
    let lenient = bin().arg("check").arg(&p).output().unwrap();
    assert_eq!(lenient.status.code(), Some(0));
    let strict = bin().arg("check").arg("--strict").arg(&p).output().unwrap();
    assert_eq!(strict.status.code(), Some(1));
}

#[test]
fn json_output_is_an_array() {
    let p = temp_file("json.lmf", ":::vendor:x\nx\n:::\n");
    let out = bin()
        .args(["check", "--format", "json"])
        .arg(&p)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim_start().starts_with('['), "{}", stdout);
    assert!(stdout.contains("\"code\":\"LMF006\""), "{}", stdout);
}

#[test]
fn to_markdown_converts_and_reports_losses() {
    let p = temp_file(
        "conv.lmf",
        ":::lmf:callout {kind=\"warning\" title=\"T\"}\nBody\n:::\n",
    );
    let out = bin().arg("to-markdown").arg(&p).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "> **Warning: T**\n>\n> Body\n"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("degraded"));
}

#[test]
fn from_markdown_converts_alerts() {
    let p = temp_file("alert.md", "> [!TIP]\n> Try this.\n");
    let out = bin().arg("from-markdown").arg(&p).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        ":::lmf:callout {kind=\"tip\"}\nTry this.\n:::\n"
    );
}

#[test]
fn missing_file_is_a_usage_error() {
    let out = bin().args(["check", "/definitely/not/here.lmf"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn ast_prints_structure() {
    let p = temp_file("ast.lmf", "intro\n\n:::lmf:callout {kind=\"note\"}\nx\n:::\n");
    let out = bin().arg("ast").arg(&p).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("directive lmf:callout"), "{}", stdout);
    assert!(stdout.contains("markdown lines 1-1"), "{}", stdout);
}
