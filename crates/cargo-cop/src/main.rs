mod application;
mod rule_catalog;

use std::{
    io::{self, Write},
    process::ExitCode,
};

use application::Application;

fn main() -> ExitCode {
    match Application::run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "{error}");
            ExitCode::from(2)
        }
    }
}
