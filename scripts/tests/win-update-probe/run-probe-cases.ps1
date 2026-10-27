# WP12-WINSAC-0.14.53 · P1 — installer pre-swap run check, measured on a fresh windows-latest runner.
#
# Called by the `win-update-probe` job in .github/workflows/windows-build.yml (needs: build). Runs in ONE
# Windows PowerShell 5.1 process on purpose: an enforced App Control policy can put every NEW PowerShell
# into Constrained Language Mode (diag lane, origin/diag/win11-update:diag/sacrules.ps1 header), so the
# policy is deployed, used and removed inside this process.
#
#   W1  probe-only worker  /S /UPDATE /CYSPROBE=.cys-probe-w1 /D=<..>\.cys-probe-w1   -> exit 0, every leaf pass
#   W2  worker outside a .cys-probe-* folder                                          -> exit 2, nothing written
#   W3  /CYSPROBE without /S (reaches the probe page)                                 -> exit 2, nothing written (B29)
#   M1  the probe's ABSOLUTE execution deadline (0.14.55 P4-IMPL r1): the shipped CYS_RUN_BOUNDED body is
#       sliced out of src-tauri/nsis-hooks.nsh, compiled, and MEASURED - healthy exit / quiet hang /
#       continuous output / unconfirmed cleanup, each on both call-site shapes (silent inline leaf 30 s,
#       GUI worker wrapper 120 s). The continuous-output case is the regression: nsExec's /TIMEOUT was an
#       inactivity timer and never fired there. No makensis on the runner => UNMEASURED, never a pass.
#   POL AllowAll + one deny-by-HASH rule on W1's extracted cysd.exe (stand-in for Smart App Control: windows-latest
#       has none). Proven effective first: the denied copy must not start, an allowed binary must.
#       If it cannot be proven, T2-F/T2 are UNMEASURED and this script exits 1 (never a silent pass).
#   T2-F first install under the policy, setup.exe /S                                 -> exit 6, %LOCALAPPDATA%\cys absent,
#       %TEMP%\cys-install-blocked.txt with kept-version:none (B7e)
#   T2  previous public release installed + its daemon running, policy, new setup.exe /S /UPDATE
#                                                                                     -> exit 6, installer-owned files and
#       the uninstall registry key unchanged, daemon pid alive and answering, record names cysd, no probe folder (B7a, B8)
#   CTL policy removed: the same /S /UPDATE installs (exit 0, new marker, daemon alive, stale record removed), and a first
#       install into a fresh folder installs (exit 0)  - the must-differ controls
#
# Not run here (UNMEASURED, they need UI Automation of the installer window): T2-U wizard uninstall-first, T2-D delayed
# initiator hand-off, T2-G dialog text, T2-P passive.
param(
    [Parameter(Mandatory = $true)][string]$NewDir,
    [Parameter(Mandatory = $true)][string]$PrevDir,
    [Parameter(Mandatory = $true)][string]$Work
)
$ErrorActionPreference = 'Stop'
$script:Results = New-Object System.Collections.Generic.List[object]
$script:Unmeasured = New-Object System.Collections.Generic.List[string]
$Ev = Join-Path $Work 'evidence'
New-Item -ItemType Directory -Force -Path $Ev | Out-Null

function Say([string]$m) { Write-Host $m; Add-Content -LiteralPath (Join-Path $Ev 'log.txt') -Value $m }

function Check([string]$Id, [bool]$Ok, [string]$Detail) {
    $state = 'FAIL'
    if ($Ok) { $state = 'PASS' }
    $script:Results.Add([pscustomobject]@{ id = $Id; result = $state; detail = $Detail })
    Say ('RESULT {0} {1}  {2}' -f $Id, $state, $Detail)
}

# Run FILE with one argument line (NSIS needs /D= last and unquoted). Returns started / code ('timeout') / output.
function Invoke-Exe([string]$File, [string]$ArgLine, [int]$TimeoutSec) {
    $o = Join-Path $Work ('out-' + [guid]::NewGuid().ToString('N') + '.txt')
    $e = $o + '.err'
    $p = $null
    try {
        if ($ArgLine) {
            $p = Start-Process -FilePath $File -ArgumentList $ArgLine -PassThru -NoNewWindow -RedirectStandardOutput $o -RedirectStandardError $e
        } else {
            $p = Start-Process -FilePath $File -PassThru -NoNewWindow -RedirectStandardOutput $o -RedirectStandardError $e
        }
    } catch {
        return [pscustomobject]@{ started = $false; code = $null; out = ''; error = $_.Exception.Message }
    }
    $null = $p.Handle   # PowerShell 5.1: keep the handle so ExitCode is available after exit
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        return [pscustomobject]@{ started = $true; code = 'timeout'; out = ''; error = 'timeout' }
    }
    $p.WaitForExit()
    $out = ''
    foreach ($f in @($o, $e)) { if (Test-Path -LiteralPath $f) { $out += [IO.File]::ReadAllText($f); Remove-Item -LiteralPath $f -Force } }
    return [pscustomobject]@{ started = $true; code = $p.ExitCode; out = $out; error = '' }
}

function Read-Text([string]$Path) { if (Test-Path -LiteralPath $Path) { return [IO.File]::ReadAllText($Path) } return $null }
function Lines-Of([string]$Text) { if ($null -eq $Text) { return @() } return @($Text -split "`r?`n" | Where-Object { $_ -ne '' }) }
function Count-Files([string]$Dir) { if (-not (Test-Path -LiteralPath $Dir)) { return 0 } return @(Get-ChildItem -LiteralPath $Dir -Force -Recurse -File).Count }

