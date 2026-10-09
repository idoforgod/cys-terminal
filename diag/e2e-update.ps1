# diag/e2e-update.ps1 -- QA: does the installed 0.14.37 app update itself to 0.14.42 through its own update path?
#   1. install 0.14.37 with /S           2. start cys-app.exe with a WebView2 remote debugging port
#   3. node diag/cdp-update.mjs: invoke('check_update') and invoke('install_update', {force:true}) over CDP
#   4. observe (timeline, screenshots)   5. verdict -> OUT\e2e-verdict.json
#   6. fallback (CDP could not attach): replica of the plugin call (se_exit.exe) instead of the app.
# This file is also a LIBRARY: when dot-sourced (. e2e-update.ps1) it only defines functions
# (Install-CysVersion, Invoke-AppUpdateRun, Watch-UpdateOutcome, ...) and runs no main.
# Invoke-AppUpdateRun has an optional -BeforeNode script block (a step between "CDP answers" and "install_update is driven");
# sacreal-e2e.ps1 uses it to turn REAL Smart App Control on while the app is already running. Without it nothing changes.
$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')

function Get-InstallDir { return (Join-Path $env:LOCALAPPDATA 'cys') }

function Get-InstalledMarker {
    try {
        $t = Read-TextUtf8 (Join-Path (Get-InstallDir) 'cys-installed-version.txt')
        if ($null -ne $t) { return $t.Trim() }
    } catch { }
    return $null
}

function Get-FailureMarker {
    try { return (Read-TextUtf8 (Join-Path (Get-InstallDir) 'cys-install-failure.txt')) } catch { return $null }
}

function Get-AppFileVersion {
    try {
        $p = Join-Path (Get-InstallDir) 'cys-app.exe'
        if (Test-Path -LiteralPath $p) {
            $vi = (Get-Item -LiteralPath $p).VersionInfo
            return ('{0} / product {1}' -f $vi.FileVersion, $vi.ProductVersion)
        }
    } catch { }
    return $null
}

function Get-UpdaterTempInfo {
    $res = New-Object System.Collections.Generic.List[object]
    try {
        $tmp = [System.IO.Path]::GetTempPath()
        foreach ($d in @(Get-ChildItem -LiteralPath $tmp -Directory -Filter 'cys-*-updater-*' -ErrorAction SilentlyContinue)) {
            $files = @(Get-ChildItem -LiteralPath $d.FullName -File -Recurse -ErrorAction SilentlyContinue)
            $bytes = [int64]0
            $names = New-Object System.Collections.Generic.List[string]
            foreach ($f in $files) { $bytes = $bytes + [int64]$f.Length; $names.Add([string]$f.Name) }
            $res.Add([pscustomobject]@{ name = [string]$d.Name; files = ($names -join ','); bytes = $bytes })
        }
    } catch { }
    return $res.ToArray()
}

# WebView2 Runtime >= 150 ignores WEBVIEW2_* environment variables AND HKCU policy overrides when the host process is
# elevated (High IL). GitHub-hosted Windows runners run as an elevated admin (UAC off). Microsoft Learn ("Develop secure
# WebView2 apps" -> "For an elevated host app, use appropriate override flags") says HKLM policy overrides are honored:
#   HKLM\SOFTWARE\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments   value name = exe name (or *), data = flags
# So the harness sets BOTH the environment variable (older runtimes) and this HKLM policy value (value name cys-app.exe),
# and removes the value again afterwards.
function Set-WebView2DebugPolicy {
    param([int]$Port)
    $key = 'HKLM\SOFTWARE\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments'
    $reg = Join-Path $env:windir 'System32\reg.exe'
    $o = [ordered]@{ key = $key; value_name = 'cys-app.exe'; key_existed = $null; old_value_present = $null; old_value_query = $null; set_rc = $null; set_out = $null; set_err = $null; verify_out = $null }
    try {
        $q = Invoke-Proc -File $reg -Arguments ('query "{0}"' -f $key) -TimeoutSec 30
        $o['key_existed'] = ($q.rc -eq 0)
        $q2 = Invoke-Proc -File $reg -Arguments ('query "{0}" /v cys-app.exe' -f $key) -TimeoutSec 30
        $o['old_value_present'] = ($q2.rc -eq 0)
        $o['old_value_query'] = ([string]$q2.out).Trim()
        $data = '--remote-debugging-port={0} --remote-allow-origins=*' -f $Port
        $a = Invoke-Proc -File $reg -Arguments ('add "{0}" /v cys-app.exe /t REG_SZ /d "{1}" /f' -f $key, $data) -TimeoutSec 30
        $o['set_rc'] = $a.rc
        $o['set_out'] = ([string]$a.out).Trim()
        $o['set_err'] = ([string]$a.err).Trim()
        $q3 = Invoke-Proc -File $reg -Arguments ('query "{0}" /v cys-app.exe' -f $key) -TimeoutSec 30
        $o['verify_out'] = ([string]$q3.out).Trim()
    } catch {
        $o['set_err'] = 'exception: ' + $_.Exception.Message
    }
    return $o
}

function Remove-WebView2DebugPolicy {
    param($State)
    if ($null -eq $State) { return $null }
    $key = [string]$State['key']
    $reg = Join-Path $env:windir 'System32\reg.exe'
    $o = [ordered]@{ value_deleted_rc = $null; key_deleted_rc = $null; note = $null }
    try {
        if ($State['old_value_present']) {
            $o['note'] = 'a cys-app.exe value existed before (left overwritten: original data is in old_value_query)'
        } else {
            $d = Invoke-Proc -File $reg -Arguments ('delete "{0}" /v cys-app.exe /f' -f $key) -TimeoutSec 30
            $o['value_deleted_rc'] = $d.rc
        }
        if (-not $State['key_existed']) {
            $k = Invoke-Proc -File $reg -Arguments ('delete "{0}" /f' -f $key) -TimeoutSec 30
            $o['key_deleted_rc'] = $k.rc
        }
    } catch {
        $o['note'] = 'exception: ' + $_.Exception.Message
    }
    return $o
}

