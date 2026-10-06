# diag/w44-procs.ps1 -- can an UNELEVATED token read the start times of system processes (wininit.exe, csrss.exe ...)?
# sysinfo 0.33.1 (windows/process.rs): OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ) first, then OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION),
# then GetProcessTimes(handle); no handle = start time 0. This script asks the same questions in the same order, for the system processes and for the whole
# wininit.exe branch (descendants by parent id), once with the runner's elevated administrator token and once as a new standard local user.
# Windows PowerShell 5.1, ASCII only. Modes: (none) = orchestrator ; -Collect <outfile> = the measuring part (runs under the token being measured).
param([string]$Collect = '')
$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'

$cs = @'
using System;
using System.Runtime.InteropServices;
using System.Collections.Generic;
public static class W44P {
    [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool GetProcessTimes(IntPtr h, out long c, out long e, out long k, out long u);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr CreateToolhelp32Snapshot(uint flags, uint pid);
    [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern bool Process32FirstW(IntPtr s, ref PE e);
    [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern bool Process32NextW(IntPtr s, ref PE e);
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    public struct PE { public uint size; public uint usage; public uint pid; public IntPtr heap; public uint module; public uint threads; public uint ppid; public int pri; public uint flags; [MarshalAs(UnmanagedType.ByValTStr, SizeConst=260)] public string exe; }
    public static List<string[]> Snapshot() {
        var res = new List<string[]>();
        IntPtr s = CreateToolhelp32Snapshot(2, 0);
        PE e = new PE(); e.size = (uint)Marshal.SizeOf(typeof(PE));
        if (Process32FirstW(s, ref e)) { do { res.Add(new string[] { e.pid.ToString(), e.ppid.ToString(), e.exe }); } while (Process32NextW(s, ref e)); }
        CloseHandle(s);
        return res;
    }
    // the sysinfo order: (QUERY_INFORMATION|VM_READ) -> QUERY_LIMITED_INFORMATION -> none. Returns: first_err (0 = ok), used ("full"|"limited"|"none"), second_err, times_ok, creation (FILETIME 100ns since 1601 or 0)
    public static string[] Probe(uint pid) {
        uint full = 0x0400 | 0x0010; uint limited = 0x1000;
        string used = "none"; int err1 = 0, err2 = 0; long creation = 0; bool tok = false; int errT = 0;
        IntPtr h = OpenProcess(full, false, pid);
        if (h == IntPtr.Zero) { err1 = Marshal.GetLastWin32Error(); h = OpenProcess(limited, false, pid); if (h == IntPtr.Zero) { err2 = Marshal.GetLastWin32Error(); } else { used = "limited"; } } else { used = "full"; }
        if (h != IntPtr.Zero) { long c, e, k, u; tok = GetProcessTimes(h, out c, out e, out k, out u); if (tok) creation = c; else errT = Marshal.GetLastWin32Error(); CloseHandle(h); }
        return new string[] { err1.ToString(), used, err2.ToString(), tok.ToString(), creation.ToString(), errT.ToString() };
    }
}
'@
Add-Type -TypeDefinition $cs -ErrorAction Stop

function Fmt-Ft { param([long]$ft) if ($ft -le 0) { return '' }; try { return [DateTime]::FromFileTimeUtc($ft).ToString('o') } catch { return 'bad:' + $ft } }

if ($Collect -ne '') {
    # ---- the measuring part ----
    $o = [ordered]@{ who = $null; groups = $null; elevated = $null; integrity = $null; system = @(); wininit_branch = [ordered]@{}; parents = @(); error = $null }
    try {
        $o['who'] = (& whoami.exe) -join ' '
        $o['groups'] = @(& whoami.exe /groups /fo csv /nh)
        $o['elevated'] = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
        $o['integrity'] = @($o['groups'] | Where-Object { $_ -match 'Mandatory Label' })
        $snap = @([W44P]::Snapshot() | ForEach-Object { [pscustomobject]@{ Pid = [uint32]$_[0]; Ppid = [uint32]$_[1]; Exe = [string]$_[2] } })
        $byPid = @{}
        foreach ($r in $snap) { $byPid[[uint32]$r.Pid] = $r }
        $o['snapshot_count'] = $snap.Count
        function ProbeRow { param($r)
            $p = [W44P]::Probe([uint32]$r.Pid)
            return [ordered]@{ pid = [int]$r.Pid; ppid = [int]$r.Ppid; exe = $r.Exe; full_open_err = [int]$p[0]; used = $p[1]; limited_open_err = [int]$p[2]; times_ok = ($p[3] -eq 'True'); times_err = [int]$p[5]; creation = (Fmt-Ft ([long]$p[4])); creation_ft = [long]$p[4] }
        }
        $names = @('wininit.exe', 'csrss.exe', 'services.exe', 'smss.exe', 'lsass.exe', 'winlogon.exe', 'System')
        $sys = New-Object System.Collections.Generic.List[object]
        foreach ($r in $snap) { if ($names -contains $r.Exe -or $r.Pid -eq 4) { $sys.Add((ProbeRow $r)) } }
        $o['system'] = $sys.ToArray()
        # parents of wininit.exe and csrss.exe: is the parent id a live process? and is it older than the child (a reused id would be NEWER)
        $par = New-Object System.Collections.Generic.List[object]
        foreach ($r in $snap) {
            if ($r.Exe -eq 'wininit.exe' -or $r.Exe -eq 'csrss.exe') {
                $child = ProbeRow $r
                $pe = $byPid[[uint32]$r.Ppid]
                $parentRow = $null
                if ($null -ne $pe) { $parentRow = ProbeRow $pe }
                $par.Add([ordered]@{ child = $child; parent_pid = [int]$r.Ppid; parent_alive_in_snapshot = ($null -ne $pe); parent_exe = $(if ($pe) { $pe.Exe } else { $null }); parent_probe = $parentRow })
            }
        }
        $o['parents'] = $par.ToArray()
        # the whole branch below wininit.exe (by parent id, like a descendant count)
        $roots = @($snap | Where-Object { $_.Exe -eq 'wininit.exe' })
        $br = New-Object System.Collections.Generic.List[object]
        foreach ($root in $roots) {
            $q = New-Object System.Collections.Generic.Queue[uint32]; $q.Enqueue([uint32]$root.Pid); $seen = @{}; $seen[[uint32]$root.Pid] = 1
            while ($q.Count -gt 0) { $c = $q.Dequeue(); foreach ($r in $snap) { if ($r.Ppid -eq $c -and -not $seen.ContainsKey([uint32]$r.Pid) -and $r.Pid -ne $r.Ppid) { $seen[[uint32]$r.Pid] = 1; $br.Add((ProbeRow $r)); $q.Enqueue([uint32]$r.Pid) } } }
        }
        $rows = $br.ToArray()
        $o['wininit_branch'] = [ordered]@{ roots = @($roots | ForEach-Object { [int]$_.Pid }); descendants = $rows.Count; start_time_read = @($rows | Where-Object { $_.times_ok }).Count; no_handle = @($rows | Where-Object { $_.used -eq 'none' }).Count; opened_by = [ordered]@{ full = @($rows | Where-Object { $_.used -eq 'full' }).Count; limited = @($rows | Where-Object { $_.used -eq 'limited' }).Count; none = @($rows | Where-Object { $_.used -eq 'none' }).Count }; error_codes_of_unreadable = @($rows | Where-Object { -not $_.times_ok } | Group-Object { '{0}/{1}' -f $_.full_open_err, $_.limited_open_err } | ForEach-Object { '{0} x{1}' -f $_.Name, $_.Count }); rows = $rows }
        # the same for ALL processes (the number the report is about: how many system processes does a seat's child count as descendants)
        $all = New-Object System.Collections.Generic.List[object]
        foreach ($r in $snap) { $all.Add((ProbeRow $r)) }
        $ar = $all.ToArray()
        $o['all_processes'] = [ordered]@{ count = $ar.Count; start_time_read = @($ar | Where-Object { $_.times_ok }).Count; none = @($ar | Where-Object { $_.used -eq 'none' }).Count; unreadable_names = @($ar | Where-Object { -not $_.times_ok } | ForEach-Object { '{0}#{1}' -f $_.exe, $_.pid } | Select-Object -First 80) }
    } catch { $o['error'] = $_.Exception.Message + ' ' + $_.InvocationInfo.PositionMessage }
    [System.IO.File]::WriteAllText($Collect, (ConvertTo-Json -InputObject $o -Depth 8), (New-Object System.Text.UTF8Encoding($false)))
    exit 0
}

# ---------------- orchestrator ----------------
. (Join-Path $PSScriptRoot 'lib.ps1')
Start-DiagScript 'w44-procs'
$OUT = $global:DiagOut
$PUB = 'C:\w44pub'
New-Item -ItemType Directory -Path $PUB -Force | Out-Null
& icacls.exe $PUB /grant 'Everyone:(OI)(CI)M' | Out-Null
Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $PUB 'w44-procs.ps1') -Force
$R = [ordered]@{ os = [string]$env:DIAG_MATRIX_OS; elevated_run = $null; standard_user_run = $null; taskkill_probe = $null; errors = @() }

# 1) elevated: the runner's own token
$f1 = Join-Path $OUT 'w44-procs-elevated.json'
$x = Invoke-Proc -File (Join-Path $env:windir 'System32\WindowsPowerShell\v1.0\powershell.exe') -Arguments ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}" -Collect "{1}"' -f (Join-Path $PUB 'w44-procs.ps1'), $f1) -TimeoutSec 300
$R['elevated_run'] = [ordered]@{ rc = $x.rc; err = (([string]$x.err).Trim()); file = 'w44-procs-elevated.json' }

# 2) a new standard local user (member of Users only), started with CreateProcessWithLogon (a new logon session, medium integrity, no admin group)
try {
    $pw = 'W44-' + [guid]::NewGuid().ToString('N').Substring(0, 12) + '!aA1'
    & net.exe user w44std $pw /add /y | Out-Null
    $sec = ConvertTo-SecureString $pw -AsPlainText -Force
    $cred = New-Object System.Management.Automation.PSCredential('.\w44std', $sec)
    $f2 = Join-Path $PUB 'w44-procs-standard.json'
    $so = Join-Path $PUB 'std-stdout.txt'; $se = Join-Path $PUB 'std-stderr.txt'
    $p = Start-Process -FilePath (Join-Path $env:windir 'System32\WindowsPowerShell\v1.0\powershell.exe') -ArgumentList ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}" -Collect "{1}"' -f (Join-Path $PUB 'w44-procs.ps1'), $f2) -Credential $cred -LoadUserProfile -WorkingDirectory $PUB -PassThru -Wait -RedirectStandardOutput $so -RedirectStandardError $se
    $R['standard_user_run'] = [ordered]@{ rc = $p.ExitCode; stderr = $(if (Test-Path $se) { (Get-Content -LiteralPath $se -Raw) } else { '' }); file = 'w44-procs-standard.json'; exists = (Test-Path -LiteralPath $f2) }
    if (Test-Path -LiteralPath $f2) { Copy-Item -LiteralPath $f2 -Destination (Join-Path $OUT 'w44-procs-standard.json') -Force }
} catch { $R['errors'] += ('standard user: ' + $_.Exception.Message) }
try { & net.exe user w44std /delete | Out-Null } catch { }

