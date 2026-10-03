# diag/replica.ps1 -- QF: does "ShellExecuteW(...); std::process::exit(0)" (updater plugin 2.10.1 pattern)
# sometimes exit before the new process is started?  Counts launches per combination, 40 runs each.
#   combos: {main, thread} x {target = marker.exe (unsigned), target = cmd.exe /c type nul > marker} x {log, nolog}
#   'log'   : the call result (HINSTANCE, GetLastError) is appended to a log right before exit(0)
#   'nolog' : exact plugin pattern, nothing between ShellExecuteW's return and exit(0)
# Output: OUT\replica.json (+ OUT\replica-build.json from Build-Replica)
$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')
Start-DiagScript -Name 'replica'

$ITER = 40
$WAIT_SEC = 5
$MISS_STOP = 8          # this many launches in a row without a marker = the combination never launches: stop early
$BUDGET_MIN = 15        # whole-script time budget (the workflow step limit is 20 minutes)
$REPLICA_START = Get-Date
$RP = [ordered]@{
    started = (Get-IsoNow)
    iterations = $ITER
    wait_sec_for_marker = $WAIT_SEC
    build = $null
    variants = [ordered]@{}
    summary = (New-Object System.Collections.Generic.List[string])
    skipped = $null
    truncated = $false
}

function Save-Replica { Save-Json 'replica.json' $RP 8 }

function Add-Dist {
    param($Dist, [string]$Key)
    if ($Dist.Contains($Key)) { $Dist[$Key] = [int]$Dist[$Key] + 1 } else { $Dist[$Key] = 1 }
}

# start se_exit.exe, wait for it (it must exit(0) by itself), return rc / timing
function Start-SeExit {
    param([string]$Exe, [string]$ArgString, [int]$TimeoutMs = 30000)
    $r = [ordered]@{ rc = $null; timed_out = $false; error = $null; ms = 0 }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        $psi = New-Object System.Diagnostics.ProcessStartInfo
        $psi.FileName = $Exe
        $psi.Arguments = $ArgString
        $psi.UseShellExecute = $false
        $psi.CreateNoWindow = $true
        $p = [System.Diagnostics.Process]::Start($psi)
        if ($p.WaitForExit($TimeoutMs)) {
            $r['rc'] = $p.ExitCode
        } else {
            $r['timed_out'] = $true
            try { $p.Kill() } catch { }
        }
        $p.Dispose()
    } catch {
        $r['error'] = $_.Exception.Message
    }
    $sw.Stop()
    $r['ms'] = [int]$sw.ElapsedMilliseconds
    return $r
}

