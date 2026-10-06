# diag/w44-measure.ps1 -- 0.14.44 Windows measurements (design section 5-4, items 1-9) on a REAL Windows runner.
# Windows PowerShell 5.1, ASCII only. Dot-sources diag/lib.ps1 (Invoke-Proc, Stop-ProcessTree, Save-Json ...).
# Everything here happens on a throw-away GitHub runner: the product installer is installed, the public 0.14.43 installer is only UNPACKED
# (7-Zip) so that "before 0.14.44" and "after 0.14.44" run side by side. No Smart App Control / Defender change. Observation only.
#
# Steps (each is wrapped: a failing step is recorded and the next one runs):
#   S0 inputs      which build run, download of the installer under test (artifact of the build job of THIS run, or of w44-input.json's run)
#   S1 install     silent install of the installer under test, facts (versions), unpack of the public 0.14.43 installer, packs
#   TA approval    items 1 (shell folder strings, backslash sign/check, launch-agent --cwd forms), A3 daemon binding: NEW vs OLD daemon
#   TB bridge      item 2 (bridge script default / HUD_WIN_NEW=1: answers, request timeout, /health, SECOND BIND on a taken port)
#   TC daemon      items 3-6 (default bridge mode line, managed mode lifeline with and without seats, forced stop, policy-file value is ignored,
#                  pack-heal --missing-only while daemon and bridge run, ledger unchanged, old bridge survives an old daemon's forced stop)
#   TD hook        item 7 (the capability gate hook under the bundled Git Bash: unpaired-surrogate command -> JSON deny, rc 0)
#   SUMMARY        w44-summary.txt (one line per item), w44-results.json

$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'
. (Join-Path $PSScriptRoot 'lib.ps1')
Start-DiagScript 'w44-measure'

$OSL = [string]$env:DIAG_MATRIX_OS
$WORK = Join-Path $env:RUNNER_TEMP 'w44'
New-Item -ItemType Directory -Path $WORK -Force | Out-Null
$R = [ordered]@{ os = $OSL; started = (Get-IsoNow); steps = [ordered]@{}; items = [ordered]@{} }
$X = @{}   # shared context: paths of the two binary sets etc.

function Step {
    param([string]$Name, [scriptblock]$Block)
    $t0 = Get-Date
    Write-Log ('--- step {0} start' -f $Name)
    $err = $null
    try { & $Block } catch { $err = Format-ErrorText $_; Add-DiagError $Name $_ }
    $R['steps'][$Name] = [ordered]@{ seconds = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1); error = $err }
    Write-Log ('--- step {0} end ({1}s) {2}' -f $Name, $R['steps'][$Name]['seconds'], $err)
    Save-Json 'w44-results.json' $R 10
}

function Item {
    param([string]$Key, $Value)
    $R['items'][$Key] = $Value
    Save-Json 'w44-results.json' $R 10
}

function Limit {
    param([string]$Text, [int]$Max = 600)
    if ($null -eq $Text) { return '' }
    $t = $Text.Trim()
    if ($t.Length -gt $Max) { return $t.Substring(0, $Max) + '...' }
    return $t
}

# run with temporary environment variables (restored afterwards)
function Use-Env {
    param([hashtable]$Env, [scriptblock]$Block)
    $old = @{}
    foreach ($k in $Env.Keys) { $old[$k] = [Environment]::GetEnvironmentVariable($k, 'Process'); [Environment]::SetEnvironmentVariable($k, [string]$Env[$k], 'Process') }
    try { & $Block } finally {
        foreach ($k in $Env.Keys) { [Environment]::SetEnvironmentVariable($k, $old[$k], 'Process') }
    }
}

function Get-ProcRows {
    param([string]$NameLike = '')
    $rows = @()
    try {
        $all = Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop
        foreach ($p in $all) {
            if ($NameLike -ne '' -and ([string]$p.Name) -notlike $NameLike) { continue }
            $rows += [pscustomobject]@{ Name = [string]$p.Name; Id = [int]$p.ProcessId; Ppid = [int]$p.ParentProcessId; Path = [string]$p.ExecutablePath; Cmd = [string]$p.CommandLine; Created = [string]$p.CreationDate }
        }
    } catch { Write-Log ('Get-ProcRows failed: ' + $_.Exception.Message) 'WARN' }
    return $rows
}

function Test-PidAlive {
    param([int]$ProcId)
    try { $p = Get-Process -Id $ProcId -ErrorAction Stop; return (-not $p.HasExited) } catch { return $false }
}

# descendants of a process (by parent id), from one process snapshot
function Get-Descendants {
    param([int]$RootId, $Rows)
    $found = New-Object System.Collections.Generic.List[object]
    $queue = New-Object System.Collections.Generic.Queue[int]
    $queue.Enqueue($RootId)
    $seen = @{}
    while ($queue.Count -gt 0) {
        $cur = $queue.Dequeue()
        foreach ($r in $Rows) {
            if ($r.Ppid -eq $cur -and -not $seen.ContainsKey($r.Id)) { $seen[$r.Id] = 1; $found.Add($r); $queue.Enqueue($r.Id) }
        }
    }
    return $found.ToArray()
}

