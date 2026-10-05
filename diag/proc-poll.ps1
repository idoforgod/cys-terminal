# diag/proc-poll.ps1 -- the FALLBACK of the process audit of scene UPGRADE (diag/app-upgrade.ps1, UPG-2). Started in a second
# PowerShell the moment the upgraded app is seen, only when Security auditing (4688 / 4689) could not be used.
# Every -IntervalMs (200) for -Seconds (30): the cys.exe / cysd.exe / cys-app.exe processes of the machine (Win32_Process: pid, parent,
# creation time, command line). A process is one row, keyed by pid + creation time; first_seen / last_seen are poll times.
# There is NO exit code in this view, and a process that lives shorter than one interval can be missed.
# Observation only. Windows PowerShell 5.1, ASCII only.
param([int]$Seconds = 30, [int]$IntervalMs = 200, [string]$OutFile = '')
$ErrorActionPreference = 'Continue'
function Get-PollIso { param($Value) return ([datetime]$Value).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.fffZ') }
$rows = @{}
$order = New-Object System.Collections.Generic.List[string]
$samples = 0
$errors = 0
$started = Get-PollIso (Get-Date)
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$lastWrite = 0
function Save-Poll {
    param([bool]$Final, [string]$File, [int]$Sec, [int]$Ms)
    if (-not $File) { return }
    try {
        $list = New-Object System.Collections.Generic.List[object]
        foreach ($k in $order.ToArray()) { $list.Add($rows[$k]) }
        $o = [ordered]@{ script = 'proc-poll.ps1'; started = $started; written = (Get-PollIso (Get-Date)); final = $Final; seconds = $Sec; interval_ms = $Ms; samples = $samples; errors = $errors; rows = $list.ToArray() }
        $json = ConvertTo-Json -InputObject $o -Depth 5
        [System.IO.File]::WriteAllText($File, $json, (New-Object System.Text.UTF8Encoding($false)))
    } catch { }
}
while ($sw.Elapsed.TotalSeconds -lt $Seconds) {
    $now = Get-PollIso (Get-Date)
    try {
        $ps = @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 10 -Filter "Name = 'cys.exe' OR Name = 'cysd.exe' OR Name = 'cys-app.exe'" -ErrorAction Stop)
        $samples++
        foreach ($p in $ps) {
            if ($null -eq $p) { continue }
            $created = ''
            try { if ($null -ne $p.CreationDate) { $created = Get-PollIso $p.CreationDate } } catch { }
            $key = ([string]$p.ProcessId) + '@' + $created
            if (-not $rows.ContainsKey($key)) {
                $rows[$key] = [ordered]@{ name = [string]$p.Name; pid = [int]$p.ProcessId; ppid = [int]$p.ParentProcessId; path = [string]$p.ExecutablePath; created = $created; cmd = [string]$p.CommandLine; first_seen = $now; last_seen = $now; polls = 0 }
                $order.Add($key)
            }
            $rows[$key]['last_seen'] = $now
            $rows[$key]['polls'] = [int]$rows[$key]['polls'] + 1
        }
    } catch { $errors++ }
    if (($sw.Elapsed.TotalSeconds - $lastWrite) -ge 5) { $lastWrite = $sw.Elapsed.TotalSeconds; Save-Poll $false $OutFile $Seconds $IntervalMs }
    Start-Sleep -Milliseconds $IntervalMs
}
Save-Poll $true $OutFile $Seconds $IntervalMs
exit 0
