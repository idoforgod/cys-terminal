# diag/p0-facts.ps1 -- environment facts of the runner (context for QA..QF).
# Output: OUT\p0-facts.txt (human) + OUT\p0-facts.json (same keys). Every item is independent:
# a failure is written as 'FAILED: <message>' and the script goes on.
$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')
Start-DiagScript -Name 'p0-facts'

$FACTS = [ordered]@{}
$FACTTEXT = New-Object System.Text.StringBuilder

function Save-Facts {
    try { Save-Json 'p0-facts.json' $FACTS 6 } catch { }
    try { Save-Text 'p0-facts.txt' $FACTTEXT.ToString() } catch { }
}

function Add-Fact {
    param([string]$Key, [scriptblock]$Block)
    $val = $null
    try {
        $val = & $Block
    } catch {
        $val = 'FAILED: ' + $_.Exception.Message
        Write-Log ('fact {0} failed: {1}' -f $Key, $_.Exception.Message) 'WARN'
    }
    if ($null -eq $val) { $val = '(null)' }
    $FACTS[$Key] = $val
    [void]$FACTTEXT.AppendLine('== ' + $Key + ' ==')
    if ($val -is [string]) {
        [void]$FACTTEXT.AppendLine($val)
    } else {
        $txt = ''
        try { $txt = ConvertTo-Json -InputObject $val -Depth 5 } catch { $txt = ($val | Out-String -Width 400) }
        [void]$FACTTEXT.AppendLine($txt)
    }
    [void]$FACTTEXT.AppendLine('')
    Save-Facts
}

$sys32 = Join-Path $env:windir 'System32'

