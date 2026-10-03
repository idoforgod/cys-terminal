# diag/sacrules.ps1 -- QB / QC / QD in ONE PowerShell process (an enforced policy turns every NEW PowerShell
# into Constrained Language Mode, so everything that needs Add-Type / new scripts is loaded BEFORE deployment).
#   QB: what is blocked under "Smart App Control like" rules  (audit events 3076, enforce probes/3077)
#   QC: what the caller sees when blocked (CreateProcessW GetLastError, ShellExecuteW ret, ShellExecuteExW)
#   QD: install 0.14.37 under AUDIT, then switch to ENFORCE (= consumer 'evaluation -> on'); does the app start,
#       and what happens when the update button is pressed (reproduction of the report)
# The policy is built from %windir%\schemas\CodeIntegrity\ExamplePolicies\SmartAppControl.xml with the option
# 'Enabled:Conditional Windows Lockdown Policy' removed (Microsoft: usable as a normal App Control policy).
# This script OBSERVES only. finally{} always tries: collect events, remove the policy.
$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')
. (Join-Path $PSScriptRoot 'e2e-update.ps1')
Start-DiagScript -Name 'sacrules'

$POLICY_NAME = 'DIAG-SAC-RULES'
$citool = Join-Path $env:windir 'System32\CiTool.exe'

$RUN = [ordered]@{
    started = (Get-IsoNow)
    matrix_os = $env:DIAG_MATRIX_OS
    measurable = $true
    policy = $null
    t0_audit_deploy_utc = $null
    t1_enforce_deploy_utc = $null
    stages = [ordered]@{}
    qc = [ordered]@{}
    qd = $null
    cleanup = [ordered]@{}
    checkpoints = [ordered]@{}
    notes = (New-Object System.Collections.Generic.List[string])
}
# GITHUB_TOKEN is only for the checkpoint uploads: keep it in a variable and take it out of the process environment so that
# no child process (the unsigned app under test, installers, ...) inherits it; it is handed to node for the upload only.
$script:GhToken = [string]$env:GITHUB_TOKEN
Remove-Item -Path 'Env:\GITHUB_TOKEN' -ErrorAction SilentlyContinue

$ST = @{}
$ST['subjects'] = @()
$ST['probe_a1'] = @{}
$ST['probe_a2'] = @{}
$ST['probe_e'] = @{}
$ST['t0'] = $null
$ST['t1'] = $null
$ST['policy_ok'] = $false
$ST['deploy_attempted'] = $false
$ST['audit_events_done'] = $false
$ST['policy'] = $null

function Save-Run { Save-Json 'sac-run.json' $RUN 9 }

# Upload what we have so far to a results branch (diag-results/<run>-<attempt>-sacrules-<tag>-<os>) with the same
# Node script the publish step uses. Insurance against an enforced policy killing the runner: the audit results are
# already safe when enforcement starts. Needs GITHUB_TOKEN in this step's env (set by the workflow; moved to $script:GhToken at start).
function Publish-Checkpoint {
    param([string]$Tag)
    try {
        if (-not $script:GhToken) { Write-Log ('checkpoint {0}: no GITHUB_TOKEN in this step -> skipped' -f $Tag); return }
        $node = Find-Exe 'node.exe'
        if (-not $node) { Write-Log ('checkpoint {0}: node.exe not found -> skipped' -f $Tag); return }
        $osName = [string]$env:DIAG_MATRIX_OS
        if (-not $osName) { $osName = 'unknown-os' }
        $script = Join-Path $global:DiagRoot 'publish-results.mjs'
        $a = '"{0}" --job {1} --os {2} --out "{3}"' -f $script, ('sacrules-' + $Tag), $osName, $global:DiagOut
        $env:GITHUB_TOKEN = $script:GhToken
        try {
            $r = Invoke-Proc -File $node -Arguments $a -TimeoutSec 300
        } finally {
            Remove-Item -Path 'Env:\GITHUB_TOKEN' -ErrorAction SilentlyContinue
        }
        $msg = (([string]$r.out) + ' ' + ([string]$r.err)).Trim()
        Write-Log ('checkpoint {0}: rc={1} {2}' -f $Tag, $r.rc, $msg)
        $RUN['checkpoints'][$Tag] = [ordered]@{ rc = $r.rc; out = $r.out; err = $r.err; time = (Get-IsoNow) }
    } catch {
        Write-Log ('checkpoint {0} failed: {1}' -f $Tag, $_.Exception.Message) 'WARN'
    }
}

function Invoke-Stage {
    param([string]$Name, [scriptblock]$Block)
    $s = [ordered]@{ started = (Get-IsoNow); ended = $null; ok = $true; error = $null }
    $RUN['stages'][$Name] = $s
    Write-Log ('--- stage {0} (minutes left {1}, language mode {2}) ---' -f $Name, (Get-MinutesLeft), $ExecutionContext.SessionState.LanguageMode)
    try {
        & $Block
    } catch {
        $s['ok'] = $false
        $s['error'] = Format-ErrorText $_
        Add-DiagError ('stage ' + $Name) $_
    }
    $s['ended'] = (Get-IsoNow)
    $s['language_mode_at_end'] = [string]$ExecutionContext.SessionState.LanguageMode
    Save-Run
}

# ---------------------------------------------------------------------------
# policy helpers
# ---------------------------------------------------------------------------
function Get-PolicyOptions {
    param([string]$XmlText)
    $res = New-Object System.Collections.Generic.List[string]
    foreach ($m in [regex]::Matches($XmlText, '<Option>([^<]+)</Option>')) { $res.Add($m.Groups[1].Value.Trim()) }
    return $res.ToArray()
}