# version marker reached the target (equal, or a newer parseable version: if releases/latest moved on, the app installs that)
function Test-VersionReached {
    param([string]$Marker, [string]$Target)
    if (-not $Marker) { return $false }
    if ($Marker -eq $Target) { return $true }
    try { return ([version]$Marker -ge [version]$Target) } catch { return $false }
}

# An installer process of the update flow: the plugin's temp installer (%TEMP%\cys-<ver>-updater-<rand>\...),
# a cys-named installer/setup, the NSIS relaunch helper (~nsu.tmp\Au_.exe), or a setup-like exe running from a Temp folder.
# (A generic name match on 'installer' would hit TrustedInstaller.exe, a Windows servicing process.)
function Test-InstallerLike {
    param([string]$Name, [string]$Path)
    if ($Path -like '*\cys-*-updater-*\*') { return $true }
    if ($Path -like '*\~nsu.tmp\*') { return $true }
    if ($Name -match '^cys[-_].*(installer|setup)') { return $true }
    if (($Path -like '*\Temp\*') -and ($Name -match 'setup|install|update|uninst|^au_')) { return $true }
    return $false
}

function Get-InterestingProcesses {
    $res = New-Object System.Collections.Generic.List[object]
    try {
        $all = Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop
        foreach ($p in $all) {
            $n = [string]$p.Name
            $path = [string]$p.ExecutablePath
            $hit = $false
            if (@('cys-app.exe', 'cysd.exe', 'cys.exe') -contains $n.ToLowerInvariant()) { $hit = $true }
            elseif (Test-InstallerLike $n $path) { $hit = $true }
            elseif ($path -like '*\Local\cys\*') { $hit = $true }
            if ($hit) {
                $res.Add([pscustomobject]@{ name = $n; id = [int]$p.ProcessId; ppid = [int]$p.ParentProcessId; path = $path; cmd = [string]$p.CommandLine })
            }
        }
    } catch { }
    return $res.ToArray()
}

function Get-ProcessListText {
    $sb = New-Object System.Text.StringBuilder
    try {
        foreach ($p in @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop | Sort-Object Name)) {
            $cmd = [string]$p.CommandLine
            $cmd = [regex]::Replace($cmd, '(?i)(token|password|passwd|secret|apikey|api-key|authorization)([=:\s]+)\S+', '$1$2<redacted>')
            if ($cmd.Length -gt 240) { $cmd = $cmd.Substring(0, 240) + '...' }
            [void]$sb.AppendLine(('{0}  pid={1} ppid={2}  path={3}  cmd={4}' -f $p.Name, $p.ProcessId, $p.ParentProcessId, $p.ExecutablePath, $cmd))
        }
    } catch { [void]$sb.AppendLine('Get-ProcessListText failed: ' + $_.Exception.Message) }
    return $sb.ToString()
}

# ---------------------------------------------------------------------------
# extra evidence after an update attempt: app-owned files (listing + small logs), Application event log tail
# ---------------------------------------------------------------------------
function Save-AppEvidence {
    param([string]$Prefix)
    $o = [ordered]@{ roots = @(); copied = @(); error = $null }
    try {
        $sb = New-Object System.Text.StringBuilder
        $roots = New-Object System.Collections.Generic.List[string]
        foreach ($base in @($env:LOCALAPPDATA, $env:APPDATA)) {
            if ($base -and (Test-Path -LiteralPath $base)) {
                foreach ($d in @(Get-ChildItem -LiteralPath $base -Directory -ErrorAction SilentlyContinue)) {
                    if ($d.Name -match 'cys') { $roots.Add($d.FullName) }
                }
            }
        }
        $o['roots'] = $roots.ToArray()
        $copied = New-Object System.Collections.Generic.List[string]
        $n = 0
        $cutoff = (Get-Date).AddMinutes(-90)
        foreach ($root in $roots) {
            [void]$sb.AppendLine('### ' + $root + '  (newest 300 files)')
            $files = @(Get-ChildItem -LiteralPath $root -Recurse -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 300)
            foreach ($f in $files) {
                [void]$sb.AppendLine(('{0}  {1,12}  {2}' -f (ConvertTo-IsoUtc $f.LastWriteTime), $f.Length, $f.FullName.Substring($root.Length)))
                $ext = [string]$f.Extension
                if ((($n -lt 15) -or ($f.Name -eq 'office-bridge.log')) -and ($f.Length -le 204800) -and (($ext -eq '.log') -or ($ext -eq '.txt')) -and ($f.LastWriteTime -gt $cutoff)) {
                    $n++
                    $dest = Join-Path $global:DiagOut ('{0}-appfile-{1:D2}-{2}' -f $Prefix, $n, $f.Name)
                    try { Copy-Item -LiteralPath $f.FullName -Destination $dest -Force -ErrorAction Stop; $copied.Add([string]$f.FullName) } catch { }
                }
            }
        }
        $o['copied'] = $copied.ToArray()
        Save-Text ($Prefix + '-appfiles.txt') $sb.ToString()
    } catch {
        $o['error'] = $_.Exception.Message
    }
    return $o
}

function Save-AppEventLog {
    param([string]$Prefix, [datetime]$SinceUtc)
    try {
        $evs = @(Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = $SinceUtc.ToUniversalTime().ToLocalTime() } -MaxEvents 200 -ErrorAction Stop)
        $lines = New-Object System.Collections.Generic.List[string]
        foreach ($e in $evs) {
            $m = ''
            try { $m = ([string]$e.Message) -replace '\s+', ' ' } catch { }
            if ($m.Length -gt 300) { $m = $m.Substring(0, 300) + '...' }
            $lines.Add(('{0} lvl={1} id={2} prov={3} :: {4}' -f (ConvertTo-IsoUtc $e.TimeCreated), $e.Level, $e.Id, $e.ProviderName, $m))
        }
        Save-Text ($Prefix + '-eventlog-application.txt') ($lines -join "`r`n")
    } catch {
        Save-Text ($Prefix + '-eventlog-application.txt') ('none or failed: ' + $_.Exception.Message)
    }
}

