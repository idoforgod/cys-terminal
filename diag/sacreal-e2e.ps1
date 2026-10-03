# diag/sacreal-e2e.ps1 -- the whole reproduction with REAL Smart App Control (SAC), windows-11-arm job only.
# Question: with SAC really ON, what happens to an installed (unsigned, unknown reputation) cys 0.14.37 and to its in-app update?
#   0  p0-facts / p1-assets ran before (workflow steps): assets, controls, subjects.json
#   1  SAC off: install inst-0.14.37 with /S, verify the version marker, fsutil EA of the installed exes, kill what the install started
#   2  (A) start the installed app FIRST (WebView2 CDP flag + HKLM policy), confirm the CDP attach, record check_update.
#      install_update is NOT called yet.
#   3  SAC on: HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy\VerifiedAndReputablePolicyState = 1 + CiTool -r.
#      10 s later 3 states are recorded (registry / CiTool VerifiedAndReputable* / Get-MpComputerStatus).
#      Not enforced -> recorded as is, measurable=false, the run ends (after the restore).
#   4  verdict matrix: CreateProcessW(CREATE_SUSPENDED) probes (no code runs) of every main subject + the installed exes;
#      Code Integrity events 3076/3077/3089/309x (raw XML) -> sacreal-matrix.json/.txt, sacreal-3077.json
#   5  (A) install_update {force:true} over CDP on the app that is ALREADY running; up to 6 minutes of timeline + screenshots
#      -> sacreal-e2e-A-verdict.json
#   6  (B) really start the installed cys-app.exe (Start-Process), then cysd.exe --version and cys.exe --version
#      -> sacreal-e2e-B.json
#   7  events -> RESTORE (value 0 + CiTool -r) -> re-check the state and that the NEG control runs again
#      -> sacreal-summary.txt (10 lines at most)
# ONE PowerShell process. Every Add-Type (DiagNative, the screenshot assemblies) and every module/cmdlet warm-up happens BEFORE
# SAC is turned on (a new unsigned managed DLL could not be loaded afterwards). While SAC is on only signed node.exe and
# Windows system tools are started. Partial results are written to OUT after every step; finally{} always restores the
# registry value and re-checks (the workflow has a second safety step, ensure-sac-restored, that needs no PowerShell).
# The language mode is logged at every step. OBSERVATION ONLY: no SAC / Defender bypass of any kind.
$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')
. (Join-Path $PSScriptRoot 'e2e-update.ps1')
Start-DiagScript -Name 'sacreal-e2e'

$K_CITOOL = Join-Path $env:windir 'System32\CiTool.exe'
$K_REG = Join-Path $env:windir 'System32\reg.exe'
$K_KEY = 'HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy'
$K_VAL = 'VerifiedAndReputablePolicyState'
$K_NEG = 'control NEG (fresh unsigned exe)'
$K_PORT = 9333
$K_VERSION_FROM = '0.14.37'
$K_VERSION_TO = '0.14.42'

$RUN = [ordered]@{
    started = (Get-IsoNow)
    matrix_os = $env:DIAG_MATRIX_OS
    measurable = $null
    sac_initially_enforced = $null
    sac_on_confirmed = $null
    sac_on_utc = $null
    restore_ok = $null
    steps = [ordered]@{}
    states = [ordered]@{}
    checkpoints = [ordered]@{}
    notes = (New-Object System.Collections.Generic.List[string])
}
$CTX = @{}
$CTX['subjects'] = @()
$CTX['installed'] = @()
$CTX['runner_node'] = $null
$CTX['probe'] = @{}
$CTX['matrix'] = $null
$CTX['install_ok'] = $false
$CTX['sac_phase_started'] = $false
$CTX['t_sac_on'] = $null
$CTX['t_before_node'] = $null
$CTX['A'] = [ordered]@{ pre = $null; skipped_reason = $null }
$CTX['A_run'] = $null
$CTX['A_verdict'] = $null
$CTX['B'] = $null
$CTX['driver_node'] = ''
# the registry value is 'touched' from the moment we try to write 1 until a restore was verified
$script:SacTouched = $false
# GITHUB_TOKEN is only for the checkpoint uploads: keep it in a variable and take it out of the process environment so that
# no child process (the unsigned app under test, installers, ...) inherits it; it is handed to node for the upload only.
$script:GhToken = [string]$env:GITHUB_TOKEN
Remove-Item -Path 'Env:\GITHUB_TOKEN' -ErrorAction SilentlyContinue

# =========================================================================
# small helpers
# =========================================================================
function Save-Run { Save-Json 'sacreal-e2e.json' $RUN 9 }

function Get-LangMode { return [string]$ExecutionContext.SessionState.LanguageMode }

function Invoke-Step {
    param([string]$Name, [scriptblock]$Block)
    $stepRec = [ordered]@{ started = (Get-IsoNow); ended = $null; ok = $true; error = $null; language_mode_at_start = (Get-LangMode); language_mode_at_end = $null; minutes_left_at_start = (Get-MinutesLeft) }
    $RUN['steps'][$Name] = $stepRec
    Write-Log ('--- step {0} (minutes left {1}, language mode {2}) ---' -f $Name, (Get-MinutesLeft), (Get-LangMode))
    try {
        & $Block
    } catch {
        $stepRec['ok'] = $false
        $stepRec['error'] = Format-ErrorText $_
        Add-DiagError ('step ' + $Name) $_
    }
    $stepRec['ended'] = (Get-IsoNow)
    $stepRec['language_mode_at_end'] = (Get-LangMode)
    Save-Run
}

function Limit-Text {
    param($Text, [int]$Max = 2000)
    $t = [string]$Text
    if ($t.Length -gt $Max) { return ($t.Substring(0, $Max) + '...[cut]') }
    return $t
}

# exception -> record with the native (Win32) error code, if there is one: shows whether a start failure is 4551
# (ERROR_CIP_POLICY_VIOLATION: "An Application Control policy has blocked this file")
function Get-NativeErrorInfo {
    param($ErrRec)
    $o = [ordered]@{ type = $null; message = $null; hresult = $null; native_error_code = $null; is_4551 = $false; message_mentions_policy = $false; inner = $null; fully_qualified_error_id = $null; raw = $null }
    try {
        $ex = $ErrRec.Exception
        $o['type'] = $ex.GetType().FullName
        $o['message'] = [string]$ex.Message
        $o['hresult'] = ('0x{0:X8}' -f [int]$ex.HResult)
        $o['fully_qualified_error_id'] = [string]$ErrRec.FullyQualifiedErrorId
        $o['message_mentions_policy'] = [bool]([string]$ex.Message -match '(?i)application control|policy|blocked')
        $cur = $ex
        $depth = 0
        $chain = New-Object System.Collections.Generic.List[string]
        while (($null -ne $cur) -and ($depth -lt 6)) {
            $chain.Add(('{0}: {1}' -f $cur.GetType().FullName, $cur.Message))
            if (($cur -is [System.ComponentModel.Win32Exception]) -and ($null -eq $o['native_error_code'])) { $o['native_error_code'] = [int]$cur.NativeErrorCode }
            $cur = $cur.InnerException
            $depth++
        }
        $o['inner'] = $chain.ToArray()
        if ($null -ne $o['native_error_code']) {
            $o['is_4551'] = ([int]$o['native_error_code'] -eq 4551)
            $o['is_4551_basis'] = 'Win32Exception.NativeErrorCode'
        } elseif ([string]$ex.Message -match '(?i)Application Control policy has blocked') {
            # Windows PowerShell 5.1 Start-Process wraps only the MESSAGE of the Win32Exception into a new InvalidOperationException
            # (no inner exception), so the number is gone; this text is what FormatMessage gives for error 4551
            $o['is_4551'] = $true
            $o['is_4551_basis'] = 'message text (the native code was not preserved by Start-Process)'
        }
        $o['raw'] = Limit-Text ($ErrRec | Out-String) 3000
    } catch {
        $o['raw'] = 'Get-NativeErrorInfo failed: ' + $_.Exception.Message
    }
    return $o
}