function Save-Raw {
    param([string]$BaseName, [string]$Text)
    $t = $Text
    if ($null -eq $t) { $t = '' }
    $ext = '.txt'
    $trim = $t.TrimStart()
    if ($trim.StartsWith('{') -or $trim.StartsWith('[')) { $ext = '.json' }
    Save-Text ($BaseName + $ext) $t
}

# Write modified policy XML with the same encoding family as the original file (UTF-8 with/without BOM or UTF-16LE),
# so that the XML declaration still matches the bytes.
function Write-TextLikeOriginal {
    param([string]$Path, [string]$Text, [string]$OriginalPath)
    $enc = New-Object System.Text.UTF8Encoding($false)
    try {
        $b = [System.IO.File]::ReadAllBytes($OriginalPath)
        if (($b.Length -ge 2) -and ($b[0] -eq 255) -and ($b[1] -eq 254)) {
            $enc = New-Object System.Text.UnicodeEncoding($false, $true)
        } elseif (($b.Length -ge 3) -and ($b[0] -eq 239) -and ($b[1] -eq 187) -and ($b[2] -eq 191)) {
            $enc = New-Object System.Text.UTF8Encoding($true)
        }
    } catch { }
    [System.IO.File]::WriteAllText($Path, $Text, $enc)
}

