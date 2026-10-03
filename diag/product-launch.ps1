# diag/product-launch.ps1 -- the PRODUCT's installer-launch module (src/update_launch.rs, copied byte for byte to
# diag/product/update_launch.rs) on REAL Smart App Control (SAC), windows-11-arm job only.
#   build    diag/product/main.rs (a driver that includes update_launch.rs unchanged) with rustc: x64 (the verdict: the app is x64)
#            and aarch64 (host, auxiliary). Done BEFORE SAC is turned on.
#   start    both drivers BEFORE SAC is turned on (a new unsigned exe could not start afterwards). Each does its OFF stage at once:
#            write_installer + launch_installer(copy of the signed cmd.exe, "/c exit 0") -> expected Ok, then writes "ready" and waits for "go".
#   SAC on   registry value 1 + CiTool -r, 3 states after 10 s (not enforced -> measurable=false, the run ends after the restore)
#   go       each driver does its ON stage: the REAL 0.14.42 installer through write_installer + launch_installer(nsis_update_params(&[]))
#            -> expected Err os_code 4551 / shell_ret 5 / AppControl, then remove_installer (file and folder must be gone), then the control:
#            the signed cmd.exe copy must still launch
#   finally  RESTORE (value 0 + CiTool -r + re-check + the NEG control runs again), CodeIntegrity events (3077 naming the updater path
#            are marked), summary product-launch-summary.txt (10 lines at most) with a PASS / FAIL verdict and its reasons
# ONE PowerShell process; everything that needs Add-Type / new unsigned exes happens BEFORE SAC is turned on. Partial results are written
# to OUT after every step. OBSERVATION ONLY: no SAC / Defender bypass. The shared SAC switch helpers live in sac-lib.ps1.
$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')
. (Join-Path $PSScriptRoot 'sac-lib.ps1')
Start-DiagScript -Name 'product-launch'

$K_CITOOL = Join-Path $env:windir 'System32\CiTool.exe'
$K_REG = Join-Path $env:windir 'System32\reg.exe'
$K_KEY = 'HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy'
$K_VAL = 'VerifiedAndReputablePolicyState'
$K_PFX = 'product'
$K_JOB = 'product-launch'
$K_NEG = 'control NEG (fresh unsigned exe)'
$K_INSTALLER_VERSION = '0.14.42'
# sha256 of diag/product/update_launch.rs = src/update_launch.rs of the product commit aa0b1adb (the file must arrive byte for byte)
$K_UL_SHA256 = '570476c28df10e9280036cee4da37301175d2fd215a49f5cb16627fd00e88ddc'
$K_READY_WAIT_SEC = 120
$K_DONE_WAIT_SEC = 180

$RUN = [ordered]@{
    started = (Get-IsoNow)
    matrix_os = $env:DIAG_MATRIX_OS
    measurable = $null
    sac_initially_enforced = $null
    sac_on_confirmed = $null
    sac_on_utc = $null
    restore_ok = $null
    verdict = $null
    steps = [ordered]@{}
    states = [ordered]@{}
    checkpoints = [ordered]@{}
    notes = (New-Object System.Collections.Generic.List[string])
    drivers = [ordered]@{}
}
$CTX = @{}
$CTX['subjects'] = @()
$CTX['driver_node'] = ''
$CTX['t_sac_on'] = $null
$CTX['t_restore_start'] = $null
$CTX['cmd_path'] = Join-Path $env:windir 'System32\cmd.exe'
$CTX['installer_path'] = ''
$CTX['build'] = $null
# Process objects must never go into $RUN (it is serialized): they are kept here, by architecture name
$CTX['procs'] = @{}
$CTX['work'] = @{}
# the registry value is 'touched' from the moment we try to write 1 until a restore was verified
$script:SacTouched = $false
# GITHUB_TOKEN is only for the checkpoint upload: keep it in a variable and take it out of the process environment so that no child
# process (the drivers, the installer if it were ever started) inherits it; it is handed to node for the upload only.
$script:GhToken = [string]$env:GITHUB_TOKEN
Remove-Item -Path 'Env:\GITHUB_TOKEN' -ErrorAction SilentlyContinue

function Save-Run { Save-Json 'product-launch.json' $RUN 9 }

# =========================================================================
# build: diag/product/main.rs -> sac-launch-x64.exe (the verdict) and sac-launch-aarch64.exe (host, auxiliary)
# =========================================================================
function Build-ProductDrivers {
    $b = [ordered]@{ started = (Get-IsoNow); rustc = $null; rustc_version = $null; host_triple = $null; variants = [ordered]@{}; log = (New-Object System.Collections.Generic.List[object]); finished = $null }
    try {
        $srcDir = Join-Path $global:DiagRoot 'product'
        $mainRs = Join-Path $srcDir 'main.rs'
        $binDir = Join-Path $global:DiagWork 'product\bin'
        New-Item -ItemType Directory -Path $binDir -Force | Out-Null
        $rustc = Find-Exe 'rustc.exe'
        if (-not $rustc) {
            $b['log'].Add('rustc.exe not found on PATH: no driver can be built')
            $b['finished'] = (Get-IsoNow)
            return $b
        }
        $b['rustc'] = $rustc
        $v = Invoke-Proc -File $rustc -Arguments '-vV' -TimeoutSec 60
        $b['rustc_version'] = ([string]$v.out).Trim()
        $hostTriple = ''
        $m = [regex]::Match([string]$v.out, '(?m)^host:\s*(\S+)')
        if ($m.Success) { $hostTriple = $m.Groups[1].Value }
        $b['host_triple'] = $hostTriple
        $variants = New-Object System.Collections.Generic.List[object]
        # x64 first: it is the verdict (the product is an x64 app); the host build (aarch64 on this runner) is auxiliary
        $x64Target = ''
        if ($hostTriple -ne 'x86_64-pc-windows-msvc') { $x64Target = 'x86_64-pc-windows-msvc' }
        $variants.Add([ordered]@{ arch = 'x64'; target = $x64Target; role = 'verdict' })
        if ($hostTriple -match '^aarch64') { $variants.Add([ordered]@{ arch = 'aarch64'; target = ''; role = 'auxiliary (host)' }) }
        if ($x64Target) {
            $rustup = Find-Exe 'rustup.exe'
            if ($rustup) {
                $ru = Invoke-Proc -File $rustup -Arguments ('target add ' + $x64Target) -TimeoutSec 300
                $b['log'].Add([ordered]@{ step = ('rustup target add ' + $x64Target); rc = $ru.rc; out = (Limit-Text $ru.out 1500); err = (Limit-Text $ru.err 1500); startError = $ru.startError })
            } else {
                $b['log'].Add('rustup.exe not found: the x64 target could not be added (the build may still work when rust-std for it is installed)')
            }
        }
        foreach ($var in $variants.ToArray()) {
            $outExe = Join-Path $binDir ('sac-launch-' + $var['arch'] + '.exe')
            $entry = [ordered]@{ arch = $var['arch']; role = $var['role']; target = $var['target']; ok = $false; exe = $outExe; rc = $null; start_error = $null; size = $null; sha256 = $null; err_tail = $null; reason = $null }
            $ra = '--edition 2021 -O -C target-feature=+crt-static'
            if ($var['target']) { $ra = $ra + ' --target ' + $var['target'] }
            $ra = $ra + (' -o "{0}" "{1}"' -f $outExe, $mainRs)
            $rr = Invoke-Proc -File $rustc -Arguments $ra -TimeoutSec 300
            $entry['rc'] = $rr.rc
            $entry['start_error'] = $rr.startError
            $entry['err_tail'] = Limit-Text (([string]$rr.err).Trim()) 3000
            if (Test-Path -LiteralPath $outExe) {
                $entry['ok'] = $true
                $entry['size'] = (Get-Item -LiteralPath $outExe).Length
                $entry['sha256'] = Get-Sha256 $outExe
            } else {
                $entry['reason'] = ('rustc rc={0}; {1}' -f $rr.rc, (Limit-Text (([string]$rr.err).Trim()) 400))
            }
            $b['variants'][$var['arch']] = $entry
        }
    } catch {
        $b['log'].Add('Build-ProductDrivers exception: ' + $_.Exception.Message)
    }
    $b['finished'] = (Get-IsoNow)
    return $b
}

