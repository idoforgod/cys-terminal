# diag/w44-scene.ps1 -- LIBRARY of diag/app-e2e.ps1 (dot-sourced there; functions only, there is no main in this file).
# 0.14.44 scene on the REAL app of a Windows 11 runner (design section 5-5): what a user sees and what the processes do.
#   Get-W44BridgeRows   the python office-bridge processes (javis_hud_bridge.py): id, parent, start time, command line. Called by the upgrade
#                       scene before the upgrade, right after it and at the end (item 3 of 5-4: does an OLD bridge survive an upgrade?).
#   Invoke-W44Scene     with the app running (or started cold): node mode w44 of diag/cdp-update.mjs (usage view mode button, approval Feed
#                       with pushed items for the head daemon and a department daemon, the office tab and the bridge), then `cys doctor`
#                       (the Antigravity line on Windows), the bridge rows, the daemon / bridge log lines.
#   files               w44-scene-<prefix>.json, w44-scene-<prefix>.txt (summary), <prefix>-cdp.json (R.w44), <prefix>-ui-w44-*.png,
#                       w44-doctor-<prefix>.txt
# OBSERVATION ONLY. The only things written into the product are the feed items pushed on purpose (`cys feed push`, answered by Allow).
# Windows PowerShell 5.1, ASCII only.

function Get-W44BridgeRows {
    param([string]$Tag = '')
    $o = [ordered]@{ tag = $Tag; time = (Get-IsoNow); rows = @(); count = 0; error = $null }
    try {
        $all = Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop
        $rows = New-Object System.Collections.Generic.List[object]
        foreach ($p in $all) {
            $cmd = [string]$p.CommandLine
            if ($cmd -and ($cmd -like '*javis_hud_bridge.py*') -and (([string]$p.Name) -like 'python*')) {
                $parentAlive = $false
                $parentName = ''
                foreach ($q in $all) { if ([int]$q.ProcessId -eq [int]$p.ParentProcessId) { $parentAlive = $true; $parentName = [string]$q.Name; break } }
                $rows.Add([ordered]@{ id = [int]$p.ProcessId; ppid = [int]$p.ParentProcessId; parent_alive = $parentAlive; parent_name = $parentName; created = [string]$p.CreationDate; cmd = (Limit-Text $cmd 260) })
            }
        }
        $o['rows'] = $rows.ToArray()
        $o['count'] = $rows.Count
    } catch { $o['error'] = $_.Exception.Message }
    return $o
}

function Get-W44LogLines {
    param([string]$Pattern, [int]$Max = 60)
    $res = [ordered]@{ files = @(); lines = @() }
    $dir = Get-InstallDir
    $lines = New-Object System.Collections.Generic.List[string]
    foreach ($n in @('cysd.log', 'office-bridge.log')) {
        $f = Join-Path $dir $n
        if (Test-Path -LiteralPath $f) {
            $res['files'] += ('{0} ({1} bytes)' -f $n, (Get-Item -LiteralPath $f).Length)
            try {
                $txt = Read-FileShared $f
                foreach ($ln in ($txt -split "`r?`n")) { if ($ln -match $Pattern) { $lines.Add(('{0}: {1}' -f $n, (Limit-Text $ln 260))) } }
            } catch { }
        }
    }
    $arr = $lines.ToArray()
    if ($arr.Count -gt $Max) { $arr = $arr[($arr.Count - $Max)..($arr.Count - 1)] }
    $res['lines'] = $arr
    return $res
}

