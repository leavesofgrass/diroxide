//! A small program for the quick view's code sample.

use std::env;

/// The greeting, by name.
fn greeting(name: &str) -> String {
	format!("Hello, {name}!")
}

/// Parses the arguments into a name.
fn parse_args() -> Option<String> {
    env::args().nth(1)
}

fn main() {
    let name = parse_args().unwrap_or_else(|| "crow".to_owned());
    println!("{}", greeting(&name));
}
