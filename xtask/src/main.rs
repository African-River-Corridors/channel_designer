//! Developer tasks. Run with `cargo xtask <task>`.
//!
//! - `regen`: run the parity cases through the frozen Python and rewrite `parity/reference/`.
//!   The only way Reference Results may change. Needs Python, never a maturin build.
//! - `oracle-guard [--base <ref>]`: fail if the parity oracle changed without `regen` and a
//!   recorded reason per changed case (see `oracle.rs`).
//! - `mutants [--base <ref>] [--out <file>]`: run cargo-mutants on the `cd-core` code this branch
//!   changed and write a short killed / survived / timeout report.
//! - `gates`: run the fail-closed fast gate, `tools/gates.sh`.

mod oracle;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("regen") => regen(),
        Some("oracle-guard") => oracle_guard(&args[1..]),
        Some("mutants") => mutants(&args[1..]),
        Some("gates") => gates(),
        _ => Err("usage: cargo xtask <regen | oracle-guard [--base <ref>] | mutants [--base <ref>] [--out <file>] | gates>".into()),
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

fn python() -> String {
    std::env::var("CD_PYTHON").unwrap_or_else(|_| "python3".into())
}

fn run_driver(out_dir: &Path) -> Result {
    run(Command::new(python())
        .current_dir(root())
        .arg("parity/driver.py")
        .arg(out_dir))
    .map(|_| ())
}

fn regen() -> Result {
    let out = root().join("parity/reference");
    run_driver(&out)?;
    println!("Reference Results rewritten in parity/reference/ from the frozen Python.");
    Ok(())
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
}

fn head_tree() -> Result<oracle::Tree> {
    let root = root();
    let mut tree = oracle::Tree::new();
    for dir in [oracle::CASES_DIR, oracle::REFERENCE_DIR] {
        for path in json_files(&root.join(dir)) {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{path:?}: {e}"))?;
            tree.insert(format!("{dir}{name}"), text);
        }
    }
    for file in [oracle::TOLERANCE, oracle::PORTED] {
        if let Ok(text) = std::fs::read_to_string(root.join(file)) {
            tree.insert(file.to_string(), text);
        }
    }
    Ok(tree)
}

fn base_tree(sha: &str) -> Result<oracle::Tree> {
    let mut tree = oracle::Tree::new();
    for path in git(&["ls-tree", "-r", "--name-only", sha, "--", "parity"])?.lines() {
        if oracle::is_guarded(path) && path.ends_with(".json") {
            tree.insert(path.to_string(), git(&["show", &format!("{sha}:{path}")])?);
        }
    }
    Ok(tree)
}

/// Re-run the frozen Python into a scratch folder; the committed Reference Results must match
/// within the parity tolerance (see `oracle::reproduces_within`; not byte for byte, because other
/// machines round the last bits of a float differently). This proves they came from
/// `cargo xtask regen` and not from a hand edit.
fn reproduces() -> Result<Vec<String>> {
    let root = root();
    let scratch = root.join("target/oracle-guard/reference");
    let _ = std::fs::remove_dir_all(&scratch);
    run_driver(&scratch)?;
    let names = |dir: &Path| -> Vec<String> {
        json_files(dir)
            .iter()
            .map(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    };
    let committed_dir = root.join(oracle::REFERENCE_DIR);
    let mut all = names(&committed_dir);
    all.extend(names(&scratch));
    all.sort();
    all.dedup();
    let tol: serde_json::Value = std::fs::read_to_string(root.join(oracle::TOLERANCE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let get = |k: &str| {
        tol.get(k)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
    };
    let (rel, abs) = (get("rel"), get("abs"));
    let mut bad = Vec::new();
    for name in all {
        let committed = std::fs::read_to_string(committed_dir.join(&name)).ok();
        let fresh = std::fs::read_to_string(scratch.join(&name)).ok();
        let why = match (&committed, &fresh) {
            (Some(c), Some(f)) => oracle::reproduces_within(c, f, rel, abs),
            _ => Some("present on one side only".to_string()),
        };
        if let Some(why) = why {
            bad.push(format!(
                "{}{name}: does not match a fresh `cargo xtask regen` ({why}); never edit Reference Results by hand",
                oracle::REFERENCE_DIR
            ));
        }
    }
    Ok(bad)
}

fn oracle_guard(args: &[String]) -> Result {
    let (base, sha) = merge_base(args)?;
    let report = oracle::check(&base_tree(&sha)?, &head_tree()?);
    if !report.changed {
        println!("oracle guard: parity oracle unchanged against {base} ({sha:.12}).");
        return Ok(());
    }
    let mut violations = report.violations;
    violations.extend(reproduces()?);
    if violations.is_empty() {
        println!(
            "oracle guard: parity oracle changed against {base} ({sha:.12}); every change came from `cargo xtask regen` and carries a recorded reason."
        );
        return Ok(());
    }
    for v in &violations {
        eprintln!("oracle guard: {v}");
    }
    Err(format!("{} oracle violation(s)", violations.len()))
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
    let diff = git(&["diff", &sha, "--", "crates/cd-core/src"])?;
    let mut report = format!("# Mutation test: `cd-core` changes against `{base}` ({sha:.12})\n\n");
    if diff.trim().is_empty() {
        report.push_str("No `cd-core` source changed, so there is nothing to mutate.\n");
    } else {
        let diff_file = work.join("core.diff");
        std::fs::write(&diff_file, &diff).map_err(|e| e.to_string())?;
        let status = Command::new("cargo")
            .current_dir(&root)
            .args([
                "mutants",
                "--package",
                "cd-core",
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
             (regenerated, with reason \"kills mutant ...\").\n",
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
