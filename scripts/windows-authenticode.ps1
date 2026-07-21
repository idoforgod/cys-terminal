param(
  [Parameter(Mandatory = $true)]
  [ValidateSet('Import', 'Sign', 'Verify', 'Remove')]
  [string]$Mode,
  [string]$Thumbprint = '',
  [string[]]$Paths = @()
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Require-Env([string[]]$Names) {
  $missing = @($Names | Where-Object { [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($_)) })
  if ($missing.Count -gt 0) {
    throw "Missing release inputs: $($missing -join ', ')"
  }
}

function Resolve-SignTool {
  $tool = Get-Command signtool.exe -ErrorAction SilentlyContinue
  if (-not $tool) { throw 'signtool.exe was not found in PATH' }
  return $tool.Source
}

function Assert-Publisher($Certificate) {
  if ($Certificate.Subject -notlike "*$env:WINDOWS_EXPECTED_PUBLISHER*") {
    throw 'Authenticode publisher does not match WINDOWS_EXPECTED_PUBLISHER'
  }
}

if ($Mode -eq 'Import') {
  Require-Env @('WINDOWS_CERTIFICATE_B64', 'WINDOWS_CERTIFICATE_PASSWORD', 'WINDOWS_EXPECTED_PUBLISHER')
  $pfx = Join-Path $env:RUNNER_TEMP 'cys-release-signing.pfx'
  try {
    [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE_B64))
    $password = ConvertTo-SecureString $env:WINDOWS_CERTIFICATE_PASSWORD -AsPlainText -Force
    $certificate = Import-PfxCertificate -FilePath $pfx -CertStoreLocation 'Cert:\CurrentUser\My' -Password $password
    if (-not $certificate -or -not $certificate.HasPrivateKey) { throw 'Imported certificate has no private key' }
    Assert-Publisher $certificate
    if ($env:GITHUB_OUTPUT) {
      Add-Content -LiteralPath $env:GITHUB_OUTPUT -Value "thumbprint=$($certificate.Thumbprint)"
    }
    Write-Host 'Windows signing certificate imported (value redacted)'
  } finally {
    Remove-Item -LiteralPath $pfx -Force -ErrorAction SilentlyContinue
  }
  exit 0
}

if ($Mode -eq 'Remove') {
  if ($Thumbprint -match '^[0-9A-Fa-f]{40}$') {
    Remove-Item -LiteralPath "Cert:\CurrentUser\My\$Thumbprint" -Force -ErrorAction SilentlyContinue
  }
  exit 0
}

Require-Env @('WINDOWS_EXPECTED_PUBLISHER', 'WINDOWS_TIMESTAMP_URL')
if ($env:WINDOWS_TIMESTAMP_URL -notmatch '^https://') { throw 'WINDOWS_TIMESTAMP_URL must use https' }
if ($Thumbprint -notmatch '^[0-9A-Fa-f]{40}$') { throw 'A SHA-1 certificate thumbprint is required' }
$certificate = Get-Item -LiteralPath "Cert:\CurrentUser\My\$Thumbprint"
Assert-Publisher $certificate
$signTool = Resolve-SignTool
if ($Paths.Count -eq 0) { throw 'At least one PE path is required' }

foreach ($path in $Paths) {
  $resolved = (Resolve-Path -LiteralPath $path).Path
  if ($Mode -eq 'Sign') {
    & $signTool sign /sha1 $Thumbprint /fd SHA256 /tr $env:WINDOWS_TIMESTAMP_URL /td SHA256 $resolved
    if ($LASTEXITCODE -ne 0) { throw "signtool sign failed for $resolved" }
  }
  $signature = Get-AuthenticodeSignature -FilePath $resolved
  if ($signature.Status -ne 'Valid') { throw "Invalid Authenticode signature: $resolved ($($signature.Status))" }
  Assert-Publisher $signature.SignerCertificate
  if (-not $signature.TimeStamperCertificate) { throw "Missing RFC3161 timestamp: $resolved" }
  & $signTool verify /pa /all $resolved
  if ($LASTEXITCODE -ne 0) { throw "signtool verify failed for $resolved" }
}

Write-Host "Authenticode $Mode complete: $($Paths.Count) file(s)"