# Upload what is in OUT so far to a results branch (diag-results/<run>-<attempt>-sacreal-e2e-<tag>-<os>) with the same Node
# script the publish step uses. Insurance against SAC killing the runner: what was measured before is already safe.
function Publish-Checkpoint {
    param([string]$Tag)
    try {
        if (-not $script:GhToken) {
            Write-Log ('checkpoint {0}: no GITHUB_TOKEN in this step -> skipped' -f $Tag)
            $RUN['checkpoints'][$Tag] = 'skipped: no GITHUB_TOKEN'
            return
        }
        $nodeForPublish = Find-Exe 'node.exe'
        if ($CTX['driver_node']) { $nodeForPublish = [string]$CTX['driver_node'] }
        if (-not $nodeForPublish) {
            Write-Log ('checkpoint {0}: node.exe not found -> skipped' -f $Tag)
            $RUN['checkpoints'][$Tag] = 'skipped: node.exe not found'
            return
        }
        $osName = [string]$env:DIAG_MATRIX_OS
        if (-not $osName) { $osName = 'unknown-os' }
        $pubScript = Join-Path $global:DiagRoot 'publish-results.mjs'
        $pubArgs = '"{0}" --job {1} --os {2} --out "{3}"' -f $pubScript, ('sacreal-e2e-' + $Tag), $osName, $global:DiagOut
        $env:GITHUB_TOKEN = $script:GhToken
        try {
            $pr = Invoke-Proc -File $nodeForPublish -Arguments $pubArgs -TimeoutSec 300
        } finally {
            Remove-Item -Path 'Env:\GITHUB_TOKEN' -ErrorAction SilentlyContinue
        }
        $msg = (([string]$pr.out) + ' ' + ([string]$pr.err)).Trim()
        Write-Log ('checkpoint {0}: rc={1} {2}' -f $Tag, $pr.rc, (Limit-Text $msg 400))
        $RUN['checkpoints'][$Tag] = [ordered]@{ rc = $pr.rc; start_error = $pr.startError; out = (Limit-Text $pr.out 3000); err = (Limit-Text $pr.err 3000); time = (Get-IsoNow) }
    } catch {
        Write-Log ('checkpoint {0} failed: {1}' -f $Tag, $_.Exception.Message) 'WARN'
        $RUN['checkpoints'][$Tag] = 'failed: ' + $_.Exception.Message
    }
}

# =========================================================================
# the real SAC switch: 3 states, set, restore
# =========================================================================
# 3 states (registry value / CiTool VerifiedAndReputable* policies / Get-MpComputerStatus) + the UMCI flag of
# NtQuerySystemInformation(103). run 1 (sac-real.json): value 1 -> VerifiedAndReputableDesktop IsEnforced=true,
# SmartAppControlState=On, UMCI flag set. The '...Evaluation' policies are listed even when SAC is off, so only IsEnforced counts.
function Get-SacRealState {
    param([string]$Tag)
    $s = [ordered]@{
        tag = $Tag; time = (Get-IsoNow); language_mode = (Get-LangMode)
        registry = [ordered]@{ rc = $null; out = $null; present = $null; value = $null }
        citool = [ordered]@{ rc = $null; json_parsed = $false; error = $null; desktop_policy = $null; enforced_names = @(); verified_and_reputable = @() }
        defender = $null
        ci_state = $null
        signals = [ordered]@{ registry_is_1 = $false; citool_desktop_enforced = $false; defender_on = $null; umci_flag = $false }
        enforced = $false
    }
    try {
        $q = Invoke-Proc -File $K_REG -Arguments ('query "{0}" /v {1}' -f $K_KEY, $K_VAL) -TimeoutSec 30
        $s['registry']['rc'] = $q.rc
        $s['registry']['out'] = (([string]$q.out) + ([string]$q.err)).Trim()
        $m = [regex]::Match([string]$q.out, ($K_VAL + '\s+REG_DWORD\s+0x([0-9a-fA-F]+)'))
        if ($m.Success) {
            $s['registry']['value'] = [Convert]::ToInt32($m.Groups[1].Value, 16)
            $s['registry']['present'] = $true
        } else {
            $s['registry']['present'] = $false
        }
        if ($s['registry']['value'] -eq 1) { $s['signals']['registry_is_1'] = $true }
    } catch {
        $s['registry']['out'] = 'exception: ' + $_.Exception.Message
    }
    try {
        $c = Invoke-Proc -File $K_CITOOL -Arguments '-lp -json' -TimeoutSec 120
        $s['citool']['rc'] = $c.rc
        $rawJson = [string]$c.out
        Save-Text ('sacreal-citool-lp-' + $Tag + '.json') $rawJson
        if ($c.err) { Save-Text ('sacreal-citool-lp-' + $Tag + '.stderr.txt') ([string]$c.err) }
        $parsed = $null
        try {
            $parsed = ConvertFrom-Json $rawJson
        } catch {
            $s['citool']['error'] = 'json parse: ' + $_.Exception.Message
        }
        if ($null -ne $parsed) {
            $s['citool']['json_parsed'] = $true
            $vrList = New-Object System.Collections.Generic.List[object]
            $enfList = New-Object System.Collections.Generic.List[string]
            foreach ($pol in @($parsed.Policies)) {
                $fn = [string]$pol.FriendlyName
                if ($fn -like 'VerifiedAndReputable*') {
                    $prec = [ordered]@{ name = $fn; policy_id = [string]$pol.PolicyID; is_enforced = [bool]$pol.IsEnforced; is_authorized = [bool]$pol.IsAuthorized; is_on_disk = [bool]$pol.IsOnDisk }
                    $vrList.Add($prec)
                    if ([bool]$pol.IsEnforced) { $enfList.Add($fn) }
                    if ($fn -eq 'VerifiedAndReputableDesktop') {
                        $s['citool']['desktop_policy'] = $prec
                        $s['signals']['citool_desktop_enforced'] = [bool]$pol.IsEnforced
                    }
                }
            }
            $s['citool']['verified_and_reputable'] = $vrList.ToArray()
            $s['citool']['enforced_names'] = $enfList.ToArray()
        }
    } catch {
        $s['citool']['error'] = 'exception: ' + $_.Exception.Message
    }
    try {
        $mp = Invoke-JobWithTimeout { Get-MpComputerStatus -ErrorAction Stop } 60
        $s['defender'] = Select-Props $mp @('SmartAppControlState', 'SmartAppControlExpiration', 'AMRunningMode', 'RealTimeProtectionEnabled')
        $s['signals']['defender_on'] = ([string]$s['defender']['SmartAppControlState'] -eq 'On')
    } catch {
        $s['defender'] = 'unavailable: ' + $_.Exception.Message
    }
    $s['ci_state'] = Get-CodeIntegrityState
    try {
        if (@($s['ci_state']['flags']) -contains 'UMCI_ENABLED') { $s['signals']['umci_flag'] = $true }
    } catch { }
    $sig = $s['signals']
    if ($s['citool']['json_parsed']) {
        $s['enforced'] = [bool]($sig['citool_desktop_enforced'] -and (($sig['defender_on'] -eq $true) -or $sig['umci_flag']))
    } else {
        $s['enforced'] = [bool](($sig['defender_on'] -eq $true) -and $sig['umci_flag'])
    }
    $RUN['states'][$Tag] = $s
    Save-Json ('sacreal-state-' + $Tag + '.json') $s 7
    Save-Run
    return $s
}

# write the registry value, then CiTool -r (CiTool prints "Press Enter to Continue": Invoke-Proc closes stdin)
function Set-SacValue {
    param([int]$Value, [string]$Why)
    $res = [ordered]@{ why = $Why; value = $Value; time = (Get-IsoNow); write = $null; refresh = $null }
    $w = Invoke-Proc -File $K_REG -Arguments ('add "{0}" /v {1} /t REG_DWORD /d {2} /f' -f $K_KEY, $K_VAL, $Value) -TimeoutSec 30
    $res['write'] = [ordered]@{ rc = $w.rc; out = (([string]$w.out).Trim()); err = (([string]$w.err).Trim()); start_error = $w.startError }
    $r = Invoke-Proc -File $K_CITOOL -Arguments '-r' -TimeoutSec 120
    Save-Text ('sacreal-citool-refresh-' + $Why + '.txt') (Format-ProcText $r)
    $res['refresh'] = [ordered]@{ rc = $r.rc; timed_out = $r.timedOut; start_error = $r.startError; out = (Limit-Text $r.out 2000); err = (Limit-Text $r.err 2000) }
    return $res
}

# value 0 + CiTool -r, then re-check; up to 3 attempts. Never throws.
function Restore-SacReal {
    param([string]$Why)
    $res = [ordered]@{ why = $Why; started = (Get-IsoNow); attempts = (New-Object System.Collections.Generic.List[object]); restored = $false; finished = $null }
    try {
        for ($i = 1; $i -le 3; $i++) {
            $att = [ordered]@{ n = $i; set = $null; value_after = $null; enforced_after = $null }
            $att['set'] = Set-SacValue 0 ('restore-' + $Why + '-' + $i)
            Start-Sleep -Seconds 5
            $chk = Get-SacRealState ('restore-' + $Why + '-' + $i)
            $att['value_after'] = $chk['registry']['value']
            $att['enforced_after'] = $chk['enforced']
            $res['attempts'].Add($att)
            if ((-not $chk['enforced']) -and ($chk['registry']['value'] -eq 0)) {
                $res['restored'] = $true
                break
            }
            Start-Sleep -Seconds 5
        }
    } catch {
        $res['error'] = $_.Exception.Message
    }
    $res['finished'] = (Get-IsoNow)
    if ($res['restored']) { $script:SacTouched = $false }
    return $res
}

