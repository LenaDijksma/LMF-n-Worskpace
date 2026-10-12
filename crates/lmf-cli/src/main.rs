//! `lmf` — command-line tool for LMF (Lovely Markdown Format) documents.
//!
//! Exit codes: 0 success, 1 problems found (`check`), 2 usage or I/O error.

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::process::ExitCode;

use lmf_core::export::{to_markdown, ExportOptions, FrontMatterMode, Loss, LossLevel, UnknownMode};
use lmf_core::import::{from_markdown, ImportOptions};
use lmf_core::{check, parse_bytes, Block, Document, LineIndex, Severity};

const USAGE: &str = "\
lmf - tools for LMF (Lovely Markdown Format) documents

USAGE:
    lmf check [--strict] [--format text|json] <FILE|->...
    lmf to-markdown [--unknown keep|drop] [--front-matter keep|strip] [-o OUT] <FILE|->
    lmf from-markdown [--stamp] [-o OUT] <FILE|->
    lmf ast <FILE|->
    lmf --help | --version

COMMANDS:
    check          Parse and validate documents; print diagnostics.
                   Exit 1 on errors (or on warnings with --strict).
    to-markdown    Export to plain Markdown. Everything that cannot be preserved
                   is reported on stderr; nothing is lost silently.
    from-markdown  Import ordinary Markdown (e.g. GitHub alerts -> lmf:callout).
    ast            Print the parsed document structure.

A path of `-` reads standard input.
";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("lmf: {}", message);
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<u8, String> {
    let Some(command) = args.first() else {
        eprint!("{}", USAGE);
        return Ok(2);
    };
    let rest = &args[1..];
    match command.as_str() {
        "check" => cmd_check(rest),
        "to-markdown" => cmd_to_markdown(rest),
        "from-markdown" => cmd_from_markdown(rest),
        "ast" => cmd_ast(rest),
        "-h" | "--help" | "help" => {
            print!("{}", USAGE);
            Ok(0)
        }
        "-V" | "--version" => {
            println!("lmf {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        other => Err(format!("unknown command `{}`; run `lmf --help`", other)),
    }
}

// --- check ------------------------------------------------------------------------------

fn cmd_check(args: &[String]) -> Result<u8, String> {
    let mut strict = false;
    let mut json = false;
    let mut files: Vec<&str> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--strict" => strict = true,
            "--format" => {
                i += 1;
                match args.get(i).map(|s| s.as_str()) {
                    Some("json") => json = true,
                    Some("text") => json = false,
                    Some(other) => {
                        return Err(format!(
                            "unknown format `{}` (expected `text` or `json`)",
                            other
                        ))
                    }
                    None => return Err("--format needs a value".to_string()),
                }
            }
            "-" => files.push("-"),
            s if s.starts_with('-') => return Err(format!("unknown option `{}`", s)),
            s => files.push(s),
        }
        i += 1;
    }
    if files.is_empty() {
        return Err("no input files; usage: lmf check <FILE>...".to_string());
    }

    let mut json_items: Vec<String> = Vec::new();
    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut io_failed = false;

    for path in &files {
        let bytes = match read_input(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("lmf: {}", e);
                io_failed = true;
                continue;
            }
        };
        let doc = parse_bytes(&bytes);
        let diags = check(&doc);
        let index = LineIndex::new(&doc.source);

        if diags.is_empty() && !json {
            println!("{}: ok", path);
        }
        for d in &diags {
            match d.severity {
                Severity::Error => errors += 1,
                Severity::Warning => warnings += 1,
                Severity::Info => {}
            }
            let (line, col) = index.line_col(&doc.source, d.span.start);
            if json {
                let (eline, ecol) = index.line_col(&doc.source, d.span.end);
                json_items.push(format!(
                    "  {{\"file\":\"{}\",\"code\":\"{}\",\"severity\":\"{}\",\"message\":\"{}\",\"start\":{{\"line\":{},\"column\":{},\"offset\":{}}},\"end\":{{\"line\":{},\"column\":{},\"offset\":{}}}}}",
                    json_escape(path),
                    d.code,
                    d.severity,
                    json_escape(&d.message),
                    line,
                    col,
                    d.span.start,
                    eline,
                    ecol,
                    d.span.end
                ));
            } else {
                println!(
                    "{}:{}:{}: {}[{}]: {}",
                    path, line, col, d.severity, d.code, d.message
                );
            }
        }
    }

    if json {
        if json_items.is_empty() {
            println!("[]");
        } else {
            println!("[\n{}\n]", json_items.join(",\n"));
        }
    } else if errors > 0 || warnings > 0 {
        eprintln!("{} error(s), {} warning(s)", errors, warnings);
    }

    if io_failed {
        Ok(2)
    } else if errors > 0 || (strict && warnings > 0) {
        Ok(1)
    } else {
        Ok(0)
    }
}

