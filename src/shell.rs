#![allow(dead_code)]

use std::io::Write as _;

use anstyle::{AnsiColor, Effects, Style};

const HEADER: Style = AnsiColor::BrightGreen.on_default().effects(Effects::BOLD);
const ERROR: Style = AnsiColor::BrightRed.on_default().effects(Effects::BOLD);
const WARN: Style = AnsiColor::BrightYellow.on_default().effects(Effects::BOLD);

pub fn status(label: &str, message: impl std::fmt::Display) {
    let mut err = anstream::stderr();
    let _ = writeln!(err, "{HEADER}{label:>12}{HEADER:#} {message}");
}

pub fn warn(message: impl std::fmt::Display) {
    let mut err = anstream::stderr();
    let _ = writeln!(err, "{WARN}warning{WARN:#}: {message}");
}

pub fn error(message: impl std::fmt::Display) {
    let mut err = anstream::stderr();
    let _ = writeln!(err, "{ERROR}error{ERROR:#}: {message}");
}

pub fn format_status(label: &str, message: impl std::fmt::Display) -> String {
    format!("{HEADER}{label:>12}{HEADER:#} {message}")
}

pub fn format_warn(label: &str, message: impl std::fmt::Display) -> String {
    format!("{WARN}{label:>12}{WARN:#} {message}")
}

pub fn format_error(label: &str, message: impl std::fmt::Display) -> String {
    format!("{ERROR}{label:>12}{ERROR:#} {message}")
}
