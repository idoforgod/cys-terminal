$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $root

if ($env:CYS_WINDOWS_RELEASE_MODE -ne 'unsigned-acknowledged') {
  throw 'CYS_WINDOWS_RELEASE_MODE must be unsigned-acknowledged'
}
python scripts/release-credentials-check.py windows
if ($LASTEXITCODE -ne 0) { throw 'Windows unsigned release contract failed' }

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

# Windows Browser Runtime PE files intentionally remain Authenticode-unsigned.
# Their exact executable/tree hashes are still signed by the compiled minisign
# trust root before Tauri can bundle them.
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

# A plain `cargo build` omits Tauri's release features, while a full Tauri build
# temporarily patches the GUI only inside the NSIS copy and restores the
# unpatched executable afterwards. Compile once with Tauri's exact release
# settings but without bundling, then apply the same official UNK -> NSS marker
# substitution before hashing. `tauri bundle` consumes that already-built PE;
# the final comparison remains fail-closed if any later step changes it.
Remove-Item Env:\CYS_WINDOWS_GUI_SHA256 -ErrorAction SilentlyContinue
$configPath = Join-Path $env:RUNNER_TEMP 'tauri-release-windows-unsigned.json'
$config = @{
  bundle = @{
    createUpdaterArtifacts = $true
    targets = @('nsis')
  }
}
$config | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $configPath -Encoding UTF8
bunx '@tauri-apps/cli@2' build --no-bundle --target $target --config $configPath
if ($LASTEXITCODE -ne 0) { throw 'Tauri exact Windows GUI compile failed' }

$gui = (Resolve-Path "target\$target\release\cys-app.exe").Path
python scripts/patch-tauri-bundle-type.py --executable $gui --bundle-type nsis
if ($LASTEXITCODE -ne 0) { throw 'Tauri NSIS bundle marker patch failed' }
$guiHash = (Get-FileHash -LiteralPath $gui -Algorithm SHA256).Hash.ToLowerInvariant()
if ($guiHash -notmatch '^[0-9a-f]{64}$') { throw 'Unsigned Windows GUI SHA-256 is invalid' }
$env:CYS_WINDOWS_GUI_SHA256 = $guiHash

# Rebuild and restage cysd with the exact installed-GUI digest. Bundle the
# already-built, already-patched PE without invoking a second Cargo app build.
bash scripts/bundle-prep.sh
if ($LASTEXITCODE -ne 0) { throw 'hash-pinned bundle-prep failed' }
$nsisDir = "target\$target\release\bundle\nsis"
if (Test-Path -LiteralPath $nsisDir) {
  Remove-Item -LiteralPath $nsisDir -Recurse -Force
}
bunx '@tauri-apps/cli@2' bundle --bundles nsis --target $target --config $configPath
if ($LASTEXITCODE -ne 0) { throw 'Tauri unsigned Windows release bundle failed' }

$finalGuiHash = (Get-FileHash -LiteralPath $gui -Algorithm SHA256).Hash.ToLowerInvariant()
if ($finalGuiHash -ne $guiHash) {
  throw 'Unsigned Windows GUI hash drifted during Tauri build'
}

$setups = @(Get-ChildItem "target\$target\release\bundle\nsis\*-setup.exe")
if ($setups.Count -ne 1) { throw "Expected exactly one NSIS setup.exe, got $($setups.Count)" }
$setup = $setups[0].FullName
if (-not (Test-Path -LiteralPath "$setup.sig" -PathType Leaf)) {
  throw 'Unsigned Windows updater signature is missing'
}

$version = (Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
$candidate = Join-Path $root 'release-candidate'
New-Item -ItemType Directory -Force -Path $candidate | Out-Null
$normalized = "cys_${version}_x64-setup.exe"
Copy-Item -LiteralPath $setup -Destination (Join-Path $candidate $normalized)
Copy-Item -LiteralPath "$setup.sig" -Destination (Join-Path $candidate "$normalized.sig")
Write-Host "Windows unsigned candidate complete: $normalized (SHA-256 pinned in cysd)"
