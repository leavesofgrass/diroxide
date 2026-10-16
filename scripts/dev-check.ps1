<#
.SYNOPSIS
  Runs the full native check of the workspace, in order, and stops at the
  first failure.

.DESCRIPTION
  Each step says its name first; the last line says Pass or Fail in words.
  Works from any folder; runs in the repository's root so the pinned
  toolchain and the cargo xtask alias apply, and returns to the folder it
  started in. Set CARGO_TARGET_DIR first when several builders share the
  machine.

.EXAMPLE
  .\scripts\dev-check.ps1
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

function Invoke-Step {
    param([string] $Name, [scriptblock] $Command)
    Write-Output "Check: $Name"
    # Cargo writes its progress to standard error. Windows PowerShell 5.1
    # turns those lines into errors when output is redirected, so a step is
    # judged by its exit code alone.
    $ErrorActionPreference = 'Continue'
    & $Command 2>&1 | ForEach-Object { "$_" }
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 0) {
        Write-Output "Fail: $Name"
        exit 1
    }
}

Push-Location -LiteralPath $root
try {
    Invoke-Step 'format' { cargo fmt --all --check }
    Invoke-Step 'clippy, warnings denied' { cargo clippy --workspace --all-targets --all-features -- -D warnings }
    Invoke-Step 'tests' { cargo nextest run --workspace --all-features }
    $previous = $env:RUSTDOCFLAGS
    $env:RUSTDOCFLAGS = '-D warnings'
    try {
        Invoke-Step 'rustdoc, warnings denied' { cargo doc --workspace --no-deps }
    }
    finally {
        $env:RUSTDOCFLAGS = $previous
    }
    Invoke-Step 'dependency direction' { cargo xtask deps --check }
    Invoke-Step 'pure Rust, no C build' { cargo xtask native --check }
    Invoke-Step 'keyboard page' { cargo xtask keyboard --check }
    Invoke-Step 'generated files' { cargo xtask regen --check }
    Write-Output 'Pass: every check.'
}
finally {
    Pop-Location
}
