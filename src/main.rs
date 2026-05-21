use std::process::ExitCode;

use jx::commands::CommandError;

fn main() -> ExitCode {
    match jx::run_from_process() {
        Ok(result) => {
            print!("{}", result.stdout);
            ExitCode::from(result.exit_code)
        }
        Err(CommandError::Usage(error)) => {
            if let Err(print_error) = error.print() {
                eprintln!("error: {print_error}");
                return ExitCode::FAILURE;
            }

            ExitCode::from(error.exit_code() as u8)
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