function Stop-AllW44Procs {
    # only things this script started: daemons / bridges / seats of the two binary sets and the bundled python
    $killed = @()
    foreach ($p in (Get-ProcRows)) {
        $isMine = $false
        if ($p.Path -and ($p.Path.StartsWith($WORK, [System.StringComparison]::OrdinalIgnoreCase))) { $isMine = $true }
        if ($p.Path -and $X['inst'] -and ($p.Path.StartsWith(([string]$X['inst']).TrimEnd('\') + '\', [System.StringComparison]::OrdinalIgnoreCase))) { $isMine = $true }
        if ($p.Cmd -and ($p.Cmd -like '*javis_hud_bridge.py*')) { $isMine = $true }
        if ($isMine) {
            try { Stop-Process -Id $p.Id -Force -ErrorAction Stop; $killed += ('{0}#{1}' -f $p.Name, $p.Id) } catch { }
        }
    }
    Start-Sleep -Milliseconds 800
    return $killed
}

function Invoke-Cys {
    param([hashtable]$Set, [string]$Pipe, [string]$ArgText, [int]$TimeoutSec = 60, [string]$WorkDir = '', [hashtable]$ExtraEnv = @{})
    $envm = @{ CYS_SOCKET = $Pipe }
    foreach ($k in $ExtraEnv.Keys) { $envm[$k] = $ExtraEnv[$k] }
    $res = $null
    Use-Env $envm { $script:res = Invoke-Proc -File $Set['cys'] -Arguments $ArgText -TimeoutSec $TimeoutSec -WorkDir $WorkDir }
    return $script:res
}

function Start-W44Daemon {
    param([hashtable]$Set, [string]$Pipe, [string]$Tag, [hashtable]$ExtraEnv = @{}, [int]$ReadySec = 90)
    $envm = @{ CYS_SOCKET = $Pipe }
    foreach ($k in $ExtraEnv.Keys) { $envm[$k] = $ExtraEnv[$k] }
    $errF = Join-Path $WORK ('cysd-' + $Tag + '.err.txt')
    $outF = Join-Path $WORK ('cysd-' + $Tag + '.out.txt')
    foreach ($f in @($errF, $outF)) { if (Test-Path -LiteralPath $f) { Remove-Item -LiteralPath $f -Force -ErrorAction SilentlyContinue } }
    $proc = $null
    Use-Env $envm {
        $script:proc = Start-Process -FilePath $Set['cysd'] -PassThru -WindowStyle Hidden -RedirectStandardError $errF -RedirectStandardOutput $outF
    }
    $proc = $script:proc
    $ready = $false
    $t0 = Get-Date
    while (((Get-Date) - $t0).TotalSeconds -lt $ReadySec) {
        Start-Sleep -Milliseconds 1000
        if ($proc.HasExited) { break }
        $pg = Invoke-Cys $Set $Pipe 'ping' 8
        if ($pg.rc -eq 0) { $ready = $true; break }
    }
    return [ordered]@{ proc = $proc; pid = $proc.Id; ready = $ready; exited = $proc.HasExited; err_file = $errF; out_file = $outF; waited = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1) }
}

function Save-DaemonLogs {
    param([string]$Tag, $D)
    foreach ($f in @($D['err_file'], $D['out_file'])) {
        try {
            if (Test-Path -LiteralPath $f) {
                $txt = Read-FileShared $f
                if ($txt.Length -gt 400000) { $txt = $txt.Substring(0, 150000) + "`r`n...[cut]...`r`n" + $txt.Substring($txt.Length - 150000) }
                Save-Text ('w44-' + $Tag + '-' + (Split-Path -Leaf $f)) $txt
            }
        } catch { }
    }
}

function Get-HttpOnce {
    param([string]$Url, [int]$TimeoutMs = 5000)
    $o = [ordered]@{ url = $Url; status = $null; body = ''; error = $null; ms = 0 }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        $req = [System.Net.HttpWebRequest]::Create($Url)
        $req.Timeout = $TimeoutMs
        $req.ReadWriteTimeout = $TimeoutMs
        $req.Proxy = $null
        $resp = $null
        try { $resp = $req.GetResponse() } catch [System.Net.WebException] { $resp = $_.Exception.Response; if ($null -eq $resp) { throw } }
        $o['status'] = [int]$resp.StatusCode
        $sr = New-Object System.IO.StreamReader($resp.GetResponseStream())
        $buf = New-Object char[] 4096
        $n = $sr.Read($buf, 0, 4096)
        if ($n -gt 0) { $o['body'] = [string]::new($buf, 0, $n) }
        $resp.Close()
    } catch { $o['error'] = $_.Exception.Message }
    $sw.Stop(); $o['ms'] = [int]$sw.ElapsedMilliseconds
    return $o
}

function Wait-Http {
    param([string]$Url, [int]$Sec = 30)
    $t0 = Get-Date
    while (((Get-Date) - $t0).TotalSeconds -lt $Sec) {
        $h = Get-HttpOnce $Url 3000
        if ($null -ne $h['status']) { return $h }
        Start-Sleep -Milliseconds 700
    }
    return $null
}

function ConvertTo-PosixPath {
    param([string]$P)
    $q = $P -replace '\\', '/'
    if ($q -match '^([A-Za-z]):(.*)$') { $q = '/' + $Matches[1].ToLowerInvariant() + $Matches[2] }
    return $q
}

# =====================================================================================================================================
# S0 / S1
# =====================================================================================================================================
Step 'S0-inputs' {
    $inp = @{ build_run_id = 0; artifact = 'cys-windows-x64-nsis' }
    try {
        $j = ConvertFrom-Json (Read-TextUtf8 (Join-Path $env:GITHUB_WORKSPACE 'diag\w44-input.json'))
        if ($j.build_run_id) { $inp['build_run_id'] = [int64]$j.build_run_id }
        if ($j.artifact) { $inp['artifact'] = [string]$j.artifact }
    } catch { Write-Log ('w44-input.json: ' + $_.Exception.Message) 'WARN' }
    $runId = [int64]$inp['build_run_id']
    if ([string]$env:W44_BUILD_IN_RUN -eq '1') { $runId = [int64]$env:W44_THIS_RUN }
    # debugging the harness itself: a public release installer instead of a build artifact (input key installer_release_tag, e.g. v0.14.43)
    $relTag = ''
    try { $jj = ConvertFrom-Json (Read-TextUtf8 (Join-Path $env:GITHUB_WORKSPACE 'diag\w44-input.json')); if ($jj.installer_release_tag) { $relTag = [string]$jj.installer_release_tag } } catch { }
    if (($relTag -ne '') -and ([string]$env:W44_BUILD_IN_RUN -ne '1')) {
        $dir0 = Join-Path $WORK 'installer'
        New-Item -ItemType Directory -Path $dir0 -Force | Out-Null
        $node0 = Find-Exe 'node.exe'
        $cmd0 = ('"{0}" "{1}" --tag {2} --suffix _x64-setup.exe --out "{3}" --report "{4}"' -f $node0, (Join-Path $env:GITHUB_WORKSPACE 'diag\fetch-release-asset.mjs'), $relTag, $dir0, (Join-Path $WORK 'installer-fetch.json'))
        $f0 = Invoke-Proc -File (Join-Path $env:windir 'System32\cmd.exe') -Arguments ('/d /c "' + $cmd0 + '"') -TimeoutSec 900
        $exe0 = Get-ChildItem -LiteralPath $dir0 -Filter '*.exe' | Select-Object -First 1
        if (-not $exe0) { throw ('release installer ' + $relTag + ' not downloaded: ' + (Limit $f0.err 300)) }
        $X['installer'] = $exe0.FullName
        $X['run_id'] = 0
        Item 'installer' ([ordered]@{ name = $exe0.Name; size = $exe0.Length; sha256 = (Get-Sha256 $exe0.FullName); source = ('release ' + $relTag + ' (harness debugging)') })
        return
    }
    $X['run_id'] = $runId
    $X['artifact'] = $inp['artifact']
    Item 'build_run' $runId
    if ($runId -le 0) { throw 'no build run id (neither [w44-build] nor w44-input.json)' }
    $repo = [string]$env:GITHUB_REPOSITORY
    $hdr = @{ Authorization = ('Bearer ' + $env:GITHUB_TOKEN); Accept = 'application/vnd.github+json'; 'X-GitHub-Api-Version' = '2022-11-28' }
    $art = $null
    $t0 = Get-Date
    $maxMin = 58
    while (((Get-Date) - $t0).TotalMinutes -lt $maxMin) {
        try {
            $l = Invoke-RestMethod -Uri ('https://api.github.com/repos/{0}/actions/runs/{1}/artifacts?per_page=100' -f $repo, $runId) -Headers $hdr -TimeoutSec 60
            foreach ($a in @($l.artifacts)) { if ([string]$a.name -eq [string]$inp['artifact'] -and -not $a.expired) { $art = $a } }
        } catch { Write-Log ('artifact list: ' + $_.Exception.Message) 'WARN' }
        if ($art) { break }
        Start-Sleep -Seconds 45
    }
    if (-not $art) { throw ('artifact ' + $inp['artifact'] + ' of run ' + $runId + ' did not appear in ' + $maxMin + ' min') }
    Write-Log ('artifact found: id {0} size {1} after {2} min' -f $art.id, $art.size_in_bytes, [math]::Round(((Get-Date) - $t0).TotalMinutes, 1))
    $zip = Join-Path $WORK 'installer.zip'
    $cfg = Join-Path $WORK 'curl.cfg'
    [System.IO.File]::WriteAllText($cfg, ('header = "Authorization: Bearer {0}"' + "`n" + 'header = "Accept: application/vnd.github+json"' + "`n") -f $env:GITHUB_TOKEN)
    $curl = Join-Path $env:windir 'System32\curl.exe'
    $d = Invoke-Proc -File $curl -Arguments ('-L --fail --retry 3 -sS -K "{0}" -o "{1}" "{2}"' -f $cfg, $zip, [string]$art.archive_download_url) -TimeoutSec 900
    Remove-Item -LiteralPath $cfg -Force -ErrorAction SilentlyContinue
    Write-Log ('download rc {0} err {1}' -f $d.rc, (Limit $d.err 200))
    if (-not (Test-Path -LiteralPath $zip)) { throw 'installer zip not downloaded' }
    $dst = Join-Path $WORK 'installer'
    Expand-Archive -LiteralPath $zip -DestinationPath $dst -Force
    $exe = Get-ChildItem -LiteralPath $dst -Filter '*.exe' -Recurse | Select-Object -First 1
    if (-not $exe) { throw 'no exe inside the artifact' }
    $X['installer'] = $exe.FullName
    Item 'installer' ([ordered]@{ name = $exe.Name; size = $exe.Length; sha256 = (Get-Sha256 $exe.FullName); artifact_id = $art.id; waited_min = [math]::Round(((Get-Date) - $t0).TotalMinutes, 1) })
}

Step 'S1-install' {
    if (-not $X['installer']) { throw 'no installer' }
    $inst = Join-Path $env:LOCALAPPDATA 'cys'
    $X['inst'] = $inst
    $p = Start-Process -FilePath $X['installer'] -ArgumentList '/S' -PassThru
    $null = $p.Handle
    if (-not $p.WaitForExit(420000)) { Stop-ProcessTree -ProcessId $p.Id }
    Start-Sleep -Seconds 8
    $killed = @(Stop-AllW44Procs)
    Write-Log ('installer rc {0}, killed after install: {1}' -f $p.ExitCode, ($killed -join ','))
    $set = @{ dir = $inst; cys = (Join-Path $inst 'cys.exe'); cysd = (Join-Path $inst 'cysd.exe'); py = (Join-Path $inst 'runtime\python\python.exe'); bash = (Join-Path $inst 'runtime\git\bin\bash.exe') }
    foreach ($k in @('cys', 'cysd', 'py', 'bash')) { if (-not (Test-Path -LiteralPath $set[$k])) { Write-Log ('MISSING ' + $k + ' ' + $set[$k]) 'WARN' } }
    $X['new'] = $set
    $v1 = Invoke-Proc -File $set['cys'] -Arguments '--version' -TimeoutSec 30
    $v2 = Invoke-Proc -File $set['cysd'] -Arguments '--version' -TimeoutSec 30
    Item 'installed' ([ordered]@{ installer_rc = $p.ExitCode; cys = (Limit $v1.out 100); cysd = (Limit $v2.out 100); has_python = (Test-Path -LiteralPath $set['py']); has_bash = (Test-Path -LiteralPath $set['bash']) })
    # pack: extract pack.tar.gz of the install dir
    $tgz = Join-Path $inst 'pack.tar.gz'
    $pk = Join-Path $WORK 'pack-new'
    New-Item -ItemType Directory -Path $pk -Force | Out-Null
    $tr = Invoke-Proc -File (Join-Path $env:windir 'System32\tar.exe') -Arguments ('-xzf "{0}" -C "{1}"' -f $tgz, $pk) -TimeoutSec 300
    Write-Log ('tar new rc {0} {1}' -f $tr.rc, (Limit $tr.err 200))
    $X['pack_new'] = $pk
}

Step 'S1b-old-0.14.43' {
    $dir = Join-Path $WORK 'old-dl'
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $node = Find-Exe 'node.exe'
    $cmd = ('"{0}" "{1}" --tag v0.14.43 --suffix _x64-setup.exe --out "{2}" --report "{3}"' -f $node, (Join-Path $env:GITHUB_WORKSPACE 'diag\fetch-release-asset.mjs'), $dir, (Join-Path $WORK 'old-fetch.json'))
    $f = Invoke-Proc -File (Join-Path $env:windir 'System32\cmd.exe') -Arguments ('/d /c "' + $cmd + '"') -TimeoutSec 900
    Write-Log ('fetch old rc {0} {1}' -f $f.rc, (Limit $f.err 300))
    $old = Get-ChildItem -LiteralPath $dir -Filter '*.exe' | Select-Object -First 1
    if (-not $old) { throw 'public 0.14.43 installer not downloaded' }
    $z7 = Find-Exe '7z.exe' @('C:\Program Files\7-Zip\7z.exe', 'C:\Program Files (x86)\7-Zip\7z.exe')
    if (-not $z7) { throw '7z.exe not found' }
    $ex = Join-Path $WORK 'old-x'
    $e = Invoke-Proc -File $z7 -Arguments ('x -y "-o{0}" "{1}" cys.exe cysd.exe pack.tar.gz -r' -f $ex, $old.FullName) -TimeoutSec 900
    Write-Log ('7z rc {0} {1}' -f $e.rc, (Limit ($e.out + $e.err) 400))
    $found = @{}
    foreach ($n in @('cys.exe', 'cysd.exe', 'pack.tar.gz')) {
        $m = Get-ChildItem -LiteralPath $ex -Filter $n -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($m) { $found[$n] = $m.FullName }
    }
    Save-Json 'w44-old-extract.json' ([ordered]@{ installer = $old.Name; rc = $e.rc; found = $found; listing_out = (Limit $e.out 3000) }) 5
    if (-not ($found['cys.exe'] -and $found['cysd.exe'])) { throw 'old cys.exe / cysd.exe not extracted' }
    $nset = $X['new']
    $X['old'] = @{ dir = (Split-Path -Parent $found['cys.exe']); cys = $found['cys.exe']; cysd = $found['cysd.exe']; py = $nset['py']; bash = $nset['bash'] }
    $v1 = Invoke-Proc -File $X['old']['cys'] -Arguments '--version' -TimeoutSec 30
    $v2 = Invoke-Proc -File $X['old']['cysd'] -Arguments '--version' -TimeoutSec 30
    Item 'old_binaries' ([ordered]@{ cys = (Limit $v1.out 100); cysd = (Limit $v2.out 100); cysd_sha = (Get-Sha256 $X['old']['cysd']) })
    $pk = Join-Path $WORK 'pack-old'
    New-Item -ItemType Directory -Path $pk -Force | Out-Null
    if ($found['pack.tar.gz']) {
        $tr = Invoke-Proc -File (Join-Path $env:windir 'System32\tar.exe') -Arguments ('-xzf "{0}" -C "{1}"' -f $found['pack.tar.gz'], $pk) -TimeoutSec 300
        Write-Log ('tar old rc {0}' -f $tr.rc)
    }
    $X['pack_old'] = $pk
}

# find the bridge script of an extracted pack (the tar may or may not have a top folder)
function Find-BridgeScript {
    param([string]$PackDir)
    $m = Get-ChildItem -LiteralPath $PackDir -Filter 'javis_hud_bridge.py' -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($m) { return $m.FullName }
    return $null
}

# =====================================================================================================================================
# TA  approval (item 1)
# =====================================================================================================================================
function Write-Wrapper {
    # returns the command line that runs a wrapper script in the given shell kind
    param([string]$Kind, [string]$Name, [string]$Dir, [string[]]$CysLines, [string]$OutFile, [hashtable]$Set)
    $base = Join-Path $WORK ('wrap-' + $Name)
    if ($Kind -eq 'cmd') {
        $bat = $base + '.bat'
        $lines = @('@echo off', ('cd /d "{0}"' -f $Dir))
        foreach ($l in $CysLines) { $lines += ('"{0}" {1} >> "{2}" 2>&1' -f $Set['cys'], $l, $OutFile); $lines += ('echo rc=%errorlevel%>> "{0}"' -f $OutFile) }
        $lines += ('echo cwd-seen-by-cmd=%CD%>> "{0}"' -f $OutFile)
        [System.IO.File]::WriteAllText($bat, (($lines -join "`r`n") + "`r`n"), (New-Object System.Text.UTF8Encoding($false)))
        return @{ file = (Join-Path $env:windir 'System32\cmd.exe'); args = ('/d /c call "{0}"' -f $bat); typed = ('call "{0}"' -f $bat); script = $bat }
    }
    if ($Kind -eq 'ps') {
        $ps1 = $base + '.ps1'
        $lines = @(('Set-Location -LiteralPath ''{0}''' -f $Dir))
        foreach ($l in $CysLines) { $lines += ('$a = {0}; & ''{1}'' @a *>> ''{2}''' -f $l, $Set['cys'], $OutFile); $lines += ('"rc=$LASTEXITCODE" | Out-File -Append -Encoding ascii -LiteralPath ''{0}''' -f $OutFile) }
        $lines += ('"cwd-seen-by-ps=$((Get-Location).Path)" | Out-File -Append -Encoding ascii -LiteralPath ''{0}''' -f $OutFile)
        [System.IO.File]::WriteAllText($ps1, (($lines -join "`r`n") + "`r`n"), (New-Object System.Text.UTF8Encoding($false)))
        $psexe = Join-Path $env:windir 'System32\WindowsPowerShell\v1.0\powershell.exe'
        return @{ file = $psexe; args = ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}"' -f $ps1); typed = ('powershell -NoProfile -ExecutionPolicy Bypass -File "{0}"' -f $ps1); script = $ps1 }
    }
    # bash (bundled Git Bash, login shell like the product's seats)
    $sh = $base + '.sh'
    $lines = @(('cd ''{0}'' || echo cd-failed >> ''{1}''' -f (ConvertTo-PosixPath $Dir), (ConvertTo-PosixPath $OutFile)))
    foreach ($l in $CysLines) {
        # the argument text is given in bash syntax by the caller (single/double quotes as typed in bash)
        $lines += ('''{0}'' {1} >> ''{2}'' 2>&1' -f (ConvertTo-PosixPath $Set['cys']), $l, (ConvertTo-PosixPath $OutFile))
        $lines += ('echo "rc=$?" >> ''{0}''' -f (ConvertTo-PosixPath $OutFile))
    }
    $lines += ('echo "cwd-seen-by-bash=$(pwd) / $(pwd -W)" >> ''{0}''' -f (ConvertTo-PosixPath $OutFile))
    [System.IO.File]::WriteAllText($sh, (($lines -join "`n") + "`n"), (New-Object System.Text.UTF8Encoding($false)))
    return @{ file = $Set['bash']; args = ('-l "{0}"' -f (ConvertTo-PosixPath $sh)); typed = ('"{0}" -l "{1}"' -f $Set['bash'], (ConvertTo-PosixPath $sh)); script = $sh }
}

# quote argument text for the three shells (the VALUE is the same text; the quoting differs)
function Get-ArgText {
    param([string]$Kind, [string]$Subcmd, [string]$Value, [string[]]$Rest)
    # Subcmd e.g. 'approval sign --prefix' ; Value = the prefix / command text (may contain double quotes and backslashes); Rest = more args
    $restItems = @()
    foreach ($r in $Rest) { foreach ($t in ($r -split ' ')) { if ($t -ne '') { $restItems += $t } } }
    if ($Kind -eq 'cmd') {
        $line = $Subcmd + ' ' + (ConvertTo-CmdArg $Value)
        foreach ($t in $restItems) { $line += ' ' + (ConvertTo-CmdArg $t) }
        return $line
    }
    if ($Kind -eq 'ps') {
        # a PowerShell array literal: PowerShell 5.1 builds the native command line from the elements itself (the way a typed command does)
        $q = [string][char]39
        $items = @()
        foreach ($t in ($Subcmd -split ' ')) { if ($t -ne '') { $items += ($q + ($t -replace $q, ($q + $q)) + $q) } }
        $items += ($q + ($Value -replace $q, ($q + $q)) + $q)
        foreach ($t in $restItems) { $items += ($q + ($t -replace $q, ($q + $q)) + $q) }
        return ('@(' + ($items -join ',') + ')')
    }
    # bash: double quotes; backslash and double quote escaped for bash (bash hands the original text to the program)
    $bs = [string][char]92
    $dq = [string][char]34
    $v = $Value.Replace($bs, $bs + $bs).Replace($dq, $bs + $dq)
    $line = $Subcmd + ' ' + $dq + $v + $dq
    foreach ($t in $restItems) { $line += ' ' + $dq + $t.Replace($bs, $bs + $bs) + $dq }
    return $line
}

function Read-OutFile {
    param([string]$F)
    if (Test-Path -LiteralPath $F) { return (Read-FileShared $F) }
    return ''
}

function Wait-OutRc {
    param([string]$F, [int]$Count, [int]$Sec = 45)
    $t0 = Get-Date
    while (((Get-Date) - $t0).TotalSeconds -lt $Sec) {
        $t = Read-OutFile $F
        $n = ([regex]::Matches($t, '(?m)^rc=')).Count
        if ($n -ge $Count -and $t -match 'cwd-seen-by-') { return $t }
        Start-Sleep -Milliseconds 700
    }
    return (Read-OutFile $F)
}

function Invoke-ApprovalSuite {
    param([string]$Tag, [hashtable]$Set)
    $S = [ordered]@{ tag = $Tag; daemon = $null; master = $null; signs = [ordered]@{}; checks = @(); store = $null; lane_file = $null; notes = @() }
    $pipe = '\\.\pipe\cys-w44a' + $Tag
    $killed = @(Stop-AllW44Procs)
    # reset the profile state of the previous daemon (approvals, policy, tokens) - a throw-away runner profile
    $dotcys = Join-Path $env:USERPROFILE '.cys'
    if (Test-Path -LiteralPath $dotcys) { Remove-Item -LiteralPath $dotcys -Recurse -Force -ErrorAction SilentlyContinue }
    $dirs = @{}
    foreach ($n in @('D1 one', 'D2 two', 'w44proj')) { $d = Join-Path $WORK ('dirs\' + $n); New-Item -ItemType Directory -Path $d -Force | Out-Null; $dirs[$n] = $d }
    $D1 = $dirs['D1 one']; $D2 = $dirs['D2 two']
    $d = Start-W44Daemon $Set $pipe ('approval-' + $Tag) @{ CYS_NO_OFFICE_BRIDGE = '1' }
    $S['daemon'] = [ordered]@{ pid = $d['pid']; ready = $d['ready']; exited = $d['exited']; waited = $d['waited'] }
    if (-not $d['ready']) { Save-DaemonLogs ('approval-' + $Tag) $d; $S['notes'] += 'daemon did not answer ping'; return $S }
    $ms = Invoke-Cys $Set $pipe 'new-surface --role master --title w44master --cmd cmd.exe' 60
    $ref = (Limit $ms.out 80)
    $S['master'] = [ordered]@{ rc = $ms.rc; out = $ref; err = (Limit $ms.err 300) }
    if ($ms.rc -ne 0 -or $ref -notmatch 'surface') { $S['notes'] += 'master surface not created'; Save-DaemonLogs ('approval-' + $Tag) $d; return $S }
    $S['master']['wait_for_cooldown_sec'] = 66
    Start-Sleep -Seconds 66
    $lst = Invoke-Cys $Set $pipe 'list' 30
    Save-Text ('w44-approval-' + $Tag + '-list.txt') ($lst.out + $lst.err)

    function Send-Master {
        param([string]$TypedLine)
        $a = Invoke-Cys $Set $pipe ('send --surface ' + $ref + ' ' + (ConvertTo-CmdArg $TypedLine)) 30
        Start-Sleep -Milliseconds 400
        $b = Invoke-Cys $Set $pipe ('send-key --surface ' + $ref + ' Return') 30
        return @{ send_rc = $a.rc; send_err = (Limit $a.err 200); key_rc = $b.rc }
    }

    # ---- sign: plain (folder bound) from three shells in D1 ----
    $signOut = Join-Path $WORK ('sign-' + $Tag + '.txt')
    if (Test-Path -LiteralPath $signOut) { Remove-Item -LiteralPath $signOut -Force }
    $kinds = @('cmd', 'ps', 'bash')
    foreach ($k in $kinds) {
        $o = Join-Path $WORK ('sign-' + $Tag + '-' + $k + '.txt')
        if (Test-Path -LiteralPath $o) { Remove-Item -LiteralPath $o -Force }
        $argt = Get-ArgText $k 'approval sign --prefix' ('git w44' + $k) @('--ttl 900')
        $w = Write-Wrapper $k ('sign-' + $Tag + '-' + $k) $D1 @($argt) $o $Set
        $sm = Send-Master $w['typed']
        $txt = Wait-OutRc $o 1 60
        $S['signs']['plain-' + $k] = [ordered]@{ typed = $w['typed']; send = $sm; out = (Limit $txt 800) }
    }
    # ---- sign: A3 daemon-bound target verb (kill) from cmd in D1 ----
    $o = Join-Path $WORK ('sign-' + $Tag + '-kill.txt')
    if (Test-Path -LiteralPath $o) { Remove-Item -LiteralPath $o -Force }
    $w = Write-Wrapper 'cmd' ('sign-' + $Tag + '-kill') $D1 @((Get-ArgText 'cmd' 'approval sign --prefix' 'cys kill 4242' @('--ttl 900'))) $o $Set
    $null = Send-Master $w['typed']
    $S['signs']['kill-4242'] = [ordered]@{ out = (Limit (Wait-OutRc $o 1 60) 800) }

    # ---- sign: launch-agent forms (cmd, D1) ----
    $proj = $dirs['w44proj']
    $forms = [ordered]@{
        'F1-unquoted-backslash' = ('cys launch-agent --role worker --agent claude --cwd ' + $proj)
        'F2-quoted-backslash'   = ('cys launch-agent --role worker --agent claude --cwd "' + $proj + '"')
        'F3-slash'              = ('cys launch-agent --role worker --agent claude --cwd ' + ($proj -replace '\\', '/'))
        'F4-drive-only'         = 'cys launch-agent --role worker --agent claude --cwd C:'
        'F5-drive-backslash'    = 'cys launch-agent --role worker --agent claude --cwd C:\'
        'F6-no-cwd'             = 'cys launch-agent --role worker --agent claude'
    }
    $S['forms'] = $forms
    foreach ($fk in $forms.Keys) {
        $o = Join-Path $WORK ('sign-' + $Tag + '-' + $fk + '.txt')
        if (Test-Path -LiteralPath $o) { Remove-Item -LiteralPath $o -Force }
        $w = Write-Wrapper 'cmd' ('sign-' + $Tag + '-' + $fk) $D1 @((Get-ArgText 'cmd' 'approval sign --prefix' $forms[$fk] @('--ttl 900'))) $o $Set
        $null = Send-Master $w['typed']
        $S['signs'][$fk] = [ordered]@{ text = $forms[$fk]; out = (Limit (Wait-OutRc $o 1 60) 600) }
    }

    # ---- the store as the daemon wrote it ----
    foreach ($sf in @('approvals-ttl.json', 'approvals.json')) {
        $pth = Join-Path $env:USERPROFILE ('.cys\' + $sf)
        if (Test-Path -LiteralPath $pth) { Save-Text ('w44-approval-' + $Tag + '-' + $sf) (Read-FileShared $pth) }
    }
    $lane = Get-ChildItem -LiteralPath $env:LOCALAPPDATA -Filter 'approval-lane' -Recurse -ErrorAction SilentlyContinue | Select-Object -First 3
    $S['lane_file'] = @($lane | ForEach-Object { $_.FullName })

    # ---- checks from the three shells, in D2 (a different folder than the signer's D1) ----
    function Run-Check {
        param([string]$Kind, [string]$Label, [string]$Text, [string]$Dir, [string]$ExtraArgs = '--require-ttl')
        $o = Join-Path $WORK ('chk-' + $Tag + '-' + $Label + '-' + $Kind + '.txt')
        if (Test-Path -LiteralPath $o) { Remove-Item -LiteralPath $o -Force }
        $argt = Get-ArgText $Kind 'approval check --prefix' $Text @($ExtraArgs)
        $w = Write-Wrapper $Kind ('chk-' + $Tag + '-' + $Label + '-' + $Kind) $Dir @($argt) $o $Set
        $envm = @{ CYS_SOCKET = $pipe }
        $x = $null
        Use-Env $envm { $script:x = Invoke-Proc -File $w['file'] -Arguments $w['args'] -TimeoutSec 60 }
        $x = $script:x
        $txt = Read-OutFile $o
        $rc = $null
        $m = [regex]::Match($txt, '(?m)^rc=(\d+)')
        if ($m.Success) { $rc = [int]$m.Groups[1].Value }
        $cwdSeen = ''
        $m2 = [regex]::Match($txt, '(?m)^cwd-seen-by-[a-z]+=(.*)$')
        if ($m2.Success) { $cwdSeen = $m2.Groups[1].Value.Trim() }
        return [ordered]@{ label = $Label; shell = $Kind; dir = $Dir; text = $Text; rc = $rc; out = (Limit $txt 700); shell_cwd = $cwdSeen; runner_rc = $x.rc }
    }
    $chk = New-Object System.Collections.Generic.List[object]
    foreach ($sk in $kinds) {
        foreach ($ck in $kinds) {
            $r = Run-Check $ck ('plain-signed-in-' + $sk) ('git w44' + $sk) $D1 '--require-ttl'
            $chk.Add($r)
        }
    }
    # same folder different folder: plain approval from D2 (must be refused for a folder bound command on both versions)
    foreach ($sk in @('cmd')) { $chk.Add((Run-Check 'cmd' ('plain-from-other-folder-' + $sk) ('git w44' + $sk) $D2 '--require-ttl')) }
    # a probe that prints the signed folder strings: different explicit folder
    foreach ($sk in $kinds) { $chk.Add((Run-Check 'cmd' ('probe-signed-folder-of-' + $sk) ('git w44' + $sk) $D1 '--cwd Z:\w44-none --require-ttl')) }
    # A3: kill 4242 from the other folder, three shells
    foreach ($ck in $kinds) { $chk.Add((Run-Check $ck 'kill-from-other-folder' 'cys kill 4242' $D2 '--require-ttl')) }
    # launch-agent forms: checked from D2 in the three shells with the very text that was signed
    foreach ($fk in $forms.Keys) {
        foreach ($ck in $kinds) { $chk.Add((Run-Check $ck ('launch-' + $fk) $forms[$fk] $D2 '--require-ttl')) }
    }
    $S['checks'] = $chk.ToArray()
    $st = Invoke-Cys $Set $pipe 'approval check --prefix "git nothing signed" --require-ttl' 30 $D1
    $S['unsigned_check'] = [ordered]@{ rc = $st.rc; err = (Limit $st.err 300) }
    Save-DaemonLogs ('approval-' + $Tag) $d
    try { Stop-Process -Id $d['pid'] -Force -ErrorAction SilentlyContinue } catch { }
    $null = Stop-AllW44Procs
    return $S
}

Step 'TA-approval' {
    $res = [ordered]@{}
    foreach ($tag in @('new', 'old')) {
        if (-not $X[$tag]) { $res[$tag] = 'binary set missing'; continue }
        $res[$tag] = Invoke-ApprovalSuite $tag $X[$tag]
        Save-Json ('w44-approval-' + $tag + '.json') $res[$tag] 10
    }
    Item 'approval' 'see w44-approval-new.json / w44-approval-old.json'
}

# =====================================================================================================================================
# TB  bridge script (item 2)
# =====================================================================================================================================
function Start-BridgeScript {
    param([hashtable]$Set, [string]$Script, [string]$Tag, [int]$Port, [hashtable]$ExtraEnv, [string]$StateDir)
    $envm = @{ HUD_PORT = [string]$Port; HUD_STATE_DIR = $StateDir; JAVIS_ROOT = (Join-Path $WORK 'jroot'); HUD_CYS_BIN = $Set['cys'] }
    foreach ($k in $ExtraEnv.Keys) { $envm[$k] = $ExtraEnv[$k] }
    $errF = Join-Path $WORK ('bridge-' + $Tag + '.err.txt')
    $outF = Join-Path $WORK ('bridge-' + $Tag + '.out.txt')
    foreach ($f in @($errF, $outF)) { if (Test-Path -LiteralPath $f) { Remove-Item -LiteralPath $f -Force -ErrorAction SilentlyContinue } }
    Use-Env $envm {
        $script:bp = Start-Process -FilePath $Set['py'] -ArgumentList ('"{0}"' -f $Script) -PassThru -WindowStyle Hidden -RedirectStandardError $errF -RedirectStandardOutput $outF
    }
    return @{ proc = $script:bp; err = $errF; out = $outF }
}

function Measure-RawIdle {
    # open a TCP connection and send nothing; wait until the server closes it (or the limit) - how long the server holds an idle connection
    param([int]$Port, [int]$LimitSec)
    $c = New-Object System.Net.Sockets.TcpClient
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $closed = $false
    try {
        $c.Connect('127.0.0.1', $Port)
        $s = $c.GetStream()
        $s.ReadTimeout = 1000
        $buf = New-Object byte[] 16
        while ($sw.Elapsed.TotalSeconds -lt $LimitSec) {
            try { $n = $s.Read($buf, 0, 16); if ($n -eq 0) { $closed = $true; break } } catch [System.IO.IOException] { }
        }
    } catch { }
    try { $c.Close() } catch { }
    return [ordered]@{ closed_by_server = $closed; seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1); limit = $LimitSec }
}

function Invoke-BridgeSuite {
    param([string]$Tag, [hashtable]$Set, [string]$Script, [string]$Mode)
    # Mode: 'default' (nothing set) or 'new' (HUD_WIN_NEW=1)
    $port = 18700
    $S = [ordered]@{ tag = $Tag; mode = $Mode; script = $Script }
    $null = Stop-AllW44Procs
    $state = Join-Path $WORK ('bstate-' + $Tag)
    if (Test-Path -LiteralPath $state) { Remove-Item -LiteralPath $state -Recurse -Force -ErrorAction SilentlyContinue }
    $envx = @{ HUD_REQ_TIMEOUT = '6' }
    if ($Mode -eq 'new') { $envx['HUD_WIN_NEW'] = '1' }
    $A = Start-BridgeScript $Set $Script ($Tag + '-A') $port $envx $state
    $h = Wait-Http ('http://127.0.0.1:{0}/health' -f $port) 40
    $S['A_up'] = ($null -ne $h)
    $S['A_pid'] = $A['proc'].Id
    $S['A_alive_after_start'] = (Test-PidAlive $A['proc'].Id)
    $S['health_first'] = $h
    $S['world'] = (Get-HttpOnce ('http://127.0.0.1:{0}/world' -f $port) 8000)
    if ($S['world']['body'].Length -gt 300) { $S['world']['body'] = $S['world']['body'].Substring(0, 300) }
    $S['root_page'] = (Get-HttpOnce ('http://127.0.0.1:{0}/' -f $port) 8000)
    if ($S['root_page']['body'].Length -gt 120) { $S['root_page']['body'] = $S['root_page']['body'].Substring(0, 120) }
    $tokFile = Join-Path $state 'token'
    $S['token_before_B'] = $(if (Test-Path -LiteralPath $tokFile) { Get-Sha256 $tokFile } else { $null })
    # idle connection: HUD_REQ_TIMEOUT=6 is honoured only where the request timeout is on
    $S['idle_connection'] = (Measure-RawIdle $port 14)
    # second bind on the taken port, same state folder
    $B = Start-BridgeScript $Set $Script ($Tag + '-B') $port $envx $state
    Start-Sleep -Seconds 6
    $B_alive = (Test-PidAlive $B['proc'].Id)
    $S['B_alive_after_6s'] = $B_alive
    $S['B_exit_code'] = $(if (-not $B_alive) { try { $B['proc'].ExitCode } catch { $null } } else { $null })
    # which instance answers? 40 /health requests, count distinct pids
    $pids = @{}
    $fail = 0
    for ($i = 0; $i -lt 40; $i++) {
        $x = Get-HttpOnce ('http://127.0.0.1:{0}/health' -f $port) 3000
        $m = [regex]::Match([string]$x['body'], '"pid"\s*:\s*(\d+)')
        if ($m.Success) { $pids[$m.Groups[1].Value] = 1 + [int]$pids[$m.Groups[1].Value] } else { $fail++ }
    }
    $S['health_pid_counts'] = $pids
    $S['health_no_pid'] = $fail
    $S['token_after_B'] = $(if (Test-Path -LiteralPath $tokFile) { Get-Sha256 $tokFile } else { $null })
    $S['token_file_changed_by_B'] = ($S['token_before_B'] -ne $S['token_after_B'])
    Start-Sleep -Milliseconds 300
    foreach ($f in @($A['err'], $B['err'])) {
        try { if (Test-Path -LiteralPath $f) { Save-Text ('w44-bridge-' + (Split-Path -Leaf $f)) (Limit (Read-FileShared $f) 12000) } } catch { }
    }
    $S['A_err_head'] = (Limit (Read-OutFile $A['err']) 500)
    $S['B_err_head'] = (Limit (Read-OutFile $B['err']) 500)
    # a stopped B / A
    try { Stop-Process -Id $B['proc'].Id -Force -ErrorAction SilentlyContinue } catch { }
    try { Stop-Process -Id $A['proc'].Id -Force -ErrorAction SilentlyContinue } catch { }
    $null = Stop-AllW44Procs
    return $S
}

Step 'TB-bridge' {
    New-Item -ItemType Directory -Path (Join-Path $WORK 'jroot') -Force | Out-Null
    $res = [ordered]@{}
    foreach ($tag in @('new', 'old')) {
        $pk = $(if ($tag -eq 'new') { $X['pack_new'] } else { $X['pack_old'] })
        if (-not $pk) { $res[$tag] = 'pack missing'; continue }
        $sc = Find-BridgeScript $pk
        if (-not $sc) { $res[$tag] = 'bridge script not found in ' + $pk; continue }
        foreach ($mode in @('default', 'new')) {
            if ($tag -eq 'old' -and $mode -eq 'new') { continue }   # the old script has no HUD_WIN_NEW
            $key = $tag + '-' + $mode
            $res[$key] = Invoke-BridgeSuite $key $X[$tag] $sc $mode
            Save-Json ('w44-bridge-' + $key + '.json') $res[$key] 8
        }
    }
    # which bind semantics does this Windows have for a python http.server style socket (SO_REUSEADDR) vs SO_EXCLUSIVEADDRUSE ?
    $probe = Join-Path $WORK 'bindprobe.py'
    $py = @'
import socket, json, sys
def srv(opt):
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    if opt == 'reuse':
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    elif opt == 'excl':
        s.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
    return s
out = {}
for a, b in [('none', 'none'), ('reuse', 'reuse'), ('reuse', 'none'), ('none', 'reuse'), ('excl', 'reuse'), ('excl', 'excl'), ('excl', 'none'), ('reuse', 'excl')]:
    port = 18900 + len(out)
    s1 = srv(a); s2 = srv(b)
    r = {}
    try:
        s1.bind(('127.0.0.1', port)); s1.listen(5); r['first'] = 'ok'
    except OSError as e:
        r['first'] = 'fail %s' % e
    try:
        s2.bind(('127.0.0.1', port)); r['second'] = 'BIND OK'
        try:
            s2.listen(5); r['second_listen'] = 'ok'
        except OSError as e:
            r['second_listen'] = 'fail %s' % e
    except OSError as e:
        r['second'] = 'bind fail winerror=%s' % getattr(e, 'winerror', None)
    out['%s+%s' % (a, b)] = r
    s1.close(); s2.close()
print(json.dumps(out, indent=1))
print('python', sys.version.split()[0])
'@
    [System.IO.File]::WriteAllText($probe, $py, (New-Object System.Text.UTF8Encoding($false)))
    $pr = Invoke-Proc -File $X['new']['py'] -Arguments ('"{0}"' -f $probe) -TimeoutSec 60
    Save-Text 'w44-bindprobe.txt' ($pr.out + "`r`n" + $pr.err)
}

# =====================================================================================================================================
# TC  real daemon + bridge (items 3-6)
# =====================================================================================================================================
function Get-BridgeProcs {
    $rows = @(Get-ProcRows)
    $br = @($rows | Where-Object { $_.Cmd -and ($_.Cmd -like '*javis_hud_bridge.py*') -and ($_.Name -like 'python*') })
    return @{ rows = $rows; bridges = $br }
}

function Measure-DaemonKill {
    # kill the daemon (Stop-Process -Force = TerminateProcess, what taskkill /F does) and watch the bridge and its children for up to 20 s
    param([int]$DaemonPid, [int]$BridgePid, $ChildIds, [int]$WatchSec = 20)
    $t0 = Get-Date
    try { Stop-Process -Id $DaemonPid -Force -ErrorAction Stop } catch { }
    $bridgeGoneAt = $null
    $childrenGoneAt = $null
    while (((Get-Date) - $t0).TotalSeconds -lt $WatchSec) {
        $ba = Test-PidAlive $BridgePid
        $ca = @($ChildIds | Where-Object { Test-PidAlive $_ }).Count
        if (-not $ba -and $null -eq $bridgeGoneAt) { $bridgeGoneAt = [math]::Round(((Get-Date) - $t0).TotalSeconds, 2) }
        if ($ca -eq 0 -and $null -eq $childrenGoneAt) { $childrenGoneAt = [math]::Round(((Get-Date) - $t0).TotalSeconds, 2) }
        if (-not $ba -and $ca -eq 0) { break }
        Start-Sleep -Milliseconds 250
    }
    return [ordered]@{ bridge_gone_after_s = $bridgeGoneAt; children_gone_after_s = $childrenGoneAt; bridge_alive_at_end = (Test-PidAlive $BridgePid); children_alive_at_end = @($ChildIds | Where-Object { Test-PidAlive $_ }).Count; children_before = @($ChildIds).Count; watched_s = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1) }
}

function Invoke-DaemonBridgeRun {
    # one real daemon run: mode env (or none), optional seat, wait for the bridge, hold, kill
    param([string]$Tag, [hashtable]$Set, [hashtable]$ExtraEnv, [bool]$WithSeat, [int]$HoldSec = 20, [bool]$DoHeal = $false, [bool]$ExpectBridge = $true)
    $S = [ordered]@{ tag = $Tag; env = $ExtraEnv; with_seat = $WithSeat }
    $null = Stop-AllW44Procs
    $dotcys = Join-Path $env:USERPROFILE '.cys'
    if ($Tag -notlike '*keepprofile*') {
        if (Test-Path -LiteralPath $dotcys) { Remove-Item -LiteralPath $dotcys -Recurse -Force -ErrorAction SilentlyContinue }
    }
    if ($ExtraEnv.ContainsKey('__policy_json')) {
        New-Item -ItemType Directory -Path $dotcys -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $dotcys 'policy.json'), [string]$ExtraEnv['__policy_json'], (New-Object System.Text.UTF8Encoding($false)))
        $ExtraEnv = @{} + $ExtraEnv
        $ExtraEnv.Remove('__policy_json')
    }
    $pipe = '\\.\pipe\cys-w44c' + ($Tag -replace '[^A-Za-z0-9]', '')
    $d = Start-W44Daemon $Set $pipe ('daemon-' + $Tag) $ExtraEnv 120
    $S['daemon'] = [ordered]@{ pid = $d['pid']; ready = $d['ready']; exited = $d['exited']; waited = $d['waited'] }
    if (-not $d['ready']) { Save-DaemonLogs ('daemon-' + $Tag) $d; $S['note'] = 'daemon not ready'; return $S }
    # the bridge (spawned by the daemon, python + javis_hud_bridge.py)
    $t0 = Get-Date
    $bp = $null
    while (((Get-Date) - $t0).TotalSeconds -lt 75) {
        $g = Get-BridgeProcs
        if ($g['bridges'].Count -gt 0) { $bp = $g['bridges'][0]; break }
        Start-Sleep -Seconds 2
    }
    $S['bridge_found'] = ($null -ne $bp)
    if ($bp) {
        $S['bridge'] = [ordered]@{ pid = $bp.Id; ppid = $bp.Ppid; cmd = (Limit $bp.Cmd 300); started_after_s = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1); parent_is_daemon = ($bp.Ppid -eq $d['pid']) }
        $h = Wait-Http 'http://127.0.0.1:8642/health' 25
        $S['health'] = $h
        $w = Get-HttpOnce 'http://127.0.0.1:8642/world' 8000
        if ($w['body'].Length -gt 200) { $w['body'] = $w['body'].Substring(0, 200) }
        $S['world'] = $w
    }
    if ($WithSeat) {
        $ms = Invoke-Cys $Set $pipe 'new-surface --title w44seat --cmd cmd.exe' 60
        $S['seat'] = [ordered]@{ rc = $ms.rc; out = (Limit $ms.out 80); err = (Limit $ms.err 200) }
        $ms2 = Invoke-Cys $Set $pipe 'new-surface --title w44seat2 --cmd cmd.exe' 60
        $S['seat2'] = [ordered]@{ rc = $ms2.rc; out = (Limit $ms2.out 80) }
        # a background child of a seat: a ping that runs for a long time, started through the seat
        $null = Invoke-Cys $Set $pipe ('send --surface ' + (Limit $ms.out 40) + ' ' + (ConvertTo-CmdArg 'start /b ping -n 600 127.0.0.1 > nul')) 30
        Start-Sleep -Milliseconds 300
        $null = Invoke-Cys $Set $pipe ('send-key --surface ' + (Limit $ms.out 40) + ' Return') 30
    }
    Start-Sleep -Seconds $HoldSec
    $g = Get-BridgeProcs
    $S['bridge_alive_after_hold'] = $(if ($bp) { Test-PidAlive $bp.Id } else { $null })
    $S['bridge_same_pid_after_hold'] = $(if ($bp) { (@($g['bridges'] | Where-Object { $_.Id -eq $bp.Id }).Count -eq 1) } else { $null })
    $S['bridge_count_after_hold'] = $g['bridges'].Count
    $kids = @()
    if ($bp) { $kids = @(Get-Descendants $bp.Id $g['rows']) }
    $S['bridge_children'] = @($kids | ForEach-Object { '{0}#{1} {2}' -f $_.Name, $_.Id, (Limit $_.Cmd 120) })
    $dd = @(Get-Descendants $d['pid'] $g['rows'])
    $S['daemon_children'] = @($dd | ForEach-Object { '{0}#{1} {2}' -f $_.Name, $_.Id, (Limit $_.Cmd 100) })
    if ($DoHeal) {
        $S['heal'] = Invoke-HealTests $Set $pipe $bp
    }
    if ($bp) {
        $kidIds = @($kids | ForEach-Object { $_.Id })
        $S['after_daemon_kill'] = Measure-DaemonKill $d['pid'] $bp.Id $kidIds 25
    } else {
        try { Stop-Process -Id $d['pid'] -Force -ErrorAction SilentlyContinue } catch { }
    }
    # whatever is left now
    Start-Sleep -Seconds 1
    $g2 = Get-BridgeProcs
    $S['bridges_left_at_end'] = $g2['bridges'].Count
    $S['port_8642_answers_at_end'] = ($null -ne (Get-HttpOnce 'http://127.0.0.1:8642/health' 2000)['status'])
    Save-DaemonLogs ('daemon-' + $Tag) $d
    $lines = @()
    try { $lines = @((Read-OutFile $d['err_file']) -split "`r?`n" | Where-Object { $_ -match 'office-bridge|office_bridge|hud-bridge' } | Select-Object -First 40) } catch { }
    $S['log_bridge_lines'] = $lines
    $null = Stop-AllW44Procs
    return $S
}