# =========================================================================
# drivers: start before SAC, wait for the signal files, read the result lines
# =========================================================================
function Wait-DriverFile {
    param([string]$Work, [string]$Name, [int]$TimeoutSec, $Proc)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $TimeoutSec) {
        if (Test-Path -LiteralPath (Join-Path $Work $Name)) { return $true }
        $gone = $false
        try { if ($null -ne $Proc) { $gone = $Proc.HasExited } } catch { }
        if ($gone) {
            # the driver ended: one last look (it may have written the file just before it exited)
            Start-Sleep -Milliseconds 400
            return [bool](Test-Path -LiteralPath (Join-Path $Work $Name))
        }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Start-ProductDriver {
    param($Variant)
    $arch = [string]$Variant['arch']
    $rec = [ordered]@{ arch = $arch; role = [string]$Variant['role']; exe = [string]$Variant['exe']; work_dir = $null; pid = $null; started = $null; start_error = $null; stdout_file = $null; stderr_file = $null; ready = $null; ready_after_sec = $null }
    try {
        # %TEMP% like the app's own temp root: the installer folder ends up at %TEMP%\diag-product-launch\<arch>\cys-0.14.42-updater-<6>\
        $work = Join-Path ([System.IO.Path]::GetTempPath()) ('diag-product-launch\' + $arch)
        if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }
        New-Item -ItemType Directory -Path $work -Force | Out-Null
        $rec['work_dir'] = $work
        $CTX['work'][$arch] = $work
        $so = Join-Path $global:DiagWork ('product\driver-' + $arch + '-stdout.txt')
        $se = Join-Path $global:DiagWork ('product\driver-' + $arch + '-stderr.txt')
        $rec['stdout_file'] = $so
        $rec['stderr_file'] = $se
        $argText = '{0} {1} {2}' -f (ConvertTo-CmdArg $work), (ConvertTo-CmdArg ([string]$CTX['cmd_path'])), (ConvertTo-CmdArg ([string]$CTX['installer_path']))
        $rec['started'] = (Get-IsoNow)
        $p = Start-Process -FilePath ([string]$Variant['exe']) -ArgumentList $argText -NoNewWindow -PassThru -RedirectStandardOutput $so -RedirectStandardError $se -ErrorAction Stop
        try { $null = $p.Handle } catch { }
        $rec['pid'] = $p.Id
        $CTX['procs'][$arch] = $p
    } catch {
        $rec['start_error'] = Format-ErrorText $_
    }
    return $rec
}

# one parsed object per jsonl line (a line that does not parse is kept as { unparsable = <text> })
function Read-DriverLog {
    param([string]$Path)
    $items = New-Object System.Collections.Generic.List[object]
    try {
        $txt = Read-FileShared $Path
        foreach ($ln in ($txt -split "`n")) {
            $t = $ln.Trim()
            if ($t.Length -eq 0) { continue }
            try { $items.Add((ConvertFrom-Json $t)) } catch { $items.Add([pscustomobject]@{ unparsable = $t }) }
        }
    } catch { }
    return $items.ToArray()
}

function Find-DriverStep {
    param($Lines, [string]$Stage, [string]$Step)
    $hit = $null
    foreach ($l in @($Lines)) {
        if (([string]$l.stage -eq $Stage) -and ([string]$l.step -eq $Step)) { $hit = $l }
    }
    return $hit
}

function Test-ShapeOk {
    param($WriteStep)
    if ($null -eq $WriteStep) { return $false }
    return [bool](($WriteStep.ok -eq $true) -and ($WriteStep.dir_prefix_ok -eq $true) -and ($WriteStep.dir_suffix_alnum6 -eq $true) -and ($WriteStep.file_name_ok -eq $true) -and ($WriteStep.parent_of_dir_is_work_dir -eq $true))
}

# the OFF stage alone (what the driver had written when it said "ready"): it is kept in OUT BEFORE SAC is turned on
function Get-OffResult {
    param($Lines)
    $w = Find-DriverStep $Lines 'off' 'write_cmd_copy'
    $l = Find-DriverStep $Lines 'off' 'launch_cmd_copy'
    $o = [ordered]@{
        lines = @($Lines).Count
        write_ok = [bool](($null -ne $w) -and ($w.ok -eq $true))
        path_shape_ok = (Test-ShapeOk $w)
        path = $null
        launch_ok = [bool](($null -ne $l) -and ($l.ok -eq $true))
        launch_ms = $null
        launch_slow = $false
    }
    if ($null -ne $w) { $o['path'] = $w.path }
    if ($null -ne $l) { $o['launch_ms'] = $l.ms; $o['launch_slow'] = [bool]($l.slow -eq $true) }
    return $o
}

# what the driver recorded -> flags (the driver writes the real return values; the judging is done here)
function Get-DriverVerdict {
    param($Lines)
    $offWrite = Find-DriverStep $Lines 'off' 'write_cmd_copy'
    $offLaunch = Find-DriverStep $Lines 'off' 'launch_cmd_copy'
    $onWrite = Find-DriverStep $Lines 'on' 'write_real_installer'
    $onLaunch = Find-DriverStep $Lines 'on' 'launch_real_installer'
    $onRemove = Find-DriverStep $Lines 'on' 'remove_installer'
    $onControl = Find-DriverStep $Lines 'on' 'launch_cmd_copy'
    $stageDone = Find-DriverStep $Lines 'on' 'stage_done'
    $startLine = Find-DriverStep $Lines 'start' 'start'
    $v = [ordered]@{
        arch_reported_by_driver = $null
        off_launch_ok = [bool](($null -ne $offLaunch) -and ($offLaunch.ok -eq $true))
        off_launch_ms = $null
        on_block_ok = $false
        on_block_detail = $null
        on_launch_ms = $null
        cleanup_ok = [bool](($null -ne $onRemove) -and ($onRemove.cleanup_ok -eq $true))
        cleanup_settled_ok = [bool](($null -ne $onRemove) -and ($onRemove.cleanup_ok_settled -eq $true))
        cleanup_settle_ms = $null
        cleanup_ran = [bool](($null -ne $onRemove) -and ($null -ne $onRemove.PSObject.Properties['cleanup_ok']))
        cleanup_file_left = $null
        cleanup_dir_left = $null
        control_ok = [bool](($null -ne $onControl) -and ($onControl.ok -eq $true))
        control_ms = $null
        control_original_ok = $null
        on_launch_timed_out = $false
        any_launch_timed_out = [bool](($null -ne $stageDone) -and ($stageDone.launch_timed_out -eq $true))
        path_shape_ok = [bool]((Test-ShapeOk $offWrite) -and (Test-ShapeOk $onWrite))
        stage_done = [bool]($null -ne $stageDone)
        alive_through_on_stage = [bool](($null -ne $stageDone) -and ($stageDone.driver_alive_through_on_stage -eq $true))
        slow_calls = @()
        attr_temporary_on_installer = $null
    }
    if ($null -ne $startLine) { $v['arch_reported_by_driver'] = $startLine.arch }
    if ($null -ne $offLaunch) { $v['off_launch_ms'] = $offLaunch.ms }
    if ($null -ne $onControl) { $v['control_ms'] = $onControl.ms }
    if ($null -ne $onWrite) { $v['attr_temporary_on_installer'] = $onWrite.attr_temporary }
    if ($null -ne $onRemove) { $v['cleanup_settle_ms'] = $onRemove.settle_ms }
    if ($v['cleanup_ran']) {
        $v['cleanup_file_left'] = [bool]($onRemove.file_exists_settled -eq $true)
        $v['cleanup_dir_left'] = [bool]($onRemove.dir_exists_settled -eq $true)
    }
    $origStep = Find-DriverStep $Lines 'on' 'launch_cmd_original'
    if ($null -ne $origStep) { $v['control_original_ok'] = [bool]($origStep.ok -eq $true) }
    if ($null -ne $onLaunch) {
        $v['on_launch_ms'] = $onLaunch.ms
        $v['on_block_detail'] = [ordered]@{ ok = $onLaunch.ok; shell_ret = $onLaunch.shell_ret; os_code = $onLaunch.os_code; block = $onLaunch.block; display = $onLaunch.display; timed_out = $onLaunch.timed_out; note = $onLaunch.note }
        $v['on_launch_timed_out'] = [bool]($onLaunch.timed_out -eq $true)
        $v['on_block_ok'] = [bool](($onLaunch.ok -eq $false) -and ([string]$onLaunch.os_code -eq '4551') -and ([string]$onLaunch.shell_ret -eq '5') -and ([string]$onLaunch.block -eq 'AppControl') -and ([string]$onLaunch.display -eq 'installer_launch_failed:4551:5'))
    }
    $slow = New-Object System.Collections.Generic.List[string]
    foreach ($l in @($Lines)) {
        try { if ($l.slow -eq $true) { $slow.Add(([string]$l.stage + '/' + [string]$l.step + ' ' + [string]$l.ms + ' ms')) } } catch { }
    }
    $v['slow_calls'] = $slow.ToArray()
    return $v
}

# processes whose image lies under a diag-product-launch work folder, whichever spelling of the temp path (8.3 short or long) the process list uses
function Stop-ProductWorkProcesses {
    $killed = New-Object System.Collections.Generic.List[string]
    try {
        $all = Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop
        foreach ($wp in $all) {
            $ep = [string]$wp.ExecutablePath
            if ($ep -and ($ep.IndexOf('\diag-product-launch\', [System.StringComparison]::OrdinalIgnoreCase) -ge 0) -and ([int]$wp.ProcessId -ne $PID)) {
                try {
                    Stop-Process -Id ([int]$wp.ProcessId) -Force -ErrorAction Stop
                    $killed.Add(('{0}#{1}' -f $wp.Name, $wp.ProcessId))
                } catch {
                    $killed.Add(('{0}#{1} FAILED {2}' -f $wp.Name, $wp.ProcessId, $_.Exception.Message))
                }
            }
        }
    } catch {
        $killed.Add('process list failed: ' + $_.Exception.Message)
    }
    return $killed.ToArray()
}

# =========================================================================
# SAC on (the matrix of sacreal-e2e is not part of this job)
# =========================================================================
function Invoke-SacOn {
    $ph = [ordered]@{ started = (Get-IsoNow); language_mode = (Get-LangMode); set = $null; after_10s = $null; after_30s = $null; confirmed = $false }
    $RUN['sac_on'] = $ph
    $script:SacTouched = $true
    $tOn = (Get-Date).ToUniversalTime()
    $CTX['t_sac_on'] = $tOn
    $RUN['sac_on_utc'] = ConvertTo-IsoUtc $tOn
    Save-Run
    $ph['set'] = Set-SacValue 1 'on'
    Start-Sleep -Seconds 10
    $chk = Get-SacRealState 'after-on-10s'
    $ph['after_10s'] = [ordered]@{ tag = 'after-on-10s'; enforced = $chk['enforced']; signals = $chk['signals'] }
    if (-not $chk['enforced']) {
        Write-Log 'SAC is not enforced 10 s after the switch: waiting 20 s and checking once more'
        Start-Sleep -Seconds 20
        $chk = Get-SacRealState 'after-on-30s'
        $ph['after_30s'] = [ordered]@{ tag = 'after-on-30s'; enforced = $chk['enforced']; signals = $chk['signals'] }
    }
    $ph['confirmed'] = [bool]$chk['enforced']
    $RUN['sac_on_confirmed'] = $ph['confirmed']
    if (-not $ph['confirmed']) {
        $RUN['measurable'] = $false
        $RUN['notes'].Add('REAL Smart App Control could not be confirmed ON (states: product-state-after-on-*.json): measurable=false, the drivers get no go signal')
    } else {
        $RUN['measurable'] = $true
    }
    Save-Run
}

# =========================================================================
# events of the SAC window (read AFTER the restore from the persisted log; the part from the start of the restore on is left out)
# =========================================================================
function Save-ProductEvents {
    if (-not $CTX['t_sac_on']) { return }
    if ((Get-MinutesLeft) -lt 2) {
        $RUN['notes'].Add('CodeIntegrity event collection skipped: job time budget nearly exhausted')
        return
    }
    Start-Sleep -Seconds 2
    $fe = [ordered]@{ started = (Get-IsoNow); evtx_rc = $null; evtx_note = 'product-ci.evtx holds everything since SAC was switched on, including the policy refresh events of the restore'; note = $null; counts_by_id = $null; events_from_restore_start_left_out = 0; installer_path_event_count = 0; installer_path_events = @(); finished = $null }
    $RUN['events'] = $fe
    $ex = Export-CIEvtx -SinceUtc $CTX['t_sac_on'] -FileName 'product-ci.evtx'
    $fe['evtx_rc'] = $ex.rc
    $evAll = Get-CIEvents -SinceUtc $CTX['t_sac_on']
    $cut = $null
    if ($CTX['t_restore_start']) { $cut = ConvertTo-IsoUtc $CTX['t_restore_start'] }
    $inWindow = New-Object System.Collections.Generic.List[object]
    $afterRestoreStart = 0
    foreach ($e in @($evAll.events)) {
        if (($null -ne $cut) -and ([string]::CompareOrdinal([string]$e.time, [string]$cut) -ge 0)) { $afterRestoreStart++ } else { $inWindow.Add($e) }
    }
    $fe['events_from_restore_start_left_out'] = $afterRestoreStart
    $ev = [pscustomobject]@{ events = $inWindow.ToArray(); note = $evAll.note }
    Save-EventsJson $ev 'product-events.json'
    $fe['note'] = $ev.note
    $fe['counts_by_id'] = Get-IdCounts $ev.events
    $bo = Save-BlockEvents $ev.events 'product-3077.json'
    # the events about the installer path the product module wrote (marked), with the process that was blocked
    $hits = New-Object System.Collections.Generic.List[object]
    try {
        foreach ($b in @($bo['block_events_3076_3077'])) {
            $pj = (@($b['file_paths']) -join ' ')
            if ($pj -match 'cys-0\.14\.42-updater-') {
                $hits.Add([ordered]@{ id = $b['id']; time = $b['time']; marked = 'path contains cys-0.14.42-updater-'; policy_names = $b['policy_names']; file_paths = $b['file_paths']; process_names = $b['process_names']; status = $b['status'] })
            }
        }
    } catch {
        $fe['error'] = 'marking the installer-path events failed: ' + $_.Exception.Message
    }
    $fe['installer_path_event_count'] = $hits.Count
    $fe['installer_path_events'] = $hits.ToArray()
    $fe['finished'] = (Get-IsoNow)
}

# =========================================================================
# summary: 10 lines at most
# =========================================================================
function Format-ArchPair {
    param([scriptblock]$Fn)
    $parts = New-Object System.Collections.Generic.List[string]
    foreach ($arch in @('x64', 'aarch64')) {
        if ($RUN['drivers'].Contains($arch)) {
            $d = $RUN['drivers'][$arch]
            $txt = 'n/a'
            try { $txt = [string](& $Fn $d) } catch { $txt = 'n/a' }
            $parts.Add(($arch + ' ' + $txt))
        }
    }
    if ($parts.Count -eq 0) { return 'no driver' }
    return ($parts.ToArray() -join '; ')
}

function Write-ProductSummary {
    $lines = New-Object System.Collections.Generic.List[string]
    # every line is built on its own: one failing line must not cost the whole file
    try {
        $pre = $RUN['prepare']
        $ulText = 'n/a'
        $mainText = 'n/a'
        $ulMatch = $null
        if ($pre -is [System.Collections.IDictionary]) {
            if ($pre['update_launch_rs'] -is [System.Collections.IDictionary]) { $ulText = [string]$pre['update_launch_rs']['sha256']; $ulMatch = $pre['update_launch_rs']['matches_expected'] }
            if ($pre['main_rs'] -is [System.Collections.IDictionary]) { $mainText = [string]$pre['main_rs']['sha256'] }
        }
        $buildText = 'not built'
        $bld = $CTX['build']
        if ($bld -is [System.Collections.IDictionary]) {
            $bp = New-Object System.Collections.Generic.List[string]
            foreach ($arch in @('x64', 'aarch64')) {
                if ($bld['variants'].Contains($arch)) {
                    $en = $bld['variants'][$arch]
                    if ($en['ok']) { $bp.Add($arch + '=ok') } else { $bp.Add($arch + '=FAILED (' + (Limit-Text $en['reason'] 120) + ')') }
                }
            }
            if ($bp.Count -gt 0) { $buildText = ($bp.ToArray() -join ', ') } elseif ($bld['log'].Count -gt 0) { $buildText = (Limit-Text ([string]($bld['log'].ToArray()[0])) 160) }
        }
        $lines.Add(('product-launch on {0}: measurable={1}; update_launch.rs sha256={2} (as expected={3}); main.rs sha256={4}; drivers built: {5}' -f $env:DIAG_MATRIX_OS, $RUN['measurable'], $ulText, $ulMatch, $mainText, $buildText))
    } catch { $lines.Add('line 1 (run, product file, build) could not be built: ' + $_.Exception.Message) }
    try {
        $onState = $null
        foreach ($tag in @('after-on-30s', 'after-on-10s')) {
            if ($null -eq $onState) { if ($RUN['states'].Contains($tag)) { $onState = $RUN['states'][$tag] } }
        }
        if ($null -ne $onState) {
            $dp = $onState['citool']['desktop_policy']
            $dpEnforced = $null
            if ($null -ne $dp) { $dpEnforced = $dp['is_enforced'] }
            $mpText = $onState['signals']['defender_on']
            if ($onState['defender'] -is [System.Collections.IDictionary]) { $mpText = $onState['defender']['SmartAppControlState'] }
            $lines.Add(('SAC on confirmed={0}: registry value={1}; CiTool VerifiedAndReputableDesktop IsEnforced={2}; Defender SmartAppControlState={3}; UMCI flag={4}' -f $RUN['sac_on_confirmed'], $onState['registry']['value'], $dpEnforced, $mpText, $onState['signals']['umci_flag']))
        } else {
            $lines.Add('SAC on: the switch was never made (see product-launch.json notes / steps)')
        }
    } catch { $lines.Add('line 2 (SAC on) could not be built: ' + $_.Exception.Message) }
    try {
        $lines.Add('OFF stage, SAC off, launch of the signed cmd.exe copy through write_installer + launch_installer (expect Ok): ' + (Format-ArchPair { param($d) $x = $d['off_result']; if ($null -eq $x) { 'no result' } else { ('ok={0} ({1} ms)' -f $x['launch_ok'], $x['launch_ms']) } }))
    } catch { $lines.Add('line 3 (OFF stage) could not be built: ' + $_.Exception.Message) }
    try {
        $lines.Add('ON stage, REAL 0.14.42 installer through launch_installer (expect Err os_code 4551 / shell_ret 5 / AppControl): ' + (Format-ArchPair { param($d) $x = $d['verdict']; if ($null -eq $x) { 'no result' } else { $det = $x['on_block_detail']; if ($null -eq $det) { 'no result' } else { $nt = ''; if ($det['note']) { $nt = '; note=' + [string]$det['note'] }; ('as expected={0}; ok={1} shell_ret={2} os_code={3} block={4} display={5} ({6} ms); timed_out={7}{8}' -f $x['on_block_ok'], $det['ok'], $det['shell_ret'], $det['os_code'], $det['block'], $det['display'], $x['on_launch_ms'], $det['timed_out'], $nt) } } }))
    } catch { $lines.Add('line 4 (ON stage) could not be built: ' + $_.Exception.Message) }
    try {
        $lines.Add('ON stage, signed control (cmd.exe copy, expect Ok): ' + (Format-ArchPair { param($d) $x = $d['verdict']; if ($null -eq $x) { 'no result' } else { ('ok={0} ({1} ms); original System32 cmd.exe launched (tried only when the copy failed)={2}' -f $x['control_ok'], $x['control_ms'], $x['control_original_ok']) } }))
    } catch { $lines.Add('line 5 (ON control) could not be built: ' + $_.Exception.Message) }
    try {
        $lines.Add('cleanup (remove_installer: file and folder gone) / installer path shape as the plugin writes it: ' + (Format-ArchPair { param($d) $x = $d['verdict']; if ($null -eq $x) { 'no result' } else { ('remove_installer ran={0}; cleanup ok={1} (after 3 s observation: {2}; file left={3}, folder left={4}); path shape ok={5}; FILE_ATTRIBUTE_TEMPORARY={6}' -f $x['cleanup_ran'], $x['cleanup_ok'], $x['cleanup_settled_ok'], $x['cleanup_file_left'], $x['cleanup_dir_left'], $x['path_shape_ok'], $x['attr_temporary_on_installer']) } }))
    } catch { $lines.Add('line 6 (cleanup) could not be built: ' + $_.Exception.Message) }
    try {
        $lines.Add('drivers: ' + (Format-ArchPair { param($d) $x = $d['verdict']; if ($null -eq $x) { 'no result' } else { ('done={0}; alive through the ON stage={1}; exit code={2}; a launch call timed out={3}; slow calls (over 5 s)={4}' -f $x['stage_done'], $x['alive_through_on_stage'], $d['exit_code'], $x['any_launch_timed_out'], (@($x['slow_calls']) -join ',')) } }))
    } catch { $lines.Add('line 7 (drivers) could not be built: ' + $_.Exception.Message) }
    try {
        $ev = $RUN['events']
        $evText = 'not collected'
        if ($ev -is [System.Collections.IDictionary]) {
            if ($null -eq $ev['finished']) {
                $er = [string]$ev['error']
                if (-not $er) { try { $er = [string]$RUN['steps']['events']['error'] } catch { } }
                $evText = 'event collection did not finish (' + $er + ')'
            } else {
                $pn = New-Object System.Collections.Generic.List[string]
                foreach ($h in @($ev['installer_path_events'])) { foreach ($x in @($h['process_names'])) { if (-not $pn.Contains([string]$x)) { $pn.Add([string]$x) } } }
                $evText = ('{0} CodeIntegrity 3076/3077 event(s) name a cys-0.14.42-updater- path (blocked processes: {1})' -f $ev['installer_path_event_count'], ($pn.ToArray() -join ' | '))
            }
        }
        $shot = $RUN['screenshot']
        $shotText = 'none'
        if ($shot -is [System.Collections.IDictionary]) { $shotText = ('{0} ok={1}' -f $shot['name'], $shot['ok']) }
        $lines.Add(('events: {0}; screenshot: {1}' -f $evText, $shotText))
    } catch { $lines.Add('line 8 (events, screenshot) could not be built: ' + $_.Exception.Message) }
    try {
        $rs = $RUN['restore']
        if ($rs -is [System.Collections.IDictionary]) {
            $fin = $rs['final_state']
            $finVal = $null
            $finEnf = $null
            $finMp = $null
            if ($fin -is [System.Collections.IDictionary]) {
                $finVal = $fin['registry']['value']
                $finEnf = $fin['enforced']
                if ($fin['defender'] -is [System.Collections.IDictionary]) { $finMp = $fin['defender']['SmartAppControlState'] }
            }
            $lines.Add(('restore: ok={0}; registry value={1}; enforced={2}; Defender SmartAppControlState={3}; NEG control runs again={4}' -f $RUN['restore_ok'], $finVal, $finEnf, $finMp, $rs['neg_control_runs_again']))
        } else {
            $lines.Add('restore: not recorded (see product-launch.json)')
        }
    } catch { $lines.Add('line 9 (restore) could not be built: ' + $_.Exception.Message) }
    try {
        $vd = $RUN['verdict']
        $vText = 'VERDICT: not decided'
        if ($vd -is [System.Collections.IDictionary]) { $vText = ('VERDICT: {0} (basis: {1}) - {2}' -f $vd['result'], $vd['basis'], (@($vd['reasons']) -join '; ')) }
        if ($RUN['notes'].Count -gt 0) { $vText = $vText + ' | notes: ' + (($RUN['notes'].ToArray()) -join ' | ') }
        $lines.Add($vText)
    } catch { $lines.Add('line 10 (verdict) could not be built: ' + $_.Exception.Message) }
    $out = New-Object System.Collections.Generic.List[string]
    foreach ($l in $lines.ToArray()) {
        $one = [regex]::Replace([string]$l, '[\r\n]+', ' ')
        $out.Add((Limit-Text $one 900))
    }
    Save-Text 'product-launch-summary.txt' (($out.ToArray()) -join "`r`n")
    foreach ($l in $out.ToArray()) { Write-Log ('SUMMARY ' + $l) }
}

# =========================================================================
# main
# =========================================================================
try {
    Invoke-Step 'prepare' {
        $prep = [ordered]@{}
        $RUN['prepare'] = $prep
        Initialize-DiagNative
        $prep['native_ready'] = $global:DiagNativeReady
        $prep['native_error'] = $global:DiagNativeError
        Initialize-Screenshot
        # warm up everything that is used later, so that nothing is loaded for the first time while SAC is on
        try { $null = Get-WinEvent -ListLog 'Microsoft-Windows-CodeIntegrity/Operational' -ErrorAction Stop } catch { }
        try { $null = Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256 -ErrorAction Stop } catch { }
        try { $null = Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop | Select-Object -First 1 } catch { }
        try { $null = Get-AuthenticodeSignature -LiteralPath (Join-Path $env:windir 'System32\cmd.exe') -ErrorAction Stop } catch { }
        try { $null = ConvertFrom-Json '{"a":[1,2]}' } catch { }
        try { $null = ConvertTo-Json -InputObject @{ a = 1 } -Depth 3 } catch { }
        try { $null = Get-Command Get-MpComputerStatus -ErrorAction SilentlyContinue } catch { }
        try { $null = Get-CIEvents -SinceUtc ((Get-Date).ToUniversalTime().AddSeconds(-1)) -Max 5 } catch { }
        try { $null = Get-VisibleWindowsText } catch { }
        $prep['language_mode'] = Get-LangMode
        try {
            $nt = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -ErrorAction Stop
            $prep['os_product'] = ('{0} {1} build {2}.{3}' -f $nt.ProductName, $nt.DisplayVersion, $nt.CurrentBuild, $nt.UBR)
        } catch { }
        Initialize-Subjects
        $prep['subjects_loaded'] = @($CTX['subjects']).Count
        # the two files under test: the product module must have arrived BYTE FOR BYTE (git must not have changed its line endings)
        $srcDir = Join-Path $global:DiagRoot 'product'
        $ul = Join-Path $srcDir 'update_launch.rs'
        $mn = Join-Path $srcDir 'main.rs'
        $ulHash = Get-Sha256 $ul
        $prep['update_launch_rs'] = [ordered]@{ path = $ul; exists = (Test-Path -LiteralPath $ul); sha256 = $ulHash; expected_sha256 = $K_UL_SHA256; matches_expected = ([string]$ulHash -eq $K_UL_SHA256) }
        $prep['main_rs'] = [ordered]@{ path = $mn; exists = (Test-Path -LiteralPath $mn); sha256 = (Get-Sha256 $mn) }
        if (-not ([string]$ulHash -eq $K_UL_SHA256)) { $RUN['notes'].Add('update_launch.rs sha256 differs from the product file (' + $K_UL_SHA256 + '): the module under test is NOT the product file') }
        # the real installer (the byte source of the ON stage) and the signed cmd.exe (the byte source of the success path / control)
        $inst = Get-AssetInstaller $K_INSTALLER_VERSION
        $ii = [ordered]@{ path = $inst; exists = (Test-Path -LiteralPath $inst); usable = (Test-AssetUsable $K_INSTALLER_VERSION); size = $null; sha256 = $null; signature = $null }
        if ($ii['exists']) {
            $ii['size'] = (Get-Item -LiteralPath $inst).Length
            $ii['sha256'] = Get-Sha256 $inst
            $ii['signature'] = (Get-SigRecord $inst).status
        } else {
            $RUN['notes'].Add('the 0.14.42 installer asset is missing (p1-assets failed?): the ON stage has no installer bytes')
        }
        $prep['installer'] = $ii
        $CTX['installer_path'] = $inst
        $cmdPath = [string]$CTX['cmd_path']
        $prep['cmd_exe'] = [ordered]@{ path = $cmdPath; exists = (Test-Path -LiteralPath $cmdPath); sha256 = (Get-Sha256 $cmdPath); signature = (Get-SigRecord $cmdPath).status }
        # the CodeIntegrity/Operational log is 1 MB by default: raise it to 128 MB before the SAC window
        $we = Join-Path $env:windir 'System32\wevtutil.exe'
        $g1 = Invoke-Proc -File $we -Arguments 'gl Microsoft-Windows-CodeIntegrity/Operational' -TimeoutSec 30
        $sl = Invoke-Proc -File $we -Arguments 'sl Microsoft-Windows-CodeIntegrity/Operational /ms:134217728' -TimeoutSec 30
        $prep['ci_log_size'] = [ordered]@{ before = $g1.out; set_rc = $sl.rc; set_err = $sl.err }
        # baseline: SAC must be OFF now (value 0, nothing enforced)
        $init = Get-SacRealState 'initial'
        $RUN['sac_initially_enforced'] = $init['enforced']
        if ($init['enforced']) { $RUN['notes'].Add('SAC was ALREADY enforced at the start of the run: the SAC-off baseline premise does not hold') }
        Save-Run
    }

    Invoke-Step 'build' {
        $CTX['build'] = Build-ProductDrivers
        $RUN['build'] = $CTX['build']
        Save-Json 'product-build.json' $CTX['build'] 6
        Save-Run
    }

    Invoke-Step 'start-drivers' {
        $bld = $CTX['build']
        $started = New-Object System.Collections.Generic.List[object]
        foreach ($arch in @('x64', 'aarch64')) {
            if (($bld -is [System.Collections.IDictionary]) -and $bld['variants'].Contains($arch) -and $bld['variants'][$arch]['ok']) {
                $rec = Start-ProductDriver ($bld['variants'][$arch])
                $RUN['drivers'][$arch] = $rec
                $started.Add($rec)
            } else {
                $RUN['notes'].Add('no ' + $arch + ' driver: it was not built (see product-build.json)')
            }
        }
        Save-Run
        # OFF stage results: every driver must have written "ready" before SAC is turned on
        foreach ($rec in $started.ToArray()) {
            if ($rec['start_error']) { $rec['ready'] = $false; continue }
            $sw = [System.Diagnostics.Stopwatch]::StartNew()
            $ok = Wait-DriverFile ([string]$rec['work_dir']) 'ready' $K_READY_WAIT_SEC ($CTX['procs'][[string]$rec['arch']])
            $rec['ready'] = [bool]$ok
            $rec['ready_after_sec'] = [int]$sw.Elapsed.TotalSeconds
            $offJsonl = Join-Path ([string]$rec['work_dir']) 'product-launch.jsonl'
            $offLines = @(Read-DriverLog $offJsonl)
            $rec['off_log'] = $offLines.Count
            Save-Text ('product-launch-' + [string]$rec['arch'] + '-off.jsonl') (Read-FileShared $offJsonl)
            $rec['off_result'] = Get-OffResult $offLines
            if (-not $ok) { $RUN['notes'].Add(([string]$rec['arch']) + ' driver did not write "ready" within ' + $K_READY_WAIT_SEC + ' s') }
        }
        Save-Run
    }

    # everything measured so far is safe on a results branch BEFORE the policy changes
    Invoke-Step 'checkpoint-pre-sac' { $null = Publish-Checkpoint 'pre-sac' }

    $anyReady = $false
    foreach ($arch in @('x64', 'aarch64')) {
        if ($RUN['drivers'].Contains($arch) -and ($RUN['drivers'][$arch]['ready'] -eq $true)) { $anyReady = $true }
    }
    if (-not $anyReady) {
        $RUN['notes'].Add('SAC-on skipped: no driver is ready, so there is nothing to measure')
    } elseif ((Get-MinutesLeft) -gt 5) {
        Invoke-Step 'sac-on' { Invoke-SacOn }
    } else {
        $RUN['notes'].Add('SAC-on skipped: not enough job time left')
    }

    Invoke-Step 'go-and-wait' {
        $gw = [ordered]@{ started = (Get-IsoNow); go_utc = $null; per_driver = [ordered]@{}; all_done = $false; screenshot_extra = $null }
        $RUN['go_and_wait'] = $gw
        $live = New-Object System.Collections.Generic.List[string]
        foreach ($arch in @('x64', 'aarch64')) {
            if ($RUN['drivers'].Contains($arch) -and ($RUN['drivers'][$arch]['ready'] -eq $true)) { $live.Add($arch) }
        }
        if ($RUN['measurable'] -ne $true) {
            # SAC is not on: nothing is measured; tell the waiting drivers to stop
            foreach ($arch in $live.ToArray()) {
                try { [System.IO.File]::WriteAllText((Join-Path ([string]$CTX['work'][$arch]) 'abort'), 'abort') } catch { }
            }
            if ($null -eq $RUN['sac_on']) {
                $RUN['notes'].Add('no go signal: SAC was never turned on (the drivers were told to abort)')
            } else {
                $RUN['notes'].Add('no go signal: SAC was not confirmed on (the drivers were told to abort)')
            }
            return
        }
        $tGo = (Get-Date).ToUniversalTime()
        $gw['go_utc'] = ConvertTo-IsoUtc $tGo
        foreach ($arch in $live.ToArray()) {
            try { [System.IO.File]::WriteAllText((Join-Path ([string]$CTX['work'][$arch]) 'go'), 'go') } catch { $RUN['notes'].Add('could not write the go file for ' + $arch + ': ' + $_.Exception.Message) }
        }
        Save-Run
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $doneAt = @{}
        $exitedAt = @{}
        $extraShot = $false
        while ($sw.Elapsed.TotalSeconds -lt $K_DONE_WAIT_SEC) {
            foreach ($arch in $live.ToArray()) {
                if ($doneAt.ContainsKey($arch) -or $exitedAt.ContainsKey($arch)) { continue }
                $doneFile = Join-Path ([string]$CTX['work'][$arch]) 'done'
                if (Test-Path -LiteralPath $doneFile) { $doneAt[$arch] = [int]$sw.Elapsed.TotalSeconds; continue }
                # a driver that ended without "done" (crashed) never will write it: do not keep SAC on for the whole wait
                $ended = $false
                try { $dp = $CTX['procs'][$arch]; if ($null -ne $dp) { $ended = [bool]$dp.HasExited } } catch { }
                if ($ended) {
                    Start-Sleep -Milliseconds 400
                    if (Test-Path -LiteralPath $doneFile) { $doneAt[$arch] = [int]$sw.Elapsed.TotalSeconds } else { $exitedAt[$arch] = [int]$sw.Elapsed.TotalSeconds }
                }
            }
            if (($doneAt.Count + $exitedAt.Count) -ge $live.Count) { break }
            # a call that Windows holds with a modal window shows here as "not done" - one extra picture of it
            if ((-not $extraShot) -and ($sw.Elapsed.TotalSeconds -ge 30)) {
                $extraShot = $true
                $gw['screenshot_extra'] = Save-Screenshot 'product-launch-screen-waiting.png'
            }
            Start-Sleep -Milliseconds 500
        }
        $exitedNames = New-Object System.Collections.Generic.List[string]
        foreach ($arch in $live.ToArray()) {
            if ($doneAt.ContainsKey($arch)) {
                $gw['per_driver'][$arch] = [ordered]@{ done = $true; exited_without_done = $false; after_sec = $doneAt[$arch] }
            } elseif ($exitedAt.ContainsKey($arch)) {
                $gw['per_driver'][$arch] = [ordered]@{ done = $false; exited_without_done = $true; after_sec = $exitedAt[$arch] }
                $exitedNames.Add([string]$arch)
            } else {
                $gw['per_driver'][$arch] = [ordered]@{ done = $false; exited_without_done = $false; after_sec = $null }
            }
        }
        $gw['all_done'] = [bool]($doneAt.Count -ge $live.Count)
        if ($exitedNames.Count -gt 0) { $RUN['notes'].Add('a driver ended without writing "done" (see its stderr and jsonl): ' + ($exitedNames.ToArray() -join ', ')) }
        if (($doneAt.Count + $exitedAt.Count) -lt $live.Count) { $RUN['notes'].Add('a driver did not write "done" within ' + $K_DONE_WAIT_SEC + ' s after go (a call held by Windows?)') }
        # the one screenshot of the run (SAC is still on): right after the drivers finished
        $RUN['screenshot'] = Save-Screenshot 'product-launch-screen.png'
        Save-Run
    }

    Invoke-Step 'collect' {
        foreach ($arch in @('x64', 'aarch64')) {
            if (-not $RUN['drivers'].Contains($arch)) { continue }
            $rec = $RUN['drivers'][$arch]
            $work = [string]$rec['work_dir']
            if (-not $work) { continue }
            $logText = Read-FileShared (Join-Path $work 'product-launch.jsonl')
            Save-Text ('product-launch-' + $arch + '.jsonl') $logText
            Save-Text ('product-launch-' + $arch + '-stdout.txt') (Read-FileShared ([string]$rec['stdout_file']))
            Save-Text ('product-launch-' + $arch + '-stderr.txt') (Read-FileShared ([string]$rec['stderr_file']))
            $rec['ready_file'] = Read-FileShared (Join-Path $work 'ready')
            $rec['done_file'] = Read-FileShared (Join-Path $work 'done')
            $lines = @(Read-DriverLog (Join-Path $work 'product-launch.jsonl'))
            $rec['log_lines'] = $lines.Count
            $rec['verdict'] = Get-DriverVerdict $lines
            $proc = $CTX['procs'][$arch]
            $rec['exit_code'] = $null
            try { if ($null -ne $proc) { if ($proc.HasExited) { $rec['exit_code'] = $proc.ExitCode } else { $rec['exit_code'] = 'still running' } } } catch { }
        }
        Save-Run
    }
} catch {
    Add-DiagError 'product-launch main' $_
} finally {
    # RESTORE first, whatever happened above (the CodeIntegrity events are read from the persisted log afterwards)
    Invoke-Step 'restore' {
        $rs = [ordered]@{ touched = $script:SacTouched; started = (Get-IsoNow); result = $null; final_state = $null; neg_control_probe = $null; neg_control_run = $null; neg_control_runs_again = $null; language_mode = (Get-LangMode) }
        $RUN['restore'] = $rs
        $CTX['t_restore_start'] = (Get-Date).ToUniversalTime()
        if ($script:SacTouched) {
            $rs['result'] = Restore-SacReal 'final'
        } else {
            $rs['result'] = 'SAC was never turned on by this run: nothing to restore'
        }
        $fin = Get-SacRealState 'final'
        $rs['final_state'] = $fin
        $negSubject = Get-SubjectByLabel $K_NEG
        if ($null -ne $negSubject) {
            $np = Invoke-ProbeCreate ([string]$negSubject.path)
            $rs['neg_control_probe'] = $np
            # the NEG control is a tiny console exe that prints 'diag-neg <uuid>': run it for real (it stays blocked while SAC is on)
            $nr = Invoke-Proc -File ([string]$negSubject.path) -TimeoutSec 15
            $rs['neg_control_run'] = [ordered]@{ rc = $nr.rc; timed_out = $nr.timedOut; start_error = $nr.startError; stdout = (Limit-Text $nr.out 300) }
            $rs['neg_control_runs_again'] = [bool](($nr.rc -eq 0) -and ([string]$nr.out -match 'diag-neg'))
        }
        $RUN['restore_ok'] = [bool]((-not $fin['enforced']) -and ($fin['registry']['value'] -eq 0))
        $rs['finished'] = (Get-IsoNow)
    }
    # last resort when the restore above did not verify (the workflow has one more safety step that needs no PowerShell)
    if ($script:SacTouched) {
        try { $RUN['restore_last_resort'] = Restore-SacReal 'last-resort' } catch { }
    }
    # the very last resort also works in Constrained Language Mode (external commands only)
    if ($script:SacTouched) {
        try { $RUN['restore_cmd_fallback'] = Invoke-CmdRestoreFallback } catch { }
    }
    # the drivers and anything that was started from their work folders (the real installer, if SAC had not blocked it)
    Invoke-Step 'cleanup' {
        $cl = [ordered]@{ drivers = [ordered]@{}; killed_under_work = @() }
        $RUN['cleanup'] = $cl
        foreach ($arch in @($CTX['procs'].Keys)) {
            $p = $CTX['procs'][$arch]
            $st = [ordered]@{ pid = $null; had_exited = $null; killed = $false }
            try { $st['pid'] = $p.Id } catch { }
            try { $st['had_exited'] = [bool]$p.HasExited } catch { }
            if ($st['had_exited'] -ne $true) {
                try { Stop-ProcessTree -ProcessId $p.Id; $st['killed'] = $true } catch { }
            }
            $cl['drivers'][[string]$arch] = $st
        }
        $cl['killed_under_work'] = @(Stop-ProductWorkProcesses)
    }
    Invoke-Step 'events' { Save-ProductEvents }
    Invoke-Step 'verdict' {
        $reasons = New-Object System.Collections.Generic.List[string]
        $result = 'FAIL'
        $basis = 'none'
        $primary = $null
        $primaryArch = ''
        foreach ($arch in @('x64', 'aarch64')) {
            if (($null -eq $primary) -and $RUN['drivers'].Contains($arch) -and ($RUN['drivers'][$arch]['verdict'] -is [System.Collections.IDictionary])) {
                $primary = $RUN['drivers'][$arch]['verdict']
                $primaryArch = $arch
            }
        }
        if ($primaryArch -eq 'x64') { $basis = 'x64' } elseif ($primaryArch) { $basis = 'aarch64 only - the x64 driver gave no result' }
        if ($RUN['measurable'] -ne $true) {
            if ($null -eq $RUN['sac_on']) {
                $reasons.Add('SAC was never turned on (no driver was ready, or not enough job time left): nothing was measured')
            } else {
                $reasons.Add('SAC could not be confirmed on: not measurable')
            }
        } elseif ($null -eq $primary) {
            $reasons.Add('no driver result (build, start or log failed)')
        } else {
            if (-not $primary['off_launch_ok']) { $reasons.Add('OFF stage: launching the signed cmd.exe copy did not return Ok') }
            if ($primary['on_launch_timed_out']) {
                $reasons.Add('ON stage: the real installer launch call did not return within the driver limit (40 s): Windows held the call (see product-launch-screen*.png and the launch_real_installer_begin line of the jsonl)')
            } elseif (-not $primary['on_block_ok']) {
                $reasons.Add('ON stage: the real installer launch was not Err 4551/5/AppControl')
            }
            if ($primary['cleanup_ran']) {
                if (-not $primary['cleanup_settled_ok']) {
                    $left = 'the installer file and its folder'
                    if (($primary['cleanup_file_left'] -ne $true) -and ($primary['cleanup_dir_left'] -eq $true)) {
                        $left = 'the (empty) folder - the file itself is gone, probably after a delayed delete; the module tries the folder only once'
                    } elseif (($primary['cleanup_file_left'] -eq $true) -and ($primary['cleanup_dir_left'] -ne $true)) {
                        $left = 'the installer file'
                    }
                    $reasons.Add('ON stage: remove_installer left ' + $left + ' behind (still there after the 3 s observation)')
                } elseif (-not $primary['cleanup_ok']) {
                    $reasons.Add('NOTE remove_installer: file/folder were not gone at once (gone after ' + [string]$primary['cleanup_settle_ms'] + ' ms: a delete that was still pending, e.g. a scanner handle)')
                }
            }
            if (-not $primary['control_ok']) {
                $cw = 'ON stage: the signed cmd.exe copy did not launch while SAC is on'
                if ($primary['control_original_ok'] -eq $true) {
                    $cw = $cw + '; the ORIGINAL System32 cmd.exe did launch (the copy is blocked by its location or name, not cmd.exe itself)'
                } elseif ($primary['control_original_ok'] -eq $false) {
                    $cw = $cw + '; the original System32 cmd.exe did not launch either'
                }
                if (($primary['any_launch_timed_out'] -eq $true) -and (-not $primary['on_launch_timed_out'])) { $cw = $cw + '; a launch call timed out (driver limit 40 s)' }
                $reasons.Add($cw)
            }
            if (-not $primary['stage_done']) { $reasons.Add('the driver did not finish the ON stage') }
            if (-not $primary['path_shape_ok']) { $reasons.Add('NOTE the written installer path does not have the plugin shape (see the write_* lines of the jsonl)') }
            if (@($primary['slow_calls']).Count -gt 0) { $reasons.Add('NOTE slow launch call(s): ' + (@($primary['slow_calls']) -join ',')) }
            $hard = 0
            foreach ($rr in $reasons.ToArray()) { if (-not $rr.StartsWith('NOTE')) { $hard++ } }
            if ($hard -eq 0) {
                $result = 'PASS'
                $reasons.Insert(0, 'the product module behaves as expected on real SAC: Err 4551/5/AppControl for the unsigned installer, Ok for the signed copy, clean-up done')
            }
        }
        if ($RUN['restore_ok'] -ne $true) { $reasons.Add('NOTE the restore could not be verified (see restore)') }
        $RUN['verdict'] = [ordered]@{ result = $result; basis = $basis; reasons = $reasons.ToArray() }
        Save-Json 'product-launch-verdict.json' $RUN['verdict'] 5
    }
    Invoke-Step 'summary' { Write-ProductSummary }
    $RUN['finished'] = (Get-IsoNow)
    Save-Run
    Complete-DiagScript 'product-launch'
}
exit 0