function Invoke-W44Scene {
    param([string]$Prefix, [bool]$StartApp = $false)
    $rec = [ordered]@{ prefix = $Prefix; started = (Get-IsoNow); finished = $null; skipped = $null; app_started_by_scene = $false; node = $null; w44 = $null; doctor = $null; bridge_rows = $null; log_lines = $null; dept_pipes = @(); notes = (New-Object System.Collections.Generic.List[string]); error = $null }
    try {
        $left = [math]::Round([double](Get-TeamMinutesLeft), 1)
        if ($left -lt 9) { $rec['skipped'] = ('fewer than 9 minutes of job time left ({0})' -f $left); return $rec }
        $node = Find-Exe 'node.exe'
        if (-not $node) { $rec['skipped'] = 'node.exe not found'; return $rec }
        $cys = Join-Path (Get-InstallDir) 'cys.exe'
        $alive = Test-TeamAppAlive
        if (-not ($alive['process'] -and $alive['cdp'])) {
            if (-not $StartApp) { $rec['skipped'] = 'the app is not running with an answering CDP port'; return $rec }
            $prevPolicyRec = $RUN['webview2_policy']
            try { $null = Remove-WebView2DebugPolicy $CTX['wv2_policy'] } catch { }
            $CTX['wv2_policy'] = $null
            $CTX['manifest_url'] = ''
            $st = Start-CysApp $Prefix
            $RUN['webview2_policy_w44'] = $RUN['webview2_policy']
            $RUN['webview2_policy'] = $prevPolicyRec
            $rec['app_started_by_scene'] = $true
            $rec['app_start'] = $st
            if (-not $st['cdp_ready']) { $rec['skipped'] = 'the app did not answer on the debugging port after the start: ' + [string]$st['error']; return $rec }
        }
        $pre = Invoke-NodePre -Prefix ($Prefix + '-pre') -NodeExe $node
        $rec['pre'] = [ordered]@{ attached = $pre['attached']; ui_ready = $pre['ui_ready']; app_version = $pre['app_version'] }
        # department daemons: named pipes cys-dept-<name>
        $pp = Get-TeamPipes
        $dp = New-Object System.Collections.Generic.List[string]
        foreach ($n in @($pp['cys'])) { if ([string]$n -like 'cys-dept-*') { $dp.Add('\\.\pipe\' + [string]$n) } }
        # a department is registered but its daemon is not running (the TEAM scene cleanup ended it): start that department daemon the way the
        # product names it (pipe from ~/.cys/depts.json) so that the Feed of the app can show - and route - a department item (C2)
        $rec['dept_daemon'] = $null
        if ($dp.Count -eq 0) {
            try {
                $regF = Join-Path $env:USERPROFILE '.cys\depts.json'
                if (Test-Path -LiteralPath $regF) {
                    $reg = ConvertFrom-Json (Read-FileShared $regF)
                    foreach ($pn in @($reg.depts.PSObject.Properties)) {
                        $sock = [string]$pn.Value.socket
                        if ($sock -like '\\.\pipe\cys-dept-*') {
                            $errF = Join-Path $global:DiagOut ('w44-dept-cysd-' + $pn.Name + '.err.txt')
                            $old = $env:CYS_SOCKET
                            $env:CYS_SOCKET = $sock
                            $dpr = $null
                            try { $dpr = Start-Process -FilePath (Join-Path (Get-InstallDir) 'cysd.exe') -PassThru -WindowStyle Hidden -RedirectStandardError $errF -RedirectStandardOutput (Join-Path $global:DiagWork ('w44-dept-cysd-' + $pn.Name + '.out.txt')) } finally { $env:CYS_SOCKET = $old }
                            $t0 = Get-Date
                            $up = $false
                            while (((Get-Date) - $t0).TotalSeconds -lt 60) {
                                Start-Sleep -Seconds 2
                                $pp2 = Get-TeamPipes
                                if (@($pp2['cys']) -contains ($sock -replace '^.*\\', '')) { $up = $true; break }
                            }
                            $rec['dept_daemon'] = [ordered]@{ name = $pn.Name; socket = $sock; pid = $dpr.Id; pipe_up = $up; waited_s = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1) }
                            if ($up) { $dp.Add($sock) }
                            break
                        }
                    }
                }
            } catch { $rec['dept_daemon'] = [ordered]@{ error = $_.Exception.Message } }
        }
        $rec['dept_pipes'] = $dp.ToArray()
        $rec['pipes_all'] = @($pp['cys'])
        $rec['bridge_rows_before'] = Get-W44BridgeRows 'scene-before'
        $extra = ('--w44-cys "{0}" --w44-office-wait-sec 150 --w44-feed-wait-sec 60' -f $cys)
        if ($dp.Count -gt 0) { $extra += (' --w44-dept-pipes "{0}"' -f ($dp.ToArray() -join ',')) }
        $x = Invoke-UpgNode -Prefix $Prefix -NodeExe $node -Mode 'w44' -Extra $extra -TimeoutSec 700
        $rec['node'] = $x
        $j = Read-UpgJson ($Prefix + '-cdp.json')
        if ($null -ne $j) { $rec['w44'] = $j.w44; $rec['attached'] = [bool]$j.attached; $rec['app_version'] = $j.app_version }
        try { $null = Save-Screenshot ($Prefix + '-screen-end.png') } catch { }
        # cys doctor (the Antigravity line and the pack asset lines on Windows)
        try {
            $d = Invoke-Proc -File $cys -Arguments 'doctor' -TimeoutSec 180
            $txt = ([string]$d['out']) + "`r`n" + ([string]$d['err'])
            Save-Text ('w44-doctor-' + $Prefix + '.txt') $txt
            $ag = @(); $asset = @()
            foreach ($ln in ($txt -split "`r?`n")) {
                if ($ln -match 'Antigravity|agy') { $ag += (Limit-Text $ln 400) }
                if ($ln -match 'office|web/|assets|pack-heal') { $asset += (Limit-Text $ln 300) }
            }
            $rec['doctor'] = [ordered]@{ rc = $d['rc']; timed_out = $d['timedOut']; antigravity_lines = $ag; office_or_asset_lines = $asset; bytes = $txt.Length }
        } catch { $rec['doctor'] = [ordered]@{ error = $_.Exception.Message } }
        try {
            $ef = Join-Path $global:DiagOut 'w44-dept-cysd-dept-1.err.txt'
            if (Test-Path -LiteralPath $ef) { $rec['dept_daemon_log_bridge_lines'] = @((Read-FileShared $ef) -split "`r?`n" | Where-Object { $_ -match 'office-bridge' } | Select-Object -First 5) }
        } catch { }
        $rec['bridge_rows'] = Get-W44BridgeRows 'scene-after'
        $rec['log_lines'] = Get-W44LogLines 'office-bridge|bridge' 60
        # one short summary text
        $L = New-Object System.Collections.Generic.List[string]
        $w = $rec['w44']
        if ($null -ne $w) {
            $us = $w.usage
            if ($null -ne $us) { $L.Add(('usage view-mode button labels over 3 clicks: [{0}] cycle_ok={1}' -f (@($us.button_labels) -join ' > '), $us.cycle_ok)) }
            foreach ($f in @($w.feed)) { $L.Add(('feed {0}: {1} (seen after {2} ms, exit code {3}, pipe {4})' -f $f.label, $f.verdict, $f.seen_after_ms, $f.exit_code, $f.pipe)) }
            $of = $w.office
            if ($null -ne $of) { $L.Add(('office tab: loaded_signal={0} after {1} ms; bridge /health status {2}; /world status {3} bytes {4}; office-boot.js {5}' -f $of.loaded_signal, $of.loaded_after_ms, $of.bridge_health.status, $of.bridge_world.status, $of.bridge_world.bytes, $of.office_boot_js.status)) }
        } else { $L.Add('w44 node mode wrote no R.w44 (see ' + $Prefix + '-cdp.json and -cdp-stdout.txt)') }
        $L.Add(('bridge processes: before the scene {0}, after {1}' -f $rec['bridge_rows_before']['count'], $rec['bridge_rows']['count']))
        if ($null -ne $rec['doctor']) { $L.Add(('doctor Antigravity line(s): {0}' -f ((@($rec['doctor']['antigravity_lines']) -join ' || ')))) }
        Save-Text ('w44-scene-' + $Prefix + '.txt') ($L -join "`r`n")
    } catch {
        $rec['error'] = 'exception: ' + $_.Exception.Message
        Add-DiagError 'Invoke-W44Scene' $_
    }
    $rec['finished'] = (Get-IsoNow)
    try { Save-Json ('w44-scene-' + $Prefix + '.json') $rec 9 } catch { }
    return $rec
}