function Get-LedgerSnapshot {
    # sha256 of every file under ~/.cys except logs / sqlite / transcripts / sockets / heartbeat files (the "pending merge ledger" and the state files)
    $snap = [ordered]@{}
    $root = Join-Path $env:USERPROFILE '.cys'
    if (-not (Test-Path -LiteralPath $root)) { return $snap }
    foreach ($f in (Get-ChildItem -LiteralPath $root -Recurse -File -ErrorAction SilentlyContinue)) {
        $rel = $f.FullName.Substring($root.Length + 1)
        if ($rel -match '(?i)\.log$|\.db$|\.db-|\.sqlite|^logs\\|heartbeat|\.lock$|\.sock$|transcripts|\.pid$|operator\.token|^state\\|spool|^run\\') { continue }
        try { $snap[$rel] = (Get-Sha256 $f.FullName) } catch { }
    }
    return $snap
}

function Invoke-HealTests {
    param([hashtable]$Set, [string]$Pipe, $Bridge)
    $H = [ordered]@{}
    $pack = Join-Path $env:USERPROFILE '.cys\pack'
    $H['pack_dir_exists'] = (Test-Path -LiteralPath $pack)
    $rel = 'web/office-boot.js'
    $abs = Join-Path $pack 'web\office-boot.js'
    $H['target_exists_before'] = (Test-Path -LiteralPath $abs)
    $H['ledger_before'] = Get-LedgerSnapshot
    # (a) the file is missing: the bridge and the daemon are running
    $bak = $abs + '.w44bak'
    if (Test-Path -LiteralPath $abs) { Move-Item -LiteralPath $abs -Destination $bak -Force }
    $r1 = Invoke-Cys $Set $Pipe ('pack-heal ' + $rel + ' --missing-only') 90
    $H['a_missing'] = [ordered]@{ rc = $r1.rc; out = (Limit $r1.out 400); err = (Limit $r1.err 400); restored = (Test-Path -LiteralPath $abs) }
    if ($H['a_missing']['restored']) {
        $hb = Get-HttpOnce 'http://127.0.0.1:8642/office-boot.js' 5000
        $H['a_missing']['bridge_serves_it'] = $hb['status']
    }
    # (b) the file is present but another process holds it open WITHOUT delete sharing (what a reader does)
    $fs = $null
    try {
        $fs = [System.IO.File]::Open($abs, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::Read)
        $r2 = Invoke-Cys $Set $Pipe ('pack-heal ' + $rel + ' --missing-only') 90
        $H['b_present_locked'] = [ordered]@{ rc = $r2.rc; out = (Limit $r2.out 300); err = (Limit $r2.err 300) }
    } catch { $H['b_present_locked'] = 'could not open: ' + $_.Exception.Message } finally { if ($fs) { $fs.Close() } }
    # (c) the file is missing while the parent folder is held open by a directory watcher (ReadDirectoryChangesW handle, like an editor)
    $fsw = $null
    try {
        Remove-Item -LiteralPath $abs -Force -ErrorAction SilentlyContinue
        $fsw = New-Object System.IO.FileSystemWatcher (Split-Path -Parent $abs)
        $fsw.EnableRaisingEvents = $true
        $r3 = Invoke-Cys $Set $Pipe ('pack-heal ' + $rel + ' --missing-only') 90
        $H['c_missing_with_watcher'] = [ordered]@{ rc = $r3.rc; out = (Limit $r3.out 300); err = (Limit $r3.err 300); restored = (Test-Path -LiteralPath $abs) }
    } catch { $H['c_missing_with_watcher'] = 'error: ' + $_.Exception.Message } finally { if ($fsw) { $fsw.Dispose() } }
    $H['ledger_after'] = Get-LedgerSnapshot
    $chg = @()
    foreach ($k in $H['ledger_after'].Keys) { if (-not $H['ledger_before'].Contains($k)) { $chg += ('NEW ' + $k) } elseif ($H['ledger_before'][$k] -ne $H['ledger_after'][$k]) { $chg += ('CHANGED ' + $k) } }
    foreach ($k in $H['ledger_before'].Keys) { if (-not $H['ledger_after'].Contains($k)) { $chg += ('GONE ' + $k) } }
    $H['ledger_changes'] = $chg
    $H['ledger_files_counted'] = $H['ledger_after'].Count
    $H.Remove('ledger_before'); $H.Remove('ledger_after')
    if (Test-Path -LiteralPath $bak) { Remove-Item -LiteralPath $bak -Force -ErrorAction SilentlyContinue }
    return $H
}

