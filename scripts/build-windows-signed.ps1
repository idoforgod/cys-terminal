$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $root
python scripts/release-credentials-check.py windows
if ($LASTEXITCODE -ne 0) { throw 'Windows credential contract failed' }
if ($env:WINDOWS_CERTIFICATE_THUMBPRINT -notmatch '^[0-9A-Fa-f]{40}$') {
  throw 'WINDOWS_CERTIFICATE_THUMBPRINT is missing or invalid'
}

$target = 'x86_64-pc-windows-msvc'
$env:CYS_TARGET = $target
bash scripts/bundle-prep.sh
if ($LASTEXITCODE -ne 0) { throw 'bundle-prep failed' }

$sidecars = @(
  "target\$target\release\cys.exe",
  "target\$target\release\cysd.exe"
)
& scripts/windows-authenticode.ps1 -Mode Sign `
  -Thumbprint $env:WINDOWS_CERTIFICATE_THUMBPRINT -Paths $sidecars

$configPath = Join-Path $env:RUNNER_TEMP 'tauri-release-windows.json'
$config = @{
  bundle = @{
    createUpdaterArtifacts = $true
    targets = @('nsis')
    windows = @{
      certificateThumbprint = $env:WINDOWS_CERTIFICATE_THUMBPRINT
      digestAlgorithm = 'sha256'
      timestampUrl = $env:WINDOWS_TIMESTAMP_URL
      tsp = $true
    }
  }
}
$config | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $configPath -Encoding UTF8
bunx '@tauri-apps/cli@2' build --target $target --config $configPath
if ($LASTEXITCODE -ne 0) { throw 'Tauri Windows release build failed' }

$setups = @(Get-ChildItem "target\$target\release\bundle\nsis\*-setup.exe")
if ($setups.Count -ne 1) { throw "Expected exactly one NSIS setup.exe, got $($setups.Count)" }
$setup = $setups[0].FullName
& scripts/windows-authenticode.ps1 -Mode Verify `
  -Thumbprint $env:WINDOWS_CERTIFICATE_THUMBPRINT -Paths @($sidecars + $setup)

# Authenticode changes the installer bytes. Recreate the Tauri updater signature
# only after Authenticode + RFC3161 verification has succeeded.
bunx '@tauri-apps/cli@2' signer sign `
  --private-key $env:TAURI_SIGNING_PRIVATE_KEY --password '' $setup
if ($LASTEXITCODE -ne 0) { throw 'Windows updater signing failed' }

$version = (Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
$candidate = Join-Path $root 'release-candidate'
New-Item -ItemType Directory -Force -Path $candidate | Out-Null
$normalized = "cys_${version}_x64-setup.exe"
Copy-Item -LiteralPath $setup -Destination (Join-Path $candidate $normalized)
Copy-Item -LiteralPath "$setup.sig" -Destination (Join-Path $candidate "$normalized.sig")
Write-Host "Windows signed candidate complete: $normalized"
