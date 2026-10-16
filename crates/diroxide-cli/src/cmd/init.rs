//! `diro init SHELL`: prints the shell function that starts diro with
//! `--cwd-file` and moves the shell to the folder diro left it, so the
//! wrapper is always in step with the flag (R14, plan answer 74). Bash,
//! zsh and dash share one text; fish, PowerShell (7 and 5.1) and cmd have
//! their own.
//!
//! The function is called `dcd`; a user who wants another name changes
//! it in the startup file, and should not pick the name of a command that
//! exists. The wrapper moves the shell only when the file holds an
//! existing folder other than the current one, and always removes its
//! temporary file, which only ever holds one path.
//!
//! Exit codes: 0 printed, 2 wrong use.

use std::io::Write;

use crate::CliError;

/// A shell `diro init` writes a wrapper for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitShell {
    /// bash.
    Bash,
    /// zsh.
    Zsh,
    /// dash, and any POSIX shell with `local`.
    Dash,
    /// fish.
    Fish,
    /// PowerShell 7 and Windows PowerShell 5.1.
    PowerShell,
    /// cmd.exe, as a `dcd.cmd` file on the path.
    Cmd,
}

impl InitShell {
    /// Every shell, in the order the help names them.
    pub const ALL: [InitShell; 6] = [
        InitShell::Bash,
        InitShell::Zsh,
        InitShell::Dash,
        InitShell::Fish,
        InitShell::PowerShell,
        InitShell::Cmd,
    ];

    /// The name typed after `diro init`.
    pub const fn name(self) -> &'static str {
        match self {
            InitShell::Bash => "bash",
            InitShell::Zsh => "zsh",
            InitShell::Dash => "dash",
            InitShell::Fish => "fish",
            InitShell::PowerShell => "powershell",
            InitShell::Cmd => "cmd",
        }
    }

    /// The shell for a typed name, ignoring case: `sh` is dash's text and
    /// `pwsh` is PowerShell.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "bash" => Some(InitShell::Bash),
            "zsh" => Some(InitShell::Zsh),
            "dash" | "sh" => Some(InitShell::Dash),
            "fish" => Some(InitShell::Fish),
            "powershell" | "pwsh" => Some(InitShell::PowerShell),
            "cmd" => Some(InitShell::Cmd),
            _ => None,
        }
    }

    /// The wrapper's text.
    pub const fn wrapper(self) -> &'static str {
        match self {
            InitShell::Bash | InitShell::Zsh | InitShell::Dash => POSIX,
            InitShell::Fish => FISH,
            InitShell::PowerShell => POWERSHELL,
            InitShell::Cmd => CMD,
        }
    }
}

const POSIX: &str = r#"# diro's wrapper: dcd starts diro and moves the shell to the folder it left.
# Add this to your shell's startup file.
dcd() {
    local tmp dir rc
    tmp=$(mktemp -t diro-cwd.XXXXXX) || return 1
    command diro --tui --cwd-file "$tmp" "$@"
    rc=$?
    if [ -s "$tmp" ]; then
        dir=$(cat -- "$tmp")
        if [ -d "$dir" ] && [ "$dir" != "$PWD" ]; then
            cd -- "$dir" || :
        fi
    fi
    command rm -f -- "${tmp:?}"
    return "$rc"
}
"#;

const FISH: &str = r#"# diro's wrapper: dcd starts diro and moves the shell to the folder it left.
# Save this as ~/.config/fish/functions/dcd.fish.
function dcd
    set -l tmp (mktemp -t diro-cwd.XXXXXX)
    if test -z "$tmp"
        return 1
    end
    command diro --tui --cwd-file $tmp $argv
    set -l rc $status
    if test -s $tmp
        set -l dir (command cat -- $tmp)
        if test -d "$dir"; and test "$dir" != "$PWD"
            builtin cd -- "$dir"
        end
    end
    command rm -f -- $tmp
    return $rc
end
"#;

const POWERSHELL: &str = r#"# diro's wrapper: dcd starts diro and moves the shell to the folder it left.
# Add this to your PowerShell profile (7 or 5.1).
function dcd {
    $tmp = [System.IO.Path]::GetTempFileName()
    $rc = 0
    try {
        & diro.exe --tui --cwd-file $tmp @args
        $rc = $LASTEXITCODE
        $utf8 = [System.Text.UTF8Encoding]::new($false)
        $dir = [System.IO.File]::ReadAllText($tmp, $utf8)
        if ($dir -and (Test-Path -LiteralPath $dir -PathType Container) -and ($dir -ne $PWD.Path)) {
            Set-Location -LiteralPath $dir
        }
    }
    finally {
        Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
    }
    $global:LASTEXITCODE = $rc
}
"#;