function New-DiagPolicies {
    $info = [ordered]@{
        ok = $false; error = $null; example = $null; missing_cmdlets = @()
        policy_id = $null; base_policy_id = $null
        options_example = @(); options_base = @(); options_audit = @(); options_enforce = @()
        conditional_lockdown_removed = $null; added_options = @()
        audit_cip = $null; enforce_cip = $null; neutral_cip = $null; steps = (New-Object System.Collections.Generic.List[object])
    }
    try {
        $example = Join-Path $env:windir 'schemas\CodeIntegrity\ExamplePolicies\SmartAppControl.xml'
        $info['example'] = $example
        if (-not (Test-Path -LiteralPath $example)) { $info['error'] = 'SmartAppControl.xml not found: measurement impossible (no other policy is substituted)'; return $info }
        $missing = New-Object System.Collections.Generic.List[string]
        foreach ($c in @('Set-CIPolicyIdInfo', 'Set-CIPolicyVersion', 'Set-RuleOption', 'ConvertFrom-CIPolicy')) {
            if (-not (Get-Command -Name $c -ErrorAction SilentlyContinue)) { $missing.Add($c) }
        }
        $info['missing_cmdlets'] = $missing.ToArray()
        if ($missing.Count -gt 0) { $info['error'] = 'ConfigCI cmdlets missing: ' + ($missing -join ', ') + ' (measurement impossible)'; return $info }

        $work = Join-Path $global:DiagWork 'sacpol'
        New-Item -ItemType Directory -Path $work -Force | Out-Null
        $orig = Read-TextUtf8 $example
        Save-Text 'sac-policy-example-original.xml' $orig
        $info['options_example'] = @(Get-PolicyOptions $orig)

        # remove 'Enabled:Conditional Windows Lockdown Policy'
        $re = '(?s)<Rule>\s*<Option>Enabled:Conditional Windows Lockdown Policy</Option>\s*</Rule>'
        $txt = [regex]::Replace($orig, $re, '')
        $info['conditional_lockdown_removed'] = ($txt.Length -ne $orig.Length)
        $base = Join-Path $work 'base.xml'
        Write-TextLikeOriginal $base $txt $example

        $idOk = $false
        try {
            $null = Set-CIPolicyIdInfo -FilePath $base -PolicyName $POLICY_NAME -ResetPolicyID -ErrorAction Stop
            $idOk = $true
            $info['steps'].Add('Set-CIPolicyIdInfo -PolicyName ' + $POLICY_NAME + ' -ResetPolicyID: ok')
        } catch {
            $info['steps'].Add('Set-CIPolicyIdInfo (combined call) failed: ' + $_.Exception.Message)
        }
        if (-not $idOk) {
            $null = Set-CIPolicyIdInfo -FilePath $base -ResetPolicyID -ErrorAction Stop
            $null = Set-CIPolicyIdInfo -FilePath $base -PolicyName $POLICY_NAME -ErrorAction Stop
            $info['steps'].Add('Set-CIPolicyIdInfo -ResetPolicyID and -PolicyName as two calls: ok')
        }
        $null = Set-CIPolicyVersion -FilePath $base -Version '1.0.0.1' -ErrorAction Stop
        $info['steps'].Add('Set-CIPolicyVersion 1.0.0.1: ok')

        $btxt = Read-TextUtf8 $base
        $m1 = [regex]::Match($btxt, '<PolicyID>\s*([^<\s]+)\s*</PolicyID>')
        if ($m1.Success) { $info['policy_id'] = $m1.Groups[1].Value }
        $m2 = [regex]::Match($btxt, '<BasePolicyID>\s*([^<\s]+)\s*</BasePolicyID>')
        if ($m2.Success) { $info['base_policy_id'] = $m2.Groups[1].Value }

        $opts = @(Get-PolicyOptions $btxt)
        $added = New-Object System.Collections.Generic.List[string]
        if ($opts -notcontains 'Enabled:UMCI') { $null = Set-RuleOption -FilePath $base -Option 0 -ErrorAction Stop; $added.Add('0 Enabled:UMCI (user-mode code integrity: SAC restricts user mode)') }
        if ($opts -notcontains 'Enabled:Intelligent Security Graph Authorization') { $null = Set-RuleOption -FilePath $base -Option 14 -ErrorAction Stop; $added.Add('14 Enabled:Intelligent Security Graph Authorization') }
        if ($opts -notcontains 'Enabled:Unsigned System Integrity Policy') { $null = Set-RuleOption -FilePath $base -Option 6 -ErrorAction Stop; $added.Add('6 Enabled:Unsigned System Integrity Policy') }
        if ($opts -notcontains 'Enabled:Update Policy No Reboot') { $null = Set-RuleOption -FilePath $base -Option 16 -ErrorAction Stop; $added.Add('16 Enabled:Update Policy No Reboot') }
        $info['added_options'] = $added.ToArray()
        $info['options_base'] = @(Get-PolicyOptions (Read-TextUtf8 $base))
        Save-Text 'sac-policy-base.xml' (Read-TextUtf8 $base)

        # audit variant (+ option 3 Audit Mode)
        $auditXml = Join-Path $work 'audit.xml'
        Copy-Item -LiteralPath $base -Destination $auditXml -Force
        $null = Set-RuleOption -FilePath $auditXml -Option 3 -ErrorAction Stop
        $cipName = 'policy'
        if ($info['policy_id']) { $cipName = '{' + ([string]$info['policy_id']).Trim('{', '}') + '}' }
        New-Item -ItemType Directory -Path (Join-Path $work 'audit') -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $work 'enforce') -Force | Out-Null
        $auditCip = Join-Path $work ('audit\' + $cipName + '.cip')
        $null = ConvertFrom-CIPolicy -XmlFilePath $auditXml -BinaryFilePath $auditCip -ErrorAction Stop
        $info['steps'].Add('audit variant: Set-RuleOption 3 + ConvertFrom-CIPolicy: ok')
        $info['options_audit'] = @(Get-PolicyOptions (Read-TextUtf8 $auditXml))
        Save-Text 'sac-policy-audit.xml' (Read-TextUtf8 $auditXml)

        # enforce variant (no option 3, version 1.0.0.2)
        $enfXml = Join-Path $work 'enforce.xml'
        Copy-Item -LiteralPath $base -Destination $enfXml -Force
        $null = Set-RuleOption -FilePath $enfXml -Option 3 -Delete -ErrorAction Stop
        $null = Set-CIPolicyVersion -FilePath $enfXml -Version '1.0.0.2' -ErrorAction Stop
        $enfCip = Join-Path $work ('enforce\' + $cipName + '.cip')
        $null = ConvertFrom-CIPolicy -XmlFilePath $enfXml -BinaryFilePath $enfCip -ErrorAction Stop
        $info['steps'].Add('enforce variant: Set-RuleOption 3 -Delete + version 1.0.0.2 + ConvertFrom-CIPolicy: ok')
        $info['options_enforce'] = @(Get-PolicyOptions (Read-TextUtf8 $enfXml))
        Save-Text 'sac-policy-enforce.xml' (Read-TextUtf8 $enfXml)

        # third variant 'neutral' = audit mode again with a higher version (same PolicyID): if removing the policy does not
        # take effect without a reboot (documented rebootless removal needs Windows 11 24H2+), deploying this one makes the
        # runner non-blocking anyway (option 16 allows the update without reboot)
        try {
            $neuXml = Join-Path $work 'neutral.xml'
            Copy-Item -LiteralPath $base -Destination $neuXml -Force
            $null = Set-RuleOption -FilePath $neuXml -Option 3 -ErrorAction Stop
            $null = Set-CIPolicyVersion -FilePath $neuXml -Version '1.0.0.3' -ErrorAction Stop
            New-Item -ItemType Directory -Path (Join-Path $work 'neutral') -Force | Out-Null
            $neuCip = Join-Path $work ('neutral\' + $cipName + '.cip')
            $null = ConvertFrom-CIPolicy -XmlFilePath $neuXml -BinaryFilePath $neuCip -ErrorAction Stop
            if (Test-Path -LiteralPath $neuCip) { $info['neutral_cip'] = $neuCip }
            $info['steps'].Add('neutral variant (audit, version 1.0.0.3): ok')
        } catch {
            $info['steps'].Add('neutral variant failed: ' + $_.Exception.Message)
        }

        if ((Test-Path -LiteralPath $auditCip) -and (Test-Path -LiteralPath $enfCip)) {
            $info['audit_cip'] = $auditCip
            $info['enforce_cip'] = $enfCip
            if ($info['policy_id']) { $info['ok'] = $true } else { $info['error'] = 'PolicyID not found in the XML (cannot remove the policy by id later)' }
        } else {
            $info['error'] = 'binary policy files were not produced'
        }
    } catch {
        $info['error'] = Format-ErrorText $_
    }
    return $info
}

# Depth-first search through a ConvertFrom-Json graph for objects mentioning our policy (name or id).
function Find-PolicyInList {
    param([string]$JsonText, [string]$PolicyId, [string]$Name)
    $res = [ordered]@{ found = $false; objects = (New-Object System.Collections.Generic.List[object]); parse_error = $null }
    try {
        $j = ConvertFrom-Json $JsonText
        $stack = New-Object System.Collections.Generic.Stack[object]
        $stack.Push($j)
        $needleId = ([string]$PolicyId).Trim('{', '}').ToLowerInvariant()
        while ($stack.Count -gt 0) {
            $n = $stack.Pop()
            if ($null -eq $n) { continue }
            if ($n -is [System.Management.Automation.PSCustomObject]) {
                $hit = $false
                foreach ($p in $n.PSObject.Properties) {
                    $v = $p.Value
                    if ($v -is [string]) {
                        if ($v -eq $Name) { $hit = $true }
                        elseif ($needleId -and ($v.Trim('{', '}').ToLowerInvariant() -eq $needleId)) { $hit = $true }
                    } elseif (($v -is [System.Management.Automation.PSCustomObject]) -or ($v -is [System.Array])) {
                        $stack.Push($v)
                    }
                }
                if ($hit) { $res['found'] = $true; $res['objects'].Add($n) }
            } elseif ($n -is [System.Array]) {
                foreach ($x in $n) { $stack.Push($x) }
            }
        }
    } catch {
        $res['parse_error'] = $_.Exception.Message
    }
    return $res
}

function Get-PolicyListing {
    param([string]$Tag)
    $l = Invoke-Proc -File $citool -Arguments '-lp -json' -TimeoutSec 120
    Save-Raw ('sac-citool-list-' + $Tag) ([string]$l.out)
    if ($l.err) { Save-Text ('sac-citool-list-' + $Tag + '.stderr.txt') ([string]$l.err) }
    $pid1 = ''
    if ($ST['policy']) { $pid1 = [string]$ST['policy']['policy_id'] }
    $m = Find-PolicyInList ([string]$l.out) $pid1 $POLICY_NAME
    return [ordered]@{ tag = $Tag; rc = $l.rc; found = $m['found']; parse_error = $m['parse_error']; objects = $m['objects'].ToArray() }
}

# CiTool is documented without -json for deploy/remove (docs: CiTool --update-policy "<file>.cip",
# CiTool --remove-policy "{GUID}"). Try the documented form first, then the variants; stop at the first rc 0.
function Invoke-CiToolVariants {
    param([string]$Tag, [string[]]$ArgVariants, [int]$TimeoutSec = 180)
    $attempts = New-Object System.Collections.Generic.List[object]
    $sb = New-Object System.Text.StringBuilder
    $decisive = $null
    foreach ($a in $ArgVariants) {
        $r = Invoke-Proc -File $citool -Arguments $a -TimeoutSec $TimeoutSec
        $attempts.Add([ordered]@{ args = $a; rc = $r.rc; timed_out = $r.timedOut; start_error = $r.startError; out = $r.out; err = $r.err })
        [void]$sb.AppendLine((Format-ProcText $r))
        $decisive = $r
        if (($r.rc -eq 0) -and (-not $r.timedOut) -and (-not $r.startError)) { break }
    }
    Save-Text ('sac-citool-' + $Tag + '.txt') $sb.ToString()
    return [ordered]@{ tag = $Tag; rc = $decisive.rc; timed_out = $decisive.timedOut; start_error = $decisive.startError; out = $decisive.out; err = $decisive.err; attempts = $attempts.ToArray() }
}

function Publish-Policy {
    param([string]$CipPath, [string]$Tag)
    $d = Invoke-CiToolVariants ('deploy-' + $Tag) @(
        ('--update-policy "{0}"' -f $CipPath),
        ('--update-policy "{0}" -json' -f $CipPath),
        ('-up "{0}"' -f $CipPath)
    )
    Start-Sleep -Seconds 3
    $lst = Get-PolicyListing ('after-' + $Tag + '-deploy')
    return [ordered]@{ tag = $Tag; cip = $CipPath; rc = $d['rc']; timed_out = $d['timed_out']; start_error = $d['start_error']; out = $d['out']; err = $d['err']; attempts = $d['attempts']; listing = $lst }
}

function Remove-DiagPolicy {
    param([string]$PolicyId)
    $res = [ordered]@{ attempted = $false; remove = $null; refresh = $null; listing_found_after = $null; neg_probe_after = $null; lifted = $null }
    if (-not $PolicyId) { $res['remove'] = 'no policy id known'; return $res }
    $res['attempted'] = $true
    $guid = '{' + ([string]$PolicyId).Trim('{', '}') + '}'
    $rm = Invoke-CiToolVariants 'remove' @(
        ('--remove-policy "{0}"' -f $guid),
        ('--remove-policy "{0}" -json' -f $guid),
        ('-rp "{0}"' -f $guid)
    )
    $res['remove'] = $rm
    $rf = Invoke-Proc -File $citool -Arguments '--refresh' -TimeoutSec 120
    if (($rf.rc -ne 0) -or $rf.startError) { $rf = Invoke-Proc -File $citool -Arguments '-r' -TimeoutSec 120 }
    Save-Text 'sac-citool-refresh-after-remove.txt' (Format-ProcText $rf)
    $res['refresh'] = [ordered]@{ rc = $rf.rc; out = $rf.out; err = $rf.err }
    Start-Sleep -Seconds 3
    $lst = Get-PolicyListing 'after-remove'
    $res['listing_found_after'] = $lst['found']
    return $res
}

function Get-AllSubjects { return @($ST['subjects']) }

function Add-InstalledSubjects {
    param([string]$Version)
    $dir = Get-InstallDir
    $cur = @($ST['subjects'])
    foreach ($n in @('cys-app.exe', 'cysd.exe', 'cys.exe')) {
        $p = Join-Path $dir $n
        if (Test-Path -LiteralPath $p) {
            $sg = Get-SigRecord $p
            $cur += [pscustomobject]@{ label = ('installed ' + $Version + ' ' + $n); path = $p; sha256 = (Get-Sha256 $p); size = (Get-Item -LiteralPath $p).Length; sig = $sg.status; sig_subject = $sg.subject; kind = ('installed-' + $Version); needle = '' }
        }
    }
    $ST['subjects'] = $cur
}

function Add-ExtraSubject {
    param([string]$Label, [string]$Path, [string]$Kind)
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $sg = Get-SigRecord $Path
    $cur = @($ST['subjects'])
    $cur += [pscustomobject]@{ label = $Label; path = $Path; sha256 = (Get-Sha256 $Path); size = (Get-Item -LiteralPath $Path).Length; sig = $sg.status; sig_subject = $sg.subject; kind = $Kind; needle = '' }
    $ST['subjects'] = $cur
}

function Find-SubjectByLabel {
    param([string]$Label)
    foreach ($s in @($ST['subjects'])) { if ([string]$s.label -eq $Label) { return $s } }
    return $null
}

# QC: what the caller sees for one blocked image (CreateProcessW is already recorded by the enforce probe)
function Measure-BlockedCalls {
    param([string]$Label, [string]$Params)
    $subj = Find-SubjectByLabel $Label
    $qc = [ordered]@{ label = $Label; path = $null; createprocess = $null; createprocess_recheck = $null; blocked = $false; shell_execute_ex = $null; shell_execute_w = $null; shell_execute_w_ui_screenshot = $null; note = $null }
    $RUN['qc'][$Label] = $qc
    if ($null -eq $subj) { $qc['note'] = 'subject not in subjects.json'; return }
    $path = [string]$subj.path
    $qc['path'] = $path
    $rec = $null
    if ($ST['probe_e'].ContainsKey($Label)) { $rec = $ST['probe_e'][$Label] }
    $qc['createprocess'] = $rec
    if ($null -ne $rec) { if ($rec['ok'] -eq $false) { $qc['blocked'] = $true } }
    if (-not $global:DiagNativeReady) { $qc['note'] = 'native helper unavailable'; return }
    if (-not $qc['blocked']) {
        $qc['note'] = 'CreateProcessW probe did not fail -> ShellExecute NOT called (an allowed installer would show its setup UI)'
        return
    }
    $ex = [DiagNative]::ShellExecEx($path, $Params, 30000)
    $qc['shell_execute_ex'] = ConvertTo-CallRecord $ex
    if ($ex.Ok) {
        $qc['note'] = 'ShellExecuteExW unexpectedly succeeded (the new process was terminated at once)'
        return
    }
    # the reputation verdict can change with time: probe once more, and never start a real installer by accident
    $again = Invoke-ProbeCreate $path
    $qc['createprocess_recheck'] = $again
    if ($again['ok'] -eq $true) {
        $qc['note'] = 'CreateProcessW probe succeeded on the re-check (verdict changed): ShellExecuteW NOT called'
        return
    }
    $w = [DiagNative]::ShellExecStart($path, $Params, 10000)
    if ($w.FirstWaitTimedOut) {
        $qc['shell_execute_w_ui_screenshot'] = Save-Screenshot ('sac-shellexecute-ui-' + ($Label -replace '[^A-Za-z0-9]+', '_') + '.png')
    }
    $w2 = [DiagNative]::ShellExecFinish(15000, $true)
    $qc['shell_execute_w'] = ConvertTo-CallRecord $w2
}

# =========================================================================
# main
# =========================================================================
try {
    Invoke-Stage 'prepare' {
        $p = [ordered]@{}
        $RUN['prepare'] = $p
        Initialize-DiagNative
        $p['native_ready'] = $global:DiagNativeReady
        $p['native_error'] = $global:DiagNativeError
        Initialize-Screenshot
        # warm up everything we will use later so nothing is loaded for the first time under enforcement
        try { $null = Get-WinEvent -ListLog 'Microsoft-Windows-CodeIntegrity/Operational' -ErrorAction Stop } catch { }
        try { $null = Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256 -ErrorAction Stop } catch { }
        try { $null = Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop | Select-Object -First 1 } catch { }
        try { $null = Get-AuthenticodeSignature -LiteralPath (Join-Path $env:windir 'System32\cmd.exe') -ErrorAction Stop } catch { }
        try { $null = Get-Command Get-MpThreatDetection -ErrorAction SilentlyContinue } catch { }
        try { $null = Get-CIEvents -SinceUtc ((Get-Date).ToUniversalTime().AddSeconds(-1)) -Max 5 } catch { }
        try { Import-Module ConfigCI -ErrorAction Stop; $p['configci_import'] = 'ok' } catch { $p['configci_import'] = 'failed: ' + $_.Exception.Message }
        $p['language_mode'] = [string]$ExecutionContext.SessionState.LanguageMode
        $p['ci_state_before'] = Get-CodeIntegrityState

        $txt = Read-TextUtf8 (Join-Path $global:DiagOut 'subjects.json')
        if ($txt) {
            # NOT @(ConvertFrom-Json $txt): 5.1 emits the top-level JSON array as ONE pipeline object (array inside the array)
            $subjParsed = ConvertFrom-Json $txt
            $ST['subjects'] = @($subjParsed)
        } else {
            $RUN['notes'].Add('OUT\subjects.json missing (p1-assets failed?): no subjects to evaluate')
        }
        $p['subjects_loaded'] = @($ST['subjects']).Count

        # the CodeIntegrity/Operational log is 1 MB by default and ~300 probes + installs can wrap it: raise it to 128 MB first
        $we = Join-Path $env:windir 'System32\wevtutil.exe'
        $g1 = Invoke-Proc -File $we -Arguments 'gl Microsoft-Windows-CodeIntegrity/Operational' -TimeoutSec 30
        $sl = Invoke-Proc -File $we -Arguments 'sl Microsoft-Windows-CodeIntegrity/Operational /ms:134217728' -TimeoutSec 30
        $g2 = Invoke-Proc -File $we -Arguments 'gl Microsoft-Windows-CodeIntegrity/Operational' -TimeoutSec 30
        $p['ci_log_size'] = [ordered]@{ before = $g1.out; set_rc = $sl.rc; set_err = $sl.err; after = $g2.out }

        $reg = Join-Path $env:windir 'System32\reg.exe'
        $x = Invoke-Proc -File $reg -Arguments 'add "HKLM\SYSTEM\CurrentControlSet\Control\CI" /v TestFlags /t REG_DWORD /d 0x300 /f' -TimeoutSec 30
        $p['testflags_reg_add'] = [ordered]@{ rc = $x.rc; out = $x.out; err = $x.err; start_error = $x.startError }
        $at = Join-Path $env:windir 'System32\appidtel.exe'
        if (Test-Path -LiteralPath $at) {
            $y = Invoke-Proc -File $at -Arguments 'start' -TimeoutSec 60
            $p['appidtel_start'] = [ordered]@{ rc = $y.rc; out = $y.out; err = $y.err; start_error = $y.startError }
        } else {
            $p['appidtel_start'] = 'appidtel.exe not present'
        }
        $sc = Invoke-Proc -File (Join-Path $env:windir 'System32\sc.exe') -Arguments 'query appidsvc' -TimeoutSec 30
        $p['appidsvc_query'] = [ordered]@{ rc = $sc.rc; out = $sc.out; err = $sc.err }
        $sc2 = Invoke-Proc -File (Join-Path $env:windir 'System32\sc.exe') -Arguments 'query AppLockerFltr' -TimeoutSec 30
        $p['applockerfltr_query'] = [ordered]@{ rc = $sc2.rc; out = $sc2.out; err = $sc2.err }
        $p['note_testflags'] = 'TestFlags=0x300 is documented to need a reboot; a hosted job cannot reboot, so events 3090-3092 may be absent. That is NOT evidence that ISG was not consulted: judge by 3076/3077, the probes and the $KERNEL.SMARTLOCKER.ORIGINCLAIM EA.'
    }

    Invoke-Stage 'policy-build' {
        # keep only the dictionary even if a cmdlet leaked something into the output stream
        $info = @(New-DiagPolicies | Where-Object { $_ -is [System.Collections.IDictionary] })[-1]
        $ST['policy'] = $info
        $RUN['policy'] = $info
        $ST['policy_ok'] = [bool]$info['ok']
        if (-not $info['ok']) {
            $RUN['measurable'] = $false
            $RUN['notes'].Add('policy could not be built -> sacrules measurement impossible: ' + [string]$info['error'])
        }
    }

    if ($ST['policy_ok']) {
        Invoke-Stage 'audit-deploy' {
            $t0 = (Get-Date).ToUniversalTime()
            $ST['t0'] = $t0
            $RUN['t0_audit_deploy_utc'] = ConvertTo-IsoUtc $t0
            $ST['deploy_attempted'] = $true
            $d = Publish-Policy ([string]$ST['policy']['audit_cip']) 'audit'
            $RUN['audit_deploy'] = $d
            $ST['audit_deployed_ok'] = ((($d['rc'] -eq 0) -and (-not $d['timed_out'])) -or $d['listing']['found'])
            $RUN['ci_state_after_audit_deploy'] = Get-CodeIntegrityState
        }

        if ($ST['audit_deployed_ok'] -and ((Get-MinutesLeft) -gt 6)) {
            Invoke-Stage 'audit-probe' {
                $subs = @($ST['subjects'])
                $RUN['audit_pass1'] = Invoke-ProbePass 'audit-pass1' $subs 240 $ST['probe_a1']
                Save-Json 'sac-audit-probe.json' @{ pass1 = $ST['probe_a1']; pass2 = $ST['probe_a2'] } 6
                Write-Log 'sleeping 90 s for late reputation answers'
                Start-Sleep -Seconds 90
                $RUN['audit_pass2'] = Invoke-ProbePass 'audit-pass2' $subs 240 $ST['probe_a2']
                Save-Json 'sac-audit-probe.json' @{ pass1 = $ST['probe_a1']; pass2 = $ST['probe_a2'] } 6
            }
            Invoke-Stage 'audit-ea' {
                $ea = [ordered]@{}
                foreach ($s in @($ST['subjects'])) {
                    if ([string]$s.kind -ne 'bundled-0.14.42') { $ea[[string]$s.label] = Get-EaText ([string]$s.path) }
                }
                Save-Json 'sac-ea-audit.json' $ea 4
            }
        }

        if ($ST['audit_deployed_ok'] -and ((Get-MinutesLeft) -gt 10)) {
            Invoke-Stage 'audit-installs' {
                $q = [ordered]@{}
                $RUN['audit_installs'] = $q
                $setup = Join-Path $global:DiagWork 'subj\ctl-pos-7zip-setup.exe'
                if (Test-Path -LiteralPath $setup) {
                    $inst7 = Join-Path $global:DiagWork '7zinst'
                    $x = Invoke-Proc -File $setup -Arguments ('/S /D=' + $inst7) -TimeoutSec 240
                    $z = Join-Path $inst7 '7z.exe'
                    $zr = [ordered]@{ rc = $x.rc; timed_out = $x.timedOut; start_error = $x.startError; installed_7z_exists = (Test-Path -LiteralPath $z); ea = $null; probe = $null }
                    $q['zip'] = $zr
                    if (Test-Path -LiteralPath $z) {
                        $zr['ea'] = Get-EaText $z
                        Add-ExtraSubject 'installed 7-Zip 7z.exe (installed under audit)' $z 'control-pos-unsigned-installed'
                        $zr['probe'] = Invoke-ProbeCreate $z
                        $ST['probe_a2']['installed 7-Zip 7z.exe (installed under audit)'] = $zr['probe']
                    }
                } else {
                    $q['zip'] = 'ctl-pos-7zip-setup.exe not available'
                }
                $inst = Install-CysVersion -Version '0.14.37'
                $q['cys_0_14_37'] = $inst
                Add-InstalledSubjects '0.14.37'
                $iea = [ordered]@{}
                $q['installed_files'] = $iea
                foreach ($n in @('cys-app.exe', 'cysd.exe', 'cys.exe')) {
                    $p = Join-Path (Get-InstallDir) $n
                    $lab = 'installed 0.14.37 ' + $n
                    if (Test-Path -LiteralPath $p) {
                        $pr = Invoke-ProbeCreate $p
                        $ST['probe_a2'][$lab] = $pr
                        $iea[$n] = [ordered]@{ ea = (Get-EaText $p); probe = $pr }
                    } else {
                        $iea[$n] = 'missing after install'
                    }
                }
                Save-Json 'sac-audit-probe.json' @{ pass1 = $ST['probe_a1']; pass2 = $ST['probe_a2'] } 6
            }
        }

        if ($ST['audit_deployed_ok']) {
            Invoke-Stage 'audit-events' {
                Start-Sleep -Seconds 10
                $ex = Export-CIEvtx -SinceUtc $ST['t0'] -FileName 'sac-audit.evtx'
                $RUN['audit_evtx_export'] = [ordered]@{ rc = $ex.rc; out = $ex.out; err = $ex.err; start_error = $ex.startError }
                $ev = Get-CIEvents -SinceUtc $ST['t0']
                Save-EventsJson $ev 'sac-audit-events.json'
                $RUN['audit_event_note'] = $ev.note
                $RUN['audit_event_counts'] = Get-IdCounts $ev.events
                $m = Build-Matrix (Get-AllSubjects) $ev.events $ST['probe_a1'] $ST['probe_a2'] 'audit' 'sac-audit'
                $RUN['audit_control_interpretation'] = $m['control_interpretation']
                $ST['audit_events_done'] = $true
                Save-Run
                Publish-Checkpoint 'audit'
            }
        }

        if ($ST['audit_deployed_ok'] -and ((Get-MinutesLeft) -gt 8)) {
            Invoke-Stage 'enforce-deploy' {
                $t1 = (Get-Date).ToUniversalTime()
                $ST['t1'] = $t1
                $RUN['t1_enforce_deploy_utc'] = ConvertTo-IsoUtc $t1
                $d = Publish-Policy ([string]$ST['policy']['enforce_cip']) 'enforce'
                $RUN['enforce_deploy'] = $d
                # only a deployment that CiTool accepted (rc 0) makes the following 'enforce' data enforce data
                $ST['enforce_deployed_ok'] = (($d['rc'] -eq 0) -and (-not $d['timed_out']))
                if (-not $ST['enforce_deployed_ok']) { $RUN['notes'].Add('enforce policy deployment was NOT accepted by CiTool (rc ' + [string]$d['rc'] + '): enforce stages skipped; see sac-citool-deploy-enforce.txt') }
                Start-Sleep -Seconds 5
                if (-not $ST['enforce_deployed_ok']) {
                    # CiTool's exit code may be misleading: if the fresh unsigned control is blocked now, enforcement IS active
                    $negS = Find-SubjectByLabel 'control NEG (fresh unsigned exe)'
                    if ($negS) {
                        $prN = Invoke-ProbeCreate ([string]$negS.path)
                        $RUN['enforce_negprobe_after_deploy'] = $prN
                        if (Test-ProbeBlocked $prN) {
                            $ST['enforce_deployed_ok'] = $true
                            $RUN['notes'].Add('CiTool rc was not 0 but the NEG control is blocked now: enforcement treated as active')
                        }
                    }
                }
                $RUN['ci_state_after_enforce_deploy'] = Get-CodeIntegrityState
                $RUN['language_mode_after_enforce_deploy'] = [string]$ExecutionContext.SessionState.LanguageMode
            }
        }

        if ($ST['t1'] -and $ST['enforce_deployed_ok']) {
            Invoke-Stage 'enforce-probe' {
                $subs = @($ST['subjects'])
                $RUN['enforce_pass'] = Invoke-ProbePass 'enforce' $subs 240 $ST['probe_e']
                Save-Json 'sac-enforce-probe.json' $ST['probe_e'] 6
                Measure-BlockedCalls 'installer 0.14.42' '/P /R /UPDATE'
                Measure-BlockedCalls 'control NEG (fresh unsigned exe)' ''
                Save-Json 'sac-qc.json' $RUN['qc'] 8
            }

            Invoke-Stage 'enforce-e2e' {
                $qd = [ordered]@{ app_probe = $null; ran = $false; app_blocked = $null; skipped_reason = $null; shell_execute_ex_app = $null; verdict = $null; why = $null; run = $null }
                $RUN['qd'] = $qd
                $appLabel = 'installed 0.14.37 cys-app.exe'
                $app = Join-Path (Get-InstallDir) 'cys-app.exe'
                if (-not (Test-Path -LiteralPath $app)) {
                    $qd['skipped_reason'] = 'cys-app.exe is not installed (audit-phase install failed or was skipped)'
                    Save-Json 'sac-e2e-verdict.json' $qd 8
                    return
                }
                $rec = $null
                if ($ST['probe_e'].ContainsKey($appLabel)) { $rec = $ST['probe_e'][$appLabel] }
                if ($null -eq $rec) { $rec = Invoke-ProbeCreate $app }
                $qd['app_probe'] = $rec
                if ((Get-MinutesLeft) -lt 14) {
                    $qd['skipped_reason'] = 'not enough job time left for the app run'
                } elseif ($rec['ok'] -eq $true) {
                    $qd['app_blocked'] = $false
                    $appRun = Invoke-AppUpdateRun -Prefix 'sac-e2e' -Port 9333 -StartupWaitSec 90 -CdpMaxSec 300 -PostNodeSec 360 -TargetVersion '0.14.42'
                    $qd['ran'] = $true
                    $qd['run'] = $appRun
                    $qd['verdict'] = $appRun['verdict']
                    $qd['why'] = $appRun['why']
                } else {
                    $qd['app_blocked'] = $true
                    $qd['skipped_reason'] = 'the installed cys-app.exe itself is blocked under the enforced rules'
                    if ($global:DiagNativeReady) {
                        $qd['shell_execute_ex_app'] = ConvertTo-CallRecord ([DiagNative]::ShellExecEx($app, '', 30000))
                    }
                }
                Save-Json 'sac-e2e-verdict.json' $qd 9
            }
        }
    }
} catch {
    Add-DiagError 'sacrules main' $_
} finally {
    # 1) take the policy off again, whatever happened above
    Invoke-Stage 'cleanup-remove' {
        $c = $RUN['cleanup']
        $c['started'] = (Get-IsoNow)
        $c['language_mode'] = [string]$ExecutionContext.SessionState.LanguageMode
        if ($ST['deploy_attempted']) {
            $polId = ''
            if ($ST['policy']) { $polId = [string]$ST['policy']['policy_id'] }
            $rm = Remove-DiagPolicy $polId
            $c['remove'] = $rm
            $negSub = Find-SubjectByLabel 'control NEG (fresh unsigned exe)'
            if ($negSub) {
                $pr = Invoke-ProbeCreate ([string]$negSub.path)
                $rm['neg_probe_after'] = $pr
                if ($pr['ok'] -eq $true) { $rm['lifted'] = $true } elseif ($pr['ok'] -eq $false) { $rm['lifted'] = $false }
            }
            $c['ci_state_after_remove'] = Get-CodeIntegrityState
            # removal can need a reboot on some builds (rebootless removal of unsigned policies is documented for Windows 11 24H2+):
            # if the policy is still listed or still blocks, deploy the audit-mode 'neutral' variant (same id, higher version)
            if (($rm['listing_found_after'] -eq $true) -or ($rm['lifted'] -eq $false)) {
                $nc = ''
                if ($ST['policy']) { $nc = [string]$ST['policy']['neutral_cip'] }
                if ($nc -and (Test-Path -LiteralPath $nc)) {
                    $np = Publish-Policy $nc 'neutralize'
                    $c['neutralize'] = $np
                    if ($negSub) {
                        $pr2 = Invoke-ProbeCreate ([string]$negSub.path)
                        $c['neg_probe_after_neutralize'] = $pr2
                    }
                    $c['ci_state_after_neutralize'] = Get-CodeIntegrityState
                } else {
                    $c['neutralize'] = 'no neutral policy file available'
                }
            }
        } else {
            $c['remove'] = 'no policy was deployed'
        }
    }
    # 2) events of the enforce phase (and of the audit phase when that stage did not finish)
    Invoke-Stage 'cleanup-events' {
        $c = $RUN['cleanup']
        Start-Sleep -Seconds 2
        if ($ST['t1'] -and $ST['enforce_deployed_ok']) {
            $ex = Export-CIEvtx -SinceUtc $ST['t1'] -FileName 'sac-enforce.evtx'
            $c['enforce_evtx_export'] = [ordered]@{ rc = $ex.rc; out = $ex.out; err = $ex.err; start_error = $ex.startError }
            $ev = Get-CIEvents -SinceUtc $ST['t1']
            Save-EventsJson $ev 'sac-enforce-events.json'
            $c['enforce_event_note'] = $ev.note
            $c['enforce_event_counts'] = Get-IdCounts $ev.events
            $m = Build-Matrix (Get-AllSubjects) $ev.events $ST['probe_e'] @{} 'enforce' 'sac-enforce'
            $c['enforce_control_interpretation'] = $m['control_interpretation']
            $ue = New-Object System.Collections.Generic.List[object]
            foreach ($e in @($ev.events)) {
                if ([string]$e.xml -match '(?i)updater|cys-[0-9.]+-installer') {
                    $ue.Add([ordered]@{ id = $e.id; time = $e.time; file_names = @(Get-EventPaths $e); policy_names = @(Get-FieldValues $e '(?i)policy\s*name'); fields = $e.fields })
                }
            }
            $c['enforce_events_about_updater_temp_installer'] = $ue.ToArray()
        }
        if ($ST['t0'] -and (-not $ST['audit_events_done'])) {
            $ex2 = Export-CIEvtx -SinceUtc $ST['t0'] -FileName 'sac-audit.evtx'
            $c['audit_evtx_export_late'] = [ordered]@{ rc = $ex2.rc; out = $ex2.out; err = $ex2.err; start_error = $ex2.startError }
            $ev2 = Get-CIEvents -SinceUtc $ST['t0']
            Save-EventsJson $ev2 'sac-audit-events.json'
            $c['audit_event_counts_late'] = Get-IdCounts $ev2.events
            $m2 = Build-Matrix (Get-AllSubjects) $ev2.events $ST['probe_a1'] $ST['probe_a2'] 'audit' 'sac-audit'
            $c['audit_control_interpretation_late'] = $m2['control_interpretation']
        }
        try {
            $td = @(Get-MpThreatDetection -ErrorAction Stop)
            $lines = New-Object System.Collections.Generic.List[string]
            foreach ($t in ($td | Select-Object -First 20)) { $lines.Add(('{0} {1} {2}' -f $t.ThreatID, (ConvertTo-IsoUtc $t.InitialDetectionTime), (($t.Resources) -join ';'))) }
            $c['mp_threat_detections'] = $lines.ToArray()
        } catch {
            $c['mp_threat_detections'] = 'unavailable: ' + $_.Exception.Message
        }
        $c['finished'] = (Get-IsoNow)
    }
    $RUN['finished'] = (Get-IsoNow)
    Save-Run
    Publish-Checkpoint 'after-enforce'
    Complete-DiagScript 'sacrules'
}
exit 0