# Last resort that also works in Constrained Language Mode: external commands through cmd.exe only (no New-Object, no .NET calls).
# Not verified afterwards; the ensure-sac-restored workflow step is the net below it.
function Invoke-CmdRestoreFallback {
    $txt = ''
    try {
        $txt = $txt + (& $env:ComSpec /d /c ('reg add "' + $K_KEY + '" /v ' + $K_VAL + ' /t REG_DWORD /d 0 /f 2>&1') | Out-String)
        $txt = $txt + (& $env:ComSpec /d /c ('"' + $K_CITOOL + '" -r < nul 2>&1') | Out-String)
        $txt = $txt + (& $env:ComSpec /d /c ('reg query "' + $K_KEY + '" /v ' + $K_VAL + ' 2>&1') | Out-String)
    } catch {
        $txt = $txt + ' exception: ' + $_.Exception.Message
    }
    return $txt
}

# =========================================================================
# subjects
# =========================================================================
function Initialize-Subjects {
    $txt = Read-TextUtf8 (Join-Path $global:DiagOut 'subjects.json')
    if ($txt) {
        # NOT @(ConvertFrom-Json $txt): 5.1 emits the top-level JSON array as ONE pipeline object (array inside the array)
        $subjParsed = ConvertFrom-Json $txt
        $CTX['subjects'] = @($subjParsed)
    } else {
        $RUN['notes'].Add('OUT\subjects.json missing (p1-assets failed?): no subjects to evaluate')
    }
}

function Get-SubjectByLabel {
    param([string]$Label)
    foreach ($s in @($CTX['subjects'])) { if ([string]$s.label -eq $Label) { return $s } }
    return $null
}

function Get-InstalledSubjects {
    $list = New-Object System.Collections.Generic.List[object]
    $dir = Get-InstallDir
    foreach ($n in @('cys-app.exe', 'cysd.exe', 'cys.exe')) {
        $p = Join-Path $dir $n
        if (Test-Path -LiteralPath $p) {
            $sg = Get-SigRecord $p
            $list.Add([pscustomobject]@{ label = ('installed ' + $K_VERSION_FROM + ' ' + $n); path = $p; sha256 = (Get-Sha256 $p); size = (Get-Item -LiteralPath $p).Length; sig = $sg.status; sig_subject = $sg.subject; kind = ('installed-' + $K_VERSION_FROM); needle = '' })
        }
    }
    return $list.ToArray()
}

# the node.exe of the runner (setup-node): the CDP driver of scenario A is started with it AFTER SAC is on
function Get-RunnerNodeSubject {
    $n = Find-Exe 'node.exe'
    if (-not $n) { return $null }
    $sg = Get-SigRecord $n
    return [pscustomobject]@{ label = 'control POS-SIGNED runner node.exe (setup-node)'; path = $n; sha256 = (Get-Sha256 $n); size = (Get-Item -LiteralPath $n).Length; sig = $sg.status; sig_subject = $sg.subject; kind = 'control-signed-runner'; needle = '' }
}

# 3 installers + 9 app exes + controls (NEG, POS-UNSIGNED, POS-SIGNED); NOT the ~370 bundled runtime files
function Get-MatrixSubjects {
    $list = New-Object System.Collections.Generic.List[object]
    $kinds = @('installer', 'app-main', 'control-neg', 'control-pos-unsigned', 'control-signed')
    foreach ($s in @($CTX['subjects'])) { if ($kinds -contains [string]$s.kind) { $list.Add($s) } }
    foreach ($s in @($CTX['installed'])) { $list.Add($s) }
    if ($null -ne $CTX['runner_node']) { $list.Add($CTX['runner_node']) }
    return $list.ToArray()
}

# =========================================================================
# events -> files
# =========================================================================
# 3076/3077 (policy name, file path, raw XML) and 3089 / 3090-3092 (signature / ISG info, raw XML)
function Save-BlockEvents {
    param($Events, [string]$FileName)
    $blocks = New-Object System.Collections.Generic.List[object]
    $others = New-Object System.Collections.Generic.List[object]
    foreach ($e in @($Events)) {
        $id = [int]$e.id
        if (($id -eq 3076) -or ($id -eq 3077)) {
            if ($blocks.Count -lt 600) {
                $blocks.Add([ordered]@{
                    id = $id; time = $e.time
                    policy_names = @(Get-FieldValues $e '(?i)^policy\s*name$')
                    file_paths = @(Get-EventPaths $e)
                    process_names = @(Get-FieldValues $e '(?i)^process\s*name$')
                    status = @(Get-FieldValues $e '(?i)^status$')
                    sha256 = @(Get-FieldValues $e '(?i)^sha256\s*hash$')
                    xml = [string]$e.xml
                })
            }
        } elseif ((($id -eq 3089) -or (($id -ge 3090) -and ($id -le 3092))) -and ($others.Count -lt 600)) {
            $others.Add([ordered]@{ id = $id; time = $e.time; file_paths = @(Get-EventPaths $e); fields = $e.fields; xml = [string]$e.xml })
        }
    }
    $o = [ordered]@{ counts_by_id = (Get-IdCounts $Events); block_events_3076_3077 = $blocks.ToArray(); signature_and_isg_events_3089_309x = $others.ToArray() }
    Save-Json $FileName $o 7
    return $o
}

# distinct blocked file -> how often / by which process (from 3076/3077)
function Get-BlockedByFile {
    param($Events)
    $map = [ordered]@{}
    foreach ($e in @($Events)) {
        $id = [int]$e.id
        if (($id -ne 3076) -and ($id -ne 3077)) { continue }
        $fp = (@(Get-EventPaths $e) -join ' | ')
        $pn = (@(Get-FieldValues $e '(?i)^process\s*name$') -join ' | ')
        $key = ('{0} :: by {1}' -f $fp, $pn)
        if ($map.Contains($key)) { $map[$key] = [int]$map[$key] + 1 } else { $map[$key] = 1 }
    }
    return $map
}

