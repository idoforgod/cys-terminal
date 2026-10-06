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
        # cys doctor as a streamed probe (also gives the Antigravity / asset lines when it ends)
        try {
            $agyDir = Join-Path $env:USERPROFILE '.gemini\antigravity-cli'; New-Item -ItemType Directory -Path $agyDir -Force | Out-Null
            $dp0 = Invoke-W44DoctorProbe ('scene-' + $Prefix) 90
            Remove-Item -LiteralPath (Join-Path $env:USERPROFILE '.gemini') -Recurse -Force -ErrorAction SilentlyContinue
            $txt = ''
            try { $txt = [System.IO.File]::ReadAllText((Join-Path $global:DiagOut ('w44-doctorprobe-scene-' + $Prefix + '-stdout.txt')), [System.Text.Encoding]::UTF8) } catch { }
            $ag = @(); $asset = @()
            foreach ($ln in ($txt -split "`r?`n")) { if ($ln -match 'Antigravity|agy') { $ag += (Limit-Text $ln 400) }; if ($ln -match 'office|web/|assets|pack-heal') { $asset += (Limit-Text $ln 300) } }
            $rec['doctor_after_bridge_killed'] = $null
            $rec['doctor'] = [ordered]@{ rc = $dp0['rc']; timed_out = $dp0['timed_out']; seconds = $dp0['seconds']; antigravity_lines = $ag; office_or_asset_lines = $asset; bytes = $txt.Length }
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

# ---- doctor probe: `cys.exe doctor` of the INSTALLED version under the conditions of the scene (app running, harness environment) ----
# Streams stdout/stderr to files line by line; waits up to MaxSec; on a timeout records the child processes of the doctor, the cys pipes, the
# loopback connections and the last printed lines. A record only.
function Invoke-W44DoctorProbe {
    param([string]$Tag, [int]$MaxSec = 300)
    $o = [ordered]@{ tag = $Tag; time = (Get-IsoNow); version = $null; seconds = $null; finished = $false; timed_out = $false; rc = $null; stdout_bytes = 0; stdout_lines = 0; last_lines = @(); snapshots = @(); env_notes = @(); error = $null }
    try {
        $cys = Join-Path (Get-InstallDir) 'cys.exe'
        if (-not (Test-Path -LiteralPath $cys)) { $o['error'] = 'no cys.exe'; return $o }
        $o['version'] = ([string](Invoke-Proc -File $cys -Arguments '--version' -TimeoutSec 30).out).Trim()
        foreach ($k in @('WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS', 'CYS_UPDATE_MANIFEST_URL', 'CYS_SOCKET', 'AITERM_SURFACE_ID')) { if ([Environment]::GetEnvironmentVariable($k, 'Process')) { $o['env_notes'] += ($k + ' is set in the harness process') } }
        $of = Join-Path $global:DiagOut ('w44-doctorprobe-' + $Tag + '-stdout.txt')
        $ef = Join-Path $global:DiagOut ('w44-doctorprobe-' + $Tag + '-stderr.txt')
        foreach ($f in @($of, $ef)) { [System.IO.File]::WriteAllText($f, '') }
        $psi = New-Object System.Diagnostics.ProcessStartInfo
        $psi.FileName = $cys; $psi.Arguments = 'doctor'; $psi.UseShellExecute = $false; $psi.CreateNoWindow = $true
        $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true; $psi.RedirectStandardInput = $true
        $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8; $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8
        $p = New-Object System.Diagnostics.Process
        $p.StartInfo = $psi
        $ho = Register-ObjectEvent -InputObject $p -EventName OutputDataReceived -Action { if ($null -ne $EventArgs.Data) { [System.IO.File]::AppendAllText($Event.MessageData.o, $EventArgs.Data + "`r`n", [System.Text.Encoding]::UTF8) } } -MessageData @{ o = $of }
        $he = Register-ObjectEvent -InputObject $p -EventName ErrorDataReceived -Action { if ($null -ne $EventArgs.Data) { [System.IO.File]::AppendAllText($Event.MessageData.o, $EventArgs.Data + "`r`n", [System.Text.Encoding]::UTF8) } } -MessageData @{ o = $ef }
        $ts = Get-Date
        [void]$p.Start()
        try { $p.StandardInput.Close() } catch { }
        $p.BeginOutputReadLine(); $p.BeginErrorReadLine()
        $snapAt = @(15, 45, 120)
        $si = 0
        while (-not $p.HasExited -and ((Get-Date) - $ts).TotalSeconds -lt $MaxSec) {
            Start-Sleep -Seconds 1
            $el = ((Get-Date) - $ts).TotalSeconds
            if ($si -lt $snapAt.Count -and $el -ge $snapAt[$si]) {
                $si++
                $kids = @()
                try {
                    $all = @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop)
                    $q = New-Object System.Collections.Generic.Queue[int]; $q.Enqueue($p.Id); $seen = @{}
                    while ($q.Count -gt 0) { $c = $q.Dequeue(); foreach ($r in $all) { if ([int]$r.ParentProcessId -eq $c -and -not $seen.ContainsKey([int]$r.ProcessId)) { $seen[[int]$r.ProcessId] = 1; $q.Enqueue([int]$r.ProcessId); $cpu = ''; try { $gp = Get-Process -Id ([int]$r.ProcessId) -ErrorAction Stop; $cpu = (' cpu_s={0} read_bytes={1} write_bytes={2} threads={3}' -f [math]::Round($gp.TotalProcessorTime.TotalSeconds, 1), $r.ReadTransferCount, $r.WriteTransferCount, $r.ThreadCount) } catch { }; $kids += ('{0}#{1}{2} {3}' -f $r.Name, $r.ProcessId, $cpu, (Limit-Text ([string]$r.CommandLine) 700)) } } }
                } catch { }
                $net = ''
                try { $net = [string](Invoke-Proc -File (Join-Path $env:windir 'System32\netstat.exe') -Arguments '-ano' -TimeoutSec 30).out } catch { }
                $lo = @($net -split "`r?`n" | Where-Object { $_ -match '127\.0\.0\.1' } | Select-Object -First 30)
                $pp = @(); try { $pp = @([System.IO.Directory]::GetFiles('\\.\pipe\') | ForEach-Object { $_.Substring($_.LastIndexOf('\') + 1) } | Where-Object { $_ -match 'cys' }) } catch { }
                $tail = ''; try { $tail = (Get-Content -LiteralPath $of -Tail 3 -ErrorAction SilentlyContinue) -join ' | ' } catch { }
                # who is busy on this machine right now: two samples 3 s apart of every process's CPU time and I/O bytes; the top ones
                $busy = @()
                try {
                    $s1 = @{}; foreach ($pr in @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 20)) { $s1[[int]$pr.ProcessId] = @([double]$pr.ReadTransferCount, [double]$pr.WriteTransferCount, [string]$pr.Name, [string]$pr.CommandLine) }
                    $c1 = @{}; foreach ($gp in @(Get-Process -ErrorAction SilentlyContinue)) { try { $c1[$gp.Id] = $gp.TotalProcessorTime.TotalSeconds } catch { } }
                    Start-Sleep -Seconds 3
                    $rowsB = New-Object System.Collections.Generic.List[object]
                    foreach ($pr in @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 20)) {
                        $id = [int]$pr.ProcessId
                        if ($s1.ContainsKey($id)) {
                            $dio = ([double]$pr.ReadTransferCount - $s1[$id][0]) + ([double]$pr.WriteTransferCount - $s1[$id][1])
                            $dcpu = 0; try { $dcpu = (Get-Process -Id $id -ErrorAction Stop).TotalProcessorTime.TotalSeconds - [double]$c1[$id] } catch { }
                            $rowsB.Add([pscustomobject]@{ n = $s1[$id][2]; id = $id; io = $dio; cpu = $dcpu; cmd = $s1[$id][3] })
                        }
                    }
                    $busy = @($rowsB.ToArray() | Sort-Object { $_.io + ($_.cpu * 1000000) } -Descending | Select-Object -First 8 | ForEach-Object { '{0}#{1} io_bytes_3s={2} cpu_s_3s={3} {4}' -f $_.n, $_.id, [int64]$_.io, [math]::Round($_.cpu, 2), (Limit-Text $_.cmd 160) })
                } catch { $busy = @('busy sample failed: ' + $_.Exception.Message) }
                $o['snapshots'] += [ordered]@{ at_sec = [int]$el; doctor_children = $kids; cys_pipes = $pp; loopback = $lo; stdout_tail = (Limit-Text $tail 400); busiest_processes_3s = $busy }
            }
        }
        $o['seconds'] = [math]::Round(((Get-Date) - $ts).TotalSeconds, 1)
        if ($p.HasExited) { $o['finished'] = $true; $p.WaitForExit(); $o['rc'] = $p.ExitCode } else {
            $o['timed_out'] = $true
            try { Stop-ProcessTree -ProcessId $p.Id } catch { }
            # the same seal verification the doctor runs, on its own, timed (is the machine slow at reading the runtime tree, or is it blocked?)
            try {
                $inst = Get-InstallDir
                $py = Join-Path $inst 'runtime\python\python3.exe'
                $sealPy = Join-Path $env:USERPROFILE '.cys\pack\bin\javis_runtime_seal.py'
                $man = Join-Path $inst 'runtime-manifest.json'
                $sw = [System.Diagnostics.Stopwatch]::StartNew()
                $vx = Invoke-Proc -File $py -Arguments ('"{0}" verify --root "{1}" --manifest "{2}"' -f $sealPy, (Join-Path $inst 'runtime'), $man) -TimeoutSec 120
                $o['direct_seal_verify'] = [ordered]@{ seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1); rc = $vx['rc']; timed_out = $vx['timedOut']; out = (Limit-Text ([string]$vx['out']) 300); err = (Limit-Text ([string]$vx['err']) 300); manifest_exists = (Test-Path -LiteralPath $man) }
            } catch { $o['direct_seal_verify'] = [ordered]@{ error = $_.Exception.Message } }
        }
        Start-Sleep -Milliseconds 800
        Unregister-Event -SourceIdentifier $ho.Name -ErrorAction SilentlyContinue; Unregister-Event -SourceIdentifier $he.Name -ErrorAction SilentlyContinue
        $txt = [System.IO.File]::ReadAllText($of, [System.Text.Encoding]::UTF8)
        $o['stdout_bytes'] = $txt.Length
        $o['stdout_lines'] = @($txt -split "`r?`n" | Where-Object { $_ -ne '' }).Count
        $o['last_lines'] = @($txt -split "`r?`n" | Where-Object { $_ -ne '' } | Select-Object -Last 6 | ForEach-Object { Limit-Text $_ 300 })
    } catch { $o['error'] = $_.Exception.Message }
    try { Save-Json ('w44-doctorprobe-' + $Tag + '.json') $o 8 } catch { }
    return $o
}