# installer-owned state of an installation (design B8): top-level executables and DLLs (canonical, .new/.prev slots,
# uninstall.exe) by hash, the version marker, the runtime tree by hash, and the uninstall registry key values.
# The daemon's own state files live in the same folder and are deliberately not part of this snapshot.
function Get-InstallerOwnedState([string]$Dir) {
    $m = [ordered]@{}
    foreach ($f in @(Get-ChildItem -LiteralPath $Dir -Force -File | Where-Object { $_.Extension -in '.exe', '.dll' } | Sort-Object Name)) {
        $m['pe:' + $f.Name] = (Get-FileHash -Algorithm SHA256 -LiteralPath $f.FullName).Hash
    }
    $mk = Join-Path $Dir 'cys-installed-version.txt'
    $m['marker'] = Read-Text $mk
    $rt = Join-Path $Dir 'runtime'
    if (Test-Path -LiteralPath $rt) {
        $sha = [Security.Cryptography.SHA256]::Create()
        foreach ($f in @(Get-ChildItem -LiteralPath $rt -Force -Recurse -File | Sort-Object FullName)) {
            $rel = $f.FullName.Substring($rt.Length)
            $fs = [IO.File]::Open($f.FullName, 'Open', 'Read', 'ReadWrite')
            try { $h = [BitConverter]::ToString($sha.ComputeHash($fs)) } finally { $fs.Dispose() }
            $m['rt:' + $rel] = $h
        }
    }
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\cys'
    if (Test-Path $key) {
        $props = Get-ItemProperty -Path $key
        foreach ($n in @($props.PSObject.Properties | Where-Object { $_.Name -notlike 'PS*' } | Sort-Object Name)) {
            $m['reg:' + $n.Name] = [string]$n.Value
        }
    }
    return $m
}

function Compare-State($Before, $After) {
    $diff = New-Object System.Collections.Generic.List[string]
    foreach ($k in $Before.Keys) { if (-not $After.Contains($k)) { $diff.Add('gone ' + $k) } elseif ($Before[$k] -ne $After[$k]) { $diff.Add('changed ' + $k) } }
    foreach ($k in $After.Keys) { if (-not $Before.Contains($k)) { $diff.Add('new ' + $k) } }
    return $diff
}

function Probe-Dirs([string]$Dir) { if (-not (Test-Path -LiteralPath $Dir)) { return @() } return @(Get-ChildItem -LiteralPath $Dir -Force -Directory -Filter '.cys-probe*') }

$Inst = Join-Path $env:LOCALAPPDATA 'cys'
$CiTool = Join-Path $env:windir 'System32\CiTool.exe'
$UnKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\cys'
$Setup = @(Get-ChildItem -LiteralPath $NewDir -Filter 'cys_*_x64-setup.exe')
$Prev = @(Get-ChildItem -LiteralPath $PrevDir -Filter 'cys_*_x64-setup.exe')
if ($Setup.Count -ne 1 -or $Prev.Count -ne 1) { throw ('expected one new and one previous setup.exe, got {0} and {1}' -f $Setup.Count, $Prev.Count) }
$Setup = $Setup[0].FullName
$Prev = $Prev[0].FullName
$V = ([IO.Path]::GetFileName($Setup) -replace '^cys_', '' -replace '_x64-setup\.exe$', '')
Say ('new installer {0} (version {1}); previous {2}' -f $Setup, $V, $Prev)
if (Test-Path -LiteralPath $Inst) { throw ('precondition: {0} already exists on this fresh runner' -f $Inst) }
if (Test-Path $UnKey) { throw 'precondition: an uninstall key for cys already exists on this fresh runner' }

