$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $root
python scripts/release-credentials-check.py windows
if ($LASTEXITCODE -ne 0) { throw 'Windows credential contract failed' }

# Explicit, auditable opt-out (ALLOW_UNSIGNED_WINDOWS='1' repository variable) for the
# owner's unsigned distribution model: unsigned exe + zip + Windows Defender guidance.
# Authenticode steps are skipped; the Tauri updater .sig (minisign-family) and the
# Browser Runtime minisign trust metadata are still produced identically, so the
# assembled asset set (cys_{v}_x64-setup.exe + .sig) is unchanged.
$unsigned = $env:ALLOW_UNSIGNED_WINDOWS -eq '1'
if ($unsigned) {
  Write-Host 'UNSIGNED WINDOWS RELEASE (explicit opt-out): skipping Authenticode signing'
} elseif ($env:WINDOWS_CERTIFICATE_THUMBPRINT -notmatch '^[0-9A-Fa-f]{40}$') {
  throw 'WINDOWS_CERTIFICATE_THUMBPRINT is missing or invalid'
}

$target = 'x86_64-pc-windows-msvc'
$env:CYS_TARGET = $target
foreach ($name in @(
  'CYS_BROWSER_RUNTIME_SECRET_KEY',
  'CYS_BROWSER_RUNTIME_PUBLIC_KEY',
  'CYS_BROWSER_RUNTIME_KEY_ID',
  'CYS_BROWSER_RUNTIME_POLICY_EPOCH',
  'CYS_BROWSER_RUNTIME_EXPIRES_AT'
)) {
  if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
    throw "Browser Runtime release metadata variable missing: $name"
  }
}
# Authenticode mutates PE bytes. Sign and verify the entire staged executable
# set before final hashes, attestation and policy minisign metadata are emitted.
# (Unsigned opt-out: skip Authenticode; the minisign metadata below still runs.)
if (-not $unsigned) {
  & scripts/runtime-stage-sign-windows.ps1 -Target $target -Thumbprint $env:WINDOWS_CERTIFICATE_THUMBPRINT
  if ($LASTEXITCODE -ne 0) { throw 'Browser Runtime Authenticode stage failed' }
}
python scripts/browser-runtime-metadata.py prepare `
  --resource-root src-tauri/resources/browser-runtime `
  --target $target `
  --key-id $env:CYS_BROWSER_RUNTIME_KEY_ID `
  --secret-key $env:CYS_BROWSER_RUNTIME_SECRET_KEY `
  --public-key $env:CYS_BROWSER_RUNTIME_PUBLIC_KEY `
  --trusted-keys cysjavis-pack/trusted-keys.json `
  --tauri-config src-tauri/tauri.conf.json `
  --policy-epoch $env:CYS_BROWSER_RUNTIME_POLICY_EPOCH `
  --expires-at $env:CYS_BROWSER_RUNTIME_EXPIRES_AT
if ($LASTEXITCODE -ne 0) { throw 'Browser Runtime metadata signing/verification failed' }
$env:CYS_BROWSER_V2_RELEASE_QUALIFIED = '1'
bash scripts/bundle-prep.sh
if ($LASTEXITCODE -ne 0) { throw 'bundle-prep failed' }

$sidecars = @(
  "target\$target\release\cys.exe",
  "target\$target\release\cysd.exe"
)
if (-not $unsigned) {
  & scripts/windows-authenticode.ps1 -Mode Sign `
    -Thumbprint $env:WINDOWS_CERTIFICATE_THUMBPRINT -Paths $sidecars
}

# NSIS + updater artifacts are produced in both modes. The Authenticode signing
# block (certificateThumbprint/timestampUrl/tsp) is included only in signed mode;
# omitting it in unsigned mode makes Tauri emit an unsigned NSIS installer.
$configPath = Join-Path $env:RUNNER_TEMP 'tauri-release-windows.json'
$bundle = @{
  createUpdaterArtifacts = $true
  targets = @('nsis')
}
if (-not $unsigned) {
  $bundle.windows = @{
    certificateThumbprint = $env:WINDOWS_CERTIFICATE_THUMBPRINT
    digestAlgorithm = 'sha256'
    timestampUrl = $env:WINDOWS_TIMESTAMP_URL
    tsp = $true
  }
}
$config = @{ bundle = $bundle }
$config | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $configPath -Encoding UTF8
bunx '@tauri-apps/cli@2' build --target $target --config $configPath
if ($LASTEXITCODE -ne 0) { throw 'Tauri Windows release build failed' }

$setups = @(Get-ChildItem "target\$target\release\bundle\nsis\*-setup.exe")
if ($setups.Count -ne 1) { throw "Expected exactly one NSIS setup.exe, got $($setups.Count)" }
$setup = $setups[0].FullName
if (-not $unsigned) {
  & scripts/windows-authenticode.ps1 -Mode Verify `
    -Thumbprint $env:WINDOWS_CERTIFICATE_THUMBPRINT -Paths @($sidecars + $setup)
}

# Authenticode changes the installer bytes. Recreate the Tauri updater signature
# only after Authenticode + RFC3161 verification has succeeded (signed mode). In
# unsigned mode the installer bytes are final, so the updater .sig is created directly.
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
