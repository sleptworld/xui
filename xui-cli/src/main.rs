use std::process::ExitCode;

fn main() -> ExitCode {
    // During `cargo xui dev`'s builds this executable is also the rustc
    // wrapper and the linker.
    if let Some(code) = xui_hot::shim::run_if_shim() {
        return code;
    }
    match xui_cli::run(std::env::args_os()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error.print();
            error.exit_code()
        }
    }
}