function Save-NotificationDb {
    param([string]$Tag)
    $res = [ordered]@{ tag = $Tag; copied = @(); error = $null }
    try {
        $dir = Join-Path $env:LOCALAPPDATA 'Microsoft\Windows\Notifications'
        $copied = New-Object System.Collections.Generic.List[string]
        foreach ($n in @('wpndatabase.db', 'wpndatabase.db-wal')) {
            $src = Join-Path $dir $n
            if (-not (Test-Path -LiteralPath $src)) { continue }
            $dst = Join-Path $global:DiagOut ('sacreal-' + $Tag + '-' + $n)
            $inStream = New-Object System.IO.FileStream($src, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
            try {
                $outStream = New-Object System.IO.FileStream($dst, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write)
                try { $inStream.CopyTo($outStream) } finally { $outStream.Dispose() }
            } finally {
                $inStream.Dispose()
            }
            $copied.Add($n)
        }
        $res['copied'] = $copied.ToArray()
    } catch {
        $res['error'] = $_.Exception.Message
    }
    return $res
}

# =========================================================================
# step 4: the verdict matrix (SAC is ON)
# =========================================================================
function Invoke-MatrixPhase {
    $mx = [ordered]@{ started = (Get-IsoNow); language_mode = (Get-LangMode); subjects = 0; probe_pass = $null; evtx_rc = $null; event_note = $null; event_counts = $null; control_interpretation = $null; neg_flagged = $null; pos_unsigned_flagged = $null; ea_after_on_file = 'sacreal-ea-installed-after-on.json' }
    $RUN['matrix'] = $mx
    $subs = @(Get-MatrixSubjects)
    $mx['subjects'] = $subs.Count
    $probe = $CTX['probe']
    $mx['probe_pass'] = Invoke-ProbePass 'sacreal' $subs 300 $probe
    Save-Json 'sacreal-probe.json' $probe 6
    Save-Run
    Start-Sleep -Seconds 5
    $ex = Export-CIEvtx -SinceUtc $CTX['t_sac_on'] -FileName 'sacreal-ci-matrix.evtx'
    $mx['evtx_rc'] = $ex.rc
    $ev = Get-CIEvents -SinceUtc $CTX['t_sac_on']
    Save-EventsJson $ev 'sacreal-events-matrix.json'
    $mx['event_note'] = $ev.note
    $mx['event_counts'] = Get-IdCounts $ev.events
    $null = Save-BlockEvents $ev.events 'sacreal-3077.json'
    $mat = Build-Matrix $subs $ev.events $probe @{} 'enforce' 'sacreal'
    $CTX['matrix'] = $mat
    $mx['control_interpretation'] = $mat['control_interpretation']
    $mx['neg_flagged'] = $mat['neg_flagged']
    $mx['pos_unsigned_flagged'] = $mat['pos_unsigned_flagged']
    # same file, one title line more: this matrix is from the REAL SAC, not from the lookalike policy of run 1
    $txt = Read-TextUtf8 (Join-Path $global:DiagOut 'sacreal-matrix.txt')
    if ($txt) {
        $hdr = ('REAL Smart App Control ON (VerifiedAndReputablePolicyState=1 + CiTool -r) on {0}; switched on at {1}; probe = CreateProcessW(CREATE_SUSPENDED) + TerminateProcess (no code runs). Rows: 3 installers, 9 app exes, controls, installed {2} exes, runner node.exe (bundled runtime files are not part of this matrix).' -f $env:DIAG_MATRIX_OS, $RUN['sac_on_utc'], $K_VERSION_FROM)
        Save-Text 'sacreal-matrix.txt' ($hdr + "`r`n" + $txt)
    }
    # fsutil EA of the installed exes again (did ORIGINCLAIM / SMARTLOCKER EAs appear while SAC evaluated them?)
    $ea = [ordered]@{}
    foreach ($s in @($CTX['installed'])) { $ea[[string]$s.label] = Get-EaText ([string]$s.path) }
    Save-Json 'sacreal-ea-installed-after-on.json' $ea 4
    Save-Run
}

# If the runner's node.exe is blocked under SAC (it is signed, so this should not happen) use the signed node.exe of the cys
# runtime (control-signed subject) as the CDP driver instead. Returns '' = keep the node.exe that is found on PATH.
function Select-DriverNode {
    $runner = $CTX['runner_node']
    $rec = $null
    if (($null -ne $runner) -and $CTX['probe'].ContainsKey([string]$runner.label)) { $rec = $CTX['probe'][[string]$runner.label] }
    if (Test-ProbeBlocked $rec) {
        foreach ($s in @($CTX['subjects'])) {
            if ([string]$s.kind -ne 'control-signed') { continue }
            $r2 = $null
            if ($CTX['probe'].ContainsKey([string]$s.label)) { $r2 = $CTX['probe'][[string]$s.label] }
            if (($null -ne $r2) -and ($r2['ok'] -eq $true)) {
                $RUN['notes'].Add('runner node.exe is BLOCKED under SAC; the CDP driver uses the signed node.exe of the cys runtime instead: ' + [string]$s.path)
                $CTX['driver_node'] = [string]$s.path
                return [string]$s.path
            }
        }
        $RUN['notes'].Add('runner node.exe is BLOCKED under SAC and no allowed signed node.exe was found: the CDP driver cannot start')
    }
    return ''
}

# =========================================================================
# step 3 (+4): turn REAL SAC on. Returns $true when it is confirmed enforced.
# =========================================================================
function Invoke-SacOnPhase {
    $ph = [ordered]@{ started = (Get-IsoNow); language_mode = (Get-LangMode); set = $null; after_10s = $null; after_30s = $null; confirmed = $false }
    $RUN['sac_on'] = $ph
    $CTX['sac_phase_started'] = $true
    # everything measured so far is safe on a results branch BEFORE the policy changes
    $null = Publish-Checkpoint 'pre-sac'
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
        $RUN['notes'].Add('REAL Smart App Control could not be confirmed ON (registry / CiTool / Defender states: sacreal-state-after-on-*.json): measurable=false, nothing more is measured')
        Save-Run
        return $false
    }
    $RUN['measurable'] = $true
    Save-Run
    $null = Invoke-MatrixPhase
    # which node.exe may be started from now on (needs the matrix probe of the runner node): on every path, not only in the hook
    $null = Select-DriverNode
    return $true
}

# =========================================================================
# step 2 + the hook of scenario A
# =========================================================================
# attach over CDP and record check_update, WITHOUT calling install_update (cdp-update.mjs --mode attach)
function Invoke-PreSacAttach {
    $pre = [ordered]@{ started = (Get-IsoNow); node_exe = $null; rc = $null; attached = $false; app_version = $null; check_update = $null; check_update_error = $null; error_summary = $null; json_file = 'sacreal-e2e-A-pre-cdp.json'; error = $null }
    $CTX['A']['pre'] = $pre
    try {
        $nodeExe = Find-Exe 'node.exe'
        $pre['node_exe'] = $nodeExe
        if (-not $nodeExe) {
            $pre['error'] = 'node.exe not found'
            return $pre
        }
        $nodeScript = Join-Path $global:DiagRoot 'cdp-update.mjs'
        $nodeArgs = '"{0}" --mode attach --port {1} --out "{2}" --prefix sacreal-e2e-A-pre --max-wait-sec 120' -f $nodeScript, $K_PORT, $global:DiagOut
        $x = Invoke-Proc -File $nodeExe -Arguments $nodeArgs -TimeoutSec 360
        $pre['rc'] = $x.rc
        Save-Text 'sacreal-e2e-A-pre-cdp-stdout.txt' ((([string]$x.out) + "`r`n" + ([string]$x.err)))
        $txt = Read-TextUtf8 (Join-Path $global:DiagOut 'sacreal-e2e-A-pre-cdp.json')
        if ($txt) {
            $cj = $null
            try { $cj = ConvertFrom-Json $txt } catch { $pre['error'] = 'cdp json parse: ' + $_.Exception.Message }
            if ($null -ne $cj) {
                $pre['attached'] = [bool]$cj.attached
                $pre['app_version'] = $cj.app_version
                $pre['check_update'] = $cj.check_update
                $pre['check_update_error'] = $cj.check_update_error
                $pre['error_summary'] = $cj.error_summary
            } else {
                $pre['attached'] = [bool]($txt -match '"attached"\s*:\s*true')
            }
        } else {
            $pre['error'] = 'sacreal-e2e-A-pre-cdp.json missing'
        }
    } catch {
        $pre['error'] = 'exception: ' + $_.Exception.Message
    }
    return $pre
}

# Runs inside Invoke-AppUpdateRun after the app is started and the CDP port answers (-BeforeNode):
#   attach + check_update (no install_update) -> checkpoint -> SAC on + 3 states -> matrix -> driver node choice.
# Returns { proceed; why; node_exe }; proceed=$false makes Invoke-AppUpdateRun stop BEFORE install_update is called.
function Invoke-ABeforeNode {
    param($AppRunRecord)
    $res = [ordered]@{ proceed = $false; why = ''; node_exe = '' }
    try {
        Write-Log ('A: before-node step (language mode {0})' -f (Get-LangMode))
        # results are read from $CTX / $RUN, not from function return values (a stray pipeline object cannot flip a verdict)
        $null = Invoke-PreSacAttach
        $pre = $CTX['A']['pre']
        Save-Json 'sacreal-e2e-A-interim.json' $CTX['A'] 6
        if (($null -eq $pre) -or (-not $pre['attached'])) {
            $res['why'] = 'CDP did not attach to the app BEFORE Smart App Control was turned on (see sacreal-e2e-A-pre-cdp.json); SAC-on and the matrix run without scenario A'
            $CTX['A']['skipped_reason'] = $res['why']
            return $res
        }
        $null = Invoke-SacOnPhase
        if ($RUN['sac_on_confirmed'] -ne $true) {
            $res['why'] = 'Smart App Control could not be confirmed ON: install_update is not called (measurable=false)'
            $CTX['A']['skipped_reason'] = $res['why']
            return $res
        }
        # SAC must still be enforced right before the call (nothing may have switched it back while the matrix ran)
        $chkBefore = Get-SacRealState 'before-install-call'
        $CTX['A']['sac_enforced_at_install_call'] = [bool]$chkBefore['enforced']
        if (-not $chkBefore['enforced']) { $RUN['notes'].Add('SAC was NOT enforced any more right before install_update was called: the A result is not valid for SAC on') }
        $res['node_exe'] = [string]$CTX['driver_node']
        $CTX['t_before_node'] = (Get-Date).ToUniversalTime()
        # the timeline file is created by Watch-UpdateOutcome (it only appends): say in its first line what the situation is
        try { Add-Utf8NoBom (Join-Path $global:DiagOut 'sacreal-e2e-A-timeline.txt') (('# context: REAL Smart App Control is ON since ' + [string]$RUN['sac_on_utc'] + ' (enforced, confirmed); the app was started and CDP-attached BEFORE that; install_update (force=true) is called next') + "`r`n") } catch { }
        $res['proceed'] = $true
        Save-Json 'sacreal-e2e-A-interim.json' $CTX['A'] 6
    } catch {
        $res['proceed'] = $false
        $res['why'] = 'exception in the before-node step: ' + $_.Exception.Message
        $CTX['A']['skipped_reason'] = $res['why']
        Add-DiagError 'A before-node step' $_
    }
    return $res
}

