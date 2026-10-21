//! `owd`: the Open Waterway Designer command-line tool. The parser and every command are in the
//! `owd_cli` library.

fn main() -> std::process::ExitCode {
    owd_cli::cli::main()
}
