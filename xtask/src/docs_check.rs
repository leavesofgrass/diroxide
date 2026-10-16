//! `cargo xtask docs --check`: the documentation rules of plan section 11
//! that a program can check.
//!
//! - Every command id in the catalog appears in `docs/keyboard.md`, so a
//!   user who rebinds a key in `keymap.toml` can find the id to write.
//! - Every relative link in `docs/` resolves: the file is there, and a
//!   `#heading` in a Markdown file names one of its headings.
//! - Every user guide (a Markdown file directly in `docs/`, the plan
//!   aside) ends with a "See also" section, and has no table and no em
//!   dash outside code.
//!
//! Each problem is one line, the file first.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use diroxide_app::CATALOG;

/// Markdown files directly in `docs/` that are not user guides.
pub const NOT_GUIDES: [&str; 1] = ["plan.md"];

/// Every Markdown file under a folder, sorted.
fn markdown_files(folder: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(current) = stack.pop() {
        for item in fs::read_dir(&current)
            .with_context(|| format!("{} could not be listed", current.display()))?
        {
            let path = item?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "md") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// The lines of a Markdown text outside fenced code, with their numbers
/// (from 1).
fn unfenced_lines(text: &str) -> Vec<(usize, &str)> {
    let mut lines = Vec::new();
    let mut fence: Option<char> = None;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let opener = ['`', '~']
            .into_iter()
            .find(|c| trimmed.starts_with(&c.to_string().repeat(3)));
        match (fence, opener) {
            (None, Some(c)) => {
                fence = Some(c);
                continue;
            }
            (Some(open), Some(c)) if open == c => {
                fence = None;
                continue;
            }
            (Some(_), _) => continue,
            (None, None) => {}
        }
        lines.push((index + 1, line));
    }
    lines
}

/// The lines of a Markdown text outside fenced code, with their numbers
/// (from 1) and inline code spans taken out.
fn prose_lines(text: &str) -> Vec<(usize, String)> {
    unfenced_lines(text)
        .into_iter()
        .map(|(number, line)| {
            let without_code: String = line
                .split('`')
                .enumerate()
                .filter(|(i, _)| i % 2 == 0)
                .map(|(_, part)| part)
                .collect();
            (number, without_code)
        })
        .collect()
}

/// A heading's anchor as GitHub makes it: lowercase, punctuation out but
/// `-` and `_`, spaces to `-`.
pub fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// The heading lines of a Markdown text: line number, level and words,
/// inline code kept (its words are part of the anchor).
fn headings(text: &str) -> Vec<(usize, usize, String)> {
    unfenced_lines(text)
        .into_iter()
        .filter_map(|(number, line)| {
            let level = line.chars().take_while(|c| *c == '#').count();
            let rest = line.get(level..)?;
            ((1..=6).contains(&level) && rest.starts_with(' ')).then(|| {
                (
                    number,
                    level,
                    rest.trim().trim_end_matches('#').trim().to_owned(),
                )
            })
        })
        .collect()
}

/// The anchors of a Markdown file's headings, with GitHub's `-1`, `-2`
/// for repeats.
fn anchors(text: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    for (_, _, words) in headings(text) {
        let base = slug(&words);
        let mut candidate = base.clone();
        let mut n = 1;
        while seen.contains(&candidate) {
            candidate = format!("{base}-{n}");
            n += 1;
        }
        seen.insert(candidate);
    }
    seen
}

/// The link targets on one line of prose: inline links and images, and
/// reference definitions.
fn link_targets(line: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let trimmed = line.trim_start();
    if trimmed.starts_with('[')
        && let Some(close) = trimmed.find("]:")
    {
        if let Some(target) = trimmed[close + 2..].split_whitespace().next() {
            targets.push(target.trim_matches(['<', '>']).to_owned());
        }
        return targets;
    }
    let mut rest = line;
    while let Some(at) = rest.find("](") {
        let after = &rest[at + 2..];
        let target = if let Some(inner) = after.strip_prefix('<') {
            inner.split('>').next().unwrap_or("").to_owned()
        } else {
            let mut depth = 0usize;
            let mut end = after.len();
            for (i, c) in after.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' if depth == 0 => {
                        end = i;
                        break;
                    }
                    ')' => depth -= 1,
                    _ => {}
                }
            }
            after[..end]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_owned()
        };
        targets.push(target);
        rest = after;
    }
    targets
}