# ---------------------------------------------------------------------------
# silent install of a released NSIS installer (same way as the existing CI: /S, then 6 s)
# ---------------------------------------------------------------------------
function Install-CysVersion {
    param([string]$Version)
    $info = [ordered]@{ version = $Version; installer = $null; started = (Get-IsoNow); rc = $null; timed_out = $false; marker = $null; failure_file = $null; file_version = $null; killed = @(); error = $null }
    try {
        $inst = Get-AssetInstaller $Version
        $info['installer'] = $inst
        if (-not (Test-Path -LiteralPath $inst)) { $info['error'] = 'installer file missing (p1-assets failed?)'; return $info }
        if (-not (Test-AssetUsable $Version)) { $info['error'] = 'installer marked unusable by p1-assets (SHA256 mismatch or download failure)'; return $info }
        $p = Start-Process -FilePath $inst -ArgumentList '/S' -PassThru -ErrorAction Stop
        $null = $p.Handle
        if (-not $p.WaitForExit(420000)) {
            $info['timed_out'] = $true
            Stop-ProcessTree -ProcessId $p.Id
        } else {
            $p.WaitForExit()
        }
        try { $info['rc'] = $p.ExitCode } catch { }
        Start-Sleep -Seconds 6
        $info['marker'] = Get-InstalledMarker
        $info['failure_file'] = Get-FailureMarker
        $info['file_version'] = Get-AppFileVersion
        $info['killed'] = @(Stop-ProcessesUnder (Get-InstallDir))
    } catch {
        $info['error'] = Format-ErrorText $_
    }
    $info['finished'] = (Get-IsoNow)
    return $info
}