# 3) taskkill /T and parent ids - OWN processes only. A starts B, A is killed, then a process that happens to get A's id is made the "new parent": does
#    `taskkill /PID <that id> /T` kill B (older than its "parent")? Only attempted when an id is really reused for one of OUR OWN processes.
try {
    $T = [ordered]@{ attempted = $true; reused = $false; spawns = 0; a_pid = $null; b_pid = $null; new_owner_pid = $null; b_alive_before = $null; taskkill_rc = $null; taskkill_out = $null; b_alive_after = $null; note = '' }
    $ping = Join-Path $env:windir 'System32\ping.exe'
    $a = Start-Process -FilePath (Join-Path $env:windir 'System32\cmd.exe') -ArgumentList '/c ping -n 600 127.0.0.1 > nul' -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds 2
    $aPid = $a.Id
    $b = @(Get-CimInstance Win32_Process -Filter ("ParentProcessId = {0}" -f $aPid))
    $T['a_pid'] = $aPid
    if ($b.Count -gt 0) { $T['b_pid'] = [int]$b[0].ProcessId }
    # A dies (TerminateProcess); B (the ping) keeps running with a parent id that no longer exists
    $a.Kill(); $a.WaitForExit(); $a.Dispose()
    Start-Sleep -Milliseconds 500
    $keep = New-Object System.Collections.Generic.List[object]
    for ($i = 0; $i -lt 4000 -and -not $T['reused']; $i++) {
        $pp = Start-Process -FilePath $ping -ArgumentList '-n 300 127.0.0.1' -PassThru -WindowStyle Hidden
        $T['spawns'] = $i + 1
        if ($pp.Id -eq $aPid) { $T['reused'] = $true; $T['new_owner_pid'] = $pp.Id; $keep.Add($pp) } else { try { $pp.Kill() } catch { }; $pp.Dispose() }
    }
    if ($T['reused'] -and $T['b_pid']) {
        $bp = Get-Process -Id $T['b_pid'] -ErrorAction SilentlyContinue
        $T['b_alive_before'] = [bool]$bp
        $T['b_ppid_now'] = [int](Get-CimInstance Win32_Process -Filter ("ProcessId = {0}" -f $T['b_pid'])).ParentProcessId
        $T['b_created'] = [string]$bp.StartTime.ToString('o')
        $T['new_owner_created'] = [string]$keep[0].StartTime.ToString('o')
        $tk = Invoke-Proc -File (Join-Path $env:windir 'System32\taskkill.exe') -Arguments ('/PID {0} /T /F' -f $aPid) -TimeoutSec 60
        $T['taskkill_rc'] = $tk.rc; $T['taskkill_out'] = (([string]$tk.out + ' ' + [string]$tk.err).Trim())
        Start-Sleep -Seconds 1
        $T['b_alive_after'] = [bool](Get-Process -Id $T['b_pid'] -ErrorAction SilentlyContinue)
        $T['note'] = 'B is OLDER than its parent-id owner (a reused id); if B died, taskkill /T followed the parent id without comparing creation times'
    } else { $T['note'] = 'the id of A was not handed to another own process within the attempts: not measured' }
    foreach ($k in $keep) { try { $k.Kill() } catch { } }
    if ($T['b_pid']) { try { Stop-Process -Id $T['b_pid'] -Force -ErrorAction SilentlyContinue } catch { } }
    $R['taskkill_probe'] = $T
} catch { $R['errors'] += ('taskkill probe: ' + $_.Exception.Message) }
Save-Json 'w44-procs-results.json' $R 8
Complete-DiagScript 'w44-procs'