# =========================================================================
# step 5: the verdict file of scenario A
# =========================================================================
# the file(s) the updater plugin handed to ShellExecuteW: %TEMP%\cys-<ver>-updater-<rand>\cys-<ver>-installer.exe
function Get-UpdaterInstallerFiles {
    $res = New-Object System.Collections.Generic.List[string]
    try {
        $tmp = [System.IO.Path]::GetTempPath()
        foreach ($d in @(Get-ChildItem -LiteralPath $tmp -Directory -Filter 'cys-*-updater-*' -ErrorAction SilentlyContinue)) {
            foreach ($f in @(Get-ChildItem -LiteralPath $d.FullName -File -Filter '*.exe' -ErrorAction SilentlyContinue)) { $res.Add([string]$f.FullName) }
        }
    } catch { }
    return $res.ToArray()
}

function Save-AVerdict {
    $aInfo = $CTX['A']
    $appRun = $CTX['A_run']
    # was install_update really CALLED after SAC was confirmed on? (the hook finishing is not enough: the driver node must have called it)
    $called = $false
    if ($null -ne $appRun) {
        if ($appRun['install_update_outcome']) { $called = $true }
        elseif (($null -ne $appRun['cdp']) -and ($appRun['cdp'].install_update_started_at)) { $called = $true }
    }
    $valid = [bool](($RUN['sac_on_confirmed'] -eq $true) -and ($null -ne $CTX['t_before_node']) -and ($aInfo['sac_enforced_at_install_call'] -ne $false) -and $called)
    $reason = [string]$aInfo['skipped_reason']
    if ((-not $reason) -and ($null -eq $appRun)) { $reason = 'scenario A was not started (baseline install failed or not enough job time)' }
    $av = [ordered]@{
        scenario = 'A'
        description = ('installed ' + $K_VERSION_FROM + ' was started and CDP-attached BEFORE Smart App Control was turned on; install_update {force:true} was called AFTER it was on, on the already running app')
        os = $env:DIAG_MATRIX_OS
        install_update_driven_with_sac_on = $valid
        sac_on_confirmed = $RUN['sac_on_confirmed']
        sac_on_utc = $RUN['sac_on_utc']
        language_mode = (Get-LangMode)
        pre_sac = $aInfo['pre']
        verdict = 'not_run'
        why = $reason
        cdp_attached = $null
        check_update_before_sac_on = $null
        check_update_after_sac_on = $null
        install_update_outcome = $null
        app_exited_at = $null
        installer_seen = $null
        installer_first_seen = $null
        app_relaunched = $null
        version_after = $null
        end_reason = $null
        screenshots = @()
        temp_installer = $null
        events_since_install_call = $null
        blocked_files_since_sac_on = $null
        notification_db = $null
        run = $null
    }
    if ($aInfo['pre'] -is [System.Collections.IDictionary]) { $av['check_update_before_sac_on'] = $aInfo['pre']['check_update'] }
    if ($null -ne $appRun) {
        $av['run'] = $appRun
        $av['verdict'] = $appRun['verdict']
        $av['why'] = $appRun['why']
        $av['cdp_attached'] = $appRun['cdp_attached']
        $av['check_update_after_sac_on'] = $appRun['check_update']
        $av['install_update_outcome'] = $appRun['install_update_outcome']
        $av['app_exited_at'] = $appRun['app_exited_at']
        $av['installer_seen'] = $appRun['installer_seen']
        $av['app_relaunched'] = $appRun['app_relaunched']
        $av['version_after'] = $appRun['version_after']
        $ob = $appRun['observe']
        if ($null -ne $ob) {
            $av['end_reason'] = $ob['end_reason']
            $av['installer_first_seen'] = $ob['installer_first_seen']
            $shots = New-Object System.Collections.Generic.List[string]
            foreach ($sr in @($ob['screenshots'])) {
                if ($null -ne $sr) { $shots.Add(('{0} ok={1} windows={2}' -f $sr['name'], $sr['ok'], (Limit-Text $sr['windows'] 300))) }
            }
            $av['screenshots'] = $shots.ToArray()
        }
    }
    try {
        if (($null -ne $CTX['t_sac_on']) -and ($null -ne $CTX['t_before_node'])) {
            $av['seconds_between_sac_on_and_install_update_call'] = [int](($CTX['t_before_node'] - $CTX['t_sac_on']).TotalSeconds)
        }
    } catch { }
    Save-Json 'sacreal-e2e-A-verdict.json' $av 9
    # the extras below touch many files and the event log: the verdict above is already on disk if one of them fails.
    # The events come FIRST: the CreateProcessW probe of the temp installer below is itself a blocked start (a 3077 whose
    # process is powershell.exe) and must not be mistaken for the app's own attempt.
    try {
        if ($CTX['t_before_node']) {
            $ev = Get-CIEvents -SinceUtc $CTX['t_before_node']
            $bo = Save-BlockEvents $ev.events 'sacreal-e2e-A-events.json'
            $compact = New-Object System.Collections.Generic.List[object]
            foreach ($b in @($bo['block_events_3076_3077'])) {
                $pj = (@($b['file_paths']) -join ' ')
                if ($pj -match '(?i)updater|installer') {
                    $compact.Add([ordered]@{ id = $b['id']; time = $b['time']; policy_names = $b['policy_names']; file_paths = $b['file_paths']; process_names = $b['process_names']; status = $b['status'] })
                }
            }
            $av['events_since_install_call'] = [ordered]@{ note = $ev.note; counts_by_id = $bo['counts_by_id']; blocked_events_naming_updater_or_installer = $compact.ToArray(); file = 'sacreal-e2e-A-events.json' }
        }
    } catch {
        $av['events_since_install_call'] = 'failed: ' + $_.Exception.Message
    }
    try {
        if ($CTX['t_sac_on']) {
            $ev2 = Get-CIEvents -SinceUtc $CTX['t_sac_on']
            $av['blocked_files_since_sac_on'] = Get-BlockedByFile $ev2.events
        }
    } catch {
        $av['blocked_files_since_sac_on'] = 'failed: ' + $_.Exception.Message
    }
    try {
        $ti = New-Object System.Collections.Generic.List[object]
        $asset = Get-SubjectByLabel 'installer 0.14.42'
        foreach ($f in @(Get-UpdaterInstallerFiles)) {
            $rec = [ordered]@{ path = $f; size = $null; sha256 = $null; same_sha256_as_asset_0_14_42 = $null; ea = $null; createprocess_probe = $null }
            try { $rec['size'] = (Get-Item -LiteralPath $f).Length } catch { }
            $rec['sha256'] = Get-Sha256 $f
            if ($null -ne $asset) { $rec['same_sha256_as_asset_0_14_42'] = ([string]$rec['sha256'] -eq [string]$asset.sha256) }
            $rec['ea'] = Get-EaText $f
            # SAC is still on here: this is what ShellExecuteW got for the very same file (no code runs)
            $rec['createprocess_probe'] = Invoke-ProbeCreate $f
            $ti.Add($rec)
        }
        $av['temp_installer'] = $ti.ToArray()
    } catch {
        $av['temp_installer'] = 'failed: ' + $_.Exception.Message
    }
    $av['notification_db'] = Save-NotificationDb 'A'
    $av['finished'] = (Get-IsoNow)
    $CTX['A_verdict'] = $av
    Save-Json 'sacreal-e2e-A-verdict.json' $av 9
}

# =========================================================================
# step 6: scenario B -- start the installed app for real while SAC is ON
# =========================================================================
function Save-BJson { Save-Json 'sacreal-e2e-B.json' $CTX['B'] 8 }

# <exe> --version: CreateProcessW probe (no code) + a real run. The probe record carries the native error (4551?).
function Test-ExeVersion {
    param([string]$Exe)
    $r = [ordered]@{ exe = $Exe; exists = (Test-Path -LiteralPath $Exe); createprocess_probe = $null; probe_blocked = $null; run = $null }
    if (-not $r['exists']) { return $r }
    $r['createprocess_probe'] = Invoke-ProbeCreate $Exe
    $r['probe_blocked'] = [bool](Test-ProbeBlocked $r['createprocess_probe'])
    $x = Invoke-Proc -File $Exe -Arguments '--version' -TimeoutSec 30 -WorkDir (Split-Path -Parent $Exe)
    $r['run'] = [ordered]@{ rc = $x.rc; timed_out = $x.timedOut; start_error = $x.startError; stdout = (Limit-Text $x.out 2000); stderr = (Limit-Text $x.err 2000); ms = $x.ms }
    try { $null = Stop-ProcessesUnder (Split-Path -Parent $Exe) } catch { }
    return $r
}