const CMD: &str = r#"@echo off
rem diro's wrapper: save as dcd.cmd in a folder on the path.
set "DIRO_CWD_FILE=%TEMP%\diro-cwd-%RANDOM%%RANDOM%.txt"
diro.exe --tui --cwd-file "%DIRO_CWD_FILE%" %*
set "DIRO_RC=%ERRORLEVEL%"
set "DIRO_DIR="
if exist "%DIRO_CWD_FILE%" set /p DIRO_DIR=<"%DIRO_CWD_FILE%"
if defined DIRO_DIR if exist "%DIRO_DIR%\" cd /d "%DIRO_DIR%"
if exist "%DIRO_CWD_FILE%" del /q "%DIRO_CWD_FILE%"
set "DIRO_DIR="
set "DIRO_CWD_FILE="
(set "DIRO_RC=" & exit /b %DIRO_RC%)
"#;

/// What `diro init` was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitArgs {
    /// Print the wrapper for this shell.
    Shell(InitShell),
    /// Print the help.
    Help,
}

/// The help for `diro init`, one line each, meaning first.
pub fn help_text() -> String {
    [
        "diro init: print the shell function that moves the shell where diro left it.",
        "",
        "Usage:",
        "diro init SHELL   SHELL is bash, zsh, dash, fish, powershell or cmd",
        "",
        "The function is dcd: start diro with dcd, quit, and the shell is in",
        "the active panel's folder. bash, zsh, dash, fish and PowerShell: add",
        "the text to the shell's startup file. cmd: save it as dcd.cmd on the path.",
        "",
        "Exit codes:",
        "0  printed",
        "2  wrong use: no shell, or a shell diro does not know",
    ]
    .join("\n")
}

/// Reads the arguments after `init`.
pub fn parse(args: &[String]) -> Result<InitArgs, CliError> {
    match args {
        [] => Err(CliError::Missing("init".to_owned())),
        [flag] if flag == "--help" || flag == "-h" => Ok(InitArgs::Help),
        [name] => InitShell::from_name(name)
            .map(InitArgs::Shell)
            .ok_or_else(|| {
                CliError::Failed(format!(
                    "there is no wrapper for {name}; use bash, zsh, dash, fish, powershell or cmd"
                ))
            }),
        [_, ..] => Err(CliError::TooMany("init".to_owned())),
    }
}

/// Prints the wrapper or the help.
pub fn run(args: InitArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    let text = match args {
        InitArgs::Help => format!("{}\n", help_text()),
        InitArgs::Shell(shell) => shell.wrapper().to_owned(),
    };
    out.write_all(text.as_bytes())
        .map_err(|e| CliError::Failed(format!("the wrapper could not be written: {e}")))?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn every_shell_gets_a_wrapper_that_uses_the_flag() {
        for shell in InitShell::ALL {
            assert_eq!(InitShell::from_name(shell.name()), Some(shell));
            let mut out = Vec::new();
            assert_eq!(run(InitArgs::Shell(shell), &mut out), Ok(0));
            let text = String::from_utf8(out).unwrap();
            assert!(text.contains("--tui --cwd-file"), "{}", shell.name());
            assert!(text.contains("dcd"), "{}", shell.name());
            assert!(!text.contains('\t'), "{}", shell.name());
        }
        assert_eq!(InitShell::from_name("PWSH"), Some(InitShell::PowerShell));
        assert_eq!(InitShell::from_name("sh"), Some(InitShell::Dash));
    }

    #[test]
    fn arguments_are_read() {
        assert_eq!(
            parse(&args(&["fish"])),
            Ok(InitArgs::Shell(InitShell::Fish))
        );
        assert_eq!(parse(&args(&["--help"])), Ok(InitArgs::Help));
        assert_eq!(parse(&args(&[])), Err(CliError::Missing("init".into())));
        assert!(matches!(parse(&args(&["tcsh"])), Err(CliError::Failed(_))));
        assert_eq!(
            parse(&args(&["bash", "zsh"])),
            Err(CliError::TooMany("init".into()))
        );
        assert!(help_text().lines().all(|l| l.chars().count() <= 80));
    }
}