/// Whether a link leaves the repository: a scheme, or mail.
fn is_external(target: &str) -> bool {
    target.contains("://") || target.starts_with("mailto:") || target.starts_with("tel:")
}

/// Whether a line is part of a Markdown table.
fn is_table_line(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return false;
    }
    let delimiter =
        trimmed.contains("---") && trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
    delimiter || (trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 1)
}

/// The words a command id is matched as: lowercase letters, digits, `.`
/// and `-`, with a sentence's final period left off.
fn id_words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-'))
        .map(|word| word.trim_matches('.'))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Every problem in a docs folder, given the catalog's command ids. The
/// folder's parent is the root that file names are said from.
pub fn problems(docs: &Path, ids: &[String]) -> Result<Vec<String>> {
    let root = docs.parent().unwrap_or(docs);
    let shown = |path: &Path| {
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    };
    let mut problems = Vec::new();

    let keyboard = docs.join("keyboard.md");
    match fs::read_to_string(&keyboard) {
        Ok(text) => {
            let words = id_words(&text);
            let missing: Vec<&str> = ids
                .iter()
                .map(String::as_str)
                .filter(|id| !words.contains(*id))
                .collect();
            if !missing.is_empty() {
                problems.push(format!(
                    "{}: {} of {} command ids are missing: {}",
                    shown(&keyboard),
                    missing.len(),
                    ids.len(),
                    missing.join(", ")
                ));
            }
        }
        Err(_) => problems.push(format!("{}: the page is missing", shown(&keyboard))),
    }

    for file in markdown_files(docs)? {
        let text = fs::read_to_string(&file)
            .with_context(|| format!("{} could not be read", file.display()))?;
        let folder = file.parent().unwrap_or(docs);
        for (number, line) in prose_lines(&text) {
            for target in link_targets(&line) {
                if is_external(&target) {
                    continue;
                }
                let (path, anchor) = match target.split_once('#') {
                    Some((path, anchor)) => (path, Some(anchor)),
                    None => (target.as_str(), None),
                };
                if path.is_empty() && anchor.is_none() {
                    problems.push(format!("{}: line {number} has an empty link", shown(&file)));
                    continue;
                }
                let linked = if path.is_empty() {
                    file.clone()
                } else {
                    folder.join(path.replace("%20", " "))
                };
                if !linked.exists() {
                    problems.push(format!(
                        "{}: line {number} links to {target}, which is not there",
                        shown(&file)
                    ));
                    continue;
                }
                if let Some(anchor) = anchor
                    && linked.extension().is_some_and(|e| e == "md")
                {
                    let linked_text = fs::read_to_string(&linked).unwrap_or_default();
                    if !anchors(&linked_text).contains(anchor) {
                        problems.push(format!(
                            "{}: line {number} links to #{anchor}, which is not a heading of {}",
                            shown(&file),
                            shown(&linked)
                        ));
                    }
                }
            }
        }

        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let is_guide = file.parent() == Some(docs) && !NOT_GUIDES.contains(&name);
        if !is_guide {
            continue;
        }
        let lines = prose_lines(&text);
        let all_headings = headings(&text);
        let see_also = all_headings
            .last()
            .filter(|(_, _, words)| words == "See also")
            .map(|(number, _, _)| *number);
        match see_also {
            Some(start) => {
                let has_item = lines.iter().any(|(number, line)| {
                    let t = line.trim_start();
                    *number > start
                        && (t.starts_with("- ")
                            || t.starts_with("* ")
                            || t.split_once(". ").is_some_and(|(n, _)| {
                                !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
                            }))
                });
                if !has_item {
                    problems.push(format!(
                        "{}: the See also section lists nothing",
                        shown(&file)
                    ));
                }
            }
            None => problems.push(format!(
                "{}: does not end with a See also section",
                shown(&file)
            )),
        }
        if let Some((number, _)) = lines.iter().find(|(_, line)| is_table_line(line)) {
            problems.push(format!(
                "{}: line {number} is a table; user guides use lists",
                shown(&file)
            ));
        }
        for (number, line) in &lines {
            if line.contains('\u{2014}') {
                problems.push(format!("{}: line {number} has an em dash", shown(&file)));
            }
        }
    }
    Ok(problems)
}