function Invoke-ScenarioB {
    $bInfo = [ordered]@{
        scenario = 'B'
        description = 'after scenario A the leftover cys processes were killed (by install folder path); then the installed cys-app.exe is started for real (Start-Process) while Smart App Control is ON'
        started = (Get-IsoNow); language_mode = (Get-LangMode); install_dir = (Get-InstallDir)
        leftovers_killed = $null; processes_after_cleanup = $null; createprocess_probe = $null
        start = $null; samples = $null; after_60s = $null; cdp = $null
        webview2_policy = $null; webview2_policy_removed = $null; killed_at_end = $null
        cysd_version = $null; cys_version = $null; exception = $null; finished = $null
    }
    $CTX['B'] = $bInfo
    $dir = Get-InstallDir
    $exe = Join-Path $dir 'cys-app.exe'
    $startedUtc = (Get-Date).ToUniversalTime()
    $polState = $null
    $proc = $null
    try {
        $bInfo['leftovers_killed'] = @(Stop-ProcessesUnder $dir)
        Start-Sleep -Seconds 2
        $bInfo['processes_after_cleanup'] = @(Get-InterestingProcesses | ForEach-Object { ('{0}#{1}' -f $_.name, $_.id) })
        Save-BJson
        if (-not (Test-Path -LiteralPath $exe)) {
            $bInfo['start'] = [ordered]@{ ok = $false; skipped = 'cys-app.exe is not installed' }
        } else {
            # no code runs here: the image check of CreateProcessW says in advance what the real start will meet
            $bInfo['createprocess_probe'] = Invoke-ProbeCreate $exe
            # CDP flag + HKLM policy again (A removed it): if the app does run, we can see whether CDP attaches
            $polState = Set-WebView2DebugPolicy -Port $K_PORT
            $bInfo['webview2_policy'] = $polState
            $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = ('--remote-debugging-port={0} --remote-allow-origins=*' -f $K_PORT)
            try {
                $proc = Start-Process -FilePath $exe -WorkingDirectory $dir -PassThru -ErrorAction Stop
                $bInfo['start'] = [ordered]@{ ok = $true; pid = $proc.Id; error = $null; time = (Get-IsoNow); handle_note = $null }
                # holding the handle keeps ExitCode readable; it can throw for a process that is already gone: that is NOT a start failure
                try { $null = $proc.Handle } catch { $bInfo['start']['handle_note'] = 'handle access failed: ' + $_.Exception.Message }
            } catch {
                $bInfo['start'] = [ordered]@{ ok = $false; pid = $null; error = (Get-NativeErrorInfo $_); time = (Get-IsoNow) }
                try {
                    $pl = $bInfo['createprocess_probe']
                    if ($pl -is [System.Collections.IDictionary]) {
                        $bInfo['start']['createprocess_probe_last_error'] = $pl['last_error']
                        $bInfo['start']['createprocess_probe_is_4551'] = ($pl['last_error'] -eq 4551)
                    }
                } catch { }
            }
            Remove-Item -Path 'Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS' -ErrorAction SilentlyContinue
            Save-BJson
            if ($bInfo['start']['ok']) {
                $sw = [System.Diagnostics.Stopwatch]::StartNew()
                $samples = New-Object System.Collections.Generic.List[object]
                $cdpHttp = $null
                while ($sw.Elapsed.TotalSeconds -lt 60) {
                    Start-Sleep -Seconds 5
                    $alive = $null
                    try { $alive = (-not $proc.HasExited) } catch { }
                    $plist = @(Get-InterestingProcesses | ForEach-Object { ('{0}#{1}' -f $_.name, $_.id) })
                    $samples.Add([ordered]@{ t = [int]$sw.Elapsed.TotalSeconds; started_process_alive = $alive; processes = ($plist -join ' ') })
                    if ($null -eq $cdpHttp) {
                        try {
                            $resp = Invoke-WebRequest -Uri ('http://127.0.0.1:{0}/json/version' -f $K_PORT) -UseBasicParsing -TimeoutSec 3 -ErrorAction Stop
                            if ($resp.StatusCode -eq 200) { $cdpHttp = [string]$resp.Content }
                        } catch { }
                    }
                }
                $bInfo['samples'] = $samples.ToArray()
                # state after 60 s: processes, windows, screenshot
                $a60 = [ordered]@{ started_process_alive = $null; started_process_exit_code = $null; interesting_processes = $null; windows = $null; screenshot = $null }
                try { $a60['started_process_alive'] = (-not $proc.HasExited) } catch { }
                try { if ($proc.HasExited) { $a60['started_process_exit_code'] = $proc.ExitCode } } catch { }
                $a60['interesting_processes'] = @(Get-InterestingProcesses | ForEach-Object { [ordered]@{ name = $_.name; id = $_.id; ppid = $_.ppid; path = $_.path } })
                $a60['windows'] = Get-VisibleWindowsText
                $a60['screenshot'] = Save-Screenshot 'sacreal-e2e-B-screen-60s.png'
                Save-Text 'sacreal-e2e-B-processes.txt' (Get-ProcessListText)
                $bInfo['after_60s'] = $a60
                # does CDP attach? (node --mode version: attach + app version)
                $cdpInfo = [ordered]@{ json_version_http = $cdpHttp; node_exe = $null; node_rc = $null; attached = $null; app_version = $null }
                if ($null -ne $cdpHttp) {
                    $nodeForB = [string]$CTX['driver_node']
                    if (-not $nodeForB) { $nodeForB = Find-Exe 'node.exe' }
                    $cdpInfo['node_exe'] = $nodeForB
                    if ($nodeForB) {
                        $vArgs = '"{0}" --mode version --port {1} --out "{2}" --prefix sacreal-e2e-B' -f (Join-Path $global:DiagRoot 'cdp-update.mjs'), $K_PORT, $global:DiagOut
                        $vx = Invoke-Proc -File $nodeForB -Arguments $vArgs -TimeoutSec 90
                        $cdpInfo['node_rc'] = $vx.rc
                        $aj = Read-TextUtf8 (Join-Path $global:DiagOut 'sacreal-e2e-B-cdp-after.json')
                        if ($aj) {
                            try {
                                $cjb = ConvertFrom-Json $aj
                                $cdpInfo['attached'] = [bool]$cjb.attached
                                $cdpInfo['app_version'] = $cjb.app_version
                            } catch { $cdpInfo['attached'] = [bool]($aj -match '"attached"\s*:\s*true') }
                        }
                    }
                }
                $bInfo['cdp'] = $cdpInfo
                try { Save-AppEventLog 'sacreal-e2e-B' $startedUtc } catch { }
            }
        }
    } catch {
        $bInfo['exception'] = Format-ErrorText $_
        Add-DiagError 'scenario B' $_
    } finally {
        Remove-Item -Path 'Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS' -ErrorAction SilentlyContinue
        try { if ($null -ne $polState) { $bInfo['webview2_policy_removed'] = Remove-WebView2DebugPolicy $polState } } catch { }
        try { $bInfo['killed_at_end'] = @(Stop-ProcessesUnder $dir) } catch { }
    }
    Save-BJson
    # then the two other binaries of the install with --version
    try {
        $bInfo['cysd_version'] = Test-ExeVersion (Join-Path $dir 'cysd.exe')
        Save-BJson
        $bInfo['cys_version'] = Test-ExeVersion (Join-Path $dir 'cys.exe')
    } catch {
        $bInfo['exception'] = ([string]$bInfo['exception'] + ' | version tests: ' + $_.Exception.Message)
    }
    $bInfo['finished'] = (Get-IsoNow)
    Save-BJson
}