# ---------------------------------------------------------------------------
# observation loop: timeline every 5 s, screenshots, installer detection, end conditions
# ---------------------------------------------------------------------------
function Watch-UpdateOutcome {
    param(
        [string]$Prefix,
        [string]$TargetVersion,
        $NodeProc = $null,
        [int]$PostNodeSec = 480,
        [int]$QuietEndSec = 180,
        [int]$MaxSec = 1500,
        [int]$TickSec = 5,
        [switch]$ShotAtExit,
        [int]$ShotEverySec = 0
    )
    $obs = [ordered]@{
        started = (Get-IsoNow); ended = $null; end_reason = $null; ticks = 0
        node_exited_at = $null; node_exit_code = $null
        app_alive_first_at = $null; app_gone_at = $null; app_relaunched = $false
        installer_seen = $false; installer_first_seen_at = $null; installer_first_seen = $null
        marker_first = $null; marker_last = $null; marker_reached_target_at = $null
        file_version_last = $null; updater_dir_max_bytes = 0; updater_dirs_seen = @()
        failure_file = $null; screenshots = (New-Object System.Collections.Generic.List[object])
    }
    $tl = Join-Path $global:DiagOut ($Prefix + '-timeline.txt')
    try { Add-Utf8NoBom $tl (('# timeline start {0} target={1}' -f (Get-IsoNow), $TargetVersion) + "`r`n") } catch { }
    $t0 = Get-Date
    $lastActivity = $t0
    $nodeExitAt = $null
    if ($null -eq $NodeProc) { $nodeExitAt = $t0 }
    $lastBytes = [int64]-1
    $prevAppAlive = $false
    $appEverAlive = $false
    $appGone = $false
    $shot20 = $false
    $shot60 = $false
    $shotInst = $false
    $shotExit = $false
    $lastPeriodicShot = 0
    $reason = $null
    $first = $true
    $seenDirs = New-Object System.Collections.Generic.List[string]
    try {
        while ($true) {
            $now = Get-Date
            $elapsed = [int]($now - $t0).TotalSeconds

            if (($null -ne $NodeProc) -and ($null -eq $nodeExitAt)) {
                $exited = $false
                try { $exited = $NodeProc.HasExited } catch { $exited = $true }
                if ($exited) {
                    $nodeExitAt = $now
                    $lastActivity = $now
                    $obs['node_exited_at'] = ConvertTo-IsoUtc $now
                    try { $obs['node_exit_code'] = $NodeProc.ExitCode } catch { }
                }
            }

            $procs = @(Get-InterestingProcesses)
            $marker = Get-InstalledMarker
            $fv = Get-AppFileVersion
            $upd = @(Get-UpdaterTempInfo)
            $fail = Get-FailureMarker

            $appAlive = $false
            $installerAlive = $false
            $installerProc = $null
            foreach ($p in $procs) {
                if (($p.name -ieq 'cys-app.exe') -and ($p.path -like '*\Local\cys\*')) { $appAlive = $true }
                $isInst = Test-InstallerLike ([string]$p.name) ([string]$p.path)
                if ($isInst) { $installerAlive = $true; if ($null -eq $installerProc) { $installerProc = $p } }
            }
            $bytes = [int64]0
            foreach ($u in $upd) {
                $bytes = $bytes + [int64]$u.bytes
                if (-not $seenDirs.Contains([string]$u.name)) { $seenDirs.Add([string]$u.name) }
            }
            if ($bytes -gt [int64]$obs['updater_dir_max_bytes']) { $obs['updater_dir_max_bytes'] = $bytes }

            if ($first) { $obs['marker_first'] = $marker; $first = $false }
            $obs['marker_last'] = $marker
            $obs['file_version_last'] = $fv
            if ($fail) { $obs['failure_file'] = $fail }

            # activity bookkeeping
            if ($bytes -ne $lastBytes) { $lastActivity = $now; $lastBytes = $bytes }
            if ($installerAlive) { $lastActivity = $now }
            if ($appAlive -ne $prevAppAlive) { $lastActivity = $now }
            if ($appAlive) {
                if (-not $appEverAlive) { $appEverAlive = $true; $obs['app_alive_first_at'] = (Get-IsoNow) }
                if ($appGone) { $obs['app_relaunched'] = $true }
            } else {
                if ($appEverAlive -and (-not $appGone)) { $appGone = $true; $obs['app_gone_at'] = (Get-IsoNow) }
            }
            $prevAppAlive = $appAlive
            if ($installerAlive -and (-not $obs['installer_seen'])) {
                $obs['installer_seen'] = $true
                $obs['installer_first_seen_at'] = (Get-IsoNow)
                $obs['installer_first_seen'] = [ordered]@{ name = $installerProc.name; id = $installerProc.id; ppid = $installerProc.ppid; path = $installerProc.path; cmd = $installerProc.cmd }
                try { Add-Utf8NoBom $tl (('# installer process first seen: {0} pid={1} path={2} cmd={3}' -f $installerProc.name, $installerProc.id, $installerProc.path, $installerProc.cmd) + "`r`n") } catch { }
            }
            if ((Test-VersionReached $marker $TargetVersion) -and ($null -eq $obs['marker_reached_target_at'])) { $obs['marker_reached_target_at'] = (Get-IsoNow); $lastActivity = $now }

            $procText = (($procs | ForEach-Object { '{0}#{1}' -f $_.name, $_.id }) -join ' ')
            $updText = (($upd | ForEach-Object { '{0}:{1}B[{2}]' -f $_.name, $_.bytes, $_.files }) -join ' ')
            $line = '[{0,5}s {1}] marker={2} appver={3} cysapp={4} installer={5} procs=[{6}] updater=[{7}] failurefile={8}' -f $elapsed, (Get-IsoNow), $marker, $fv, $appAlive, $installerAlive, $procText, $updText, [bool]$fail
            try { Add-Utf8NoBom $tl ($line + "`r`n") } catch { }
            $obs['ticks'] = [int]$obs['ticks'] + 1

            # screenshots relative to the moment the CDP session ended (or to the start for the replica path)
            if (($installerAlive) -and (-not $shotInst)) {
                $shotInst = $true
                $sr = Save-Screenshot ($Prefix + '-screen-installer.png')
                $obs['screenshots'].Add($sr)
                try { Add-Utf8NoBom $tl ('# screenshot installer: ok={0} windows={1}' -f $sr['ok'], $sr['windows']) } catch { }
                try { Add-Utf8NoBom $tl "`r`n" } catch { }
            }
            # optional (sacreal-e2e.ps1): one screenshot as soon as the CDP session ended or the app is gone, so that a
            # Windows block notification / dialog right after the failed installer start is on the picture
            if ($ShotAtExit -and (-not $shotExit) -and (($null -ne $obs['node_exited_at']) -or $appGone)) {
                $shotExit = $true
                $sr = Save-Screenshot ($Prefix + '-screen-exit.png')
                $obs['screenshots'].Add($sr)
                try { Add-Utf8NoBom $tl (('# screenshot at exit: ok={0} windows={1}' -f $sr['ok'], $sr['windows']) + "`r`n") } catch { }
            }
            # optional: one screenshot every $ShotEverySec seconds (a transient notification or dialog can be anywhere in the flow)
            if (($ShotEverySec -gt 0) -and (($elapsed - $lastPeriodicShot) -ge $ShotEverySec)) {
                $lastPeriodicShot = $elapsed
                $sr = Save-Screenshot ('{0}-screen-t{1:D4}s.png' -f $Prefix, $elapsed)
                $obs['screenshots'].Add($sr)
            }
            if ($null -ne $nodeExitAt) {
                $sinceExit = ($now - $nodeExitAt).TotalSeconds
                if (($sinceExit -ge 20) -and (-not $shot20)) {
                    $shot20 = $true
                    $sr = Save-Screenshot ($Prefix + '-screen-20s.png')
                    $obs['screenshots'].Add($sr)
                    try { Add-Utf8NoBom $tl (('# screenshot 20s: ok={0} windows={1}' -f $sr['ok'], $sr['windows']) + "`r`n") } catch { }
                }
                if (($sinceExit -ge 60) -and (-not $shot60)) {
                    $shot60 = $true
                    $sr = Save-Screenshot ($Prefix + '-screen-60s.png')
                    $obs['screenshots'].Add($sr)
                    try { Add-Utf8NoBom $tl (('# screenshot 60s: ok={0} windows={1}' -f $sr['ok'], $sr['windows']) + "`r`n") } catch { }
                }
            }

            # end conditions
            if ((Test-VersionReached $marker $TargetVersion) -and $appAlive) { $reason = 'success: version marker reached the target and cys-app.exe is running'; break }
            if ($null -ne $nodeExitAt) {
                $sinceExit = ($now - $nodeExitAt).TotalSeconds
                if ($sinceExit -ge $PostNodeSec) { $reason = ('observation window after CDP end elapsed ({0}s)' -f $PostNodeSec); break }
                if (($sinceExit -ge 60) -and ((($now - $lastActivity).TotalSeconds) -ge $QuietEndSec)) { $reason = ('quiet: no installer/updater/marker/app activity for {0}s after CDP end' -f $QuietEndSec); break }
            }
            if ($elapsed -ge $MaxSec) { $reason = ('max watch seconds reached ({0})' -f $MaxSec); break }
            if ((Get-MinutesLeft) -lt 3) { $reason = 'job time budget exhausted'; break }
            Start-Sleep -Seconds $TickSec
        }
    } catch {
        $reason = 'watch loop exception: ' + $_.Exception.Message
        Add-DiagError ('Watch-UpdateOutcome ' + $Prefix) $_
    }
    $obs['end_reason'] = $reason
    $obs['ended'] = (Get-IsoNow)
    $obs['updater_dirs_seen'] = $seenDirs.ToArray()
    $sr = Save-Screenshot ($Prefix + '-screen-end.png')
    $obs['screenshots'].Add($sr)
    try { Add-Utf8NoBom $tl (('# end: {0}' -f $reason) + "`r`n") } catch { }
    Save-Json ($Prefix + '-observe.json') $obs 6
    return $obs
}

