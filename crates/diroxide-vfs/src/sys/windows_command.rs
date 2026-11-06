//! Windows command lines as the registry holds them, read into a program
//! and an argument list (R30's commit 27): the environment expanded, the
//! C runtime's quoting undone, and the file put where `%1` is, so nothing
//! is run as a shell string. Pure parsing, compiled on Windows and in the
//! tests everywhere.

use super::open_with::{ArgPart, OpenCommand};

/// Expands `%NAME%` from the environment, as a Windows registry value of
/// the expanding kind wants; `%1`, `%*` and a name the environment does
/// not hold stay as they are.
pub fn expand_environment(text: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end)
                if end > 0
                    && after[..end]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '(' | ')'))
                    && !after[..end].chars().all(|c| c.is_ascii_digit()) =>
            {
                let name = &after[..end];
                match lookup(name) {
                    Some(value) => out.push_str(&value),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Splits a Windows command line as the C runtime does: spaces separate
/// arguments outside quotes; a quote opens or closes quoting, and two
/// quotes inside quotes are one; backslashes are themselves unless they
/// come before a quote, where each pair is one backslash and an odd one
/// makes the quote literal.
fn split_windows_args(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut count = 1;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    count += 1;
                }
                if chars.peek() == Some(&'"') {
                    current.extend(std::iter::repeat_n('\\', count / 2));
                    if count % 2 == 1 {
                        chars.next();
                        current.push('"');
                    }
                } else {
                    current.extend(std::iter::repeat_n('\\', count));
                }
                in_arg = true;
            }
            '"' => {
                if quoted && chars.peek() == Some(&'"') {
                    chars.next();
                    current.push('"');
                } else {
                    quoted = !quoted;
                }
                in_arg = true;
            }
            c if c.is_whitespace() && !quoted => {
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                }
            }
            c => {
                current.push(c);
                in_arg = true;
            }
        }
    }
    if in_arg {
        args.push(current);
    }
    args
}

