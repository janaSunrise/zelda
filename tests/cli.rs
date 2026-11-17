//! End-to-end tests for the `zelda` binary: exit codes, output formats, and
//! how errors are attributed to files.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

/// Write `files` into a fresh temp dir.
fn project(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for (name, content) in files {
        std::fs::write(dir.path().join(name), content).unwrap();
    }
    dir
}

fn zelda(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zelda"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

/// Run `zelda check --format json` and return (exit code, all errors).
fn check_json(dir: &Path, files: &[&str]) -> (i32, Vec<Value>) {
    let mut args = vec!["check", "--format", "json"];
    args.extend(files);
    let out = zelda(dir, &args);
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let errors = json["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|f| f["errors"].as_array().unwrap().clone())
        .collect();
    (out.status.code().unwrap(), errors)
}

#[test]
fn clean_file_exits_zero() {
    let dir = project(&[("ok.ts", "const x: number = 1;\n")]);
    let out = zelda(dir.path(), &["check", "ok.ts"]);

    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout, "✓ ok.ts: No errors\n", "no color codes when piped");
}

#[test]
fn type_error_exits_one_with_code_in_output() {
    let dir = project(&[("bad.ts", "const x: number = 'a';\n")]);
    let out = zelda(dir.path(), &["check", "bad.ts"]);

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("TS2322"), "{stderr}");
}

#[test]
fn json_reports_type_errors() {
    let dir = project(&[("bad.ts", "const x: number = 'a';\n")]);
    let (code, errors) = check_json(dir.path(), &["bad.ts"]);

    assert_eq!(code, 1);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["type"], "type");
    assert_eq!(errors[0]["code"], 2322);
}

#[test]
fn json_attributes_errors_to_the_imported_file() {
    let dir = project(&[
        ("dep.ts", "export const x: number = 'a';\n"),
        ("main.ts", "import { x } from './dep';\n"),
    ]);
    let (code, errors) = check_json(dir.path(), &["main.ts"]);

    assert_eq!(code, 1);
    assert_eq!(errors.len(), 1);
    let file = errors[0]["file"].as_str().unwrap();
    assert!(file.ends_with("dep.ts"), "{file}");
}

#[test]
fn syntax_errors_are_reported_not_ignored() {
    let dir = project(&[("bad.ts", "let = ;\n")]);
    let (code, errors) = check_json(dir.path(), &["bad.ts"]);

    assert_eq!(code, 1);
    assert!(!errors.is_empty());
    assert!(errors.iter().all(|e| e["type"] == "parse"));
}

#[test]
fn missing_file_is_an_error() {
    let dir = project(&[]);
    let (code, errors) = check_json(dir.path(), &["missing.ts"]);

    assert_eq!(code, 1);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["type"], "file_read_error");
}

#[test]
fn check_requires_files() {
    let dir = project(&[]);
    let out = zelda(dir.path(), &["check"]);
    assert_eq!(out.status.code(), Some(2), "clap usage error");
}
