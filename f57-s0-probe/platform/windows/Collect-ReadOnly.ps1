# F57-S0 template. Not executed by the preparing agent. Queries only; writes only the named evidence file.
[CmdletBinding()]
param(
  [Parameter(Mandatory=$true)][string]$RigRoot,
  [Parameter(Mandatory=$true)][string]$ProbeExe,
  [Parameter(Mandatory=$true)][string]$OutputFile,
  [string[]]$ExistingBinaries = @()
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($RigRoot).TrimEnd('\') + '\'
$output = [IO.Path]::GetFullPath($OutputFile)
if (-not $output.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) { throw 'Output must be below RigRoot' }
if (Test-Path -LiteralPath $output) { throw 'Use a fresh output filename; evidence is never overwritten' }
$probe = (Resolve-Path -LiteralPath $ProbeExe).Path
function Capture([scriptblock]$Action) {
  try { return @{ measured=$true; value=@(& $Action) } }
  catch { return @{ measured=$false; error=$_.Exception.Message } }
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
$signatures = foreach ($file in @($probe) + $ExistingBinaries) {
  Capture {
    $resolved = (Resolve-Path -LiteralPath $file).Path
    $sig = Get-AuthenticodeSignature -LiteralPath $resolved
    [pscustomobject]@{ path=$resolved; sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $resolved).Hash;
      status=[string]$sig.Status; status_message=$sig.StatusMessage;
      signer_thumbprint=if($sig.SignerCertificate){$sig.SignerCertificate.Thumbprint}else{$null} }
  }
}
$report = [ordered]@{
  schema='f57-s0-platform-preflight-v1'; observed_utc=[DateTime]::UtcNow.ToString('o');
  verdict='UNMEASURED'; requirement='NEEDS-ISOLATED-MACHINE';
  current_token_admin=$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator);
  account_non_admin_operator_verified=$null;
  powershell=$PSVersionTable.PSVersion.ToString();
  os=Capture { Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber,OSArchitecture };
  profiles=Capture { Get-NetConnectionProfile | Select-Object InterfaceAlias,InterfaceIndex,NetworkCategory,IPv4Connectivity,IPv6Connectivity };
  adapters=Capture { Get-NetAdapter -IncludeHidden | Select-Object Name,InterfaceDescription,ifIndex,Status,MacAddress,LinkSpeed };
  firewall_profiles=Capture { Get-NetFirewallProfile | Select-Object Name,Enabled,DefaultInboundAction,DefaultOutboundAction,NotifyOnListen };
  probe_rules=Capture { Get-NetFirewallApplicationFilter -Program $probe | Get-NetFirewallRule | Select-Object Name,Enabled,Direction,Action,Profile,PolicyStoreSourceType };
  winhttp_proxy=Capture { & "$env:SystemRoot\System32\netsh.exe" winhttp show proxy };
  signatures=@($signatures);
  ci_policies=Capture {
    $text = & "$env:SystemRoot\System32\CiTool.exe" -lp -json 2>&1
    $code = $LASTEXITCODE
    [pscustomobject]@{ exit_code=$code; raw=($text -join "`n") }
  }
}
$parent = Split-Path -Parent $output
if (-not (Test-Path -LiteralPath $parent -PathType Container)) { throw 'Create the evidence directory explicitly first' }
$report | ConvertTo-Json -Depth 12 | Out-File -LiteralPath $output -Encoding utf8
Write-Output $output