# =====================================================================================================================================
# Invoke-W44Scene2 -- tag-candidate checks on the REAL app (after Invoke-W44Scene, the app is still up):
#   assets   office tab with ONE screen asset deleted from the pack (web/office-boot.js | web/vendor/three.module.js): does the app still load the frame
#   late     office tab opened while the bridge is down (it comes back by itself): does a "could not load" banner stay (photo)
#   hold     5 minutes with the app + daemon + two seats (+ a master seat): proc_count_high lines, seat status; one seat closed -> only its processes go
#   approval `cys approval sign --ttl` from a master seat: the approval-lane state file exists, no temp leftovers
# =====================================================================================================================================
function Get-W44ProcTable { try { return @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop | ForEach-Object { [pscustomobject]@{ Name = [string]$_.Name; Id = [int]$_.ProcessId; Ppid = [int]$_.ParentProcessId; Cmd = [string]$_.CommandLine } }) } catch { return @() } }
function Get-W44Tree { param([int]$Root, $Rows)
    $out = New-Object System.Collections.Generic.List[int]; $q = New-Object System.Collections.Generic.Queue[int]; $q.Enqueue($Root); $seen = @{}
    while ($q.Count -gt 0) { $c = $q.Dequeue(); foreach ($r in $Rows) { if ($r.Ppid -eq $c -and -not $seen.ContainsKey($r.Id)) { $seen[$r.Id] = 1; $out.Add($r.Id); $q.Enqueue($r.Id) } } }
    return $out.ToArray()
}
function Get-W44SysCount {
    $rows = Get-W44ProcTable
    $names = @('wininit.exe', 'csrss.exe', 'services.exe', 'smss.exe', 'lsass.exe', 'winlogon.exe')
    $h = [ordered]@{ total = $rows.Count }
    foreach ($n in $names) { $h[$n] = @($rows | Where-Object { $_.Name -eq $n }).Count }
    $h['svchost.exe'] = @($rows | Where-Object { $_.Name -eq 'svchost.exe' }).Count
    return $h
}
function Invoke-W44Scene2 {
    param([string]$Prefix)
    $rec = [ordered]@{ prefix = $Prefix; started = (Get-IsoNow); finished = $null; skipped = $null; assets = [ordered]@{}; late = $null; hold = $null; approval = $null; error = $null }
    try {
        if ((Get-TeamMinutesLeft) -lt 14) { $rec['skipped'] = ('fewer than 14 minutes of job time left ({0})' -f [math]::Round([double](Get-TeamMinutesLeft), 1)); return $rec }
        $node = Find-Exe 'node.exe'
        $alive = Test-TeamAppAlive
        if (-not ($node -and $alive['process'] -and $alive['cdp'])) { $rec['skipped'] = 'the app is not running with an answering CDP port'; return $rec }
        $cys = Join-Path (Get-InstallDir) 'cys.exe'
        $web = Join-Path $env:USERPROFILE '.cys\pack\web'
        function GetCode { param([string]$u) try { $r = [System.Net.HttpWebRequest]::Create($u); $r.Timeout = 5000; $r.Proxy = $null; try { $x = $r.GetResponse(); $c = [int]$x.StatusCode; $x.Close(); return $c } catch [System.Net.WebException] { if ($_.Exception.Response) { return [int]$_.Exception.Response.StatusCode }; return -1 } } catch { return -2 } }

        # ---- assets: one file removed at a time ----
        foreach ($c in @(@('boot-js-missing', 'office-boot.js'), @('three-missing', 'vendor\three.module.js'))) {
            $f = Join-Path $web $c[1]
            $bak = $f + '.w44bak'
            $r = [ordered]@{ file = $f; existed = (Test-Path -LiteralPath $f); health_assets = $null; node = $null; office = $null; boot_js_http = $null; root_http = $null; restored = $null }
            try {
                if ($r['existed']) { Move-Item -LiteralPath $f -Destination $bak -Force }
                Start-Sleep -Seconds 2
                try { $h = Invoke-RestMethod -Uri 'http://127.0.0.1:8642/health' -TimeoutSec 5; $r['health_assets'] = $h.assets } catch { $r['health_assets'] = 'error: ' + $_.Exception.Message }
                $r['boot_js_http'] = GetCode 'http://127.0.0.1:8642/office-boot.js'
                $r['root_http'] = GetCode 'http://127.0.0.1:8642/'
                $pfx = $Prefix + '-' + $c[0]
                $r['node'] = Invoke-UpgNode -Prefix $pfx -NodeExe $node -Mode 'w44office' -Extra ('--w44-office-wait-sec 60 --w44-settle-sec 10 --w44-shot w44-{0}' -f $c[0]) -TimeoutSec 240
                $j = Read-UpgJson ($pfx + '-cdp.json')
                if ($null -ne $j -and $null -ne $j.w44office) {
                    $wo = $j.w44office
                    $first = @($wo.samples)[0]
                    $r['office'] = [ordered]@{ loaded = $wo.loaded; loaded_after_ms = $wo.loaded_after_ms; first_sample = $first; final = $wo.final; samples = @($wo.samples).Count; shot = $wo.shot }
                }
            } catch { $r['error'] = $_.Exception.Message }
            finally { try { if (Test-Path -LiteralPath $bak) { Move-Item -LiteralPath $bak -Destination $f -Force }; $r['restored'] = (Test-Path -LiteralPath $f) } catch { $r['restored'] = 'error: ' + $_.Exception.Message } }
            $rec['assets'][$c[0]] = $r
        }

        $assetsOnly = $false
        try { $inp2 = ConvertFrom-Json (Read-TextUtf8 (Join-Path $env:GITHUB_WORKSPACE 'diag\w44-input.json')); if ($inp2.scene2_assets_only) { $assetsOnly = $true } } catch { }
        if ($assetsOnly) { $rec['note'] = 'assets only (late / hold / approval skipped by w44-input.json scene2_assets_only)'; return $rec }
        # ---- late: the bridge is down when the tab opens; the daemon brings it back (legacy loop: about 60 s) ----
        $late = [ordered]@{ killed = @(); node = $null; office = $null; bridge_rows_after = $null }
        foreach ($pr in (Get-W44ProcTable)) { if ($pr.Cmd -like '*javis_hud_bridge.py*') { try { Stop-Process -Id $pr.Id -Force -ErrorAction Stop; $late['killed'] += $pr.Id } catch { } } }
        Start-Sleep -Seconds 2
        $pfx = $Prefix + '-late'
        $late['node'] = Invoke-UpgNode -Prefix $pfx -NodeExe $node -Mode 'w44office' -Extra '--w44-office-wait-sec 150 --w44-settle-sec 25 --w44-shot w44-late' -TimeoutSec 400
        $j = Read-UpgJson ($pfx + '-cdp.json')
        if ($null -ne $j -and $null -ne $j.w44office) {
            $wo = $j.w44office
            $texts = @($wo.samples | ForEach-Object { [string]$_.hint_text } | Where-Object { $_ } | Select-Object -Unique)
            $late['office'] = [ordered]@{ loaded = $wo.loaded; loaded_after_ms = $wo.loaded_after_ms; hint_texts_seen = $texts; final = $wo.final; samples = @($wo.samples).Count; shot = $wo.shot }
        }
        # the first pass only shows the "preparing" state; now wait until the daemon has brought the bridge back (legacy loop: about 60 s), then open the tab again
        $tb = Get-Date; $back = $null
        while (((Get-Date) - $tb).TotalSeconds -lt 180) {
            $rows = Get-W44BridgeRows 'poll'
            $hc = GetCode 'http://127.0.0.1:8642/health'
            if ($rows['count'] -gt 0 -and $hc -eq 200) { $back = [math]::Round(((Get-Date) - $tb).TotalSeconds, 1); break }
            Start-Sleep -Seconds 3
        }
        $late['bridge_back_after_s_from_first_pass_end'] = $back
        if ($null -ne $back) {
            $pfx2 = $Prefix + '-late2'
            $late['node2'] = Invoke-UpgNode -Prefix $pfx2 -NodeExe $node -Mode 'w44office' -Extra '--w44-office-wait-sec 60 --w44-settle-sec 30 --w44-shot w44-late2' -TimeoutSec 240
            $j2 = Read-UpgJson ($pfx2 + '-cdp.json')
            if ($null -ne $j2 -and $null -ne $j2.w44office) { $w2 = $j2.w44office; $late['office2'] = [ordered]@{ loaded = $w2.loaded; final = $w2.final; shot = $w2.shot } }
        }
        $late['bridge_rows_after'] = Get-W44BridgeRows 'after-late'
        $rec['late'] = $late
        Save-Json ('w44-scene2-' + $Prefix + '.json') $rec 9

        # ---- hold (5 min): two seats + a master seat; approval sign from the master seat; close one seat ----
        $H = [ordered]@{ seats = [ordered]@{}; samples = @(); proc_count_high_lines = 0; log_lines = @(); close_test = $null; sys_before = Get-W44SysCount; sys_after = $null; feed_items_matching = 0 }
        $rec['hold'] = $H
        $logF = Join-Path (Get-InstallDir) 'cysd.log'
        $logBefore = 0
        try { if (Test-Path -LiteralPath $logF) { $logBefore = @((Read-FileShared $logF) -split "`r?`n").Count } } catch { }
        function CysRun { param([string]$a, [int]$sec = 60) return (Invoke-Proc -File $cys -Arguments $a -TimeoutSec $sec) }
        $evOut = Join-Path $env:RUNNER_TEMP 'w44-events.txt'
        $evProc = $null
        try { $evProc = Start-Process -FilePath $cys -ArgumentList 'events --reconnect' -PassThru -WindowStyle Hidden -RedirectStandardOutput $evOut -RedirectStandardError (Join-Path $env:RUNNER_TEMP 'w44-events.err.txt') } catch { }
        $refs = @{}
        foreach ($nm in @('w44a', 'w44b')) { $o = CysRun ('new-surface --title {0} --cmd cmd.exe' -f $nm); $refs[$nm] = ([string]$o.out).Trim(); $H['seats'][$nm] = [ordered]@{ ref = $refs[$nm]; rc = $o.rc } }
        $om = CysRun 'new-surface --role master --title w44m --cmd cmd.exe'
        $refs['w44m'] = ([string]$om.out).Trim(); $H['seats']['w44m'] = [ordered]@{ ref = $refs['w44m']; rc = $om.rc }
        $tm0 = Get-Date
        foreach ($nm in @('w44a', 'w44b')) { $null = CysRun ('send --surface {0} {1}' -f $refs[$nm], (ConvertTo-CmdArg 'ping -n 900 127.0.0.1 > nul')) 30; Start-Sleep -Milliseconds 300; $null = CysRun ('send-key --surface {0} Return' -f $refs[$nm]) 30 }
        Start-Sleep -Seconds 3
        function SeatPid { param([string]$ref) $lst = [string](CysRun 'list' 30).out; foreach ($ln in ($lst -split "`r?`n")) { if ($ln -like ($ref + "`t*")) { $m = [regex]::Match($ln, 'pid=(\d+)'); if ($m.Success) { return [int]$m.Groups[1].Value } } }; return 0 }
        $approvalDone = $false; $closeDone = $false
        $apOut = Join-Path $env:RUNNER_TEMP 'w44-approval-sign.txt'
        for ($i = 0; ((Get-Date) - $tm0).TotalSeconds -lt 300; $i++) {
            $el = [int]((Get-Date) - $tm0).TotalSeconds
            $lst = [string](CysRun 'list' 30).out
            $tab = Get-W44ProcTable
            $smp = [ordered]@{ at_sec = $el; list = ((($lst -split "`r?`n") | Where-Object { $_ } | ForEach-Object { if ($_.Length -gt 120) { $_.Substring(0, 120) } else { $_ } }) -join ' | '); procs_total = $tab.Count }
            $H['samples'] += $smp
            if ((-not $approvalDone) -and $el -ge 75) {
                $approvalDone = $true
                $ap = [ordered]@{ sign_out = $null; lane_file = $null; lane_len = $null; tmp_leftovers = @(); state_dir = (Get-InstallDir); ttl_store_has_record = $null; check_other_folder_rc = $null }
                $bat = Join-Path $env:RUNNER_TEMP 'w44-approval-sign.bat'
                if (Test-Path -LiteralPath $apOut) { Remove-Item -LiteralPath $apOut -Force }
                [System.IO.File]::WriteAllText($bat, ("@echo off`r`n`"{0}`" approval sign --prefix `"cys kill 4242`" --ttl 600 > `"{1}`" 2>&1`r`necho rc=%errorlevel% >> `"{1}`"`r`n" -f $cys, $apOut))
                $null = CysRun ('send --surface {0} {1}' -f $refs['w44m'], (ConvertTo-CmdArg ('call "{0}"' -f $bat))) 30
                Start-Sleep -Milliseconds 300
                $null = CysRun ('send-key --surface {0} Return' -f $refs['w44m']) 30
                $tw = Get-Date
                while (((Get-Date) - $tw).TotalSeconds -lt 40 -and -not ((Test-Path -LiteralPath $apOut) -and ((Read-FileShared $apOut) -match 'rc='))) { Start-Sleep -Seconds 1 }
                $ap['sign_out'] = $(if (Test-Path -LiteralPath $apOut) { ((Read-FileShared $apOut) -replace "`r?`n", ' | ') } else { 'no output' })
                $lane = Join-Path (Get-InstallDir) 'approval-lane'
                $ap['lane_file'] = (Test-Path -LiteralPath $lane)
                if ($ap['lane_file']) { $ap['lane_len'] = (Get-Item -LiteralPath $lane).Length }
                $tmp = @()
                foreach ($d in @((Get-InstallDir), (Join-Path $env:USERPROFILE '.cys'))) { foreach ($t in @(Get-ChildItem -LiteralPath $d -Force -ErrorAction SilentlyContinue | Where-Object { $_.Name -like '*.tmp' -or $_.Name -like '.*.tmp*' })) { $tmp += ('{0} ({1} B)' -f $t.FullName, $t.Length) } }
                $ap['tmp_leftovers'] = $tmp
                $ttl = Join-Path $env:USERPROFILE '.cys\approvals-ttl.json'
                $ap['ttl_store_has_record'] = $(if (Test-Path -LiteralPath $ttl) { ((Read-FileShared $ttl) -match 'kill') } else { $false })
                $chk = Invoke-Proc -File $cys -Arguments 'approval check --prefix "cys kill 4242" --require-ttl' -TimeoutSec 30 -WorkDir $env:SystemRoot
                $ap['check_other_folder_rc'] = $chk.rc
                $rec['approval'] = $ap
                Save-Json ('w44-scene2-' + $Prefix + '.json') $rec 9
            }
            if ((-not $closeDone) -and $el -ge 170) {
                $closeDone = $true
                $rowsB = Get-W44ProcTable
                $pa = SeatPid $refs['w44a']; $pb = SeatPid $refs['w44b']
                $treeA = @(); if ($pa -gt 0) { $treeA = @($pa) + @(Get-W44Tree $pa $rowsB) }
                $treeB = @(); if ($pb -gt 0) { $treeB = @($pb) + @(Get-W44Tree $pb $rowsB) }
                $sysB = Get-W44SysCount
                $cl = CysRun ('close-surface {0}' -f $refs['w44a']) 60
                Start-Sleep -Seconds 6
                $rowsA = Get-W44ProcTable
                $idsA = @($rowsA | ForEach-Object { $_.Id })
                $H['close_test'] = [ordered]@{ closed = $refs['w44a']; rc = $cl.rc; out = (Limit-Text (([string]$cl.out) + ' ' + ([string]$cl.err)) 300); seat_a_pid = $pa; seat_a_tree = $treeA.Count; seat_a_alive_after = @($treeA | Where-Object { $idsA -contains $_ }).Count; seat_b_pid = $pb; seat_b_tree = $treeB.Count; seat_b_alive_after = @($treeB | Where-Object { $idsA -contains $_ }).Count; sys_before = $sysB; sys_after = (Get-W44SysCount) }
            }
            Start-Sleep -Seconds 20
        }
        $H['sys_after'] = Get-W44SysCount
        try {
            if ($evProc -and -not $evProc.HasExited) { Stop-Process -Id $evProc.Id -Force -ErrorAction SilentlyContinue }
            $evLines = @(); if (Test-Path -LiteralPath $evOut) { $evLines = @((Read-FileShared $evOut) -split "`r?`n" | Where-Object { $_ }) }
            $H['events_total_lines'] = $evLines.Count
            $H['events_proc_count_high'] = @($evLines | Where-Object { $_ -match 'proc_count_high' }).Count
            $H['events_watchdog_lines'] = @($evLines | Where-Object { $_ -match 'watchdog\.' } | Select-Object -First 8 | ForEach-Object { Limit-Text $_ 200 })
            $H['events_names_seen'] = @($evLines | ForEach-Object { $m = [regex]::Match($_, '"name"\s*:\s*"([^"]+)"'); if ($m.Success) { $m.Groups[1].Value } } | Group-Object | Sort-Object Count -Descending | Select-Object -First 15 | ForEach-Object { '{0} x{1}' -f $_.Name, $_.Count })
            $H['events_sample_head'] = @($evLines | Select-Object -First 2 | ForEach-Object { Limit-Text $_ 200 })
        } catch { $H['events_error'] = $_.Exception.Message }
        try {
            if (Test-Path -LiteralPath $logF) {
                $all = @((Read-FileShared $logF) -split "`r?`n")
                $new = @($all | Select-Object -Skip $logBefore)
                $hit = @($new | Where-Object { $_ -match 'proc_count_high|proc_count|process count' })
                $H['proc_count_high_lines'] = $hit.Count
                $H['log_lines'] = @($hit | Select-Object -First 5 | ForEach-Object { Limit-Text $_ 240 })
                $H['daemon_log_new_lines'] = $new.Count
            } else { $H['log_lines'] = @('no cysd.log at ' + $logF) }
        } catch { $H['log_lines'] = @('log read failed: ' + $_.Exception.Message) }
        try { $fl = [string](CysRun 'feed list' 30).out; $H['feed_items_matching'] = @(($fl -split "`r?`n") | Where-Object { $_ -match 'proc_count|process count' }).Count } catch { }
        foreach ($nm in @('w44b', 'w44m')) { $null = CysRun ('close-surface {0}' -f $refs[$nm]) 30 }
    } catch { $rec['error'] = 'exception: ' + $_.Exception.Message; Add-DiagError 'Invoke-W44Scene2' $_ }
    $rec['finished'] = (Get-IsoNow)
    try { Save-Json ('w44-scene2-' + $Prefix + '.json') $rec 9 } catch { }
    return $rec
}