# ---------------------------------------------------------------------------
# steps 2-5: start the installed app with CDP, drive install_update, observe, judge
# ---------------------------------------------------------------------------
function Invoke-AppUpdateRun {
    param(
        [string]$Prefix = 'e2e',
        [int]$Port = 9333,
        [int]$StartupWaitSec = 120,
        [int]$CdpMaxSec = 720,
        [int]$PostNodeSec = 480,
        [string]$TargetVersion = '0.14.42',
        [scriptblock]$BeforeNode = $null,
        [int]$WatchTickSec = 5,
        [switch]$ShotAtExit,
        [int]$WatchShotEverySec = 0
    )
    $v = [ordered]@{
        prefix = $Prefix; started = (Get-IsoNow); installed_before = $null; app_started = $false
        cdp_attached = $false; check_update = $null; install_update_outcome = $null; app_exited_at = $null
        installer_seen = $false; version_after = $null; app_relaunched = $false
        verdict = 'inconclusive'; why = ''; verdict_source = 'cdp'; observe = $null; cdp = $null; diagnostics = [ordered]@{}
    }
    $nodeProc = $null
    $wv2Policy = $null
    $startedUtc = (Get-Date).ToUniversalTime()
    try {
        $installDir = Get-InstallDir
        $appExe = Join-Path $installDir 'cys-app.exe'
        $v['installed_before'] = Get-InstalledMarker
        if (-not (Test-Path -LiteralPath $appExe)) {
            $v['why'] = 'cys-app.exe not found in the install folder'
            return $v
        }

        # 2. start the app with a WebView2 remote debugging port (env var for old runtimes + HKLM policy for elevated hosts)
        $appProc = $null
        $wv2Policy = Set-WebView2DebugPolicy -Port $Port
        $v['diagnostics']['webview2_hklm_policy'] = $wv2Policy
        $v['diagnostics']['webview2_runtime_pv'] = $(try { [string](Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -Name pv -ErrorAction Stop).pv } catch { 'unknown' })
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = ('--remote-debugging-port={0} --remote-allow-origins=*' -f $Port)
        try {
            $appProc = Start-Process -FilePath $appExe -WorkingDirectory $installDir -PassThru -ErrorAction Stop
            $null = $appProc.Handle
            $v['app_started'] = $true
            $v['diagnostics']['app_pid'] = $appProc.Id
        } catch {
            $v['diagnostics']['app_start_error'] = $_.Exception.Message
        }
        Remove-Item -Path 'Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS' -ErrorAction SilentlyContinue

        $ready = $false
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $exitedAt = $null
        while ($sw.Elapsed.TotalSeconds -lt $StartupWaitSec) {
            try {
                $resp = Invoke-WebRequest -Uri ('http://127.0.0.1:{0}/json/version' -f $Port) -UseBasicParsing -TimeoutSec 3 -ErrorAction Stop
                if ($resp.StatusCode -eq 200) { $ready = $true; $v['diagnostics']['cdp_json_version'] = [string]$resp.Content; break }
            } catch { }
            # the app process itself may have exited (crash / refused to start): do not wait the full time for nothing
            try {
                if ($appProc -and $appProc.HasExited) {
                    if ($null -eq $exitedAt) { $exitedAt = [int]$sw.Elapsed.TotalSeconds }
                    if (([int]$sw.Elapsed.TotalSeconds - $exitedAt) -ge 20) { $v['diagnostics']['wait_ended_early'] = ('app process exited at {0}s' -f $exitedAt); break }
                }
            } catch { }
            Start-Sleep -Seconds 2
        }
        $v['diagnostics']['cdp_wait_sec'] = [int]$sw.Elapsed.TotalSeconds
        if (-not $ready) {
            $alive = $false
            try { if ($appProc) { $alive = (-not $appProc.HasExited) } } catch { }
            $v['diagnostics']['app_process_alive_at_timeout'] = $alive
            $v['diagnostics']['windows'] = Get-VisibleWindowsText
            $v['diagnostics']['screenshot'] = Save-Screenshot ($Prefix + '-nocdp.png')
            Save-Text ($Prefix + '-nocdp-processes.txt') (Get-ProcessListText)
            # full command lines of the WebView2 processes (the browser process, the one without --type=, must show the debugging flag)
            try {
                $wl = New-Object System.Collections.Generic.List[string]
                foreach ($wp in @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -Filter "Name = 'msedgewebview2.exe'" -ErrorAction Stop)) {
                    $wc = [regex]::Replace([string]$wp.CommandLine, '(?i)(token|password|secret|authorization)([=:\s]+)\S+', '$1$2<redacted>')
                    $wl.Add(('pid={0} ppid={1} browser_process={2} :: {3}' -f $wp.ProcessId, $wp.ParentProcessId, ($wc -notmatch '--type='), $wc))
                }
                if ($wl.Count -eq 0) { $wl.Add('no msedgewebview2.exe process is running (WebView2 never started for the app?)') }
                Save-Text ($Prefix + '-nocdp-webview2-cmdlines.txt') (($wl.ToArray()) -join "`r`n")
            } catch { Save-Text ($Prefix + '-nocdp-webview2-cmdlines.txt') ('failed: ' + $_.Exception.Message) }
            try {
                $ns = Invoke-Proc -File (Join-Path $env:windir 'System32\netstat.exe') -Arguments '-ano -p tcp' -TimeoutSec 30
                $nl = @(([string]$ns.out) -split "`n" | Where-Object { $_ -match 'LISTENING' })
                Save-Text ($Prefix + '-nocdp-listening-ports.txt') ($nl -join "`r`n")
            } catch { }
            $v['why'] = ('CDP port {0} did not answer within {1}s (app started={2}, alive={3})' -f $Port, $StartupWaitSec, $v['app_started'], $alive)
            return $v
        }

        # optional caller step between "CDP answers" and "drive install_update" (used by sacreal-e2e.ps1: attach + check_update,
        # then turn Smart App Control on). It receives $v. It may return a dictionary with proceed = $false (+ why) to stop
        # here, and node_exe = '<path>' to choose another (signed) node.exe for the driver. A throwing step also stops here.
        $nodeOverride = ''
        if ($null -ne $BeforeNode) {
            $hookRes = $null
            try {
                $hookOut = @(& $BeforeNode $v)
                if ($hookOut.Count -gt 0) { $hookRes = $hookOut[$hookOut.Count - 1] }
            } catch {
                $v['diagnostics']['before_node_error'] = Format-ErrorText $_
                Add-DiagError ('Invoke-AppUpdateRun before-node step ' + $Prefix) $_
                $v['verdict'] = 'inconclusive'
                $v['why'] = 'before-node step failed: ' + $_.Exception.Message
                return $v
            }
            if ($hookRes -is [System.Collections.IDictionary]) {
                if ($hookRes.Contains('node_exe') -and $hookRes['node_exe']) { $nodeOverride = [string]$hookRes['node_exe'] }
                if ($hookRes.Contains('proceed') -and ($hookRes['proceed'] -eq $false)) {
                    $v['verdict'] = 'inconclusive'
                    $v['why'] = 'stopped before install_update by the caller step: ' + [string]$hookRes['why']
                    return $v
                }
            }
        }

        # 3. CDP driver (node) runs in the background; the watch loop below observes meanwhile
        $nodeExe = Find-Exe 'node.exe'
        if ($nodeOverride) {
            $nodeExe = $nodeOverride
            $v['diagnostics']['node_exe_override'] = $nodeOverride
        }
        if (-not $nodeExe) {
            $v['why'] = 'node.exe not found'
            return $v
        }
        $nodeScript = Join-Path $global:DiagRoot 'cdp-update.mjs'
        # cdp-update.mjs carries its own WebSocket client (Node >= 18 with fetch is enough)
        $nv = Invoke-Proc -File $nodeExe -Arguments '--version' -TimeoutSec 30
        $v['diagnostics']['node_version'] = ([string]$nv.out).Trim()
        $nodeArgs = '"{0}" --port {1} --out "{2}" --prefix {3} --max-wait-sec {4}' -f $nodeScript, $Port, $global:DiagOut, $Prefix, $CdpMaxSec
        $nOut = Join-Path $global:DiagOut ($Prefix + '-cdp-stdout.txt')
        $nErr = Join-Path $global:DiagOut ($Prefix + '-cdp-stderr.txt')
        $nodeProc = Start-Process -FilePath $nodeExe -ArgumentList $nodeArgs -NoNewWindow -PassThru -RedirectStandardOutput $nOut -RedirectStandardError $nErr -ErrorAction Stop
        $null = $nodeProc.Handle
        $v['diagnostics']['node_pid'] = $nodeProc.Id

        # 4. observation
        $obs = Watch-UpdateOutcome -Prefix $Prefix -TargetVersion $TargetVersion -NodeProc $nodeProc -PostNodeSec $PostNodeSec -MaxSec ($CdpMaxSec + $PostNodeSec + 60) -TickSec $WatchTickSec -ShotAtExit:$ShotAtExit -ShotEverySec $WatchShotEverySec
        $v['observe'] = $obs
        try { if (-not $nodeProc.HasExited) { Stop-ProcessTree -ProcessId $nodeProc.Id; $v['diagnostics']['node_killed'] = $true } } catch { }

        # CDP result
        $cj = $null
        $txt = Read-TextUtf8 (Join-Path $global:DiagOut ($Prefix + '-cdp.json'))
        if ($txt) { try { $cj = ConvertFrom-Json $txt } catch { $v['diagnostics']['cdp_json_parse_error'] = $_.Exception.Message } }
        if ($null -ne $cj) {
            $v['cdp'] = $cj
            $v['cdp_attached'] = [bool]$cj.attached
            $v['check_update'] = $cj.check_update
            $v['install_update_outcome'] = $cj.install_update_outcome
            $v['app_exited_at'] = $cj.socket_closed_at
        } else {
            $v['diagnostics']['cdp_json'] = 'missing or unreadable'
            if ($txt) {
                # ConvertFrom-Json (5.1) can fail on odd page data: read the key facts from the raw text instead
                $v['cdp_attached'] = [bool]($txt -match '"attached"\s*:\s*true')
                $mo = [regex]::Match($txt, '"install_update_outcome"\s*:\s*"((?:[^"\\]|\\.)*)"')
                if ($mo.Success) { $v['install_update_outcome'] = $mo.Groups[1].Value }
                $ms = [regex]::Match($txt, '"socket_closed_at"\s*:\s*"([^"]*)"')
                if ($ms.Success) { $v['app_exited_at'] = $ms.Groups[1].Value }
                $v['diagnostics']['cdp_json_fallback_regex'] = $true
            }
        }
        if (-not $v['app_exited_at']) { $v['app_exited_at'] = $obs['app_gone_at'] }
        $v['installer_seen'] = [bool]$obs['installer_seen']
        $v['app_relaunched'] = [bool]$obs['app_relaunched']

        # version after
        $after = [ordered]@{ marker = (Get-InstalledMarker); file_version = (Get-AppFileVersion); cdp_app_version = $null }
        $v['version_after'] = $after
        if ((Test-VersionReached ([string]$after['marker']) $TargetVersion) -and $obs['app_relaunched']) {
            $vr = Invoke-Proc -File $nodeExe -Arguments ('"{0}" --mode version --port {1} --out "{2}" --prefix {3}' -f $nodeScript, $Port, $global:DiagOut, $Prefix) -TimeoutSec 90
            $v['diagnostics']['post_update_cdp_rc'] = $vr.rc
            $aj = Read-TextUtf8 (Join-Path $global:DiagOut ($Prefix + '-cdp-after.json'))
            if ($aj) { try { $after['cdp_app_version'] = (ConvertFrom-Json $aj).app_version } catch { } }
        }

        # 5. verdict
        $outcome = [string]$v['install_update_outcome']
        if (-not $v['cdp_attached']) {
            $v['verdict'] = 'inconclusive'
            $v['why'] = 'CDP endpoint answered but the driver did not attach (see cdp json / stderr)'
        } elseif (Test-VersionReached ([string]$after['marker']) $TargetVersion) {
            $v['verdict'] = 'updated'
            $v['why'] = ('version marker became {0} (target {1}); outcome={2}; installer_seen={3}; app_relaunched={4}' -f $after['marker'], $TargetVersion, $outcome, $v['installer_seen'], $v['app_relaunched'])
        } elseif ($outcome -like 'rejected:*') {
            $v['verdict'] = 'not_updated'
            $v['why'] = ('install_update rejected, marker stays {0}: {1}' -f $after['marker'], $outcome)
        } elseif ($v['app_exited_at']) {
            $v['verdict'] = 'not_updated'
            $v['why'] = ('app exited at {0} (outcome={1}) but marker is still {2} after the observation (installer_seen={3}, updater_dir_max_bytes={4}); end: {5}' -f $v['app_exited_at'], $outcome, $after['marker'], $v['installer_seen'], $obs['updater_dir_max_bytes'], $obs['end_reason'])
        } elseif ($outcome -eq 'timeout') {
            $v['verdict'] = 'not_updated'
            $v['why'] = ('install_update did not finish within the CDP wait and the app stayed alive; marker={0}; installer_seen={1}' -f $after['marker'], $v['installer_seen'])
        } else {
            $v['verdict'] = 'inconclusive'
            $v['why'] = ('outcome={0}; marker={1}; end: {2}' -f $outcome, $after['marker'], $obs['end_reason'])
        }
    } catch {
        $v['why'] = 'exception: ' + $_.Exception.Message
        Add-DiagError ('Invoke-AppUpdateRun ' + $Prefix) $_
    } finally {
        # runs for the early 'return $v' paths too ($v is a reference: the caller sees these additions)
        try { $v['app_evidence'] = Save-AppEvidence $Prefix } catch { }
        try { Save-AppEventLog $Prefix $startedUtc } catch { }
        try { $v['diagnostics']['webview2_hklm_policy_removed'] = Remove-WebView2DebugPolicy $wv2Policy } catch { }
        $v['finished'] = (Get-IsoNow)
    }
    return $v
}