# =========================================================================
# step 7: events of the whole SAC-on window, then the summary
# =========================================================================
function Save-FinalEvents {
    if (-not $CTX['t_sac_on']) { return }
    # this runs LAST (after the restore): when the job budget is nearly gone it is skipped so that the summary and the publish steps still fit
    if ((Get-MinutesLeft) -lt 2) {
        $RUN['notes'].Add('final event collection skipped: job time budget nearly exhausted (the SAC window events of the matrix and of scenario A were saved earlier)')
        return
    }
    Start-Sleep -Seconds 2
    $fe = [ordered]@{ started = (Get-IsoNow); evtx_rc = $null; note = $null; counts_by_id = $null; matrix_interpretation = $null }
    $RUN['final_events'] = $fe
    $ex = Export-CIEvtx -SinceUtc $CTX['t_sac_on'] -FileName 'sacreal-ci-final.evtx'
    $fe['evtx_rc'] = $ex.rc
    $fe['evtx_note'] = 'the evtx holds everything since SAC was switched on, including the policy refresh events of the restore'
    $evAll = Get-CIEvents -SinceUtc $CTX['t_sac_on']
    # the JSON files and the matrix cover the SAC-on window only: events from the start of the restore on are counted, not listed
    $cut = $null
    if ($CTX['t_restore_start']) { $cut = ConvertTo-IsoUtc $CTX['t_restore_start'] }
    $inWindow = New-Object System.Collections.Generic.List[object]
    $afterRestoreStart = 0
    foreach ($e in @($evAll.events)) {
        if (($null -ne $cut) -and ([string]::CompareOrdinal([string]$e.time, [string]$cut) -ge 0)) { $afterRestoreStart++ } else { $inWindow.Add($e) }
    }
    $fe['events_from_restore_start_left_out'] = $afterRestoreStart
    $ev = [pscustomobject]@{ events = $inWindow.ToArray(); note = $evAll.note }
    Save-EventsJson $ev 'sacreal-events-final.json'
    $fe['note'] = $ev.note
    $fe['counts_by_id'] = Get-IdCounts $ev.events
    $null = Save-BlockEvents $ev.events 'sacreal-3077-final.json'
    $subs = @(Get-MatrixSubjects)
    $fm = Build-Matrix $subs $ev.events $CTX['probe'] @{} 'enforce' 'sacreal-final'
    $fe['matrix_interpretation'] = $fm['control_interpretation']
    $fe['finished'] = (Get-IsoNow)
}

function Get-KindSummary {
    param($Rows, [string[]]$Kinds)
    $n = 0
    $blocked = 0
    $unprobed = 0
    $codes = New-Object System.Collections.Generic.List[string]
    foreach ($r0 in @($Rows)) {
        if ($Kinds -contains [string]$r0['kind']) {
            $n++
            if (@('-', 'n/a') -contains [string]$r0['probe1']) { $unprobed++ }
            if (Test-ProbeBlocked $r0['probe1_detail']) {
                $blocked++
                if (-not $codes.Contains([string]$r0['probe1'])) { $codes.Add([string]$r0['probe1']) }
            }
        }
    }
    return [ordered]@{ total = $n; blocked = $blocked; allowed = ($n - $blocked - $unprobed); unprobed = $unprobed; codes = ($codes.ToArray() -join ',') }
}

function Format-KindCount {
    param($K)
    $t = ('{0}/{1}' -f $K['blocked'], $K['allowed'])
    if ([int]$K['unprobed'] -gt 0) { $t = $t + (' +{0} unprobed' -f $K['unprobed']) }
    return $t
}