function Invoke-ReplicaCombo {
    param($Variant, [string]$VariantName, [string]$Mode, [string]$Target, [string]$Flavor)
    $res = [ordered]@{
        variant = $VariantName; mode = $Mode; target = $Target; flavor = $Flavor
        runs = 0; launched = 0; not_launched = 0
        ret_dist = [ordered]@{}; lasterr_dist = [ordered]@{}; exit_code_dist = [ordered]@{}
        launch_ms_min = $null; launch_ms_max = $null; launch_ms_avg = $null
        truncated = $null; run_log = (New-Object System.Collections.Generic.List[string])
    }
    $dirM = Join-Path $global:DiagWork 'replica-markers'
    New-Item -ItemType Directory -Path $dirM -Force | Out-Null
    $times = New-Object System.Collections.Generic.List[double]
    $cmdExe = Join-Path $env:windir 'System32\cmd.exe'
    $miss = 0
    for ($i = 1; $i -le $ITER; $i++) {
        if ((Get-MinutesLeft) -lt 5) { $res['truncated'] = 'job time budget'; break }
        if ((((Get-Date) - $REPLICA_START).TotalMinutes) -gt $BUDGET_MIN) { $res['truncated'] = ('replica time budget ({0} min)' -f $BUDGET_MIN); break }
        $stem = '{0}-{1}-{2}-{3}-{4:D2}' -f $VariantName, $Mode, $Target, $Flavor, $i
        $mk = Join-Path $dirM ($stem + '.mark')
        $lg = Join-Path $dirM ($stem + '.log')
        foreach ($f in @($mk, $lg)) { try { Remove-Item -LiteralPath $f -Force -ErrorAction SilentlyContinue } catch { } }
        if ($Target -eq 'marker') {
            $file = [string]$Variant.marker
            $params = '"' + $mk + '"'
        } else {
            $file = $cmdExe
            $params = '/c type nul > "' + $mk + '"'
        }
        $argStr = '{0} {1} {2} {3}' -f $Mode, (ConvertTo-CmdArg $file), (ConvertTo-CmdArg $params), (ConvertTo-CmdArg $lg)
        if ($Flavor -eq 'nolog') { $argStr = $argStr + ' nolog' }

        $run = Start-SeExit -Exe ([string]$Variant.se) -ArgString $argStr
        $t0 = Get-Date
        $found = $false
        $deadline = $t0.AddSeconds($WAIT_SEC)
        while ((Get-Date) -lt $deadline) {
            if (Test-Path -LiteralPath $mk) { $found = $true; break }
            Start-Sleep -Milliseconds 100
        }
        if ((-not $found) -and (Test-Path -LiteralPath $mk)) { $found = $true }
        $ms = [int]((Get-Date) - $t0).TotalMilliseconds

        $res['runs'] = [int]$res['runs'] + 1
        if ($found) { $res['launched'] = [int]$res['launched'] + 1; $times.Add([double]$ms) } else { $res['not_launched'] = [int]$res['not_launched'] + 1 }
        $ec = 'timeout'
        if (-not $run['timed_out']) { $ec = [string]$run['rc'] }
        if ($run['error']) { $ec = 'error:' + $run['error'] }
        Add-Dist $res['exit_code_dist'] $ec

        $retS = '-'
        $errS = '-'
        if ($Flavor -eq 'log') {
            $lt = Read-TextUtf8 $lg
            if ($lt) {
                $m = [regex]::Match($lt, 'ret=(-?\d+)\s+lasterr=(\d+)')
                if ($m.Success) { $retS = $m.Groups[1].Value; $errS = $m.Groups[2].Value } else { $retS = 'unparsed'; $errS = 'unparsed' }
            } else {
                $retS = 'no-log-line'
                $errS = 'no-log-line'
            }
            Add-Dist $res['ret_dist'] $retS
            Add-Dist $res['lasterr_dist'] $errS
        }
        $res['run_log'].Add(('{0:D2}: launched={1} ms_to_marker={2} se_exit_rc={3} se_exit_ms={4} ret={5} lasterr={6}' -f $i, $found, $ms, $ec, $run['ms'], $retS, $errS))
        if ($found) { $miss = 0 } else { $miss = $miss + 1 }
        if ($miss -ge $MISS_STOP) { $res['truncated'] = ('early stop after {0} consecutive runs without a launch' -f $MISS_STOP); break }
    }
    if ($times.Count -gt 0) {
        $res['launch_ms_min'] = [int]($times | Measure-Object -Minimum).Minimum
        $res['launch_ms_max'] = [int]($times | Measure-Object -Maximum).Maximum
        $res['launch_ms_avg'] = [int]($times | Measure-Object -Average).Average
    }
    return $res
}

try {
    if ((Get-MinutesLeft) -lt 8) {
        $RP['skipped'] = ('not enough job time left (minutes left {0})' -f (Get-MinutesLeft))
        $RP['summary'].Add($RP['skipped'])
    } else {
        $build = Build-Replica
        $RP['build'] = $build
        $order = New-Object System.Collections.Generic.List[string]
        foreach ($n in @('x64', 'host')) {
            $p = $build.variants.PSObject.Properties[$n]
            if ($p -and $p.Value.ok) { $order.Add($n) }
        }
        if ($order.Count -eq 0) {
            $RP['skipped'] = 'no replica variant could be built (see build)'
            $RP['summary'].Add($RP['skipped'])
        }
        foreach ($vn in $order) {
            $variant = $build.variants.PSObject.Properties[$vn].Value
            $vres = [ordered]@{ target_triple = [string]$variant.target; se_exit = [string]$variant.se; combos = [ordered]@{} }
            $RP['variants'][$vn] = $vres
            foreach ($mode in @('main', 'thread')) {
                foreach ($tgt in @('marker', 'cmd')) {
                    foreach ($flavor in @('log', 'nolog')) {
                        if ((Get-MinutesLeft) -lt 5) { $RP['truncated'] = $true; break }
                        if ((((Get-Date) - $REPLICA_START).TotalMinutes) -gt $BUDGET_MIN) { $RP['truncated'] = $true; break }
                        $key = '{0}|{1}|{2}' -f $mode, $tgt, $flavor
                        Write-Log ('combo {0} {1} ...' -f $vn, $key)
                        $c = Invoke-ReplicaCombo -Variant $variant -VariantName $vn -Mode $mode -Target $tgt -Flavor $flavor
                        $vres['combos'][$key] = $c
                        $line = '{0} {1}: launched {2}/{3} (not launched {4}); ret_dist={5}; exit_code_dist={6}' -f $vn, $key, $c['launched'], $c['runs'], $c['not_launched'], (ConvertTo-Json -InputObject $c['ret_dist'] -Compress), (ConvertTo-Json -InputObject $c['exit_code_dist'] -Compress)
                        $RP['summary'].Add($line)
                        Write-Log $line
                        Save-Replica
                    }
                }
            }
        }
    }
} catch {
    Add-DiagError 'replica main' $_
} finally {
    $RP['finished'] = (Get-IsoNow)
    Save-Replica
    Save-Text 'replica-summary.txt' (($RP['summary'].ToArray()) -join "`r`n")
    Complete-DiagScript 'replica'
}
exit 0
