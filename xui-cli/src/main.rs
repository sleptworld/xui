use std::process::ExitCode;

fn main() -> ExitCode {
    match xui_cli::run(std::env::args_os()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error.print();
            error.exit_code()
        }
    }
}