Step 'TC-daemon' {
    $res = [ordered]@{}
    $new = $X['new']
    # (1) the default on Windows: legacy. Heal tests run here too (daemon + bridge running).
    $res['new-default'] = Invoke-DaemonBridgeRun 'newdefault' $new @{} $true 25 $true
    Save-Json 'w44-daemon-new-default.json' $res['new-default'] 10
    # (2) managed mode through the environment, without a seat
    $res['new-managed-noseat'] = Invoke-DaemonBridgeRun 'newmanagednoseat' $new @{ CYS_OFFICE_BRIDGE_MODE = 'managed' } $false 25
    Save-Json 'w44-daemon-new-managed-noseat.json' $res['new-managed-noseat'] 10
    # (3) managed mode with seats and a background child of a seat
    $res['new-managed-seat'] = Invoke-DaemonBridgeRun 'newmanagedseat' $new @{ CYS_OFFICE_BRIDGE_MODE = 'managed' } $true 25
    Save-Json 'w44-daemon-new-managed-seat.json' $res['new-managed-seat'] 10
    # (4) the policy file asks for managed: it must be ignored on Windows (default stays legacy) and the log says so
    $res['new-policy-managed'] = Invoke-DaemonBridgeRun 'newpolicymanaged' $new @{ __policy_json = '{"CYS_OFFICE_BRIDGE_MODE": "managed"}' } $false 20
    Save-Json 'w44-daemon-new-policy-managed.json' $res['new-policy-managed'] 10
    # (5) the 0.14.43 daemon as it is: legacy bridge; after its forced stop does the bridge survive?  (the "old bridge after an upgrade" mechanism)
    if ($X['old']) {
        $res['old-default'] = Invoke-DaemonBridgeRun 'olddefault' $X['old'] @{} $false 20
        Save-Json 'w44-daemon-old-default.json' $res['old-default'] 10
    }
}

