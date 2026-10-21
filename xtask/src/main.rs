//! Developer tasks. Run with `cargo xtask <task>`.
//!
//! - `mutants [--base <ref>] [--out <file>]`: run cargo-mutants on the `owd-core` code this branch
//!   changed and write a short killed / survived / timeout report.
//! - `gates`: run the fail-closed fast gate, `tools/gates.sh`.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("mutants") => mutants(&args[1..]),
        Some("gates") => gates(),
        _ => Err("usage: cargo xtask <mutants [--base <ref>] [--out <file>] | gates>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("xtask: {msg}");
            ExitCode::FAILURE
        }
    }
}

type Result<T = ()> = std::result::Result<T, String>;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the repo root")
        .to_path_buf()
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn run(cmd: &mut Command) -> Result<String> {
    let out = cmd
        .output()
        .map_err(|e| format!("cannot run {cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd:?} failed ({}):\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn git(args: &[&str]) -> Result<String> {
    run(Command::new("git").current_dir(root()).args(args))
}

/// The commit this branch forked from. `--base`, else `CD_BASE`, else the PR base on GitHub
/// Actions, else `origin/main`.
fn merge_base(args: &[String]) -> Result<(String, String)> {
    let base = flag(args, "--base")
        .or_else(|| std::env::var("CD_BASE").ok())
        .or_else(|| {
            std::env::var("GITHUB_BASE_REF")
                .ok()
                .filter(|b| !b.is_empty())
                .map(|b| format!("origin/{b}"))
        })
        .unwrap_or_else(|| "origin/main".into());
    let sha = git(&["merge-base", "HEAD", &base])?.trim().to_string();
    Ok((base, sha))
}

fn count(text: &str) -> usize {
    text.lines().filter(|l| !l.trim().is_empty()).count()
}

fn mutants(args: &[String]) -> Result {
    let root = root();
    let (base, sha) = merge_base(args)?;
    let out_file = flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target/mutants-report.md"));
    let work = root.join("target/mutants");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let diff = git(&["diff", &sha, "--", "crates/owd-core/src"])?;
    let mut report =
        format!("# Mutation test: `owd-core` changes against `{base}` ({sha:.12})\n\n");
    if diff.trim().is_empty() {
        report.push_str("No `owd-core` source changed, so there is nothing to mutate.\n");
    } else {
        let diff_file = work.join("core.diff");
        std::fs::write(&diff_file, &diff).map_err(|e| e.to_string())?;
        let status = Command::new("cargo")
            .current_dir(&root)
            .args([
                "mutants",
                "--package",
                "owd-core",
                "--test-workspace=true",
                "--no-shuffle",
            ])
            .arg("--in-diff")
            .arg(&diff_file)
            .arg("--output")
            .arg(&work)
            .status()
            .map_err(|e| {
                format!(
                    "cannot run cargo-mutants (install with `cargo install cargo-mutants`): {e}"
                )
            })?;
        // cargo-mutants: 0 all caught, 2 some survived, 3 some timed out. Anything else is a failure
        // of the run itself (bad usage, or the unmutated tree does not pass its tests).
        if !matches!(status.code(), Some(0 | 2 | 3)) {
            return Err(format!("cargo-mutants failed ({status})"));
        }
        let read = |name: &str| {
            std::fs::read_to_string(work.join("mutants.out").join(name)).unwrap_or_default()
        };
        let (caught, missed, timeout, unviable) = (
            read("caught.txt"),
            read("missed.txt"),
            read("timeout.txt"),
            read("unviable.txt"),
        );
        report.push_str(&format!(
            "| killed | survived | timeout | unviable |\n|---|---|---|---|\n| {} | {} | {} | {} |\n",
            count(&caught),
            count(&missed),
            count(&timeout),
            count(&unviable)
        ));
        for (title, list) in [
            ("Survived", &missed),
            ("Timed out", &timeout),
            ("Killed", &caught),
        ] {
            if count(list) > 0 {
                report.push_str(&format!("\n## {title}\n\n"));
                for line in list.lines().filter(|l| !l.trim().is_empty()) {
                    report.push_str(&format!("- `{line}`\n"));
                }
            }
        }
        report.push_str(
            "\nA surviving mutant that changes a public output is closed by a new parity case \
             (a new test case).\n",
        );
    }
    std::fs::write(&out_file, &report).map_err(|e| format!("{out_file:?}: {e}"))?;
    print!("{report}");
    println!("\nReport written to {}", out_file.display());
    Ok(())
}

fn gates() -> Result {
    let status = Command::new("bash")
        .current_dir(root())
        .arg("tools/gates.sh")
        .status()
        .map_err(|e| format!("cannot run tools/gates.sh: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("gates failed ({status})"))
    }
}
