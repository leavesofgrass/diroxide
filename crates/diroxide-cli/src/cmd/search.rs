//! `diro search ROOT PATTERN`: print the places under ROOT whose names
//! match the comma-separated globs in PATTERN, one a line, then a summary.

use std::io::Write;

use diroxide_core::{Message, Place};
use diroxide_vfs::{OpContext, Registry};

use crate::CliError;
use crate::open::current_place;

/// Runs the search. Returns 0 when something was found, 1 when nothing was.
pub fn run(root: &str, pattern: &str, out: &mut dyn Write) -> Result<i32, CliError> {
    let here = current_place()?;
    let root = Place::resolve(root, &here)
        .map_err(|e| CliError::Failed(Message::from_error("search", root, &e).text))?;
    let query = diroxide_search::SearchQuery::names(root.clone(), pattern);
    let mut write_failed = None;
    let summary = diroxide_search::search(
        &query,
        &Registry::with_local(),
        &OpContext::new(),
        &mut |hit| {
            if write_failed.is_none()
                && let Err(e) = writeln!(out, "{}", hit.place)
            {
                write_failed = Some(e.to_string());
            }
        },
    )
    .map_err(|e| CliError::Failed(Message::from_error("search", &root.to_string(), &e).text))?;
    if let Some(message) = write_failed {
        return Err(CliError::Failed(message));
    }
    writeln!(out, "{}", summary.describe()).map_err(|e| CliError::Failed(e.to_string()))?;
    Ok(if summary.hits > 0 { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_this_file() {
        let mut out = Vec::new();
        let code = run(env!("CARGO_MANIFEST_DIR"), "search.rs", &mut out).unwrap();
        assert_eq!(code, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.lines().next().unwrap().ends_with("search.rs"),
            "{text}"
        );
        assert!(text.lines().last().unwrap().starts_with("1 found in "));
    }
}