# =====================================================================================================================================
# TD  hook (item 7)
# =====================================================================================================================================
Step 'TD-hook' {
    $pk = $X['pack_new']
    $hook = Get-ChildItem -LiteralPath $pk -Filter 'role-capability-gate.sh' -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $hook) { throw 'role-capability-gate.sh not found in the pack' }
    $bash = $X['new']['bash']
    $cases = [ordered]@{
        'plain-denied'        = '{"tool_name":"Bash","tool_input":{"command":"cys kill 4242"}}'
        'quote-backslash'     = '{"tool_name":"Bash","tool_input":{"command":"cys launch-agent --cwd \"C:\\x\\y\" --role worker"}}'
        'korean'              = '{"tool_name":"Bash","tool_input":{"command":"cys kill 4242 # \uD55C\uAE00"}}'
        'unpaired-surrogate'  = '{"tool_name":"Bash","tool_input":{"command":"cys kill \ud800 4242"}}'
    }
    $rows = [ordered]@{}
    foreach ($k in $cases.Keys) {
        $inF = Join-Path $WORK ('hook-in-' + $k + '.json')
        [System.IO.File]::WriteAllText($inF, $cases[$k], (New-Object System.Text.UTF8Encoding($false)))
        $sh = Join-Path $WORK ('hook-run-' + $k + '.sh')
        $script = ('export CYS_ROLE=cso' + "`n" + 'export CYS_SOCKET=''\\.\pipe\cys-w44-nohook''' + "`n" + '''{0}'' < ''{1}''' + "`n") -f (ConvertTo-PosixPath $hook.FullName), (ConvertTo-PosixPath $inF)
        [System.IO.File]::WriteAllText($sh, $script, (New-Object System.Text.UTF8Encoding($false)))
        $r = Invoke-Proc -File $bash -Arguments ('-l "{0}"' -f (ConvertTo-PosixPath $sh)) -TimeoutSec 60
        $js = $null
        try { $js = ConvertFrom-Json $r.out } catch { }
        $rows[$k] = [ordered]@{ rc = $r.rc; stdout_bytes = $r.out.Length; json_valid = ($null -ne $js); decision = $(if ($js -and $js.hookSpecificOutput) { [string]$js.hookSpecificOutput.permissionDecision } else { $null }); stdout = (Limit $r.out 300); stderr = (Limit $r.err 300) }
    }
    Save-Json 'w44-hook.json' ([ordered]@{ hook = $hook.FullName; cases = $rows }) 6
    Item 'hook' $rows
}

# =====================================================================================================================================
# SUMMARY (short, human readable; the details are in the json files)
# =====================================================================================================================================
Step 'SUMMARY' {
    $L = New-Object System.Collections.Generic.List[string]
    $L.Add(('w44 summary on {0} ({1})' -f $OSL, (Get-IsoNow)))
    foreach ($k in $R['steps'].Keys) { $L.Add(('step {0}: {1}s {2}' -f $k, $R['steps'][$k]['seconds'], $R['steps'][$k]['error'])) }
    Save-Text 'w44-summary.txt' ($L -join "`r`n")
}

Complete-DiagScript 'w44-measure'