$policy = $null
$daemonPid = $null
try {
    # ── W1 / W2 / W3 ─────────────────────────────────────────────────────────────
    $w1 = Join-Path $Work 'w1\.cys-probe-w1'
    New-Item -ItemType Directory -Force -Path (Split-Path $w1) | Out-Null
    $r = Invoke-Exe $Setup ('/S /UPDATE /CYSPROBE=.cys-probe-w1 /D=' + $w1) 300
    $pr = Read-Text (Join-Path $w1 'probe-result.txt')
    Copy-Item -LiteralPath (Join-Path $w1 'probe-result.txt') -Destination (Join-Path $Ev 'w1-probe-result.txt') -ErrorAction SilentlyContinue
    $want = @('probe:cysd=pass', 'probe:cys=pass', 'probe:cys-app=pass', ('new-version:' + $V), 'result=pass')
    $got = Lines-Of $pr
    Check 'W1-worker-pass' (($r.code -eq 0) -and (($got -join "`n") -ceq ($want -join "`n"))) ('exit={0} result={1}' -f $r.code, ($got -join ' | '))
    Check 'W1-app-probe-file' ((Read-Text (Join-Path $w1 'cys-app.ok')) -ceq ('cys-app ' + $V)) 'cys-app.ok == "cys-app <V>"'
    Check 'W1-touched-nothing-outside' ((-not (Test-Path -LiteralPath $Inst)) -and (-not (Test-Path $UnKey))) 'no %LOCALAPPDATA%\cys, no uninstall key'
    # the real --version output of the extracted binaries (F3 grammar input, B10)
    $vec = [ordered]@{ version = $V }
    foreach ($b in @('cysd', 'cys')) {
        $x = Invoke-Exe (Join-Path $w1 ($b + '.exe')) '--version' 30
        $vec[$b] = [ordered]@{ code = $x.code; out = $x.out }
    }
    ($vec | ConvertTo-Json -Depth 4) | Set-Content -LiteralPath (Join-Path $Ev 'binprobe-real-output.json') -Encoding UTF8
    $cysdLine = (Lines-Of $vec['cysd'].out | Select-Object -First 1)
    Check 'W1-real-cysd-grammar' ($cysdLine -cmatch ('^cysd ' + [regex]::Escape($V) + ' \(build [!-(*-~]+\)$')) ('"{0}"' -f $cysdLine)
    $cysLine = (Lines-Of $vec['cys'].out | Select-Object -First 1)
    Check 'W1-real-cys-grammar' ($cysLine -ceq ('cys ' + $V)) ('"{0}"' -f $cysLine)

    $w2 = Join-Path $Work 'w2\elsewhere'
    $r = Invoke-Exe $Setup ('/S /UPDATE /CYSPROBE=.cys-probe-w2 /D=' + $w2) 120
    Check 'W2-worker-outside-probe-folder' (($r.code -eq 2) -and ((Count-Files $w2) -eq 0)) ('exit={0} files={1}' -f $r.code, (Count-Files $w2))

    $w3 = Join-Path $Work 'w3\.cys-probe-w3'
    $r = Invoke-Exe $Setup ('/CYSPROBE=.cys-probe-w3 /D=' + $w3) 120
    Check 'W3-worker-without-silent' (($r.code -eq 2) -and ((Count-Files $w3) -eq 0) -and (-not (Test-Path -LiteralPath $Inst))) ('exit={0} files={1}' -f $r.code, (Count-Files $w3))

    # ── M1: the probe's ABSOLUTE execution deadline, measured natively ───────────
    # 0.14.55 P4-IMPL r1 M1 (codex-3). The probe leaf used `nsExec::ExecToStack /TIMEOUT=30000`,
    # whose /TIMEOUT is an INACTIVITY timer: every write by the child resets it, so a blocked
    # candidate that prints one byte a second is never bounded by it, and in silent mode the
    # PREINSTALL path runs the leaf inline with no outer watchdog. CYS_RUN_BOUNDED replaced it
    # with CreateProcessW -> WaitForSingleObject(MS) -> TerminateProcess -> WaitForSingleObject(KILLMS)
    # on a handle the installer owns. A `/TIMEOUT=` token - or a CreateProcessW token - in the
    # source proves nothing about elapsed time, so this block MEASURES the return bound.
    #
    # It runs the SHIPPED macro body: the block is sliced out of src-tauri/nsis-hooks.nsh and
    # compiled as-is (its sha256 is recorded in the evidence). The hook cannot simply be
    # !included here - it hard-requires src-tauri\binaries\*.exe at compile time, which this job
    # does not have - so the slice is the honest equivalent, and the compile harness
    # (scripts/tests/nsis-hook-compile) is what proves the whole hook still compiles.
    #
    # Four behaviours x both call sites, 8 runs in one installer process (~5 min of deliberate
    # waiting; the job's budget is 60 min):
    #   healthy exit        -> status 0 and the child's line captured
    #   quiet hang          -> "timeout" at the deadline (the old timer also caught this one)
    #   continuous output   -> "timeout" at the SAME deadline  ★the M1 regression: the old
    #                          inactivity timer never fired here
    #   unconfirmed cleanup -> "cleanup-unconfirmed" when the kill is not confirmed in KILLMS
    # "leaf" mirrors the silent inline path (capture file in the probe directory, 30 s / 5 s);
    # "wrk" mirrors the GUI worker wrapper (capture in $PLUGINSDIR - the WORKER creates the probe
    # dir, so it does not exist at that call site - 120 s / 5 s). The unconfirmed-cleanup pair
    # uses a short deadline on purpose: the arm under test there is the KILL confirmation, and
    # the shipped deadline values are what the quiet/continuous pairs measure.
    $m1 = Join-Path $Work 'm1'
    $m1cap = Join-Path $m1 'leafcap'
    New-Item -ItemType Directory -Force -Path $m1cap | Out-Null
    # Join-Path throws on a null root under ErrorActionPreference='Stop', and ProgramFiles(x86)
    # is not guaranteed, so each root is checked before it is joined.
    $makensis = $null
    $mkCand = New-Object System.Collections.Generic.List[string]
    if ($env:MAKENSIS) { $mkCand.Add($env:MAKENSIS) }
    foreach ($pair in @(
            @(${env:ProgramFiles(x86)}, 'NSIS\makensis.exe'),
            @($env:ProgramFiles,        'NSIS\makensis.exe'),
            @($env:ProgramData,         'chocolatey\bin\makensis.exe'),
            @($env:LOCALAPPDATA,        'tauri\NSIS\makensis.exe'))) {
        if ($pair[0]) { $mkCand.Add((Join-Path $pair[0] $pair[1])) }
    }
    foreach ($c in $mkCand) {
        if (Test-Path -LiteralPath $c) { $makensis = $c; break }
    }
    if (-not $makensis) {
        $w = Get-Command makensis.exe -ErrorAction SilentlyContinue
        if ($w) { $makensis = $w.Source }
    }
    $hook = Join-Path $PSScriptRoot '..\..\..\src-tauri\nsis-hooks.nsh'
    $hookText = Read-Text $hook
    $runner = $null
    if ($hookText) {
        $hl = @($hookText -split "`r?`n")
        $st = -1
        for ($i = 0; $i -lt $hl.Count; $i++) { if ($hl[$i] -clike '!macro CYS_RUN_BOUNDED *') { $st = $i; break } }
        if ($st -ge 0) {
            for ($j = $st + 1; $j -lt $hl.Count; $j++) {
                if ($hl[$j].Trim() -ceq '!macroend') { $runner = ($hl[$st..$j] -join "`r`n"); break }
            }
        }
    }
    $sliceOk = $false
    if ($runner) {
        $sliceOk = ($runner -cmatch 'kernel32::CreateProcessW') -and
                   (([regex]::Matches($runner, 'kernel32::WaitForSingleObject')).Count -ge 2) -and
                   ($runner -cmatch 'kernel32::TerminateProcess') -and
                   ($runner -cmatch 'kernel32::GetExitCodeProcess')
        # the sha must be the sha OF THE SAVED FILE, so write the bytes that were hashed (no BOM)
        $sl = [Text.Encoding]::UTF8.GetBytes($runner)
        [IO.File]::WriteAllBytes((Join-Path $Ev 'm1-runner-slice.nsh'), $sl)
        $sh = [BitConverter]::ToString(([Security.Cryptography.SHA256]::Create()).ComputeHash($sl)).Replace('-', '')
        Say ('M1: CYS_RUN_BOUNDED slice {0} lines, sha256 {1}' -f @($runner -split "`r`n").Count, $sh)
    }
    if (-not $makensis) {
        $script:Unmeasured.Add('M1 bounded-runner cases: no makensis on this runner, so the native execution bound of CYS_RUN_BOUNDED was not measured')
    } elseif (-not $sliceOk) {
        $script:Unmeasured.Add('M1 bounded-runner cases: CYS_RUN_BOUNDED could not be sliced out of nsis-hooks.nsh (or lost its mechanism), so nothing was measured')
    } else {
        Say ('M1: makensis at ' + $makensis)
        $ps = Join-Path $env:windir 'System32\WindowsPowerShell\v1.0\powershell.exe'
        $cmdExe = Join-Path $env:windir 'System32\cmd.exe'
        # stand-ins. Each is ONE process: a cmd-with-ping form would leave the ping orphaned when
        # the cmd is terminated, and an orphan is exactly what this must not create.
        $quietPs = Join-Path $m1 'cys-m1-quiet.ps1'
        $chattyPs = Join-Path $m1 'cys-m1-chatty.ps1'
        Set-Content -LiteralPath $quietPs -Encoding ASCII -Value @'
# quiet hang: writes nothing at all, far past any deadline under test
Start-Sleep -Seconds 400
'@
        Set-Content -LiteralPath $chattyPs -Encoding ASCII -Value @'
# continuous output: keeps writing, so an INACTIVITY timer is reset forever
while ($true) { Write-Output ('cys-m1-tick ' + [DateTime]::UtcNow.ToString('o')); Start-Sleep -Milliseconds 500 }
'@
        $healthyCmd = ('"{0}" /c echo cys-m1-healthy-exit' -f $cmdExe)
        $quietCmd = ('"{0}" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{1}"' -f $ps, $quietPs)
        $chattyCmd = ('"{0}" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{1}"' -f $ps, $chattyPs)
        # ★every one of these command lines starts with a QUOTED path, which is why the macro must
        #   pass it to CreateProcessW by register: an inline `t "<cmd>"` ends at the first inner quote.
        $cases = @(
            [pscustomobject]@{ id = 'leafhealthy'; kind = 'healthy'; site = 'leaf'; ms = 30000;  kill = 5000 },
            [pscustomobject]@{ id = 'leafquiet';   kind = 'quiet';   site = 'leaf'; ms = 30000;  kill = 5000 },
            [pscustomobject]@{ id = 'leafchatty';  kind = 'chatty';  site = 'leaf'; ms = 30000;  kill = 5000 },
            [pscustomobject]@{ id = 'leafunconf';  kind = 'quiet';   site = 'leaf'; ms = 3000;   kill = 0    },
            [pscustomobject]@{ id = 'wrkhealthy';  kind = 'healthy'; site = 'wrk';  ms = 120000; kill = 5000 },
            [pscustomobject]@{ id = 'wrkquiet';    kind = 'quiet';   site = 'wrk';  ms = 120000; kill = 5000 },
            [pscustomobject]@{ id = 'wrkchatty';   kind = 'chatty';  site = 'wrk';  ms = 120000; kill = 5000 },
            [pscustomobject]@{ id = 'wrkunconf';   kind = 'quiet';   site = 'wrk';  ms = 3000;   kill = 0    }
        )
        $lines = New-Object System.Collections.Generic.List[string]
        foreach ($c in $cases) {
            $cl = $healthyCmd
            if ($c.kind -eq 'quiet') { $cl = $quietCmd }
            if ($c.kind -eq 'chatty') { $cl = $chattyCmd }
            # ★the capture location is the difference between the two call sites
            $cap = (Join-Path $m1cap ('cap-' + $c.id + '.txt'))
            if ($c.site -eq 'wrk') { $cap = '$PLUGINSDIR\cap-' + $c.id + '.txt' }
            $lines.Add(('  !insertmacro M1CASE "{0}" ''{1}'' "{2}" {3} {4}' -f $c.id, $cl, $cap, $c.ms, $c.kill))
        }
        $outExe = Join-Path $m1 'cys-m1-runner.exe'
        $resFile = Join-Path $m1 'm1-results.txt'
        Remove-Item -LiteralPath $resFile -Force -ErrorAction SilentlyContinue
        $nsi = @'
; GENERATED by run-probe-cases.ps1 (0.14.55 P4-IMPL r1 M1). The CYS_RUN_BOUNDED body below is
; sliced verbatim out of src-tauri/nsis-hooks.nsh - do not edit it here.
Unicode true
Name "cys-m1-bounded-runner"
OutFile "@@OUTEXE@@"
RequestExecutionLevel user
SilentInstall silent
ShowInstDetails nevershow

@@RUNNER@@

; one case: time the SHIPPED macro with kernel32::GetTickCount and record what it returned.
; Parameter names are deliberately distinct from CYS_RUN_BOUNDED's own.
!macro M1CASE CASEID CMDLINE CAPFILE MSX KILLX
  System::Call 'kernel32::GetTickCount() i .r7'
  !insertmacro CYS_RUN_BOUNDED '${CMDLINE}' "${CAPFILE}" ${MSX} ${KILLX} "${CASEID}"
  System::Call 'kernel32::GetTickCount() i .r8'
  IntOp $6 $8 - $7
  FileWrite $9 "### ${CASEID}$\r$\n"
  FileWrite $9 "status=$R7$\r$\n"
  FileWrite $9 "ms=$6$\r$\n"
  FileWrite $9 "out=$R8$\r$\n"
  FileWrite $9 "###end$\r$\n"
!macroend

Section "m1"
  InitPluginsDir
  ClearErrors
  FileOpen $9 "@@RESULT@@" w
  IfErrors done
@@CASES@@
  FileClose $9
done:
SectionEnd
'@
        $nsi = $nsi.Replace('@@OUTEXE@@', $outExe).Replace('@@RUNNER@@', $runner).Replace('@@RESULT@@', $resFile).Replace('@@CASES@@', ($lines -join "`r`n"))
        $nsiPath = Join-Path $m1 'cys-m1-runner.nsi'
        [IO.File]::WriteAllText($nsiPath, $nsi, (New-Object Text.UTF8Encoding $false))
        Copy-Item -LiteralPath $nsiPath -Destination (Join-Path $Ev 'm1-runner.nsi')
        # /INPUTCHARSET UTF8 mirrors the tauri bundler's own makensis call (the slice carries
        # non-ASCII comment characters). Warnings are NOT errors here on purpose: the whole hook's
        # warnings-as-errors compile is scripts/tests/nsis-hook-compile's job, not this one's.
        $mk = Invoke-Exe $makensis ('/INPUTCHARSET UTF8 "' + $nsiPath + '"') 300
        Set-Content -LiteralPath (Join-Path $Ev 'm1-makensis.log') -Value $mk.out -Encoding UTF8
        Check 'M1-runner-compiles' (($mk.code -eq 0) -and (Test-Path -LiteralPath $outExe)) ('makensis rc={0}' -f $mk.code)
        if ((Test-Path -LiteralPath $outExe)) {
            # the outer control on the inner clock: PowerShell times the whole run itself
            $sw = [Diagnostics.Stopwatch]::StartNew()
            $rr = Invoke-Exe $outExe '/S' 540
            $sw.Stop()
            $wall = [int]$sw.Elapsed.TotalMilliseconds
            Say ('M1: runner exit={0} wall={1} ms' -f $rr.code, $wall)
            # ★the liveness proof: an UNBOUNDED probe is exactly a run that never comes back
            Check 'M1-runner-returns' ($rr.code -ne 'timeout') ('exit={0} after {1} ms (540 s cap)' -f $rr.code, $wall)
            $rt = Read-Text $resFile
            if ($rt) { Set-Content -LiteralPath (Join-Path $Ev 'm1-results.txt') -Value $rt -Encoding UTF8 }
            $seen = @{}
            if ($rt) {
                foreach ($m in [regex]::Matches($rt, '(?s)###\s(?<id>\S+)\r?\nstatus=(?<st>[^\r\n]*)\r?\nms=(?<ms>\d+)\r?\nout=(?<out>.*?)\r?\n###end')) {
                    $seen[$m.Groups['id'].Value] = [pscustomobject]@{
                        status = $m.Groups['st'].Value
                        ms = [int]$m.Groups['ms'].Value
                        out = $m.Groups['out'].Value
                    }
                }
            }
            Check 'M1-all-8-cases-recorded' ($seen.Count -eq 8) ('{0} of 8 case records parsed' -f $seen.Count)
            $SLACK = 10000
            $expectWait = 0
            foreach ($c in $cases) {
                $id = $c.id
                $r = $seen[$id]
                if ($null -eq $r) { Check ('M1-' + $id) $false 'no record'; continue }
                $detail = ('status="{0}" ms={1} deadline={2} kill={3}' -f $r.status, $r.ms, $c.ms, $c.kill)
                if ($c.kill -eq 0) {
                    # the kill-confirmation arm. TerminateProcess is asynchronous, so a 0 ms
                    # confirmation wait normally expires - but "the kill landed instantly" is a
                    # legal OS outcome, not a product failure, so it is UNMEASURED, never a pass.
                    $expectWait += $c.ms
                    if ($r.status -ceq 'cleanup-unconfirmed') {
                        Check ('M1-' + $id + '-cleanup-unconfirmed') (($r.ms -ge $c.ms) -and ($r.ms -le $c.ms + $SLACK)) $detail
                    } elseif ($r.status -ceq 'timeout') {
                        $script:Unmeasured.Add(('M1 {0}: the kill was confirmed inside a 0 ms wait, so the cleanup-unconfirmed arm did not execute on this runner ({1})' -f $id, $detail))
                    } else {
                        Check ('M1-' + $id + '-cleanup-unconfirmed') $false $detail
                    }
                } elseif ($c.kind -eq 'healthy') {
                    Check ('M1-' + $id + '-healthy-exit') (($r.status -ceq '0') -and ($r.ms -lt $c.ms)) $detail
                    Check ('M1-' + $id + '-capture') ($r.out -cmatch 'cys-m1-healthy-exit') ('out="{0}"' -f $r.out.Trim())
                } else {
                    # ★quiet hang AND continuous output must be bounded by the SAME deadline.
                    $expectWait += $c.ms
                    Check ('M1-' + $id + '-bounded-at-deadline') (($r.status -ceq 'timeout') -and ($r.ms -ge $c.ms) -and ($r.ms -le $c.ms + $SLACK)) $detail
                }
            }
            # the inner clock cannot be lying low: the whole run had to absorb every deadline
            Check 'M1-wall-clock-agrees' (($wall -ge $expectWait) -and ($wall -le $expectWait + 8 * $SLACK + 60000)) ('wall={0} ms, deadlines summed={1} ms' -f $wall, $expectWait)
            # the capture is BOUNDED, not a slurp: the chatty child wrote far more than was kept
            $chattyCap = Join-Path $m1cap 'cap-leafchatty.txt'
            $capLen = 0
            if (Test-Path -LiteralPath $chattyCap) { $capLen = (Get-Item -LiteralPath $chattyCap).Length }
            $keptLen = 0
            if ($seen['leafchatty']) { $keptLen = $seen['leafchatty'].out.Length }
            Check 'M1-capture-is-bounded' (($capLen -gt 1024) -and ($keptLen -le 1024)) ('capture file {0} bytes, kept {1} chars (cap 1024)' -f $capLen, $keptLen)
            # ★and the children are actually gone - a bound that leaves the process running is no bound
            $left = @(Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" -ErrorAction SilentlyContinue |
                        Where-Object { $_.CommandLine -and $_.CommandLine -like '*cys-m1-*' })
            Check 'M1-no-surviving-stand-in' ($left.Count -eq 0) ('{0} stand-in process(es) still running' -f $left.Count)
            foreach ($l in $left) { Stop-Process -Id $l.ProcessId -Force -ErrorAction SilentlyContinue }
        }
    }

    # ── POL: AllowAll + deny-by-hash(new cysd.exe), proven effective before use ──
    $denied = Join-Path $w1 'cysd.exe'
    $allowXml = Join-Path $env:windir 'schemas\CodeIntegrity\ExamplePolicies\AllowAll.xml'
    $pd = Join-Path $Work 'policy'
    New-Item -ItemType Directory -Force -Path $pd | Out-Null
    $effDir = Join-Path $Work 'eff'
    New-Item -ItemType Directory -Force -Path $effDir | Out-Null
    Copy-Item -LiteralPath $denied -Destination (Join-Path $effDir 'cysd.exe')
    $policyOk = $false
    try {
        if (-not (Test-Path -LiteralPath $allowXml)) { throw 'AllowAll.xml example policy not found' }
        # C-09 (P4 D11): Merge-CIPolicy needs write access to its -PolicyPaths input, so passing
        # the example policy IN PLACE under %windir%\schemas\CodeIntegrity\ExamplePolicies throws
        # UnauthorizedAccessException on a hosted runner and CiTool is never reached at all
        # (measured: run 40085224043 job 120852489545, 7 of 8 checks passed, this one did not).
        # Merge from a COPY in the writable work dir - the route the diag lane already proved.
        # The Test-Path above deliberately still checks the ORIGINAL: a missing example policy is
        # a different and honest UNMEASURED, and must not be reported as a copy failure.
        $allowCopy = Join-Path $pd 'AllowAll.xml'
        try {
            Copy-Item -LiteralPath $allowXml -Destination $allowCopy -Force -ErrorAction Stop
        } catch {
            throw ('AllowAll.xml copy into the work dir failed: ' + $_.Exception.Message)
        }
        $xml = Join-Path $pd 'cys-wp12-t2.xml'
        $cip = Join-Path $pd 'cys-wp12-t2.cip'
        $rules = New-CIPolicyRule -DriverFilePath $denied -Level Hash -Deny
        Merge-CIPolicy -PolicyPaths $allowCopy -Rules $rules -OutputFilePath $xml | Out-Null
        try {
            Set-CIPolicyIdInfo -FilePath $xml -PolicyName 'CYS-WP12-T2' -ResetPolicyID | Out-Null
        } catch {
            Say ('Set-CIPolicyIdInfo combined call failed, two calls: ' + $_.Exception.Message)
            Set-CIPolicyIdInfo -FilePath $xml -ResetPolicyID | Out-Null
            Set-CIPolicyIdInfo -FilePath $xml -PolicyName 'CYS-WP12-T2' | Out-Null
        }
        foreach ($opt in @(@{ o = 3; del = $true }, @{ o = 0; del = $false }, @{ o = 6; del = $false }, @{ o = 16; del = $false })) {
            try {
                if ($opt.del) { Set-RuleOption -FilePath $xml -Option $opt.o -Delete } else { Set-RuleOption -FilePath $xml -Option $opt.o }
            } catch { Say ('Set-RuleOption {0}: {1}' -f $opt.o, $_.Exception.Message) }
        }
        $id = ([xml](Get-Content -Raw -LiteralPath $xml)).SiPolicy.PolicyID
        ConvertFrom-CIPolicy -XmlFilePath $xml -BinaryFilePath $cip | Out-Null
        Copy-Item -LiteralPath $xml -Destination (Join-Path $Ev 'policy.xml')
        $ct = Invoke-Exe $CiTool ('--update-policy "' + $cip + '"') 120
        Say ('CiTool --update-policy started={0} rc={1}: {2} {3}' -f $ct.started, $ct.code, $ct.out, $ct.error)
        if ($ct.code -ne 0) { throw ('CiTool --update-policy failed rc=' + $ct.code) }
        $policy = $id
        # effective = the denied copy does not start AND an allowed binary still does
        for ($i = 0; $i -lt 10 -and -not $policyOk; $i++) {
            $d = Invoke-Exe (Join-Path $effDir 'cysd.exe') '--version' 30
            $a = Invoke-Exe (Join-Path $w1 'cys.exe') '--version' 30
            $blocked = (-not $d.started) -or ($d.code -ne 0 -and $d.out -notmatch '^cysd ')
            $allowed = $a.started -and $a.code -eq 0
            Say ('policy check {0}: denied copy started={1} code={2} err="{3}"; allowed cys.exe code={4}' -f $i, $d.started, $d.code, $d.error, $a.code)
            if ($blocked -and $allowed) { $policyOk = $true } else { Start-Sleep -Seconds 2 }
        }
    } catch {
        Say ('policy setup failed: ' + $_.Exception.Message)
    }
    Check 'POL-deny-hash-effective' $policyOk 'denied cysd.exe copy cannot start; an allowed binary runs'
    if (-not $policyOk) {
        $script:Unmeasured.Add('T2-F and T2: the deny-by-hash App Control policy could not be shown effective on this runner (U5)')
    } else {
        # ── T2-F: first install refused ──────────────────────────────────────
        $rec = Join-Path $env:TEMP 'cys-install-blocked.txt'
        Remove-Item -LiteralPath $rec -Force -ErrorAction SilentlyContinue
        $r = Invoke-Exe $Setup '/S' 600
        $t = Read-Text $rec
        if ($t) { Set-Content -LiteralPath (Join-Path $Ev 't2f-record.txt') -Value $t }
        $l = Lines-Of $t
        Check 'T2F-exit-6' ($r.code -eq 6) ('exit={0}' -f $r.code)
        Check 'T2F-nothing-installed' ((-not (Test-Path -LiteralPath $Inst)) -and (-not (Test-Path $UnKey))) ('{0} absent, no uninstall key' -f $Inst)
        Check 'T2F-record' (($l -contains 'kept-version:none') -and ($l -contains 'handoff:not-needed') -and (@($l | Where-Object { $_ -like 'blocked:*cysd*' }).Count -eq 1) -and ($l -notcontains 'probe:cysd=pass')) ($l -join ' | ')

        # ── T2: silent /UPDATE refused over the previous release, daemon alive ──
        $r = Invoke-Exe $Prev '/S' 600
        Check 'T2-previous-release-installed' ($r.code -eq 0 -and (Test-Path -LiteralPath (Join-Path $Inst 'cysd.exe'))) ('exit={0}' -f $r.code)
        Start-Process -FilePath (Join-Path $Inst 'cysd.exe') -WindowStyle Hidden
        $up = $null
        for ($i = 0; $i -lt 20; $i++) { Start-Sleep -Seconds 1; $up = Invoke-Exe (Join-Path $Inst 'cys.exe') 'list' 15; if ($up.code -eq 0) { break } }
        $dp = @(Get-Process -Name cysd -ErrorAction SilentlyContinue)
        if ($dp.Count -eq 1) { $daemonPid = $dp[0].Id }
        Check 'T2-daemon-up' ($up.code -eq 0 -and $null -ne $daemonPid) ('list code={0} pid={1}' -f $up.code, $daemonPid)
        $kept = (Read-Text (Join-Path $Inst 'cys-installed-version.txt'))
        if ($kept) { $kept = (Lines-Of $kept | Select-Object -First 1) }
        $before = Get-InstallerOwnedState $Inst
        $r = Invoke-Exe $Setup '/S /UPDATE' 600
        $after = Get-InstallerOwnedState $Inst
        $t = Read-Text (Join-Path $Inst 'cys-update-blocked.txt')
        if ($t) { Set-Content -LiteralPath (Join-Path $Ev 't2-record.txt') -Value $t }
        $l = Lines-Of $t
        $diff = Compare-State $before $after
        Check 'T2-exit-6' ($r.code -eq 6) ('exit={0}' -f $r.code)
        Check 'T2-installer-owned-state-unchanged' ($diff.Count -eq 0) ('{0} entries compared; diff: {1}' -f $before.Count, ($diff -join ', '))
        $alive = $null -ne (Get-Process -Id $daemonPid -ErrorAction SilentlyContinue)
        $up = Invoke-Exe (Join-Path $Inst 'cys.exe') 'list' 15
        Check 'T2-daemon-alive' ($alive -and $up.code -eq 0) ('pid {0} alive={1} list code={2}' -f $daemonPid, $alive, $up.code)
        Check 'T2-record' (($l -contains ('kept-version:' + $kept)) -and ($l -contains 'handoff:not-needed') -and ($l -contains 'cleanup:ok') -and (@($l | Where-Object { $_ -like 'blocked:*cysd*' }).Count -eq 1)) ($l -join ' | ')
        Check 'T2-no-probe-folder' ((Probe-Dirs $Inst).Count -eq 0) ('left: {0}' -f ((Probe-Dirs $Inst) -join ','))
    }
} finally {
    if ($policy) {
        $ct = Invoke-Exe $CiTool ('--remove-policy "' + $policy + '"') 120
        Say ('CiTool --remove-policy {0} rc={1}: {2} {3}' -f $policy, $ct.code, $ct.out, $ct.error)
        $ct = Invoke-Exe $CiTool '--refresh' 120
        Say ('CiTool --refresh rc={0}' -f $ct.code)
    }
    try {
        Get-WinEvent -LogName 'Microsoft-Windows-CodeIntegrity/Operational' -MaxEvents 200 -ErrorAction Stop |
            Where-Object { $_.Id -in 3033, 3076, 3077, 3099 } | Select-Object -First 40 TimeCreated, Id, Message |
            Format-List | Out-String -Width 400 | Set-Content -LiteralPath (Join-Path $Ev 'codeintegrity-events.txt')
    } catch { Say ('CodeIntegrity events: ' + $_.Exception.Message) }
}

