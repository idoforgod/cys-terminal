# diag/sac-lib.ps1 -- helpers shared by sacreal-e2e.ps1 and product-launch.ps1: the REAL Smart App Control (SAC) switch (3 states, set,
# restore, a last resort that also works in Constrained Language Mode), the results-branch checkpoint, step logging, subjects.json
# loading and the CodeIntegrity block-event files. Dot-source it AFTER lib.ps1.
# Moved VERBATIM out of sacreal-e2e.ps1 (that job ran successfully on a real runner); the only text changes are that the result file
# names and the checkpoint job name use $K_PFX / $K_JOB (sacreal-e2e.ps1: 'sacreal' / 'sacreal-e2e', so its file names are unchanged).
# CONTRACT: the including script defines, before it calls any of these functions:
#   $RUN  ordered dictionary with 'steps' 'states' 'checkpoints' (ordered dictionaries) and 'notes' (List[string])
#   $CTX  hashtable (keys used here: 'subjects' 'driver_node')
#   $K_PFX (result file prefix)  $K_JOB (checkpoint job name)  $K_CITOOL $K_REG $K_KEY $K_VAL
#   function Save-Run, $script:SacTouched, $script:GhToken (the script takes GITHUB_TOKEN out of the environment at its start)

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
        $pubArgs = '"{0}" --job {1} --os {2} --out "{3}"' -f $pubScript, ($K_JOB + '-' + $Tag), $osName, $global:DiagOut
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
        Save-Text ($K_PFX + '-citool-lp-' + $Tag + '.json') $rawJson
        if ($c.err) { Save-Text ($K_PFX + '-citool-lp-' + $Tag + '.stderr.txt') ([string]$c.err) }
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
    Save-Json ($K_PFX + '-state-' + $Tag + '.json') $s 7
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
    Save-Text ($K_PFX + '-citool-refresh-' + $Why + '.txt') (Format-ProcText $r)
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