// --- to-markdown ------------------------------------------------------------------------

fn cmd_to_markdown(args: &[String]) -> Result<u8, String> {
    let mut opts = ExportOptions::default();
    let mut output: Option<String> = None;
    let mut input: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--unknown" => {
                i += 1;
                opts.unknown = match args.get(i).map(|s| s.as_str()) {
                    Some("keep") => UnknownMode::Keep,
                    Some("drop") => UnknownMode::Drop,
                    _ => return Err("--unknown expects `keep` or `drop`".to_string()),
                };
            }
            "--front-matter" => {
                i += 1;
                opts.front_matter = match args.get(i).map(|s| s.as_str()) {
                    Some("keep") => FrontMatterMode::Keep,
                    Some("strip") => FrontMatterMode::Strip,
                    _ => return Err("--front-matter expects `keep` or `strip`".to_string()),
                };
            }
            "-o" | "--output" => {
                i += 1;
                match args.get(i) {
                    Some(p) => output = Some(p.clone()),
                    None => return Err("-o needs a path".to_string()),
                }
            }
            s if s.starts_with('-') && s != "-" => {
                return Err(format!("unknown option `{}`", s));
            }
            s => {
                if input.is_some() {
                    return Err("expected exactly one input file".to_string());
                }
                input = Some(s.to_string());
            }
        }
        i += 1;
    }
    let input = input.ok_or_else(|| "no input file; usage: lmf to-markdown <FILE>".to_string())?;

    let bytes = read_input(&input)?;
    let doc = parse_bytes(&bytes);
    if doc.diagnostics.iter().any(|d| d.code == "LMF001") {
        return Err("input is not valid UTF-8; refusing to convert".to_string());
    }
    let syntax_errors = doc
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    if syntax_errors > 0 {
        eprintln!(
            "lmf: warning: input has {} syntax error(s); run `lmf check` for details",
            syntax_errors
        );
    }

    let conversion = to_markdown(&doc, &opts);
    print_losses(&doc, &conversion.losses);
    write_output(output.as_deref(), &conversion.output)?;
    Ok(0)
}

// --- from-markdown ----------------------------------------------------------------------

fn cmd_from_markdown(args: &[String]) -> Result<u8, String> {
    let mut opts = ImportOptions::default();
    let mut output: Option<String> = None;
    let mut input: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--stamp" => opts.stamp_version = true,
            "-o" | "--output" => {
                i += 1;
                match args.get(i) {
                    Some(p) => output = Some(p.clone()),
                    None => return Err("-o needs a path".to_string()),
                }
            }
            s if s.starts_with('-') && s != "-" => {
                return Err(format!("unknown option `{}`", s));
            }
            s => {
                if input.is_some() {
                    return Err("expected exactly one input file".to_string());
                }
                input = Some(s.to_string());
            }
        }
        i += 1;
    }
    let input = input.ok_or_else(|| "no input file; usage: lmf from-markdown <FILE>".to_string())?;

    let bytes = read_input(&input)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| "input is not valid UTF-8; refusing to convert".to_string())?;
    let conversion = from_markdown(&text, &opts);
    let doc = lmf_core::parse(&text);
    print_losses(&doc, &conversion.losses);
    write_output(output.as_deref(), &conversion.output)?;
    Ok(0)
}

