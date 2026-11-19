//! Entry point of the `infra` command line. All logic lives in the library so it can be tested.

use std::io::{stderr, stdout};
use std::process::ExitCode;

fn main() -> ExitCode {
    // `args()` panics on non-UTF-8 input (and the profile aborts on panic); convert lossily.
    let args = cli::args_from_os(std::env::args_os().skip(1));
    let lang = std::env::var("LANG").ok();
    let code = cli::run(
        &args,
        lang.as_deref(),
        &mut stdout().lock(),
        &mut stderr().lock(),
    );
    ExitCode::from(code)
}
