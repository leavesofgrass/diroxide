//! `cargo xtask native --check`: the pure-Rust gate of plan section 4,
//! principle 5. The main workspace builds with no C compiler, so no crate
//! in its build tree may depend on a native build tool: `cc`, `cmake`,
//! `cxx` or `cxx-build`.
//!
//! The allow list is the inventory of plan section 4, principle 5: pure
//! Rust is a goal, not a rule (the owner, answer 46), so a crate that
//! carries C enters only with an entry here, its reason in words, in the
//! same commit as the dependency. The entries: `cc` as the build
//! dependency of `ring`, the TLS provider of `rustls`, reached only
//! through `ureq`, the optional-components downloader's client behind the
//! `fetch` feature of `diro` (the owner's exception of Sunday, October 11,
//! 2026, answer 40); and `cc` as the build dependency of `psm` and of
//! `stacker`, reached only through `carta`, the converter of answer 27
//! (alpha.2's contract step, Friday, October 16, 2026). Anything else
//! fails, with the chain that brought it in.
//!
//! The check reads `cargo tree` inverted on each tool, over every feature,
//! every target and the normal, build and dev edges, so a test dependency
//! that compiles C fails it too.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// The build tools that mean C, C++ or assembly is compiled.
pub const NATIVE_TOOLS: [&str; 4] = ["cc", "cmake", "cxx", "cxx-build"];

/// One allowed way for a native build tool into the workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allow {
    /// The tool, such as `cc`.
    pub tool: &'static str,
    /// The crate whose build dependency it is.
    pub user: &'static str,
    /// Crates every path from `user` to a workspace member must pass, in
    /// order from `user` outward.
    pub through: &'static [&'static str],
    /// Why it is allowed, for the report.
    pub reason: &'static str,
}

/// The allow list, the inventory: each entry with its reason, and nothing
/// else.
pub const ALLOWED: [Allow; 3] = [
    Allow {
        tool: "cc",
        user: "ring",
        through: &["rustls", "ureq"],
        reason: "the fetch feature's TLS provider, the owner's exception (answer 40)",
    },
    Allow {
        tool: "cc",
        user: "psm",
        through: &["stacker", "carta"],
        reason: "carta grows its stack through stacker on deeply nested documents, and psm \
                 assembles the small stack switch; no pure-Rust reader covers Org, \
                 reStructuredText, MediaWiki, DokuWiki and Jira (answers 27 and 46)",
    },
    Allow {
        tool: "cc",
        user: "stacker",
        through: &["carta"],
        reason: "stacker compiles one small C file for the same stack switch, for carta \
                 (answers 27 and 46)",
    },
];

/// One line of an inverted `cargo tree --prefix depth --format "{p}"`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Line {
    depth: usize,
    name: String,
    member: bool,
}

fn parse_line(text: &str) -> Option<Line> {
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let depth = text.get(..digits)?.parse().ok()?;
    let rest = text.get(digits..)?.trim();
    let name = rest.split_whitespace().next()?.to_owned();
    // A workspace member is printed with its folder in parentheses.
    let member = rest.contains(" (") && !rest.ends_with("(*)") && rest.contains(['/', '\\']);
    Some(Line {
        depth,
        name,
        member,
    })
}

/// The problems in one inverted tree for one tool, in words. Every path
/// from the tool to a workspace member must match an allow entry.
pub fn problems_in_tree(tool: &str, tree: &str) -> Vec<String> {
    let lines: Vec<Line> = tree.lines().filter_map(parse_line).collect();
    let mut problems = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    for line in &lines {
        stack.truncate(line.depth);
        stack.push(line.name.clone());
        if !line.member || line.depth == 0 {
            continue;
        }
        // The chain from the tool outward: tool, its user, ..., the member.
        let chain = &stack;
        let allowed = ALLOWED.iter().any(|allow| {
            allow.tool == tool
                && chain.get(1).is_some_and(|user| user == allow.user)
                && contains_in_order(&chain[2..], allow.through)
        });
        if !allowed {
            problems.push(format!(
                "Native build: {tool} reaches {} through {}",
                line.name,
                chain.join(", then ")
            ));
        }
    }
    problems
}

fn contains_in_order(chain: &[String], wanted: &[&str]) -> bool {
    let mut wanted = wanted.iter().peekable();
    for name in chain {
        if wanted.peek().is_some_and(|w| *w == name) {
            wanted.next();
        }
    }
    wanted.peek().is_none()
}