// --- ast --------------------------------------------------------------------------------

fn cmd_ast(args: &[String]) -> Result<u8, String> {
    if args.len() != 1 {
        return Err("usage: lmf ast <FILE|->".to_string());
    }
    let bytes = read_input(&args[0])?;
    let doc = parse_bytes(&bytes);
    let index = LineIndex::new(&doc.source);

    println!("document ({} bytes)", doc.source.len());
    if let Some(fm) = &doc.front_matter {
        let (l1, _) = index.line_col(&doc.source, fm.span.start);
        let (l2, _) = index.line_col(&doc.source, fm.span.end);
        println!("  front_matter lines {}-{}", l1, l2);
        for e in &fm.entries {
            println!("    {}: {}", e.key, e.value);
        }
    }
    print_blocks(&doc, &index, &doc.blocks, 1);
    Ok(0)
}

fn print_blocks(doc: &Document, index: &LineIndex, blocks: &[Block], depth: usize) {
    let pad = "  ".repeat(depth);
    for block in blocks {
        match block {
            Block::Markdown(m) => {
                let (l1, _) = index.line_col(&doc.source, m.span.start);
                let (l2, _) = index.line_col(&doc.source, m.span.end);
                println!("{}markdown lines {}-{}", pad, l1, l2);
            }
            Block::Directive(d) => {
                let (l1, _) = index.line_col(&doc.source, d.span.start);
                let (l2, _) = index.line_col(&doc.source, d.span.end);
                let attrs = d
                    .attrs
                    .iter()
                    .map(|a| format!("{}={}", a.key, a.value))
                    .collect::<Vec<String>>()
                    .join(" ");
                let attrs = if attrs.is_empty() {
                    String::new()
                } else {
                    format!(" {{{}}}", attrs)
                };
                let state = if d.closed { "" } else { " (UNCLOSED)" };
                println!(
                    "{}directive {}{} lines {}-{}{}",
                    pad,
                    d.qualified_name(),
                    attrs,
                    l1,
                    l2,
                    state
                );
                print_blocks(doc, index, &d.body, depth + 1);
            }
        }
    }
}

// --- helpers ----------------------------------------------------------------------------

fn print_losses(doc: &Document, losses: &[Loss]) {
    if losses.is_empty() {
        return;
    }
    let index = LineIndex::new(&doc.source);
    for loss in losses {
        let (line, col) = index.line_col(&doc.source, loss.span.start);
        eprintln!(
            "lmf: {}[{}] {}:{}: {}",
            "loss",
            loss.level.as_str(),
            line,
            col,
            loss.message
        );
    }
    let dropped = losses.iter().filter(|l| l.level == LossLevel::Dropped).count();
    let degraded = losses.iter().filter(|l| l.level == LossLevel::Degraded).count();
    eprintln!(
        "lmf: {} item(s) removed, {} item(s) reduced to plain Markdown",
        dropped, degraded
    );
}

fn read_input(path: &str) -> Result<Vec<u8>, String> {
    if path == "-" {
        let mut buf = Vec::new();
        io::stdin()
            .read_to_end(&mut buf)
            .map_err(|e| format!("cannot read standard input: {}", e))?;
        Ok(buf)
    } else {
        fs::read(path).map_err(|e| format!("cannot read `{}`: {}", path, e))
    }
}

fn write_output(path: Option<&str>, text: &str) -> Result<(), String> {
    match path {
        Some(p) => fs::write(p, text).map_err(|e| format!("cannot write `{}`: {}", p, e)),
        None => io::stdout()
            .write_all(text.as_bytes())
            .map_err(|e| format!("cannot write to standard output: {}", e)),
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_json() {
        assert_eq!(json_escape("a\"b\\c\n"), "a\\\"b\\\\c\\n");
        assert_eq!(json_escape("\u{1}"), "\\u0001");
    }
}