/// `cargo xtask docs --check`.
pub fn check(root: &Path) -> Result<bool> {
    let docs = root.join("docs");
    let ids: Vec<String> = CATALOG
        .iter()
        .map(|info| info.id.as_str().to_owned())
        .collect();
    let files = markdown_files(&docs)?.len();
    let problems = problems(&docs, &ids)?;
    if problems.is_empty() {
        println!(
            "docs: Pass. {files} pages checked: every link resolves, every guide ends with See also and has no table or em dash, and all {} command ids are in docs/keyboard.md.",
            ids.len()
        );
        Ok(true)
    } else {
        println!(
            "docs: Fail. {}:",
            diroxide_core::format::count(problems.len() as u64, "problem", "problems")
        );
        for problem in &problems {
            println!("{problem}");
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(docs: &Path, name: &str, text: &str) {
        let path = docs.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn good_docs() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        write(
            &docs,
            "keyboard.md",
            "# Keyboard\n\n- F5: Copy (`file.copy`).\n- Ctrl+Q: Quit, app.quit.\n\n## See also\n\n- [Index](README.md)\n",
        );
        write(
            &docs,
            "README.md",
            "# Docs\n\n- [Keys](keyboard.md#keyboard)\n- [Same](#docs)\n- [Dev](dev/notes.md)\n- [Web](https://diroxide.app)\n\n```text\n| a | b |\n[broken](nowhere.md) and an em dash \u{2014}\n```\n\nSome `[code](nowhere.md)` and `a \u{2014} b`.\n\n## See also\n\n1. [Keys](keyboard.md)\n",
        );
        write(
            &docs,
            "dev/notes.md",
            "# Notes\n\n| a | b |\n|---|---|\n\nNo See also needed here \u{2014} not a guide.\n",
        );
        write(&docs, "plan.md", "# Plan\n\n| a | b |\n");
        dir
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn good_docs_pass() {
        let dir = good_docs();
        let found = problems(&dir.path().join("docs"), &ids(&["file.copy", "app.quit"])).unwrap();
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn each_rule_names_its_file_and_line() {
        let dir = good_docs();
        let docs = dir.path().join("docs");
        write(
            &docs,
            "quickstart.md",
            "# Quick start\n\nSee [panels](panels.md) and [keys](keyboard.md#nothing-here).\n\n| key | does |\n|---|---|\n\nOne \u{2014} two.\n",
        );
        write(
            &docs,
            "operations.md",
            "# Operations\n\n## See also\n\nNothing listed.\n",
        );
        let found = problems(&docs, &ids(&["file.copy", "app.quit", "file.rename"])).unwrap();
        assert_eq!(
            found,
            [
                "docs/keyboard.md: 1 of 3 command ids are missing: file.rename",
                "docs/operations.md: the See also section lists nothing",
                "docs/quickstart.md: line 3 links to panels.md, which is not there",
                "docs/quickstart.md: line 3 links to #nothing-here, which is not a heading of docs/keyboard.md",
                "docs/quickstart.md: does not end with a See also section",
                "docs/quickstart.md: line 5 is a table; user guides use lists",
                "docs/quickstart.md: line 8 has an em dash",
            ]
        );
    }

    #[test]
    fn ids_match_whole_words_only() {
        let words = id_words("Use file.copy-all, not file.copy. Also `app.quit`.");
        assert!(words.contains("file.copy-all"));
        assert!(words.contains("file.copy"));
        assert!(words.contains("app.quit"));
        assert!(!id_words("file.copy-all").contains("file.copy"));
    }

    #[test]
    fn anchors_follow_github() {
        assert_eq!(slug("See also"), "see-also");
        assert_eq!(
            slug("`cargo xtask bench`: the bench"),
            "cargo-xtask-bench-the-bench"
        );
        assert_eq!(slug("Exit codes 0, 1, 2 and 3"), "exit-codes-0-1-2-and-3");
        let found =
            anchors("# A\n\n## Same\n\n## Same\n\n## `diro ls -`\n\n```\n# not a heading\n```\n");
        assert_eq!(
            found.into_iter().collect::<Vec<_>>(),
            ["a", "diro-ls--", "same", "same-1"]
        );
    }

    #[test]
    fn this_repositorys_links_resolve() {
        let docs = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("docs");
        let found = problems(&docs, &[]).unwrap();
        let broken: Vec<&String> = found.iter().filter(|p| p.contains(" links to ")).collect();
        assert!(broken.is_empty(), "{broken:?}");
    }
}