# ---------------------------------------------------------------------------
# fallback: no CDP -> call ShellExecuteW+exit(0) with the replica binary against the 0.14.42 installer
# ---------------------------------------------------------------------------
function Invoke-ReplicaUpdate {
    param([string]$Prefix = 'e2e-replica', [string]$TargetVersion = '0.14.42', [int]$PostSec = 420)
    $startedUtc = (Get-Date).ToUniversalTime()
    $v = [ordered]@{
        verdict_source = 'replica'; started = (Get-IsoNow); replica_variant = $null; installer_seen = $false
        version_after = $null; app_relaunched = $false; verdict = 'inconclusive'; why = ''; call = $null; observe = $null
    }
    try {
        $build = Build-Replica
        $se = $null
        foreach ($name in @('x64', 'host')) {
            $ve = $build.variants.PSObject.Properties[$name]
            if ($ve -and $ve.Value.ok) { $se = [string]$ve.Value.se; $v['replica_variant'] = $name; break }
        }
        if (-not $se) { $v['why'] = 'replica binaries could not be built'; return $v }
        $inst42 = Get-AssetInstaller $TargetVersion
        if ((-not (Test-Path -LiteralPath $inst42)) -or (-not (Test-AssetUsable $TargetVersion))) { $v['why'] = 'target installer missing or unusable'; return $v }

        [void](Stop-ProcessesUnder (Get-InstallDir))
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ('cys-{0}-updater-diag' -f $TargetVersion)
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        $dest = Join-Path $dir ('cys-{0}-installer.exe' -f $TargetVersion)
        Copy-Item -LiteralPath $inst42 -Destination $dest -Force

        $cysd = Join-Path (Get-InstallDir) 'cysd.exe'
        if (Test-Path -LiteralPath $cysd) {
            try {
                $dp = Start-Process -FilePath $cysd -WindowStyle Hidden -PassThru -ErrorAction Stop
                $v['cysd_pid'] = $dp.Id
                Start-Sleep -Seconds 3
            } catch { $v['cysd_start_error'] = $_.Exception.Message }
        }

        $callLog = Join-Path $global:DiagOut ($Prefix + '-call.txt')
        $argStr = 'thread {0} {1} {2}' -f (ConvertTo-CmdArg $dest), (ConvertTo-CmdArg '/P /R /UPDATE'), (ConvertTo-CmdArg $callLog)
        $c = [ordered]@{ exe = $se; args = $argStr; rc = $null; timed_out = $false; log = $null; error = $null }
        $v['call'] = $c
        try {
            $sp = Start-Process -FilePath $se -ArgumentList $argStr -PassThru -ErrorAction Stop
            $null = $sp.Handle
            if (-not $sp.WaitForExit(60000)) { $c['timed_out'] = $true; Stop-ProcessTree -ProcessId $sp.Id } else { $sp.WaitForExit() }
            try { $c['rc'] = $sp.ExitCode } catch { }
        } catch { $c['error'] = $_.Exception.Message }
        $c['log'] = Read-TextUtf8 $callLog

        $obs = Watch-UpdateOutcome -Prefix $Prefix -TargetVersion $TargetVersion -NodeProc $null -PostNodeSec $PostSec -MaxSec ($PostSec + 60)
        $v['observe'] = $obs
        $v['installer_seen'] = [bool]$obs['installer_seen']
        $v['app_relaunched'] = [bool]$obs['app_relaunched']
        $after = [ordered]@{ marker = (Get-InstalledMarker); file_version = (Get-AppFileVersion) }
        $v['version_after'] = $after
        if (Test-VersionReached ([string]$after['marker']) $TargetVersion) {
            $v['verdict'] = 'updated'
            $v['why'] = ('replica call: marker became {0} (target {1}); installer_seen={2}' -f $after['marker'], $TargetVersion, $v['installer_seen'])
        } else {
            $v['verdict'] = 'not_updated'
            $v['why'] = ('replica call: marker still {0}; installer_seen={1}; call log: {2}' -f $after['marker'], $v['installer_seen'], ([string]$c['log']).Trim())
        }
    } catch {
        $v['why'] = 'exception: ' + $_.Exception.Message
        Add-DiagError ('Invoke-ReplicaUpdate ' + $Prefix) $_
    } finally {
        try { $v['app_evidence'] = Save-AppEvidence $Prefix } catch { }
        try { Save-AppEventLog $Prefix $startedUtc } catch { }
        $v['finished'] = (Get-IsoNow)
    }
    return $v
}