# at most 10 human readable lines
function Write-Summary {
    $lines = New-Object System.Collections.Generic.List[string]
    $nowIso = Get-IsoNow
    $lines.Add(('sacreal-e2e on {0}: measurable={1}; SAC switched on at {2}; run {3} .. {4}' -f $env:DIAG_MATRIX_OS, $RUN['measurable'], $RUN['sac_on_utc'], $RUN['started'], $nowIso))
    # 2: SAC on confirmed?
    $onState = $null
    foreach ($tag in @('after-on-30s', 'after-on-10s')) {
        if ($null -eq $onState) { if ($RUN['states'].Contains($tag)) { $onState = $RUN['states'][$tag] } }
    }
    if ($null -ne $onState) {
        $dp = $onState['citool']['desktop_policy']
        $dpEnforced = $null
        if ($null -ne $dp) { $dpEnforced = $dp['is_enforced'] }
        $mpOn = $onState['signals']['defender_on']
        $mpText = $mpOn
        if ($onState['defender'] -is [System.Collections.IDictionary]) { $mpText = $onState['defender']['SmartAppControlState'] }
        $lines.Add(('SAC on confirmed={0}: registry value={1}; CiTool VerifiedAndReputableDesktop IsEnforced={2}; Defender SmartAppControlState={3}; UMCI flag={4}' -f $RUN['sac_on_confirmed'], $onState['registry']['value'], $dpEnforced, $mpText, $onState['signals']['umci_flag']))
    } else {
        $lines.Add('SAC on: the switch was never made (see sacreal-e2e.json notes / steps)')
    }
    # 3-4: matrix
    $mat = $CTX['matrix']
    if ($null -ne $mat) {
        $rows = @($mat['rows'])
        $kIns = Get-KindSummary $rows @('installer')
        $kApp = Get-KindSummary $rows @('app-main')
        $kInst = Get-KindSummary $rows @('installed-' + $K_VERSION_FROM)
        $kNeg = Get-KindSummary $rows @('control-neg')
        $kPosU = Get-KindSummary $rows @('control-pos-unsigned')
        $kPosS = Get-KindSummary $rows @('control-signed')
        $kPosR = Get-KindSummary $rows @('control-signed-runner')
        $lines.Add(('matrix (CreateProcessW probes; blocked/allowed): installers {0} [{1}]; app exes {2} [{3}]; installed {4} exes {5} [{6}]' -f (Format-KindCount $kIns), $kIns['codes'], (Format-KindCount $kApp), $kApp['codes'], $K_VERSION_FROM, (Format-KindCount $kInst), $kInst['codes']))
        $lines.Add(('controls (blocked/allowed): NEG {0}; POS-UNSIGNED {1}; POS-SIGNED (cys runtime node) {2}; runner node.exe {3}; {4}' -f (Format-KindCount $kNeg), (Format-KindCount $kPosU), (Format-KindCount $kPosS), (Format-KindCount $kPosR), $mat['control_interpretation']))
    } else {
        $lines.Add('matrix: not produced (SAC was not confirmed on, or the matrix step failed)')
    }
    # 5-6: scenario A
    $av = $CTX['A_verdict']
    if ($null -ne $av) {
        $markerAfter = $null
        if ($av['version_after'] -is [System.Collections.IDictionary]) { $markerAfter = $av['version_after']['marker'] }
        $lines.Add(('A (app running before SAC, install_update after; driven with SAC on={0}): verdict={1}; outcome={2}; installer process seen={3}; app exited at {4}; version marker after={5}; app relaunched={6}' -f $av['install_update_driven_with_sac_on'], $av['verdict'], $av['install_update_outcome'], $av['installer_seen'], $av['app_exited_at'], $markerAfter, $av['app_relaunched']))
        $tiProbe = 'n/a'
        $tiList = @($av['temp_installer'])
        if (($tiList.Count -gt 0) -and ($tiList[0] -is [System.Collections.IDictionary])) {
            $tp = $tiList[0]['createprocess_probe']
            if ($tp -is [System.Collections.IDictionary]) { $tiProbe = ('ok={0} last_error={1}' -f $tp['ok'], $tp['last_error']) }
        }
        $evCount = 'n/a'
        if ($av['events_since_install_call'] -is [System.Collections.IDictionary]) { $evCount = @($av['events_since_install_call']['blocked_events_naming_updater_or_installer']).Count }
        $lines.Add(('A details: temp installer probe now: {0}; blocked CodeIntegrity events naming the updater/installer file: {1}; screenshots: {2}; why: {3}' -f $tiProbe, $evCount, @($av['screenshots']).Count, $av['why']))
    } else {
        $lines.Add('A: no verdict (scenario A did not run)')
    }
    # 7-8: scenario B
    $bi = $CTX['B']
    if ($null -ne $bi) {
        $stRec = $bi['start']
        $startOk = $null
        $errText = ''
        if ($stRec -is [System.Collections.IDictionary]) {
            $startOk = $stRec['ok']
            if ($stRec['error'] -is [System.Collections.IDictionary]) {
                $is4551 = [bool]($stRec['error']['is_4551'] -or ($stRec['createprocess_probe_is_4551'] -eq $true))
                $errText = ('Start-Process failed: is 4551={0} (native error code {1}, CreateProcessW probe last_error {2}): {3}' -f $is4551, $stRec['error']['native_error_code'], $stRec['createprocess_probe_last_error'], $stRec['error']['message'])
            }
            elseif ($stRec['skipped']) { $errText = [string]$stRec['skipped'] }
        }
        $alive60 = $null
        $cdpAttached = $null
        if ($bi['after_60s'] -is [System.Collections.IDictionary]) { $alive60 = $bi['after_60s']['started_process_alive'] }
        if ($bi['cdp'] -is [System.Collections.IDictionary]) { $cdpAttached = $bi['cdp']['attached'] }
        $lines.Add(('B (installed cys-app.exe started with Start-Process under SAC): started={0}; {1}; alive after 60 s={2}; CDP attached={3}' -f $startOk, $errText, $alive60, $cdpAttached))
        $dRun = $null
        $cRun = $null
        if ($bi['cysd_version'] -is [System.Collections.IDictionary]) { $dRun = $bi['cysd_version']['run'] }
        if ($bi['cys_version'] -is [System.Collections.IDictionary]) { $cRun = $bi['cys_version']['run'] }
        $dText = 'not run'
        $cText = 'not run'
        if ($dRun -is [System.Collections.IDictionary]) { $dText = ('rc={0} startError={1}' -f $dRun['rc'], (Limit-Text $dRun['start_error'] 120)) }
        if ($cRun -is [System.Collections.IDictionary]) { $cText = ('rc={0} startError={1}' -f $cRun['rc'], (Limit-Text $cRun['start_error'] 120)) }
        $lines.Add(('B: cysd.exe --version: {0}; cys.exe --version: {1}' -f $dText, $cText))
    } else {
        $lines.Add('B: did not run (SAC not confirmed on, or not enough job time)')
    }
    # 9: restore
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
        $lines.Add('restore: not recorded (see sacreal-e2e.json)')
    }
    # 10: notes
    if ($RUN['notes'].Count -gt 0) { $lines.Add(('notes ({0}): {1}' -f $RUN['notes'].Count, (($RUN['notes'].ToArray()) -join ' | '))) }
    $out = New-Object System.Collections.Generic.List[string]
    foreach ($l in $lines) {
        $one = [regex]::Replace([string]$l, '[\r\n]+', ' ')
        $out.Add((Limit-Text $one 700))
    }
    Save-Text 'sacreal-summary.txt' (($out.ToArray()) -join "`r`n")
    foreach ($l in $out) { Write-Log ('SUMMARY ' + $l) }
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
        try { $null = Invoke-WebRequest -Uri 'http://127.0.0.1:1/' -UseBasicParsing -TimeoutSec 1 -ErrorAction Stop } catch { }
        try { $null = Get-Command Get-MpComputerStatus -ErrorAction SilentlyContinue } catch { }
        try { $null = Get-CIEvents -SinceUtc ((Get-Date).ToUniversalTime().AddSeconds(-1)) -Max 5 } catch { }
        try { $null = Get-VisibleWindowsText } catch { }
        try { $null = Get-ChildItem -LiteralPath ([System.IO.Path]::GetTempPath()) -Directory -ErrorAction SilentlyContinue | Select-Object -First 1 } catch { }
        $prep['language_mode'] = Get-LangMode
        try {
            $nt = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -ErrorAction Stop
            $prep['os_product'] = ('{0} {1} build {2}.{3}' -f $nt.ProductName, $nt.DisplayVersion, $nt.CurrentBuild, $nt.UBR)
        } catch { }
        Initialize-Subjects
        $prep['subjects_loaded'] = @($CTX['subjects']).Count
        # the CodeIntegrity/Operational log is 1 MB by default: raise it to 128 MB before the SAC window
        $we = Join-Path $env:windir 'System32\wevtutil.exe'
        $g1 = Invoke-Proc -File $we -Arguments 'gl Microsoft-Windows-CodeIntegrity/Operational' -TimeoutSec 30
        $sl = Invoke-Proc -File $we -Arguments 'sl Microsoft-Windows-CodeIntegrity/Operational /ms:134217728' -TimeoutSec 30
        $prep['ci_log_size'] = [ordered]@{ before = $g1.out; set_rc = $sl.rc; set_err = $sl.err }
        # baseline: SAC must be OFF now (value 0, nothing enforced)
        $init = Get-SacRealState 'initial'
        $RUN['sac_initially_enforced'] = $init['enforced']
        if ($init['enforced']) { $RUN['notes'].Add('SAC was ALREADY enforced at the start of the run: the SAC-off baseline premise does not hold') }
        $CTX['runner_node'] = Get-RunnerNodeSubject
        if ($null -ne $CTX['runner_node']) { $prep['runner_node'] = $CTX['runner_node'] }
        Save-Run
    }

    Invoke-Step 'install-0.14.37' {
        $ins = [ordered]@{}
        $RUN['install'] = $ins
        $res = Install-CysVersion -Version $K_VERSION_FROM
        $ins['result'] = $res
        $ins['marker_ok'] = ([string]$res['marker'] -eq $K_VERSION_FROM)
        $CTX['install_ok'] = [bool]$ins['marker_ok']
        $ins['sac_registry_value_during_install'] = $RUN['states']['initial']['registry']['value']
        $left = @(Get-ProcessesUnder (Get-InstallDir) | ForEach-Object { ('{0}#{1}' -f $_.Name, $_.Id) })
        $ins['processes_under_install_dir_after_kill'] = $left
        if ($left.Count -gt 0) { $ins['killed_second_pass'] = @(Stop-ProcessesUnder (Get-InstallDir)) }
        $CTX['installed'] = @(Get-InstalledSubjects)
        $eaInfo = [ordered]@{}
        foreach ($s in @($CTX['installed'])) {
            $eaInfo[[string]$s.label] = [ordered]@{ path = $s.path; size = $s.size; sha256 = $s.sha256; signature = $s.sig; ea = (Get-EaText ([string]$s.path)) }
        }
        Save-Json 'sacreal-ea-installed-before.json' $eaInfo 4
        $ins['ea_file'] = 'sacreal-ea-installed-before.json'
        # are the installed files the same bytes as the extracted subjects (same ISG hash verdict expected)?
        $same = [ordered]@{}
        foreach ($s in @($CTX['installed'])) {
            $leaf = ([string]$s.label).Substring(('installed ' + $K_VERSION_FROM + ' ').Length)
            $twin = Get-SubjectByLabel ('app ' + $K_VERSION_FROM + ' ' + $leaf)
            if ($null -ne $twin) { $same[[string]$s.label] = ([string]$s.sha256 -eq [string]$twin.sha256) }
        }
        $ins['same_sha256_as_extracted_subject'] = $same
        if (-not $CTX['install_ok']) { $RUN['notes'].Add('baseline install of ' + $K_VERSION_FROM + ' failed (marker=' + [string]$res['marker'] + '): scenario A is skipped; the SAC-on matrix still runs and B tries whatever is installed') }
    }

    # scenario A: the app is started and attached while SAC is off; the BeforeNode hook turns SAC on and measures the matrix;
    # then Invoke-AppUpdateRun drives install_update and observes (timeline every 2 s, a screenshot right after the exit, at 20 s, 60 s ...)
    if ($CTX['install_ok'] -and ((Get-MinutesLeft) -gt 12)) {
        Invoke-Step 'A' {
            $appRun = Invoke-AppUpdateRun -Prefix 'sacreal-e2e-A' -Port $K_PORT -StartupWaitSec 120 -CdpMaxSec 240 -PostNodeSec 90 -TargetVersion $K_VERSION_TO -BeforeNode { param($AppRunRecord) Invoke-ABeforeNode $AppRunRecord } -WatchTickSec 2 -ShotAtExit -WatchShotEverySec 20
            $CTX['A_run'] = $appRun
        }
    } else {
        $CTX['A']['skipped_reason'] = 'baseline install failed or not enough job time left'
    }

    # SAC-on + matrix on their own when the hook never ran (install failed, the app or CDP did not come up before SAC)
    if (-not $CTX['sac_phase_started']) {
        if ((Get-MinutesLeft) -gt 8) {
            Invoke-Step 'sac-on-without-scenario-A' { $null = Invoke-SacOnPhase }
        } else {
            $RUN['notes'].Add('SAC-on phase skipped: not enough job time left')
        }
    }

    Invoke-Step 'A-verdict' { Save-AVerdict }
    # SAC is still on: everything measured so far goes to a results branch before scenario B
    if ($RUN['measurable'] -eq $true) {
        Invoke-Step 'checkpoint-after-A' { $null = Publish-Checkpoint 'after-A' }
    }

    if (($RUN['measurable'] -eq $true) -and ((Get-MinutesLeft) -gt 8)) {
        Invoke-Step 'B' { Invoke-ScenarioB }
    } else {
        $RUN['notes'].Add('scenario B skipped (SAC not confirmed on, or not enough job time left)')
    }
} catch {
    Add-DiagError 'sacreal-e2e main' $_
} finally {
    # The RESTORE comes first, whatever happened above: the CodeIntegrity events of the SAC window are read from the persisted log
    # afterwards (nothing is lost) and SAC must not stay on while that export runs. (Two independent reviews asked for this order.)
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
    # the very last resort also works in Constrained Language Mode (external commands only); the workflow step is the net below it
    if ($script:SacTouched) {
        try { $RUN['restore_cmd_fallback'] = Invoke-CmdRestoreFallback } catch { }
    }
    Invoke-Step 'final-events' { Save-FinalEvents }
    Invoke-Step 'summary' { Write-Summary }
    $RUN['finished'] = (Get-IsoNow)
    Save-Run
    Complete-DiagScript 'sacreal-e2e'
}
exit 0
