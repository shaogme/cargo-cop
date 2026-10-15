mod application;

use std::process::ExitCode;

use application::Application;

fn main() -> ExitCode {
    Application::run()
}
