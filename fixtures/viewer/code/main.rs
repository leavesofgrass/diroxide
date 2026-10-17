//! A small program for the viewer's tests: functions, types, an impl and
//! a module, with comments and strings.

use std::env;

/// The settings read from the command line.
#[derive(Debug, Default)]
pub struct Config {
    verbose: bool,
    name: String,
}

/// What went wrong.
pub enum Problem {
    /// No name was given.
    NoName,
}

impl Config {
    /// Reads the arguments.
    pub fn parse_args(args: &[String]) -> Result<Config, Problem> {
        let mut config = Config::default();
        for arg in args {
            if arg == "--verbose" {
                config.verbose = true;
            } else {
                config.name = arg.clone();
            }
        }
        if config.name.is_empty() {
            return Err(Problem::NoName);
        }
        Ok(config)
    }
}

mod greeting {
    /// The greeting, with the name.
    pub fn hello(name: &str) -> String {
        format!("Hello, {name}!")
    }
}

// A comment that runs
// over three lines before
// the code starts again.
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match Config::parse_args(&args) {
        Ok(config) => println!("{}", greeting::hello(&config.name)),
        Err(Problem::NoName) => eprintln!("Give a name."),
    }
    let total = 40 + 2;
    println!("{total}");
}
