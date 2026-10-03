# diag/lib.ps1 -- common helpers for the diag-win11-update harness.
# Dot-source this file:  . (Join-Path $PSScriptRoot 'lib.ps1')
# Target: Windows PowerShell 5.1 (also fine on 7.x). ASCII only. No ??, ?., ternary, && or ||.
# Design rule: helpers never throw to the caller; failures are logged and returned as data.
# Output folder (OUT)  = $env:RUNNER_TEMP\diag-out   (plain text / json / png / evtx only; published)
# Work folder   (WORK) = $env:RUNNER_TEMP\diag-work  (big files; never published)

$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'

if ($null -eq $global:DiagLibLoaded) {
    $global:DiagLibLoaded = $true
    $global:DiagRoot = $PSScriptRoot
    if (-not $env:RUNNER_TEMP) { $env:RUNNER_TEMP = [System.IO.Path]::GetTempPath().TrimEnd('\') }
    $global:DiagOut = Join-Path $env:RUNNER_TEMP 'diag-out'
    $global:DiagWork = Join-Path $env:RUNNER_TEMP 'diag-work'
    foreach ($d in @($global:DiagOut, $global:DiagWork)) {
        if (-not (Test-Path -LiteralPath $d)) {
            try { New-Item -ItemType Directory -Path $d -Force | Out-Null } catch { }
        }
    }
    $global:DiagLogFile = $null
    $global:DiagScriptName = 'unknown'
    $global:DiagNativeReady = $false
    $global:DiagNativeError = $null
    $global:DiagScreenReady = $false
    $global:DiagErrors = New-Object System.Collections.Generic.List[object]
}

# ---------------------------------------------------------------------------
# basic I/O and logging
# ---------------------------------------------------------------------------

function Get-IsoNow {
    return (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
}

function ConvertTo-IsoUtc {
    param($Value)
    try {
        if ($null -eq $Value) { return $null }
        return ([datetime]$Value).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
    } catch { return [string]$Value }
}

function Write-Utf8NoBom {
    param([string]$Path, [string]$Text)
    $enc = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Text, $enc)
}

function Add-Utf8NoBom {
    param([string]$Path, [string]$Text)
    $enc = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::AppendAllText($Path, $Text, $enc)
}

function Read-TextUtf8 {
    param([string]$Path)
    try {
        if (-not (Test-Path -LiteralPath $Path)) { return $null }
        return [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8)
    } catch { return $null }
}

function Read-FileShared {
    param([string]$Path, [int]$MaxChars = 4000000)
    try {
        if (-not (Test-Path -LiteralPath $Path)) { return '' }
        $fs = New-Object System.IO.FileStream($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
        try {
            $sr = New-Object System.IO.StreamReader($fs, [System.Text.Encoding]::UTF8, $true)
            try { $t = $sr.ReadToEnd() } finally { $sr.Dispose() }
        } finally { $fs.Dispose() }
        if ($t.Length -gt $MaxChars) { $t = $t.Substring(0, $MaxChars) + '...[truncated]' }
        return $t
    } catch { return ('[read failed: ' + $_.Exception.Message + ']') }
}

function Write-Log {
    param([string]$Message, [string]$Level = 'INFO')
    $line = '{0} [{1}] {2}' -f (Get-IsoNow), $Level, $Message
    try { Write-Host $line } catch { }
    try {
        if ($global:DiagLogFile) { Add-Utf8NoBom $global:DiagLogFile ($line + "`r`n") }
    } catch { }
}

function Save-Text {
    param([string]$Name, [string]$Text)
    try {
        Write-Utf8NoBom (Join-Path $global:DiagOut $Name) $Text
    } catch {
        Write-Log ('Save-Text failed for {0}: {1}' -f $Name, $_.Exception.Message) 'WARN'
    }
}

function Save-Json {
    param([string]$Name, $Object, [int]$Depth = 8)
    try {
        $json = ConvertTo-Json -InputObject $Object -Depth $Depth
        Write-Utf8NoBom (Join-Path $global:DiagOut $Name) $json
    } catch {
        Write-Log ('Save-Json failed for {0}: {1}' -f $Name, $_.Exception.Message) 'WARN'
        try {
            $dump = ($_ | Out-String) + "`r`n" + ($Object | Out-String)
            Write-Utf8NoBom (Join-Path $global:DiagOut ($Name + '.error.txt')) $dump
        } catch { }
    }
}

function Format-ErrorText {
    param($ErrRec)
    try {
        $pos = ''
        try { $pos = [string]$ErrRec.InvocationInfo.PositionMessage } catch { }
        return ('{0}: {1} {2}' -f $ErrRec.Exception.GetType().FullName, $ErrRec.Exception.Message, $pos)
    } catch { return 'unprintable error' }
}

function Add-DiagError {
    param([string]$Step, $ErrRec)
    try {
        $txt = Format-ErrorText $ErrRec
        Write-Log ('STEP FAILED [{0}]: {1}' -f $Step, $txt) 'ERROR'
        $global:DiagErrors.Add([ordered]@{ step = $Step; time = (Get-IsoNow); error = $txt })
    } catch { }
}

# Run a script block; never throws. Blocks must mutate shared state through reference types
# (hashtables / lists), not through scalar assignment (the block runs in a child scope).
function Invoke-Safe {
    param([string]$Name, [scriptblock]$Block)
    try {
        & $Block
    } catch {
        Add-DiagError $Name $_
    }
}

function Start-DiagScript {
    param([string]$Name)
    $global:DiagScriptName = $Name
    $global:DiagLogFile = Join-Path $global:DiagOut ('log-' + $Name + '.txt')
    try {
        $sf = Join-Path $global:DiagOut 'job-start.txt'
        if (-not (Test-Path -LiteralPath $sf)) { Write-Utf8NoBom $sf (Get-IsoNow) }
    } catch { }
    Write-Log ('=== {0} start (PowerShell {1}, LanguageMode {2}) ===' -f $Name, $PSVersionTable.PSVersion, $ExecutionContext.SessionState.LanguageMode)
}

function Complete-DiagScript {
    param([string]$Name)
    try {
        $o = [ordered]@{ script = $Name; finished = (Get-IsoNow); error_count = $global:DiagErrors.Count; errors = $global:DiagErrors.ToArray() }
        Save-Json ('done-' + $Name + '.json') $o 5
    } catch { }
    Write-Log ('=== {0} end ===' -f $Name)
}

# ---------------------------------------------------------------------------
# job time budget (so that publish/upload still run inside timeout-minutes)
# ---------------------------------------------------------------------------

function Get-JobElapsedMin {
    try {
        $f = Join-Path $global:DiagOut 'job-start.txt'
        $s = Read-TextUtf8 $f
        if ($s) {
            $dt = [datetime]::Parse($s.Trim(), [System.Globalization.CultureInfo]::InvariantCulture, [System.Globalization.DateTimeStyles]::RoundtripKind)
            return [math]::Round(((Get-Date).ToUniversalTime() - $dt.ToUniversalTime()).TotalMinutes, 1)
        }
    } catch { }
    return 0
}

function Get-JobLimitMin {
    $n = 0
    if ([int]::TryParse([string]$env:DIAG_JOB_LIMIT_MIN, [ref]$n)) {
        if ($n -gt 0) { return $n }
    }
    return 55
}

# minutes left before we must stop measuring (keeps a 4 minute reserve for publish/upload)
function Get-MinutesLeft {
    return ((Get-JobLimitMin) - (Get-JobElapsedMin) - 4)
}

# ---------------------------------------------------------------------------
# process helpers
# ---------------------------------------------------------------------------

function Find-Exe {
    param([string]$Name, [string[]]$Fallbacks = @())
    try {
        $c = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($c) { return [string]$c.Source }
    } catch { }
    foreach ($f in $Fallbacks) {
        if ($f -and (Test-Path -LiteralPath $f)) { return $f }
    }
    return $null
}

function Stop-ProcessTree {
    param([int]$ProcessId)
    try {
        $psi = New-Object System.Diagnostics.ProcessStartInfo
        $psi.FileName = Join-Path $env:windir 'System32\taskkill.exe'
        $psi.Arguments = ('/PID {0} /T /F' -f $ProcessId)
        $psi.UseShellExecute = $false
        $psi.CreateNoWindow = $true
        $psi.RedirectStandardOutput = $true
        $psi.RedirectStandardError = $true
        $k = [System.Diagnostics.Process]::Start($psi)
        $null = $k.StandardOutput.ReadToEndAsync()
        $null = $k.StandardError.ReadToEndAsync()
        [void]$k.WaitForExit(30000)
        $k.Dispose()
    } catch { }
}

# Run an external program: System.Diagnostics.Process with redirected pipes read asynchronously.
# (Start-Process -PassThru loses ExitCode / throws on .Handle for children that exit within milliseconds
#  such as reg.exe and sc.exe; Process.Start keeps its own handle, so rc is reliable.)
# The child gets an empty stdin (never waits for console input). We wait on the process itself with a timeout and
# give the pipes 5 s more to drain (a grandchild that inherited a pipe cannot hang us). Returns an ordered hashtable.
function Invoke-Proc {
    param(
        [string]$File,
        [string]$Arguments = '',
        [int]$TimeoutSec = 120,
        [string]$WorkDir = ''
    )
    $r = [ordered]@{ file = $File; args = $Arguments; rc = $null; timedOut = $false; startError = $null; out = ''; err = ''; ms = 0; pipes_not_drained = $false }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p = $null
    try {
        $psi = New-Object System.Diagnostics.ProcessStartInfo
        $psi.FileName = $File
        $psi.Arguments = $Arguments
        $psi.UseShellExecute = $false
        $psi.CreateNoWindow = $true
        $psi.RedirectStandardOutput = $true
        $psi.RedirectStandardError = $true
        $psi.RedirectStandardInput = $true
        if ($WorkDir -ne '') { $psi.WorkingDirectory = $WorkDir }
        $p = [System.Diagnostics.Process]::Start($psi)
        try { $p.StandardInput.Close() } catch { }
        $tOut = $p.StandardOutput.ReadToEndAsync()
        $tErr = $p.StandardError.ReadToEndAsync()
        if (-not $p.WaitForExit($TimeoutSec * 1000)) {
            $r['timedOut'] = $true
            Stop-ProcessTree -ProcessId $p.Id
            [void]$p.WaitForExit(10000)
        }
        try { $r['rc'] = $p.ExitCode } catch { $r['rc'] = $null }
        try {
            if ($tOut.Wait(5000)) { $r['out'] = [string]$tOut.Result } else { $r['pipes_not_drained'] = $true }
        } catch { $r['out'] = '[stdout read failed: ' + $_.Exception.Message + ']' }
        try {
            if ($tErr.Wait(5000)) { $r['err'] = [string]$tErr.Result } else { $r['pipes_not_drained'] = $true }
        } catch { $r['err'] = '[stderr read failed: ' + $_.Exception.Message + ']' }
    } catch {
        $r['startError'] = $_.Exception.Message
    } finally {
        if ($null -ne $p) { try { $p.Dispose() } catch { } }
    }
    $sw.Stop()
    $r['ms'] = [int]$sw.ElapsedMilliseconds
    foreach ($k in @('out', 'err')) {
        $t = [string]$r[$k]
        if ($t.Length -gt 4000000) { $r[$k] = $t.Substring(0, 4000000) + '...[truncated]' }
    }
    return $r
}

# Quote one argument for a Windows command line (CommandLineToArgvW rules).
function ConvertTo-CmdArg {
    param([string]$Arg)
    if ($null -eq $Arg) { return '""' }
    if ($Arg.Length -gt 0) {
        if ($Arg.IndexOfAny([char[]]@([char]32, [char]9, [char]34, [char]10, [char]11)) -lt 0) { return $Arg }
    }
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append('"')
    $bs = 0
    foreach ($ch in $Arg.ToCharArray()) {
        if ($ch -eq [char]92) {
            $bs++
        } elseif ($ch -eq [char]34) {
            [void]$sb.Append(('\' * (($bs * 2) + 1)))
            [void]$sb.Append('"')
            $bs = 0
        } else {
            if ($bs -gt 0) { [void]$sb.Append(('\' * $bs)); $bs = 0 }
            [void]$sb.Append($ch)
        }
    }
    if ($bs -gt 0) { [void]$sb.Append(('\' * ($bs * 2))) }
    [void]$sb.Append('"')
    return $sb.ToString()
}

function Get-ProcessesUnder {
    param([string]$Dir)
    $res = New-Object System.Collections.Generic.List[object]
    try {
        $prefix = $Dir.TrimEnd('\') + '\'
        $all = Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop
        foreach ($p in $all) {
            $ep = [string]$p.ExecutablePath
            if ($ep -and $ep.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
                $res.Add([pscustomobject]@{ Name = [string]$p.Name; Id = [int]$p.ProcessId; ParentId = [int]$p.ParentProcessId; Path = $ep; Cmd = [string]$p.CommandLine })
            }
        }
    } catch {
        Write-Log ('Get-ProcessesUnder failed: ' + $_.Exception.Message) 'WARN'
    }
    return $res.ToArray()
}

function Stop-ProcessesUnder {
    param([string]$Dir)
    $killed = New-Object System.Collections.Generic.List[string]
    foreach ($p in @(Get-ProcessesUnder -Dir $Dir)) {
        try {
            Stop-Process -Id $p.Id -Force -ErrorAction Stop
            $killed.Add(('{0}#{1}' -f $p.Name, $p.Id))
        } catch {
            $killed.Add(('{0}#{1} FAILED {2}' -f $p.Name, $p.Id, $_.Exception.Message))
        }
    }
    return $killed.ToArray()
}

function Get-VisibleWindowsText {
    try {
        $items = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 -and $_.MainWindowTitle } | ForEach-Object { '{0}#{1}:{2}' -f $_.ProcessName, $_.Id, $_.MainWindowTitle })
        return ($items -join ' | ')
    } catch { return ('error: ' + $_.Exception.Message) }
}

function Get-Sha256 {
    param([string]$Path)
    try {
        return (Get-FileHash -LiteralPath $Path -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
    } catch { return $null }
}

function Get-SigRecord {
    param([string]$Path)
    $r = [ordered]@{ status = 'unknown'; message = $null; subject = $null; issuer = $null; thumbprint = $null; timestamped = $null }
    try {
        $s = Get-AuthenticodeSignature -LiteralPath $Path -ErrorAction Stop
        $r['status'] = [string]$s.Status
        $r['message'] = [string]$s.StatusMessage
        if ($s.SignerCertificate) {
            $r['subject'] = [string]$s.SignerCertificate.Subject
            $r['issuer'] = [string]$s.SignerCertificate.Issuer
            $r['thumbprint'] = [string]$s.SignerCertificate.Thumbprint
        }
        if ($s.TimeStamperCertificate) { $r['timestamped'] = $true } else { $r['timestamped'] = $false }
    } catch {
        $r['status'] = 'error'
        $r['message'] = $_.Exception.Message
    }
    return $r
}

function Invoke-Download {
    param([string]$Url, [string]$Dest, [int]$TimeoutSec = 900)
    $r = [ordered]@{ url = $Url; dest = $Dest; ok = $false; rc = $null; err = $null; ms = 0; size = 0 }
    try {
        $dir = Split-Path -Parent $Dest
        if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
        $curl = Join-Path $env:windir 'System32\curl.exe'
        if (-not (Test-Path -LiteralPath $curl)) { $curl = Find-Exe 'curl.exe' }
        if (-not $curl) { $r['err'] = 'curl.exe not found'; return $r }
        $a = ('-L --fail --retry 3 --retry-delay 3 --connect-timeout 30 --max-time {0} -sS -o "{1}" "{2}"' -f $TimeoutSec, $Dest, $Url)
        $x = Invoke-Proc -File $curl -Arguments $a -TimeoutSec ($TimeoutSec + 60)
        $r['rc'] = $x.rc
        $r['err'] = $x.err
        $r['ms'] = $x.ms
        if (($x.rc -eq 0) -and (Test-Path -LiteralPath $Dest)) {
            $r['ok'] = $true
            $r['size'] = (Get-Item -LiteralPath $Dest).Length
        }
    } catch {
        $r['err'] = $_.Exception.Message
    }
    return $r
}

# ---------------------------------------------------------------------------
# screenshots (desktop, via GDI; failure allowed)
# ---------------------------------------------------------------------------

function Initialize-Screenshot {
    try {
        Add-Type -AssemblyName System.Windows.Forms -ErrorAction Stop
        Add-Type -AssemblyName System.Drawing -ErrorAction Stop
        $global:DiagScreenReady = $true
    } catch {
        Write-Log ('Initialize-Screenshot failed: ' + $_.Exception.Message) 'WARN'
    }
}

function Save-Screenshot {
    param([string]$Name)
    $r = [ordered]@{ name = $Name; ok = $false; error = $null; width = 0; height = 0; windows = $null }
    try {
        $r['windows'] = Get-VisibleWindowsText
        if (-not $global:DiagScreenReady) { Initialize-Screenshot }
        $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
        $bmp = New-Object System.Drawing.Bitmap($b.Width, $b.Height)
        try {
            $g = [System.Drawing.Graphics]::FromImage($bmp)
            try { $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size) } finally { $g.Dispose() }
            $bmp.Save((Join-Path $global:DiagOut $Name), [System.Drawing.Imaging.ImageFormat]::Png)
            $r['ok'] = $true
            $r['width'] = $b.Width
            $r['height'] = $b.Height
        } finally { $bmp.Dispose() }
    } catch {
        $r['error'] = $_.Exception.Message
    }
    return $r
}

# ---------------------------------------------------------------------------
# native probes (P/Invoke via Add-Type, C# 5 only). MUST be compiled BEFORE any App Control policy is deployed.
# ---------------------------------------------------------------------------

$global:DiagNativeSource = @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public class DiagCall
{
    public string Api;
    public bool Ok;
    public long Ret;
    public int LastError;
    public string LastErrorText;
    public bool FirstWaitTimedOut;
    public bool TimedOut;
    public bool Completed;
    public bool HadProcessHandle;
    public string Note;
}

public static class DiagNative
{
    private const uint CREATE_SUSPENDED = 0x00000004;
    private const uint SEE_MASK_NOCLOSEPROCESS = 0x00000040;
    private const uint SEE_MASK_NOASYNC = 0x00000100;
    private const uint SEE_MASK_FLAG_NO_UI = 0x00000400;
    private const uint WM_CLOSE = 0x0010;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct STARTUPINFO
    {
        public int cb;
        public string lpReserved;
        public string lpDesktop;
        public string lpTitle;
        public int dwX;
        public int dwY;
        public int dwXSize;
        public int dwYSize;
        public int dwXCountChars;
        public int dwYCountChars;
        public int dwFillAttribute;
        public int dwFlags;
        public short wShowWindow;
        public short cbReserved2;
        public IntPtr lpReserved2;
        public IntPtr hStdInput;
        public IntPtr hStdOutput;
        public IntPtr hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct PROCESS_INFORMATION
    {
        public IntPtr hProcess;
        public IntPtr hThread;
        public int dwProcessId;
        public int dwThreadId;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct SHELLEXECUTEINFO
    {
        public int cbSize;
        public uint fMask;
        public IntPtr hwnd;
        public string lpVerb;
        public string lpFile;
        public string lpParameters;
        public string lpDirectory;
        public int nShow;
        public IntPtr hInstApp;
        public IntPtr lpIDList;
        public string lpClass;
        public IntPtr hkeyClass;
        public uint dwHotKey;
        public IntPtr hIconOrMonitor;
        public IntPtr hProcess;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct SYSTEM_CODEINTEGRITY_INFORMATION
    {
        public int Length;
        public uint CodeIntegrityOptions;
    }

    private delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CreateProcessW(string lpApplicationName, StringBuilder lpCommandLine,
        IntPtr lpProcessAttributes, IntPtr lpThreadAttributes, bool bInheritHandles, uint dwCreationFlags,
        IntPtr lpEnvironment, string lpCurrentDirectory, ref STARTUPINFO lpStartupInfo,
        out PROCESS_INFORMATION lpProcessInformation);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool TerminateProcess(IntPtr hProcess, uint uExitCode);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr hObject);

    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr ShellExecuteW(IntPtr hwnd, string lpOperation, string lpFile,
        string lpParameters, string lpDirectory, int nShowCmd);

    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool ShellExecuteExW(ref SHELLEXECUTEINFO lpExecInfo);

    [DllImport("ntdll.dll")]
    private static extern int NtQuerySystemInformation(int SystemInformationClass,
        ref SYSTEM_CODEINTEGRITY_INFORMATION SystemInformation, int SystemInformationLength, out int ReturnLength);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);

    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll")]
    private static extern bool PostMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);

    private static Thread seThread;
    private static DiagCall seResult;

    // CreateProcessW(CREATE_SUSPENDED) then TerminateProcess: only the image check happens, no code runs.
    public static DiagCall ProbeCreate(string path)
    {
        DiagCall r = new DiagCall();
        r.Api = "CreateProcessW(CREATE_SUSPENDED)";
        STARTUPINFO si = new STARTUPINFO();
        si.cb = Marshal.SizeOf(typeof(STARTUPINFO));
        PROCESS_INFORMATION pi;
        StringBuilder cmd = new StringBuilder(path.Length + 64);
        cmd.Append('"').Append(path).Append('"');
        bool ok = CreateProcessW(path, cmd, IntPtr.Zero, IntPtr.Zero, false, CREATE_SUSPENDED,
            IntPtr.Zero, null, ref si, out pi);
        int err = Marshal.GetLastWin32Error();
        r.Completed = true;
        if (ok)
        {
            r.Ok = true;
            r.HadProcessHandle = true;
            r.LastError = 0;
            TerminateProcess(pi.hProcess, 1);
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
        }
        else
        {
            r.Ok = false;
            r.LastError = err;
            r.LastErrorText = new Win32Exception(err).Message;
        }
        return r;
    }

    // ShellExecuteW exactly like the updater plugin (open verb, SW_SHOW, no COM init on the calling thread).
    // Runs on a worker thread because a block UI may be shown by the shell. Returns after waitMs even if
    // the call is still pending (FirstWaitTimedOut/TimedOut); call ShellExecFinish to dismiss dialogs and wait.
    public static DiagCall ShellExecStart(string file, string args, int waitMs)
    {
        DiagCall r = new DiagCall();
        r.Api = "ShellExecuteW(open,SW_SHOW)";
        seResult = r;
        Thread t = new Thread(new ThreadStart(delegate
        {
            IntPtr h = ShellExecuteW(IntPtr.Zero, "open", file, args, null, 5);
            int err = Marshal.GetLastWin32Error();
            r.Ret = h.ToInt64();
            r.LastError = err;
            r.LastErrorText = new Win32Exception(err).Message;
            r.Ok = (r.Ret > 32);
            r.Completed = true;
        }));
        t.IsBackground = true;
        seThread = t;
        t.Start();
        if (!t.Join(waitMs))
        {
            r.FirstWaitTimedOut = true;
            r.TimedOut = true;
            r.Note = "no return within " + waitMs.ToString() + " ms";
        }
        return r;
    }

    public static DiagCall ShellExecFinish(int waitMs, bool closeDialogs)
    {
        DiagCall r = seResult;
        if (r == null)
        {
            r = new DiagCall();
            r.Api = "ShellExecuteW(finish)";
            r.Note = "no call in flight";
            return r;
        }
        Thread t = seThread;
        if (t != null && !r.Completed)
        {
            if (closeDialogs)
            {
                string w = ListWindows(true);
                r.Note = (r.Note == null ? "" : r.Note + " | ") + "windows: " + w;
            }
            t.Join(waitMs);
        }
        r.TimedOut = !r.Completed;
        return r;
    }

    // ShellExecuteExW with SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC.
    // If a process handle comes back the process is terminated immediately.
    public static DiagCall ShellExecEx(string file, string args, int waitMs)
    {
        DiagCall r = new DiagCall();
        r.Api = "ShellExecuteExW(NOCLOSEPROCESS|NO_UI|NOASYNC)";
        Thread t = new Thread(new ThreadStart(delegate
        {
            SHELLEXECUTEINFO sei = new SHELLEXECUTEINFO();
            sei.cbSize = Marshal.SizeOf(typeof(SHELLEXECUTEINFO));
            sei.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC;
            sei.hwnd = IntPtr.Zero;
            sei.lpVerb = "open";
            sei.lpFile = file;
            sei.lpParameters = args;
            sei.lpDirectory = null;
            sei.nShow = 5;
            bool ok = ShellExecuteExW(ref sei);
            int err = Marshal.GetLastWin32Error();
            r.Ok = ok;
            r.LastError = err;
            r.LastErrorText = new Win32Exception(err).Message;
            r.Ret = sei.hInstApp.ToInt64();
            if (sei.hProcess != IntPtr.Zero)
            {
                r.HadProcessHandle = true;
                TerminateProcess(sei.hProcess, 1);
                CloseHandle(sei.hProcess);
            }
            r.Completed = true;
        }));
        t.IsBackground = true;
        t.Start();
        if (!t.Join(waitMs))
        {
            r.FirstWaitTimedOut = true;
            r.TimedOut = true;
            r.Note = "no return within " + waitMs.ToString() + " ms";
        }
        return r;
    }

    // Visible top-level windows as "pid|class|title; ..." ; optionally WM_CLOSE own #32770 dialogs.
    public static string ListWindows(bool closeOwnDialogs)
    {
        StringBuilder sb = new StringBuilder();
        uint me = (uint)System.Diagnostics.Process.GetCurrentProcess().Id;
        int count = 0;
        EnumWindowsProc cb = delegate(IntPtr h, IntPtr l)
        {
            if (!IsWindowVisible(h)) { return true; }
            uint wpid;
            GetWindowThreadProcessId(h, out wpid);
            StringBuilder t = new StringBuilder(256);
            GetWindowText(h, t, 256);
            StringBuilder c = new StringBuilder(128);
            GetClassName(h, c, 128);
            string title = t.ToString();
            string cls = c.ToString();
            if (title.Length == 0 && wpid != me) { return true; }
            if (count < 60)
            {
                sb.Append(wpid.ToString());
                sb.Append('|');
                sb.Append(cls);
                sb.Append('|');
                sb.Append(title.Replace('|', '/'));
                sb.Append("; ");
            }
            count++;
            if (closeOwnDialogs && wpid == me && cls == "#32770")
            {
                PostMessage(h, WM_CLOSE, IntPtr.Zero, IntPtr.Zero);
                sb.Append("[WM_CLOSE sent]; ");
            }
            return true;
        };
        EnumWindows(cb, IntPtr.Zero);
        return sb.ToString();
    }

    // NtQuerySystemInformation(SystemCodeIntegrityInformation = 103): independent view of CI/UMCI state.
    public static string CodeIntegrityOptionsInfo()
    {
        try
        {
            SYSTEM_CODEINTEGRITY_INFORMATION info = new SYSTEM_CODEINTEGRITY_INFORMATION();
            info.Length = Marshal.SizeOf(typeof(SYSTEM_CODEINTEGRITY_INFORMATION));
            int retLen;
            int status = NtQuerySystemInformation(103, ref info, info.Length, out retLen);
            return "status=0x" + status.ToString("X8") + ";options=0x" + info.CodeIntegrityOptions.ToString("X8") + ";retLen=" + retLen.ToString();
        }
        catch (Exception ex)
        {
            return "error:" + ex.GetType().Name + ":" + ex.Message;
        }
    }
}
'@

function Initialize-DiagNative {
    if ($global:DiagNativeReady) { return }
    try {
        if (-not ('DiagNative' -as [type])) {
            Add-Type -TypeDefinition $global:DiagNativeSource -Language CSharp -IgnoreWarnings -ErrorAction Stop
        }
        $global:DiagNativeReady = $true
        $global:DiagNativeError = $null
    } catch {
        $global:DiagNativeError = $_.Exception.Message
        Write-Log ('Initialize-DiagNative failed: ' + $_.Exception.Message) 'ERROR'
    }
}

function Get-ShellExecuteRetMeaning {
    param([int64]$Ret)
    if ($Ret -gt 32) { return 'success (HINSTANCE > 32)' }
    switch ($Ret) {
        0 { return 'out of memory or resources' }
        2 { return 'ERROR_FILE_NOT_FOUND' }
        3 { return 'ERROR_PATH_NOT_FOUND' }
        5 { return 'SE_ERR_ACCESSDENIED' }
        8 { return 'ERROR_NOT_ENOUGH_MEMORY' }
        11 { return 'ERROR_BAD_FORMAT' }
        26 { return 'SE_ERR_SHARE' }
        27 { return 'SE_ERR_ASSOCINCOMPLETE' }
        28 { return 'SE_ERR_DDETIMEOUT' }
        29 { return 'SE_ERR_DDEFAIL' }
        30 { return 'SE_ERR_DDEBUSY' }
        31 { return 'SE_ERR_NOASSOC' }
        32 { return 'ERROR_DLL_NOT_FOUND' }
        default { return 'failure (value <= 32, unlisted)' }
    }
}

function ConvertTo-CallRecord {
    param($Call)
    if ($null -eq $Call) { return $null }
    $meaning = $null
    if ([string]$Call.Api -like 'ShellExecute*') { $meaning = Get-ShellExecuteRetMeaning ([int64]$Call.Ret) }
    return [ordered]@{
        api = [string]$Call.Api
        ok = [bool]$Call.Ok
        ret = [int64]$Call.Ret
        ret_meaning = $meaning
        last_error = [int]$Call.LastError
        last_error_hex = ('0x{0:X8}' -f [int]$Call.LastError)
        last_error_text = [string]$Call.LastErrorText
        first_wait_timed_out = [bool]$Call.FirstWaitTimedOut
        timed_out = [bool]$Call.TimedOut
        completed = [bool]$Call.Completed
        had_process_handle = [bool]$Call.HadProcessHandle
        note = [string]$Call.Note
    }
}

function Invoke-ProbeCreate {
    param([string]$Path)
    if (-not $global:DiagNativeReady) {
        return [ordered]@{ api = 'CreateProcessW(CREATE_SUSPENDED)'; ok = $null; skipped = ('native helper not available: ' + [string]$global:DiagNativeError) }
    }
    try {
        return (ConvertTo-CallRecord ([DiagNative]::ProbeCreate($Path)))
    } catch {
        return [ordered]@{ api = 'CreateProcessW(CREATE_SUSPENDED)'; ok = $null; exception = $_.Exception.Message }
    }
}

function Get-CodeIntegrityState {
    $r = [ordered]@{ raw = $null; options_hex = $null; flags = @() }
    try {
        if (-not $global:DiagNativeReady) { $r['raw'] = 'native helper not ready'; return $r }
        $s = [DiagNative]::CodeIntegrityOptionsInfo()
        $r['raw'] = $s
        $m = [regex]::Match($s, 'options=0x([0-9A-Fa-f]+)')
        if ($m.Success) {
            $v = [Convert]::ToUInt32($m.Groups[1].Value, 16)
            $r['options_hex'] = ('0x{0:X8}' -f $v)
            # flag names written from memory of the CODEINTEGRITY_OPTION_* constants ([guess] for the names; the raw hex is the fact)
            $names = [ordered]@{ ENABLED = 1; TESTSIGN = 2; UMCI_ENABLED = 4; UMCI_AUDITMODE_ENABLED = 8; UMCI_EXCLUSIONPATHS_ENABLED = 16; TEST_BUILD = 32; PREPRODUCTION_BUILD = 64; DEBUGMODE_ENABLED = 128; FLIGHT_BUILD = 256; FLIGHTING_ENABLED = 512; HVCI_KMCI_ENABLED = 1024; HVCI_KMCI_AUDITMODE_ENABLED = 2048; HVCI_KMCI_STRICTMODE_ENABLED = 4096; HVCI_IUM_ENABLED = 8192 }
            $fl = New-Object System.Collections.Generic.List[string]
            foreach ($k in $names.Keys) {
                if (($v -band [uint32]$names[$k]) -ne 0) { $fl.Add([string]$k) }
            }
            $r['flags'] = $fl.ToArray()
        }
    } catch {
        $r['raw'] = ('error: ' + $_.Exception.Message)
    }
    return $r
}

# ---------------------------------------------------------------------------
# Code Integrity event helpers
# ---------------------------------------------------------------------------

# Returns @{ events = [...]; note = '...' } with the events since SinceUtc, oldest first.
# Each event: id, time (UTC ISO), level, fields (Data Name -> value), message, xml (raw).
function Get-CIEvents {
    param([datetime]$SinceUtc, [int]$Max = 20000)
    $list = New-Object System.Collections.Generic.List[object]
    $note = $null
    try {
        $start = $SinceUtc.ToUniversalTime().ToLocalTime()
        $evs = @(Get-WinEvent -FilterHashtable @{ LogName = 'Microsoft-Windows-CodeIntegrity/Operational'; StartTime = $start } -MaxEvents $Max -ErrorAction Stop)
        [array]::Reverse($evs)
        foreach ($e in $evs) {
            $xmlText = ''
            try { $xmlText = [string]$e.ToXml() } catch { }
            $fields = [ordered]@{}
            try {
                $x = New-Object System.Xml.XmlDocument
                $x.LoadXml($xmlText)
                $nsm = New-Object System.Xml.XmlNamespaceManager($x.NameTable)
                $nsm.AddNamespace('e', 'http://schemas.microsoft.com/win/2004/08/events/event')
                $i = 0
                foreach ($d in $x.SelectNodes('//e:EventData/e:Data', $nsm)) {
                    $n = $d.GetAttribute('Name')
                    if ([string]::IsNullOrEmpty($n)) { $n = 'Data' + $i }
                    $fields[$n] = [string]$d.InnerText
                    $i++
                }
            } catch { }
            $msg = ''
            try { $msg = [string]$e.Message } catch { }
            if ($msg.Length -gt 800) { $msg = $msg.Substring(0, 800) + '...' }
            $list.Add([pscustomobject]@{
                id = [int]$e.Id
                time = (ConvertTo-IsoUtc $e.TimeCreated)
                level = [int]$e.Level
                fields = $fields
                message = $msg
                xml = $xmlText
            })
        }
    } catch {
        if ($_.Exception.Message -match 'No events were found') { $note = 'no events' } else { $note = $_.Exception.Message }
    }
    return [pscustomobject]@{ events = $list.ToArray(); note = $note }
}

# wevtutil epl for the CodeIntegrity/Operational log since a UTC time. Returns the Invoke-Proc record.
function Export-CIEvtx {
    param([datetime]$SinceUtc, [string]$FileName)
    $path = Join-Path $global:DiagOut $FileName
    try { if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force } } catch { }
    $iso = $SinceUtc.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.000Z')
    $q = "*[System[TimeCreated[@SystemTime>='" + $iso + "']]]"
    $wa = ('epl "Microsoft-Windows-CodeIntegrity/Operational" "{0}" "/q:{1}" /ow:true' -f $path, $q)
    $we = Join-Path $env:windir 'System32\wevtutil.exe'
    return (Invoke-Proc -File $we -Arguments $wa -TimeoutSec 120)
}

# ---------------------------------------------------------------------------
# assets produced by p1-assets.ps1
# ---------------------------------------------------------------------------

function Get-AssetInstaller {
    param([string]$Version)
    return (Join-Path $global:DiagWork ('dl\{0}\cys_{0}_x64-setup.exe' -f $Version))
}

function Test-AssetUsable {
    param([string]$Version)
    try {
        $f = Join-Path $global:DiagOut 'p1-assets.json'
        $txt = Read-TextUtf8 $f
        if (-not $txt) { return (Test-Path -LiteralPath (Get-AssetInstaller $Version)) }
        $j = ConvertFrom-Json $txt
        $e = $j.versions.PSObject.Properties[$Version]
        if ($null -eq $e) { return $false }
        return [bool]$e.Value.usable
    } catch { return (Test-Path -LiteralPath (Get-AssetInstaller $Version)) }
}

# ---------------------------------------------------------------------------
# replica binaries (QF + fallback path of the E2E). Builds diag/replica/*.rs with rustc.
# ---------------------------------------------------------------------------

function Build-Replica {
    $cacheFile = Join-Path $global:DiagWork 'replica-build.json'
    try {
        $c = Read-TextUtf8 $cacheFile
        if ($c) { return (ConvertFrom-Json $c) }
    } catch { }
    $b = [ordered]@{ started = (Get-IsoNow); rustc = $null; rustc_version = $null; host_triple = $null; variants = [ordered]@{}; log = (New-Object System.Collections.Generic.List[object]) }
    try {
        $srcDir = Join-Path $global:DiagRoot 'replica'
        $binRoot = Join-Path $global:DiagWork 'replica-bin'
        New-Item -ItemType Directory -Path $binRoot -Force | Out-Null
        $rustc = Find-Exe 'rustc.exe'
        if (-not $rustc) {
            $b['log'].Add('rustc.exe not found on PATH')
        } else {
            $b['rustc'] = $rustc
            $v = Invoke-Proc -File $rustc -Arguments '-vV' -TimeoutSec 60
            $b['rustc_version'] = $v.out
            $hostTriple = ''
            $m = [regex]::Match([string]$v.out, '(?m)^host:\s*(\S+)')
            if ($m.Success) { $hostTriple = $m.Groups[1].Value }
            $b['host_triple'] = $hostTriple
            $variants = New-Object System.Collections.Generic.List[object]
            $variants.Add(@{ name = 'host'; target = '' })
            if ($hostTriple -ne 'x86_64-pc-windows-msvc') {
                $rustup = Find-Exe 'rustup.exe'
                if ($rustup) {
                    $ru = Invoke-Proc -File $rustup -Arguments 'target add x86_64-pc-windows-msvc' -TimeoutSec 300
                    $b['log'].Add([ordered]@{ step = 'rustup target add x86_64-pc-windows-msvc'; rc = $ru.rc; out = $ru.out; err = $ru.err; startError = $ru.startError })
                    $variants.Add(@{ name = 'x64'; target = 'x86_64-pc-windows-msvc' })
                } else {
                    $b['log'].Add('rustup.exe not found; x64 variant skipped')
                }
            }
            foreach ($var in $variants) {
                $dir = Join-Path $binRoot $var.name
                New-Item -ItemType Directory -Path $dir -Force | Out-Null
                $entry = [ordered]@{ target = $var.target; ok = $false; se = $null; marker = $null; steps = (New-Object System.Collections.Generic.List[object]) }
                $allOk = $true
                foreach ($pair in @(@('se_exit', 'se_exit.rs'), @('marker', 'marker.rs'))) {
                    $outExe = Join-Path $dir ($pair[0] + '.exe')
                    $srcFile = Join-Path $srcDir $pair[1]
                    $ra = '-O --edition 2021 -C target-feature=+crt-static'
                    if ($var.target) { $ra = $ra + ' --target ' + $var.target }
                    $ra = $ra + (' -o "{0}" "{1}"' -f $outExe, $srcFile)
                    $rr = Invoke-Proc -File $rustc -Arguments $ra -TimeoutSec 300
                    $errTail = [string]$rr.err
                    if ($errTail.Length -gt 3000) { $errTail = $errTail.Substring($errTail.Length - 3000) }
                    $built = Test-Path -LiteralPath $outExe
                    $entry['steps'].Add([ordered]@{ source = $pair[1]; exe = $outExe; rc = $rr.rc; built = $built; startError = $rr.startError; err_tail = $errTail })
                    if (-not $built) { $allOk = $false }
                }
                if ($allOk) {
                    $entry['ok'] = $true
                    $entry['se'] = (Join-Path $dir 'se_exit.exe')
                    $entry['marker'] = (Join-Path $dir 'marker.exe')
                }
                $b['variants'][$var.name] = $entry
            }
        }
    } catch {
        $b['log'].Add(('Build-Replica exception: ' + $_.Exception.Message))
    }
    $json = ConvertTo-Json -InputObject $b -Depth 8
    try { Write-Utf8NoBom $cacheFile $json } catch { }
    try { Write-Utf8NoBom (Join-Path $global:DiagOut 'replica-build.json') $json } catch { }
    return (ConvertFrom-Json $json)
}

# Run a script block in a background job (separate PowerShell process) and give up after TimeoutSec.
# For cmdlets that can stall without a timeout parameter (Defender CDXML cmdlets). Returns the job output
# (deserialized objects: properties are readable), throws on timeout or job error.
function Invoke-JobWithTimeout {
    param([scriptblock]$Block, [int]$TimeoutSec = 60)
    $job = $null
    try {
        $job = Start-Job -ScriptBlock $Block
        if (Wait-Job -Job $job -Timeout $TimeoutSec) {
            return (Receive-Job -Job $job -ErrorAction Stop)
        }
        try { Stop-Job -Job $job -ErrorAction SilentlyContinue } catch { }
        throw ('timed out after {0} s' -f $TimeoutSec)
    } finally {
        if ($null -ne $job) { try { Remove-Job -Job $job -Force -ErrorAction SilentlyContinue } catch { } }
    }
}

# ---------------------------------------------------------------------------
# small formatting helpers
# ---------------------------------------------------------------------------

# Human readable rendering of an Invoke-Proc record (keeps raw stdout/stderr verbatim).
function Format-ProcText {
    param($R)
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.AppendLine(('cmd: {0} {1}' -f $R.file, $R.args))
    [void]$sb.AppendLine(('rc={0} timedOut={1} ms={2} startError={3} pipes_not_drained={4}' -f $R.rc, $R.timedOut, $R.ms, $R.startError, $R.pipes_not_drained))
    if ($R.out) { [void]$sb.AppendLine('--- stdout ---'); [void]$sb.AppendLine([string]$R.out) }
    if ($R.err) { [void]$sb.AppendLine('--- stderr ---'); [void]$sb.AppendLine([string]$R.err) }
    return $sb.ToString()
}

# Copy named properties of an object into an ordered hashtable with JSON friendly values.
function Select-Props {
    param($Obj, [string[]]$Names)
    $o = [ordered]@{}
    foreach ($n in $Names) {
        $v = $null
        try { $v = $Obj.$n } catch { }
        if ($null -eq $v) {
            $o[$n] = $null
        } elseif ($v -is [datetime]) {
            $o[$n] = ConvertTo-IsoUtc $v
        } elseif (($v -is [string]) -or ($v -is [bool]) -or ($v -is [int]) -or ($v -is [long]) -or ($v -is [double]) -or ($v -is [uint32]) -or ($v -is [uint16])) {
            $o[$n] = $v
        } else {
            $o[$n] = [string]$v
        }
    }
    return $o
}
