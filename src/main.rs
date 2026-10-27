// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod card;
mod gui;
mod report;
mod save;

use std::path::PathBuf;
use std::process::ExitCode;

/// A crash report or a game folder dragged onto the exe opens straight away.
fn main() -> ExitCode {
    let start = std::env::args_os().nth(1).map(PathBuf::from);
    match gui::run(start) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Can't open the window: {e}");
            ExitCode::FAILURE
        }
    }
}
