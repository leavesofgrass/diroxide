<#
.SYNOPSIS
  Removes files or folders safely, by reference, inside one allowed root.

.DESCRIPTION
  The one way agents remove files in diroxide (the owner's rule, Tuesday,
  September 29, 2026). Shell commands cannot be trusted to escape names:
  on Saturday, September 26, 2026 a shell delete meant for a folder named
  "C" plus a private-use character became "C:" and erased most of drive C.

  This script never takes a typed name to delete. It lists the children of
  -Root (recursively only with -Recurse), selects them with -NameLike (a
  PowerShell wildcard on the name, matched with -like) or -CharCode (a
  character code the name contains, for names with hidden characters), and:
    1. refuses unless -Root is inside D:\diroxide or
       D:\diroxide-planning, and is not one of those roots itself;
    2. prints each match with its full path, spelling out hidden characters
       as <U+XXXX>, its kind, and its size;
    3. without -Apply, stops there (a dry run is the default);
    4. with -Apply, checks each match again is strictly inside -Root and
       removes that object by reference (Remove-Item -LiteralPath on the
       object's own full path).

.EXAMPLE
  # Dry run: what would be removed.
  .\safe-remove.ps1 -Root D:\diroxide\target -NameLike 'w5*'

.EXAMPLE
  # A name with a hidden character, found by its code.
  .\safe-remove.ps1 -Root D:\diroxide\.claude\worktrees\agent-x -CharCode 0xF03A -Apply
#>
param(
  [Parameter(Mandatory = $true)] [string] $Root,
  [string] $NameLike,
  [int] $CharCode = -1,
  [switch] $Recurse,
  [switch] $Apply,
  [int] $ExpectCount = -1
)
$ErrorActionPreference = 'Stop'

function Show-Name([string] $s) {
  -join ($s.ToCharArray() | ForEach-Object {
    $c = [int]$_
    if ($c -lt 32 -or ($c -ge 0xE000 -and $c -le 0xF8FF)) { '<U+{0:X4}>' -f $c } else { $_ }
  })
}

if (-not $NameLike -and $CharCode -lt 0) { throw 'Give -NameLike or -CharCode.' }
$rootFull = [IO.Path]::GetFullPath($Root).TrimEnd('\')
$allowed = @('D:\diroxide', 'D:\diroxide-planning')
if ($allowed -contains $rootFull) { throw "Refused: -Root may not be an allowed folder itself ($rootFull)." }
$inside = $false
foreach ($a in $allowed) { if ($rootFull.StartsWith($a + '\', [StringComparison]::OrdinalIgnoreCase)) { $inside = $true } }
if (-not $inside) { throw "Refused: $rootFull is not inside D:\diroxide or D:\diroxide-planning." }
if (-not (Test-Path -LiteralPath $rootFull -PathType Container)) { throw "Refused: $rootFull is not a folder." }

$items = if ($Recurse) { Get-ChildItem -LiteralPath $rootFull -Force -Recurse } else { Get-ChildItem -LiteralPath $rootFull -Force }
$found = @($items | Where-Object {
  $ok = $true
  if ($NameLike) { $ok = $ok -and ($_.Name -like $NameLike) }
  if ($CharCode -ge 0) { $ok = $ok -and (@($_.Name.ToCharArray() | Where-Object { [int]$_ -eq $CharCode }).Count -gt 0) }
  $ok
})
# A folder that is itself inside another match is removed with it.
$found = @($found | Where-Object { $m = $_; -not ($found | Where-Object { $_.PSIsContainer -and $m.FullName.StartsWith($_.FullName + '\') }) })

"Root: $rootFull"
"Matches: $($found.Count)"
foreach ($m in $found) {
  $size = if ($m.PSIsContainer) { (Get-ChildItem -LiteralPath $m.FullName -Recurse -Force -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum } else { $m.Length }
  $kind = if ($m.PSIsContainer) { 'folder' } else { 'file' }
  "  $kind  $(Show-Name $m.FullName)  $([math]::Round(($size + 0) / 1MB, 2)) MB"
}
if ($ExpectCount -ge 0 -and $found.Count -ne $ExpectCount) { throw "Stopped: expected $ExpectCount matches, found $($found.Count). Nothing removed." }
if (-not $Apply) { 'Dry run: nothing removed. Run again with -Apply to remove exactly these.'; return }

foreach ($m in $found) {
  $full = [IO.Path]::GetFullPath($m.FullName)
  if (-not $full.StartsWith($rootFull + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "Stopped: $(Show-Name $full) is not strictly inside the root. Nothing more removed." }
  if ($m.PSIsContainer) { Remove-Item -LiteralPath $full -Recurse -Force -Confirm:$false } else { Remove-Item -LiteralPath $full -Force -Confirm:$false }
  "Removed: $(Show-Name $full)"
}