/// The inverted tree for one tool, or `None` when nothing depends on it.
fn inverted_tree(root: &Path, tool: &str) -> Result<Option<String>> {
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .current_dir(root)
        .args([
            "tree",
            "--workspace",
            "--all-features",
            "--edges",
            "normal,build,dev",
            "--target",
            "all",
            "--prefix",
            "depth",
            "--format",
            "{p}",
            "--invert",
            tool,
        ])
        .output()
        .context("cargo tree could not be started")?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        return Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()));
    }
    if stderr.contains("did not match any packages") {
        return Ok(None);
    }
    bail!("cargo tree failed for {tool}: {}", stderr.trim())
}

/// Runs the check over the workspace, printing each problem on its own
/// line. Returns whether everything passed.
pub fn check(root: &Path) -> Result<bool> {
    let mut problems = Vec::new();
    for tool in NATIVE_TOOLS {
        if let Some(tree) = inverted_tree(root, tool)? {
            problems.extend(problems_in_tree(tool, &tree));
        }
    }
    if problems.is_empty() {
        println!(
            "native: Pass. No C build in the main workspace but the inventory's: ring, for the fetch feature; psm and stacker, for carta."
        );
        Ok(true)
    } else {
        println!("native: Fail. {} problems:", problems.len());
        for problem in &problems {
            println!("{problem}");
        }
        println!(
            "Move the crate into a plugin under plugins/, choose a pure-Rust crate, or ask the owner."
        );
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RING_UNDER_UREQ: &str = "0cc v1.8.0\n\
        1ring v0.17.14\n\
        2rustls v0.23.46\n\
        3ureq v3.4.2\n\
        4diro v0.1.0-alpha.1 (D:\\diroxide\\crates\\diro)\n\
        2rustls-webpki v0.103.15\n\
        3rustls v0.23.46 (*)\n";

    #[test]
    fn ring_through_ureq_is_the_one_allowed_path() {
        assert_eq!(
            problems_in_tree("cc", RING_UNDER_UREQ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn any_other_cc_user_fails_with_its_chain() {
        let tree = "0cc v1.8.0\n\
            1blake3 v1.8.2\n\
            2diroxide-search v0.1.0-alpha.1 (/work/crates/diroxide-search)\n\
            3diroxide-app v0.1.0-alpha.1 (/work/crates/diroxide-app)\n";
        let problems = problems_in_tree("cc", tree);
        assert_eq!(problems.len(), 2);
        assert_eq!(
            problems[0],
            "Native build: cc reaches diroxide-search through cc, then blake3, then diroxide-search"
        );
    }

    #[test]
    fn ring_reached_another_way_fails() {
        let tree = "0cc v1.8.0\n\
            1ring v0.17.14\n\
            2russh v0.54.0\n\
            3diroxide-app v0.1.0-alpha.1 (D:\\w\\crates\\diroxide-app)\n";
        assert_eq!(problems_in_tree("cc", tree).len(), 1);
    }

    #[test]
    fn the_allow_entry_is_for_cc_only() {
        let tree = RING_UNDER_UREQ.replacen("0cc v1.8.0", "0cmake v0.1.54", 1);
        assert_eq!(problems_in_tree("cmake", &tree).len(), 1);
    }

    #[test]
    fn the_inventory_lists_each_entry_with_its_reason() {
        assert_eq!(ALLOWED.len(), 3);
        for allow in &ALLOWED {
            assert!(NATIVE_TOOLS.contains(&allow.tool), "{allow:?}");
            assert!(
                !allow.reason.is_empty() && !allow.through.is_empty(),
                "{allow:?}"
            );
        }
    }

    #[test]
    fn psm_and_stacker_through_carta_are_allowed_and_nowhere_else() {
        let tree = "0cc v1.8.0\n\
            1psm v0.1.24\n\
            2stacker v0.1.25\n\
            3carta v0.0.11\n\
            4diroxide-preview v0.1.0-alpha.1 (D:\\w\\crates\\diroxide-preview)\n\
            1stacker v0.1.25 (*)\n";
        assert_eq!(problems_in_tree("cc", tree), Vec::<String>::new());
        let elsewhere = "0cc v1.8.0\n\
            1stacker v0.1.25\n\
            2rustc-demangle v0.1.0\n\
            3diroxide-app v0.1.0-alpha.1 (D:\\w\\crates\\diroxide-app)\n";
        assert_eq!(problems_in_tree("cc", elsewhere).len(), 1);
    }

    #[test]
    fn this_workspace_passes() {
        assert!(check(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()).unwrap());
    }
}