# ── controls without the policy (the runs that must differ) ───────────────────
if ($policy) {
    $lifted = $false
    for ($i = 0; $i -lt 10 -and -not $lifted; $i++) {
        $d = Invoke-Exe (Join-Path (Join-Path $Work 'eff') 'cysd.exe') '--version' 30
        if ($d.started -and $d.code -eq 0) { $lifted = $true } else { Start-Sleep -Seconds 2 }
    }
    Check 'POL-removed' $lifted 'the formerly denied copy runs again'
    if ($null -ne $daemonPid) {
        $r = Invoke-Exe $Setup '/S /UPDATE' 600
        $mk = Read-Text (Join-Path $Inst 'cys-installed-version.txt')
        if ($mk) { $mk = (Lines-Of $mk | Select-Object -First 1) }
        # the new build and the previous release can carry the same version text; prove the install by bytes
        $newBytes = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $Work 'w1\.cys-probe-w1\cysd.exe')).Hash
        $instBytes = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $Inst 'cysd.exe')).Hash
        Check 'CTL-T2-update-installs' ($r.code -eq 0 -and $mk -ceq $V -and $instBytes -eq $newBytes) ('exit={0} marker={1} cysd.exe is the new build={2}' -f $r.code, $mk, ($instBytes -eq $newBytes))
        Check 'CTL-T2-daemon-survives' ($null -ne (Get-Process -Id $daemonPid -ErrorAction SilentlyContinue)) ('pid {0}' -f $daemonPid)
        Check 'CTL-T2-stale-record-removed' (-not (Test-Path -LiteralPath (Join-Path $Inst 'cys-update-blocked.txt'))) 'cys-update-blocked.txt deleted on success'
        Check 'CTL-T2-no-probe-folder' ((Probe-Dirs $Inst).Count -eq 0) ('left: {0}' -f ((Probe-Dirs $Inst) -join ','))
    }
    $fresh = Join-Path $Work 'fresh-control'
    $r = Invoke-Exe $Setup ('/S /D=' + $fresh) 600
    $ok = $false
    if (Test-Path -LiteralPath (Join-Path $fresh 'cysd.exe')) {
        $ok = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $fresh 'cysd.exe')).Hash -eq (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $Work 'w1\.cys-probe-w1\cysd.exe')).Hash
    }
    Check 'CTL-T2F-first-install-installs' ($r.code -eq 0 -and $ok -and (Probe-Dirs $fresh).Count -eq 0) ('exit={0} cysd.exe is the new build={1}' -f $r.code, $ok)
}
Get-Process -Name cysd, cys, cys-app -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

$fails = @($script:Results | Where-Object { $_.result -ne 'PASS' })
$summary = [ordered]@{
    version = $V
    results = $script:Results
    unmeasured = $script:Unmeasured
    not_run = @('T2-U wizard uninstall-first', 'T2-D delayed initiator hand-off', 'T2-G dialog text', 'T2-P passive') # need UI Automation
    pass = ($fails.Count -eq 0 -and $script:Unmeasured.Count -eq 0)
}
($summary | ConvertTo-Json -Depth 5) | Set-Content -LiteralPath (Join-Path $Ev 'probe-cases-result.json') -Encoding UTF8
foreach ($u in $script:Unmeasured) { Say ('UNMEASURED ' + $u) }
Say ('SUMMARY {0} checks, {1} failed, {2} unmeasured' -f $script:Results.Count, $fails.Count, $script:Unmeasured.Count)
if ($fails.Count -gt 0 -or $script:Unmeasured.Count -gt 0) { exit 1 }
exit 0
