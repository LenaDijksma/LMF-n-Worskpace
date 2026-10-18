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

#[test]
fn assets_flag_reports_missing_images_and_respects_strict() {
    let dir = std::env::temp_dir().join(format!("lmf-cli-assets-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("assets")).unwrap();
    // A valid 1x1-header PNG so the existing image passes.
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&1u32.to_be_bytes());
    png.extend_from_slice(&1u32.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0]);
    fs::write(dir.join("assets/ok.png"), png).unwrap();
    let note = dir.join("note.md");
    fs::write(&note, "![ok](assets/ok.png)\n\n![gone](assets/gone.png)\n").unwrap();

    // Without --assets nothing is checked.
    let plain = bin().arg("check").arg(&note).output().unwrap();
    assert_eq!(plain.status.code(), Some(0));

    let with = bin().args(["check", "--assets"]).arg(&note).output().unwrap();
    let stdout = String::from_utf8_lossy(&with.stdout);
    assert!(stdout.contains("LMF018"), "{}", stdout);
    assert!(!stdout.contains("ok.png"), "{}", stdout);
    assert_eq!(with.status.code(), Some(0)); // a warning, not an error

    let strict = bin().args(["check", "--assets", "--strict"]).arg(&note).output().unwrap();
    assert_eq!(strict.status.code(), Some(1));
    let _ = fs::remove_dir_all(&dir);
}

fn tasks_folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lmf-cli-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("projects")).unwrap();
    fs::write(
        dir.join("a.md"),
        "# A\n\n- [ ] Write parser [due: 2026-11-03] [priority: high] #dev ^t-parser\n- [ ] Ship it [after: t-parser]\n- [x] Choose name [done: 2026-10-20]\n",
    )
    .unwrap();
    fs::write(
        dir.join("projects/b.md"),
        "- [ ] Draw logo [status: active] #design\n- [ ] Overdue thing [due: 2026-10-01]\n",
    )
    .unwrap();
    dir
}

#[test]
fn tasks_lists_and_queries() {
    let dir = tasks_folder("list");
    let all = bin().arg("tasks").arg(&dir).output().unwrap();
    assert_eq!(all.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&all.stdout);
    assert_eq!(stdout.lines().count(), 5, "{}", stdout);
    assert!(stdout.contains("a.md:3: [ ] Write parser #dev"), "{}", stdout);
    assert!(stdout.contains("due 2026-11-03"), "{}", stdout);
    assert!(stdout.contains("blocked"), "{}", stdout);

    let overdue = bin()
        .args(["tasks", "--today", "2026-11-03", "--query", "due:overdue"])
        .arg(&dir)
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&overdue.stdout);
    assert_eq!(out.lines().count(), 1, "{}", out);
    assert!(out.contains("Overdue thing"), "{}", out);

    let active = bin()
        .args(["tasks", "--query", "status:active note:projects/"])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&active.stdout).contains("Draw logo"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn tasks_json_and_errors() {
    let dir = tasks_folder("json");
    let out = bin()
        .args(["tasks", "--format", "json", "--query", "id:t-parser"])
        .arg(&dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"id\":\"t-parser\""), "{}", stdout);
    assert!(stdout.contains("\"due\":\"2026-11-03\""), "{}", stdout);
    assert!(stdout.contains("\"priority\":\"high\""), "{}", stdout);

    let bad = bin().args(["tasks", "--query", "colour:red"]).arg(&dir).output().unwrap();
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown filter"));
    let missing = bin().args(["tasks", "/definitely/not/here"]).output().unwrap();
    assert_eq!(missing.status.code(), Some(2));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn task_ids_dry_run_then_write_is_idempotent() {
    let dir = tasks_folder("ids");
    let before = fs::read_to_string(dir.join("projects/b.md")).unwrap();

    let dry = bin().arg("task-ids").arg(&dir).output().unwrap();
    assert_eq!(dry.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&dry.stdout).contains("projects/b.md: 2 task(s) get ids"));
    assert_eq!(fs::read_to_string(dir.join("projects/b.md")).unwrap(), before);

    let write = bin().args(["task-ids", "--write"]).arg(&dir).output().unwrap();
    assert_eq!(write.status.code(), Some(0));
    let after = fs::read_to_string(dir.join("projects/b.md")).unwrap();
    assert_ne!(after, before);
    assert_eq!(after.matches(" ^t-").count(), 2, "{}", after);
    // Existing ids and everything else are untouched.
    let a = fs::read_to_string(dir.join("a.md")).unwrap();
    assert!(a.contains("^t-parser"));

    let again = bin().args(["task-ids", "--write"]).arg(&dir).output().unwrap();
    assert!(String::from_utf8_lossy(&again.stderr).contains("already have ids"));
    assert_eq!(fs::read_to_string(dir.join("projects/b.md")).unwrap(), after);
    let _ = fs::remove_dir_all(&dir);
}
