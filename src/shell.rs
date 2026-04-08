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
