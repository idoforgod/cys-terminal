param(
  [Parameter(Mandatory = $true)][string]$Target,
  [Parameter(Mandatory = $true)][string]$Thumbprint
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($Target -ne 'x86_64-pc-windows-msvc') {
  throw "Unsupported Windows Browser Runtime target: $Target"
}
if ($Thumbprint -notmatch '^[0-9A-Fa-f]{40}$') {
  throw 'Browser Runtime Authenticode thumbprint is invalid'
}

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$targetRoot = Join-Path $root "src-tauri/resources/browser-runtime/runtime/$Target"
if (-not (Test-Path -LiteralPath $targetRoot -PathType Container)) {
  throw "Browser Runtime target tree missing: $targetRoot"
}

$binaries = @(
  Get-ChildItem -LiteralPath $targetRoot -Recurse -File |
    Where-Object { $_.Name -like '*.exe' -or $_.Name -like '*.dll' } |
    Sort-Object -Property FullName
)
if ($binaries.Count -lt 3) {
  throw "Browser Runtime executable set is incomplete: $($binaries.Count)"
}
foreach ($binary in $binaries) {
  if (($binary.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Browser Runtime executable is a reparse point: $($binary.FullName)"
  }
}
$paths = @($binaries | ForEach-Object { $_.FullName })

& "$PSScriptRoot/windows-authenticode.ps1" -Mode Sign -Thumbprint $Thumbprint -Paths $paths
if ($LASTEXITCODE -ne 0) { throw 'Browser Runtime Authenticode signing failed' }
& "$PSScriptRoot/windows-authenticode.ps1" -Mode Verify -Thumbprint $Thumbprint -Paths $paths
if ($LASTEXITCODE -ne 0) { throw 'Browser Runtime Authenticode verification failed' }

Write-Host "Browser Runtime Authenticode signing verified: target=$Target files=$($paths.Count)"
