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

  The full check runs the slow tests: it sets DIRO_SLOW to 1 for its own
  run unless it is already set (docs/dev/building.md, "Slow tests").
  DIRO_SLOW=0 runs the same check without them, as a builder's quick run
  does.

.EXAMPLE
  .\scripts\dev-check.ps1
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$previousSlow = $env:DIRO_SLOW
if ([string]::IsNullOrEmpty($env:DIRO_SLOW)) {
    $env:DIRO_SLOW = '1'
}
if ($env:DIRO_SLOW -eq '1') {
    Write-Output 'Slow tests: run, DIRO_SLOW is 1.'
}
else {
    Write-Output 'Slow tests: skipped, DIRO_SLOW is not 1.'
}

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
    $env:DIRO_SLOW = $previousSlow
    Pop-Location
}