try {
    Add-Fact 'run_meta' {
        [ordered]@{
            started = (Get-IsoNow)
            runner_os = $env:RUNNER_OS
            runner_arch = $env:RUNNER_ARCH
            image_os = $env:ImageOS
            image_version = $env:ImageVersion
            github_run_id = $env:GITHUB_RUN_ID
            github_run_attempt = $env:GITHUB_RUN_ATTEMPT
            github_job = $env:GITHUB_JOB
            github_sha = $env:GITHUB_SHA
            runner_temp = $env:RUNNER_TEMP
            workspace = $env:GITHUB_WORKSPACE
            localappdata = $env:LOCALAPPDATA
            temp = $env:TEMP
            diag_job_limit_min = $env:DIAG_JOB_LIMIT_MIN
            time_zone = [System.TimeZoneInfo]::Local.Id
            ps_version = $PSVersionTable.PSVersion.ToString()
            ps_edition = [string]$PSVersionTable.PSEdition
            clr_version = [string]$PSVersionTable.CLRVersion
            language_mode = [string]$ExecutionContext.SessionState.LanguageMode
        }
    }

    Add-Fact 'ver' {
        Format-ProcText (Invoke-Proc -File (Join-Path $sys32 'cmd.exe') -Arguments '/c ver' -TimeoutSec 30)
    }

    Add-Fact 'nt_currentversion' {
        $k = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -ErrorAction Stop
        Select-Props $k @('ProductName', 'DisplayVersion', 'ReleaseId', 'CurrentBuild', 'CurrentBuildNumber', 'UBR', 'EditionID', 'InstallationType', 'BuildLabEx', 'CompositionEditionID')
    }

    Add-Fact 'architecture' {
        [ordered]@{
            PROCESSOR_ARCHITECTURE = $env:PROCESSOR_ARCHITECTURE
            PROCESSOR_ARCHITEW6432 = $env:PROCESSOR_ARCHITEW6432
            PROCESSOR_IDENTIFIER = $env:PROCESSOR_IDENTIFIER
            Is64BitOperatingSystem = [Environment]::Is64BitOperatingSystem
            Is64BitProcess = [Environment]::Is64BitProcess
            OSVersion = [Environment]::OSVersion.VersionString
            ProcessorCount = [Environment]::ProcessorCount
        }
    }

    Add-Fact 'whoami_groups' {
        $r = Invoke-Proc -File (Join-Path $sys32 'whoami.exe') -Arguments '/groups' -TimeoutSec 30
        $integrity = $null
        $m = [regex]::Match([string]$r.out, 'Mandatory Label\\([A-Za-z ]+ Mandatory Level)')
        if ($m.Success) { $integrity = $m.Groups[1].Value }
        $isAdmin = $null
        try {
            $id = [System.Security.Principal.WindowsIdentity]::GetCurrent()
            $pr = New-Object System.Security.Principal.WindowsPrincipal($id)
            $isAdmin = $pr.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)
        } catch { }
        ('is_admin_role={0} integrity={1} user={2}' -f $isAdmin, $integrity, [Environment]::UserName) + "`r`n" + (Format-ProcText $r)
    }

    Add-Fact 'user_interactive' {
        [ordered]@{
            UserInteractive = [Environment]::UserInteractive
            UserName = [Environment]::UserName
            UserDomainName = [Environment]::UserDomainName
            SESSIONNAME = $env:SESSIONNAME
            MachineName = [Environment]::MachineName
        }
    }

    Add-Fact 'query_session' {
        Format-ProcText (Invoke-Proc -File (Join-Path $sys32 'query.exe') -Arguments 'session' -TimeoutSec 30)
    }

    Add-Fact 'mp_status' {
        $s = Invoke-JobWithTimeout { Get-MpComputerStatus -ErrorAction Stop } 90
        Select-Props $s @('AMRunningMode', 'AMServiceEnabled', 'AntivirusEnabled', 'RealTimeProtectionEnabled', 'IsTamperProtected', 'SmartAppControlState', 'SmartAppControlExpiration', 'AMProductVersion', 'AMEngineVersion', 'AntivirusSignatureVersion', 'NISEnabled', 'OnAccessProtectionEnabled', 'BehaviorMonitorEnabled', 'IoavProtectionEnabled', 'DefenderSignaturesOutOfDate')
    }

    Add-Fact 'mp_status_full' {
        (Invoke-JobWithTimeout { Get-MpComputerStatus -ErrorAction Stop | Format-List * | Out-String -Width 300 } 90)
    }

    Add-Fact 'mp_preference' {
        $p = Invoke-JobWithTimeout { Get-MpPreference -ErrorAction Stop } 90
        Select-Props $p @('DisableRealtimeMonitoring', 'MAPSReporting', 'SubmitSamplesConsent', 'CloudBlockLevel', 'CloudExtendedTimeout', 'EnableNetworkProtection', 'PUAProtection', 'DisableBehaviorMonitoring', 'DisableIOAVProtection', 'DisableScriptScanning')
    }

    Add-Fact 'reg_ci_policy' {
        Format-ProcText (Invoke-Proc -File (Join-Path $sys32 'reg.exe') -Arguments 'query "HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy"' -TimeoutSec 30)
    }
    Add-Fact 'reg_ci_protected' {
        Format-ProcText (Invoke-Proc -File (Join-Path $sys32 'reg.exe') -Arguments 'query "HKLM\SYSTEM\CurrentControlSet\Control\CI\Protected"' -TimeoutSec 30)
    }
    Add-Fact 'reg_ci' {
        Format-ProcText (Invoke-Proc -File (Join-Path $sys32 'reg.exe') -Arguments 'query "HKLM\SYSTEM\CurrentControlSet\Control\CI"' -TimeoutSec 30)
    }
    Add-Fact 'reg_ci_testflags' {
        Format-ProcText (Invoke-Proc -File (Join-Path $sys32 'reg.exe') -Arguments 'query "HKLM\SYSTEM\CurrentControlSet\Control\CI" /v TestFlags' -TimeoutSec 30)
    }

    Add-Fact 'citool_lp_json' {
        Format-ProcText (Invoke-Proc -File (Join-Path $sys32 'CiTool.exe') -Arguments '-lp -json' -TimeoutSec 60)
    }
    Add-Fact 'citool_lp_text' {
        Format-ProcText (Invoke-Proc -File (Join-Path $sys32 'CiTool.exe') -Arguments '-lp' -TimeoutSec 60)
    }

    Add-Fact 'services' {
        $sc = Join-Path $sys32 'sc.exe'
        $sb = New-Object System.Text.StringBuilder
        foreach ($n in @('AppIDSvc', 'applockerfltr', 'appid', 'WinDefend', 'MDCoreSvc', 'SecurityHealthService', 'wscsvc')) {
            [void]$sb.AppendLine('##### ' + $n)
            [void]$sb.AppendLine((Format-ProcText (Invoke-Proc -File $sc -Arguments ('query ' + $n) -TimeoutSec 30)))
            [void]$sb.AppendLine((Format-ProcText (Invoke-Proc -File $sc -Arguments ('qc ' + $n) -TimeoutSec 30)))
        }
        $sb.ToString()
    }

    Add-Fact 'example_policies_folder' {
        $d = Join-Path $env:windir 'schemas\CodeIntegrity\ExamplePolicies'
        if (-not (Test-Path -LiteralPath $d)) { 'folder not found: ' + $d }
        else {
            $items = New-Object System.Collections.Generic.List[object]
            foreach ($f in @(Get-ChildItem -LiteralPath $d -File -ErrorAction Stop)) {
                $items.Add([ordered]@{ name = $f.Name; length = $f.Length; last_write_utc = (ConvertTo-IsoUtc $f.LastWriteTime) })
            }
            [ordered]@{ folder = $d; files = $items.ToArray() }
        }
    }

    Add-Fact 'example_policy_copy' {
        $src = Join-Path $env:windir 'schemas\CodeIntegrity\ExamplePolicies\SmartAppControl.xml'
        if (Test-Path -LiteralPath $src) {
            Copy-Item -LiteralPath $src -Destination (Join-Path $global:DiagOut 'p0-example-SmartAppControl.xml') -Force -ErrorAction Stop
            'copied SmartAppControl.xml to OUT\p0-example-SmartAppControl.xml'
        } else {
            'SmartAppControl.xml not found at ' + $src
        }
    }

    Add-Fact 'ci_policy_files' {
        $res = [ordered]@{}
        foreach ($d in @('System32\CodeIntegrity\CiPolicies\Active', 'System32\CodeIntegrity')) {
            $full = Join-Path $env:windir $d
            $names = New-Object System.Collections.Generic.List[string]
            if (Test-Path -LiteralPath $full) {
                foreach ($f in @(Get-ChildItem -LiteralPath $full -File -ErrorAction SilentlyContinue)) {
                    if ($f.Extension -in @('.cip', '.p7b', '.bin')) { $names.Add(('{0} ({1} bytes)' -f $f.Name, $f.Length)) }
                }
            } else { $names.Add('folder not found') }
            $res[$d] = $names.ToArray()
        }
        $res
    }

    Add-Fact 'ci_cmdlets' {
        $res = [ordered]@{}
        foreach ($n in @('ConvertFrom-CIPolicy', 'Set-RuleOption', 'Set-CIPolicyIdInfo', 'Set-CIPolicyVersion', 'New-CIPolicy', 'Merge-CIPolicy')) {
            $c = Get-Command -Name $n -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($c) { $res[$n] = ('{0} ({1})' -f $c.Name, $c.Source) } else { $res[$n] = 'none (not found)' }
        }
        $m = Get-Module -ListAvailable -Name ConfigCI -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($m) { $res['ConfigCI_module'] = ('{0} {1} {2}' -f $m.Name, $m.Version, $m.Path) } else { $res['ConfigCI_module'] = 'none (not found)' }
        $res
    }

    Add-Fact 'webview2_runtime' {
        $guid = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
        $res = [ordered]@{}
        $keys = @(
            ('HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\' + $guid),
            ('HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\' + $guid),
            ('HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\' + $guid)
        )
        foreach ($k in $keys) {
            $pv = $null
            try { $pv = (Get-ItemProperty -LiteralPath $k -Name pv -ErrorAction Stop).pv } catch { $pv = 'absent' }
            $res[$k] = $pv
        }
        foreach ($d in @('C:\Program Files (x86)\Microsoft\EdgeWebView\Application', 'C:\Program Files\Microsoft\EdgeWebView\Application')) {
            $names = @()
            if (Test-Path -LiteralPath $d) { $names = @(Get-ChildItem -LiteralPath $d -Directory -ErrorAction SilentlyContinue | ForEach-Object { $_.Name }) } else { $names = @('folder not found') }
            $res[$d] = ($names -join ', ')
        }
        $res
    }

    Add-Fact 'device_guard' {
        $o = Get-CimInstance -Namespace 'root\Microsoft\Windows\DeviceGuard' -ClassName Win32_DeviceGuard -OperationTimeoutSec 30 -ErrorAction Stop
        Select-Props $o @('SecurityServicesConfigured', 'SecurityServicesRunning', 'VirtualizationBasedSecurityStatus', 'CodeIntegrityPolicyEnforcementStatus', 'UsermodeCodeIntegrityPolicyEnforcementStatus', 'RequiredSecurityProperties', 'AvailableSecurityProperties')
    }

    Add-Fact 'secure_boot' {
        try { [string](Confirm-SecureBootUEFI -ErrorAction Stop) } catch { 'not available: ' + $_.Exception.Message }
    }

    Add-Fact 'ci_log' {
        $l = Get-WinEvent -ListLog 'Microsoft-Windows-CodeIntegrity/Operational' -ErrorAction Stop
        Select-Props $l @('LogName', 'IsEnabled', 'LogMode', 'RecordCount', 'FileSize', 'MaximumSizeInBytes')
    }

    Add-Fact 'appidtel_present' {
        $p = Join-Path $sys32 'appidtel.exe'
        ('{0} exists={1}' -f $p, (Test-Path -LiteralPath $p))
    }

    Add-Fact 'native_helper' {
        Initialize-DiagNative
        $st = Get-CodeIntegrityState
        [ordered]@{ ready = $global:DiagNativeReady; error = $global:DiagNativeError; code_integrity = $st }
    }

    Add-Fact 'execution_policy' {
        (Get-ExecutionPolicy -List | Out-String -Width 200)
    }

    Add-Fact 'tools' {
        $res = [ordered]@{}
        $node = Find-Exe 'node.exe'
        if ($node) { $res['node'] = (Format-ProcText (Invoke-Proc -File $node -Arguments '--version' -TimeoutSec 30)) } else { $res['node'] = 'not found' }
        $rustc = Find-Exe 'rustc.exe'
        if ($rustc) { $res['rustc'] = (Format-ProcText (Invoke-Proc -File $rustc -Arguments '-vV' -TimeoutSec 60)) } else { $res['rustc'] = 'not found' }
        $rustup = Find-Exe 'rustup.exe'
        if ($rustup) { $res['rustup_targets'] = (Format-ProcText (Invoke-Proc -File $rustup -Arguments 'target list --installed' -TimeoutSec 60)) } else { $res['rustup_targets'] = 'rustup not found' }
        $z = Find-Exe '7z.exe' @('C:\Program Files\7-Zip\7z.exe', 'C:\Program Files (x86)\7-Zip\7z.exe')
        if ($z) { $res['7z'] = $z } else { $res['7z'] = 'not found' }
        $curl = Join-Path $sys32 'curl.exe'
        if (Test-Path -LiteralPath $curl) { $res['curl'] = (Format-ProcText (Invoke-Proc -File $curl -Arguments '--version' -TimeoutSec 30)) } else { $res['curl'] = 'not found' }
        $res
    }

    Add-Fact 'systeminfo' {
        Format-ProcText (Invoke-Proc -File (Join-Path $sys32 'systeminfo.exe') -TimeoutSec 120)
    }
} catch {
    Add-DiagError 'p0-facts main' $_
} finally {
    $FACTS['_finished'] = (Get-IsoNow)
    Save-Facts
    Complete-DiagScript 'p0-facts'
}
exit 0
