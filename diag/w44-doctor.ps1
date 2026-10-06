# diag/w44-doctor.ps1 -- is `cys.exe doctor` slow/hanging on Windows a 0.14.44 regression? Runs the installed cys.exe doctor of 0.14.44 (artifact)
# and of public 0.14.43 under the same conditions, streaming stdout/stderr to files line by line (max 600 s). Windows PowerShell 5.1, ASCII only.
$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'
. (Join-Path $PSScriptRoot 'lib.ps1')
Start-DiagScript 'w44-doctor'
$WORK = Join-Path $env:RUNNER_TEMP 'w44d'
New-Item -ItemType Directory -Path $WORK -Force | Out-Null
$INST = Join-Path $env:LOCALAPPDATA 'cys'
$R = [ordered]@{ os = [string]$env:DIAG_MATRIX_OS; runs = [ordered]@{}; errors = @() }
function Limit-Text { param([string]$Text, [int]$Max = 400) if ($null -eq $Text) { return '' }; $t = $Text.Trim(); if ($t.Length -gt $Max) { return $t.Substring(0, $Max) + '...' }; return $t }
function SaveR { Save-Json 'w44-doctor-results.json' $R 10 }

function Get-Rows { try { return @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop | ForEach-Object { [pscustomobject]@{ Name = [string]$_.Name; Id = [int]$_.ProcessId; Ppid = [int]$_.ParentProcessId; Cmd = [string]$_.CommandLine; Path = [string]$_.ExecutablePath } }) } catch { return @() } }
function Get-Desc { param([int]$Root, $Rows)
    $out = New-Object System.Collections.Generic.List[object]; $q = New-Object System.Collections.Generic.Queue[int]; $q.Enqueue($Root); $seen = @{}
    while ($q.Count -gt 0) { $c = $q.Dequeue(); foreach ($r in $Rows) { if ($r.Ppid -eq $c -and -not $seen.ContainsKey($r.Id)) { $seen[$r.Id] = 1; $out.Add($r); $q.Enqueue($r.Id) } } }
    return $out.ToArray()
}
function Stop-Mine {
    foreach ($p in (Get-Rows)) {
        $mine = $false
        if ($p.Path -and $p.Path.StartsWith($INST + '\', [System.StringComparison]::OrdinalIgnoreCase)) { $mine = $true }
        if ($p.Cmd -and $p.Cmd -like '*javis_hud_bridge.py*') { $mine = $true }
        if ($mine) { try { Stop-Process -Id $p.Id -Force -ErrorAction Stop } catch { } }
    }
    Start-Sleep -Milliseconds 800
}
function Install-From { param([string]$Exe)
    Stop-Mine
    $p = Start-Process -FilePath $Exe -ArgumentList '/S' -PassThru
    $null = $p.Handle
    if (-not $p.WaitForExit(420000)) { Stop-ProcessTree -ProcessId $p.Id }
    Start-Sleep -Seconds 8
    Stop-Mine
    return $p.ExitCode
}

# installers: 0.14.44 = artifact of the build run in diag/w44-input.json (build_run_id), 0.14.43 = public release
$inp = ConvertFrom-Json (Read-TextUtf8 (Join-Path $env:GITHUB_WORKSPACE 'diag\w44-input.json'))
$hdr = @{ Authorization = ('Bearer ' + $env:GITHUB_TOKEN); Accept = 'application/vnd.github+json' }
$new = $null
try {
    $l = Invoke-RestMethod -Uri ('https://api.github.com/repos/{0}/actions/runs/{1}/artifacts?per_page=100' -f $env:GITHUB_REPOSITORY, $inp.build_run_id) -Headers $hdr -TimeoutSec 60
    $art = @($l.artifacts | Where-Object { $_.name -eq 'cys-windows-x64-nsis' -and -not $_.expired })[0]
    $zip = Join-Path $WORK 'new.zip'; $cfg = Join-Path $WORK 'curl.cfg'
    [System.IO.File]::WriteAllText($cfg, ('header = "Authorization: Bearer {0}"' + "`n") -f $env:GITHUB_TOKEN)
    $null = Invoke-Proc -File (Join-Path $env:windir 'System32\curl.exe') -Arguments ('-L --fail --retry 3 -sS -K "{0}" -o "{1}" "{2}"' -f $cfg, $zip, [string]$art.archive_download_url) -TimeoutSec 900
    Remove-Item -LiteralPath $cfg -Force -ErrorAction SilentlyContinue
    Expand-Archive -LiteralPath $zip -DestinationPath (Join-Path $WORK 'new') -Force
    $new = (Get-ChildItem -LiteralPath (Join-Path $WORK 'new') -Filter '*.exe' -Recurse | Select-Object -First 1).FullName
} catch { $R['errors'] += ('new installer: ' + $_.Exception.Message) }
$old = $null
try {
    $d = Join-Path $WORK 'oldinst'; New-Item -ItemType Directory -Path $d -Force | Out-Null
    $node = Find-Exe 'node.exe'
    $cmd = ('"{0}" "{1}" --tag v0.14.43 --suffix _x64-setup.exe --out "{2}" --report "{3}"' -f $node, (Join-Path $env:GITHUB_WORKSPACE 'diag\fetch-release-asset.mjs'), $d, (Join-Path $WORK 'old-fetch.json'))
    $null = Invoke-Proc -File (Join-Path $env:windir 'System32\cmd.exe') -Arguments ('/d /c "' + $cmd + '"') -TimeoutSec 900
    $old = (Get-ChildItem -LiteralPath $d -Filter '*.exe' | Select-Object -First 1).FullName
} catch { $R['errors'] += ('old installer: ' + $_.Exception.Message) }
$R['installers'] = [ordered]@{ new = $new; old = $old }
SaveR

$DEPT = '{"depts":{"dept-1":{"socket":"\\\\.\\pipe\\cys-dept-dept-1","pack_dir":"C:\\Users\\runneradmin/.cys/pack-dept-dept-1","role":"dept-master","reserved_at":1791264527.7,"account_dir":"C:/Users/runneradmin/.cys/claude-default-dept-1","cwd":"C:/Users/runneradmin"}}}'

# one doctor run: scenario = 'plain' (default-pipe daemon only) | 'dept' (+ a department registered whose daemon is dead, the state after the TEAM scene)
function Invoke-Doctor { param([string]$Label, [string]$Scenario, [int]$MaxSec = 600)
    $o = [ordered]@{ label = $Label; scenario = $Scenario; version = $null; daemon_ready = $false; seconds = $null; finished = $false; timed_out = $false; rc = $null; stdout_lines = 0; stderr_lines = 0; stdout_bytes = 0; last_lines = @(); snapshots = @() }
    $R['runs'][$Label] = $o
    Stop-Mine
    $dot = Join-Path $env:USERPROFILE '.cys'
    if (Test-Path -LiteralPath $dot) { Remove-Item -LiteralPath $dot -Recurse -Force -ErrorAction SilentlyContinue }
    $cys = Join-Path $INST 'cys.exe'; $cysd = Join-Path $INST 'cysd.exe'
    $o['version'] = (Invoke-Proc -File $cys -Arguments '--version' -TimeoutSec 30).out.Trim()
    # the daemon on the DEFAULT pipe, like the app's
    $errF = Join-Path $WORK ($Label + '-cysd.err.txt')
    $dp = Start-Process -FilePath $cysd -PassThru -WindowStyle Hidden -RedirectStandardError $errF -RedirectStandardOutput (Join-Path $WORK ($Label + '-cysd.out.txt'))
    $t0 = Get-Date
    while (((Get-Date) - $t0).TotalSeconds -lt 90) { Start-Sleep -Seconds 2; $pg = Invoke-Proc -File $cys -Arguments 'ping' -TimeoutSec 10; if ($pg.rc -eq 0) { $o['daemon_ready'] = $true; break } }
    if ($Scenario -eq 'dept') {
        New-Item -ItemType Directory -Path $dot -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $dot 'depts.json'), $DEPT, (New-Object System.Text.UTF8Encoding($false)))
        Start-Sleep -Seconds 2
    }
    Start-Sleep -Seconds 8
    # doctor, output streamed line by line into files
    $of = Join-Path $global:DiagOut ('w44-doctor-' + $Label + '-stdout.txt'); $ef = Join-Path $global:DiagOut ('w44-doctor-' + $Label + '-stderr.txt')
    foreach ($f in @($of, $ef)) { [System.IO.File]::WriteAllText($f, '') }
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $cys; $psi.Arguments = 'doctor'; $psi.UseShellExecute = $false; $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true; $psi.RedirectStandardInput = $true
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8; $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $p = New-Object System.Diagnostics.Process
    $p.StartInfo = $psi
    $lock = New-Object object
    $script:dOut = New-Object System.Collections.Generic.List[string]
    $script:dErr = New-Object System.Collections.Generic.List[string]
    $ho = Register-ObjectEvent -InputObject $p -EventName OutputDataReceived -Action { if ($null -ne $EventArgs.Data) { [System.IO.File]::AppendAllText($Event.MessageData.o, $EventArgs.Data + "`r`n", [System.Text.Encoding]::UTF8) } } -MessageData @{ o = $of }
    $he = Register-ObjectEvent -InputObject $p -EventName ErrorDataReceived -Action { if ($null -ne $EventArgs.Data) { [System.IO.File]::AppendAllText($Event.MessageData.o, $EventArgs.Data + "`r`n", [System.Text.Encoding]::UTF8) } } -MessageData @{ o = $ef }
    $ts = Get-Date
    [void]$p.Start()
    try { $p.StandardInput.Close() } catch { }
    $p.BeginOutputReadLine(); $p.BeginErrorReadLine()
    $snapAt = @(20, 60, 180, 400)
    $si = 0
    while (-not $p.HasExited -and ((Get-Date) - $ts).TotalSeconds -lt $MaxSec) {
        Start-Sleep -Seconds 1
        $el = ((Get-Date) - $ts).TotalSeconds
        if ($si -lt $snapAt.Count -and $el -ge $snapAt[$si]) {
            $si++
            $rows = Get-Rows
            $kids = @(Get-Desc $p.Id $rows | ForEach-Object { '{0}#{1} (parent {2}) {3}' -f $_.Name, $_.Id, $_.Ppid, (Limit-Text $_.Cmd 260) })
            $net = (Invoke-Proc -File (Join-Path $env:windir 'System32\netstat.exe') -Arguments '-ano' -TimeoutSec 30).out
            $mine = @($net -split "`r?`n" | Where-Object { $_ -match '\b127\.0\.0\.1:(8642|\d+)\b' } | Select-Object -First 40)
            $pp = @(); try { $pp = @([System.IO.Directory]::GetFiles('\\.\pipe\') | ForEach-Object { $_.Substring($_.LastIndexOf('\') + 1) } | Where-Object { $_ -match 'cys' }) } catch { }
            $lastOut = ''; try { $lastOut = (Get-Content -LiteralPath $of -Tail 3 -ErrorAction SilentlyContinue) -join ' | ' } catch { }
            $o['snapshots'] += [ordered]@{ at_sec = [int]$el; doctor_children = $kids; cys_pipes = $pp; netstat_loopback = $mine; stdout_tail = (Limit-Text $lastOut 400) }
            Save-Json 'w44-doctor-results.json' $R 10
        }
    }
    $o['seconds'] = [math]::Round(((Get-Date) - $ts).TotalSeconds, 1)
    if ($p.HasExited) { $o['finished'] = $true; $p.WaitForExit(); $o['rc'] = $p.ExitCode } else {
        $o['timed_out'] = $true
        $rows = Get-Rows
        $o['children_at_timeout'] = @(Get-Desc $p.Id $rows | ForEach-Object { '{0}#{1} {2}' -f $_.Name, $_.Id, (Limit-Text $_.Cmd 260) })
        try { Stop-ProcessTree -ProcessId $p.Id } catch { }
    }
    Start-Sleep -Milliseconds 800
    Unregister-Event -SourceIdentifier $ho.Name -ErrorAction SilentlyContinue; Unregister-Event -SourceIdentifier $he.Name -ErrorAction SilentlyContinue
    $txt = [System.IO.File]::ReadAllText($of, [System.Text.Encoding]::UTF8)
    $o['stdout_bytes'] = $txt.Length
    $o['stdout_lines'] = @($txt -split "`r?`n" | Where-Object { $_ -ne '' }).Count
    $o['stderr_lines'] = @([System.IO.File]::ReadAllText($ef) -split "`r?`n" | Where-Object { $_ -ne '' }).Count
    $o['last_lines'] = @($txt -split "`r?`n" | Where-Object { $_ -ne '' } | Select-Object -Last 6 | ForEach-Object { Limit-Text $_ 300 })
    try { Stop-Process -Id $dp.Id -Force -ErrorAction SilentlyContinue } catch { }
    Stop-Mine
    SaveR
}

foreach ($v in @(@('new', $new), @('old', $old))) {
    if (-not $v[1]) { continue }
    $code = Install-From $v[1]
    $R[$v[0] + '_install_rc'] = $code
    Invoke-Doctor ($v[0] + '-plain') 'plain'
    Invoke-Doctor ($v[0] + '-dept') 'dept'
    # a second pass with the doctor run twice in a row on the same daemon is not needed; repeat the dept case once more for timing noise
    Invoke-Doctor ($v[0] + '-dept2') 'dept'
}
SaveR
Complete-DiagScript 'w44-doctor'