# ===========================================================================
# main (skipped when dot-sourced as a library)
# ===========================================================================
if ($MyInvocation.InvocationName -ne '.') {
    Start-DiagScript -Name 'e2e-update'
    $mainStartUtc = (Get-Date).ToUniversalTime()
    $nt = $null
    try { $nt = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -ErrorAction Stop } catch { }
    $VERDICT = [ordered]@{
        os = $env:DIAG_MATRIX_OS
        os_product = $(if ($nt) { ('{0} {1} build {2}.{3}' -f $nt.ProductName, $nt.DisplayVersion, $nt.CurrentBuild, $nt.UBR) } else { $null })
        arch = $env:PROCESSOR_ARCHITECTURE
        from_version = '0.14.37'
        to_version = '0.14.42'
        installed_before = $null
        cdp_attached = $false
        check_update = $null
        install_update_outcome = $null
        app_exited_at = $null
        installer_seen = $false
        version_after = $null
        app_relaunched = $false
        verdict = 'inconclusive'
        why = ''
        verdict_source = $null
        baseline_install = $null
        cdp_run = $null
        replica_run = $null
        started = (Get-IsoNow)
    }
    try {
        $inst = Install-CysVersion -Version '0.14.37'
        $VERDICT['baseline_install'] = $inst
        $VERDICT['installed_before'] = $inst['marker']
        Save-Json 'e2e-verdict.json' $VERDICT 9
        if ($inst['marker'] -ne '0.14.37') {
            $VERDICT['verdict'] = 'inconclusive'
            $VERDICT['verdict_source'] = 'none'
            $VERDICT['why'] = ('baseline 0.14.37 install failed: marker={0} error={1} failure_file={2}' -f $inst['marker'], $inst['error'], $inst['failure_file'])
        } else {
            $run = Invoke-AppUpdateRun -Prefix 'e2e' -Port 9333 -StartupWaitSec 120 -CdpMaxSec 720 -PostNodeSec 480 -TargetVersion '0.14.42'
            $VERDICT['cdp_run'] = $run
            $VERDICT['cdp_attached'] = $run['cdp_attached']
            $VERDICT['check_update'] = $run['check_update']
            $VERDICT['install_update_outcome'] = $run['install_update_outcome']
            $VERDICT['app_exited_at'] = $run['app_exited_at']
            $VERDICT['installer_seen'] = $run['installer_seen']
            $VERDICT['version_after'] = $run['version_after']
            $VERDICT['app_relaunched'] = $run['app_relaunched']
            $VERDICT['verdict'] = $run['verdict']
            $VERDICT['why'] = $run['why']
            $VERDICT['verdict_source'] = 'cdp'
            Save-Json 'e2e-verdict.json' $VERDICT 9
            if (-not $run['cdp_attached']) {
                if ((Get-MinutesLeft) -gt 12) {
                    $rep = Invoke-ReplicaUpdate -Prefix 'e2e-replica' -TargetVersion '0.14.42' -PostSec 420
                    $VERDICT['replica_run'] = $rep
                    $VERDICT['verdict'] = $rep['verdict']
                    $VERDICT['why'] = ('CDP did not attach (' + $run['why'] + '); replica path: ' + $rep['why'])
                    $VERDICT['verdict_source'] = 'replica'
                    $VERDICT['installer_seen'] = $rep['installer_seen']
                    $VERDICT['version_after'] = $rep['version_after']
                    $VERDICT['app_relaunched'] = $rep['app_relaunched']
                } else {
                    $VERDICT['why'] = ($run['why'] + ' ; replica fallback skipped: not enough job time left')
                }
            }
        }
    } catch {
        $VERDICT['why'] = 'main exception: ' + $_.Exception.Message
        Add-DiagError 'e2e-update main' $_
    } finally {
        try { $VERDICT['final_cleanup_killed'] = @(Stop-ProcessesUnder (Get-InstallDir)) } catch { }
        try {
            # Code Integrity events of the whole run (no policy of ours here: shows whatever the OS itself blocked/audited)
            $ce = Get-CIEvents -SinceUtc $mainStartUtc
            Save-Json 'e2e-ci-events.json' ([ordered]@{ note = $ce.note; count = @($ce.events).Count; events = @($ce.events | Select-Object -First 300) }) 6
            $VERDICT['ci_events_during_run'] = @($ce.events).Count
        } catch { }
        $VERDICT['finished'] = (Get-IsoNow)
        Save-Json 'e2e-verdict.json' $VERDICT 9
        Complete-DiagScript 'e2e-update'
    }
    exit 0
}
