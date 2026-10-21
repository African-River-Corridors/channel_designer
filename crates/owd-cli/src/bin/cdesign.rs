//! `cdesign`: the old name of `owd`, kept for one release (spec 0006). It prints one line on stderr and
//! then does exactly what `owd` does, so stdout and the exit code are the same.

fn main() -> std::process::ExitCode {
    eprintln!("cdesign is now owd; the cdesign name will be removed in a later release");
    owd_cli::cli::main()
}
