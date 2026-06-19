use bazel_compile_commands_extractor::parse_args;
use bazel_compile_commands_extractor::run;
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match parse_args(arguments).and_then(|config| run(&config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}