/// A Windows command as the registry holds it (`"C:\Program
/// Files\App\app.exe" "%1"`, `%SystemRoot%\system32\NOTEPAD.EXE %1`),
/// read into a program and arguments. The environment is expanded
/// first; the program is the quoted first word, or, unquoted, the text
/// up to `.exe` when one follows (a path with spaces registered without
/// quotes), else the first word. `%1`, `%L` and `%V` are the file, `%*`
/// the files; an argument of another code alone (`%I`, `%W`) is
/// dropped. `None` for an empty command.
pub fn parse_windows_command(
    command: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<OpenCommand> {
    let expanded = expand_environment(command.trim(), lookup);
    let text = expanded.trim();
    if text.is_empty() {
        return None;
    }
    let (program, rest) = if let Some(after) = text.strip_prefix('"') {
        match after.find('"') {
            Some(end) => (after[..end].to_owned(), &after[end + 1..]),
            None => (after.to_owned(), ""),
        }
    } else {
        let lower = text.to_ascii_lowercase();
        match lower.find(".exe") {
            Some(at)
                if text[at + 4..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace) =>
            {
                (text[..at + 4].to_owned(), &text[at + 4..])
            }
            _ => match text.find(char::is_whitespace) {
                Some(at) => (text[..at].to_owned(), &text[at..]),
                None => (text.to_owned(), ""),
            },
        }
    };
    let has_file = rest.contains("%1")
        || rest.contains("%L")
        || rest.contains("%l")
        || rest.contains("%V")
        || rest.contains("%v");
    let args = split_windows_args(rest)
        .into_iter()
        .filter_map(|arg| windows_arg(&arg, has_file))
        .collect();
    Some(OpenCommand { program, args })
}

/// One Windows argument into its pieces; `None` to drop it.
fn windows_arg(arg: &str, has_file: bool) -> Option<Vec<ArgPart>> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut chars = arg.chars().peekable();
    let mut dropped_code = false;
    while let Some(c) = chars.next() {
        if c != '%' {
            text.push(c);
            continue;
        }
        let code = chars.peek().copied();
        let part = match code {
            Some('1' | 'L' | 'l' | 'V' | 'v') => Some(ArgPart::File),
            Some('*') if !has_file => Some(ArgPart::Files),
            Some('*') => None,
            Some('%') => {
                chars.next();
                text.push('%');
                continue;
            }
            Some(c) if c.is_ascii_alphanumeric() || c == '~' => None,
            _ => {
                text.push('%');
                continue;
            }
        };
        chars.next();
        if !text.is_empty() {
            parts.push(ArgPart::Text(std::mem::take(&mut text)));
        }
        match part {
            Some(part) => parts.push(part),
            None => dropped_code = true,
        }
    }
    if !text.is_empty() {
        parts.push(ArgPart::Text(text));
    }
    if parts.is_empty() && dropped_code {
        return None;
    }
    if parts.is_empty() {
        // An empty argument, written "", stays: a program may want it.
        parts.push(ArgPart::Text(String::new()));
    }
    Some(parts)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    fn env(name: &str) -> Option<String> {
        match name {
            "SystemRoot" => Some(r"C:\Windows".to_owned()),
            "ProgramFiles" => Some(r"C:\Program Files".to_owned()),
            _ => None,
        }
    }

    fn argv(command: &str, file: &str) -> Vec<String> {
        let parsed = parse_windows_command(command, &env).unwrap();
        let (program, args) = parsed.for_file(OsStr::new(file));
        std::iter::once(program)
            .chain(args)
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// The registry's command forms, each into a program and an argument
    /// list with the file put where `%1` is, whole.
    #[test]
    fn windows_commands_become_argument_lists() {
        let file = r"D:\My Files\report $1 (final).txt";
        assert_eq!(
            argv(r#""C:\Program Files\App\app.exe" "%1""#, file),
            [r"C:\Program Files\App\app.exe", file]
        );
        assert_eq!(
            argv(r"%SystemRoot%\system32\NOTEPAD.EXE %1", file),
            [r"C:\Windows\system32\NOTEPAD.EXE", file]
        );
        assert_eq!(
            argv(r"C:\Program Files\Old App\old.exe %1", file),
            [r"C:\Program Files\Old App\old.exe", file],
            "a path with spaces registered without quotes"
        );
        assert_eq!(
            argv(r#""C:\Tools\viewer.exe" /open "%1" /readonly"#, file),
            [r"C:\Tools\viewer.exe", "/open", file, "/readonly"]
        );
        assert_eq!(
            argv(r#""C:\Tools\viewer.exe" --file=%1"#, file),
            [r"C:\Tools\viewer.exe", &format!("--file={file}")]
        );
        assert_eq!(
            argv(r#""C:\Tools\viewer.exe""#, file),
            [r"C:\Tools\viewer.exe", file],
            "no %1: the file goes last"
        );
        assert_eq!(
            argv(r#""C:\Tools\viewer.exe" "%1" %*"#, file),
            [r"C:\Tools\viewer.exe", file],
            "%* beside %1 adds nothing for one file"
        );
        assert_eq!(
            argv(r#""C:\Tools\viewer.exe" %I "%L""#, file),
            [r"C:\Tools\viewer.exe", file],
            "another code alone is dropped"
        );
        assert_eq!(
            argv(
                r#"%SystemRoot%\System32\rundll32.exe "%ProgramFiles%\Windows Photo Viewer\PhotoViewer.dll", ImageView_Fullscreen %1"#,
                file
            ),
            [
                r"C:\Windows\System32\rundll32.exe",
                r"C:\Program Files\Windows Photo Viewer\PhotoViewer.dll,",
                "ImageView_Fullscreen",
                file
            ]
        );
        assert!(parse_windows_command("  ", &env).is_none());
        let parsed = parse_windows_command(r#""C:\a\Foxit Reader.EXE" "%1""#, &env).unwrap();
        assert_eq!(parsed.program_stem(), "Foxit Reader");
    }

    #[test]
    fn windows_quoting_follows_the_c_runtime() {
        assert_eq!(
            split_windows_args(r#"a "b c" d\"e "f""g" h\\"i j" k\\\"l"#),
            ["a", "b c", "d\"e", "f\"g", r"h\i j", r#"k\"l"#]
        );
        assert_eq!(split_windows_args(r#""" x"#), ["", "x"]);
        assert_eq!(split_windows_args(r"C:\dir\ x"), [r"C:\dir\", "x"]);
    }

    #[test]
    fn the_environment_expands_and_codes_stay() {
        assert_eq!(
            expand_environment(r"%SystemRoot%\x %1 %UNSET% 50% %*", &env),
            r"C:\Windows\x %1 %UNSET% 50% %*"
        );
    }
}
