# diag/sac-real.ps1 -- QE: can the REAL Smart App Control state be switched on from a runner
# (registry value + CiTool -r)?  windows-11-arm job only; last measuring step of the e2e job.
#   HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy\VerifiedAndReputablePolicyState
#   Meaning of the values: the n4r1b.com internals article says 0 off, 1 enforce, 2 evaluation; some other pages say
#   1 and 2 the other way round [uncertain]. So BOTH values are tried and the OBSERVED state decides: the policy names
#   CiTool lists (VerifiedAndReputableDesktop = enforce, ...DesktopEvaluation = evaluation), Is Currently Enforced,
#   Get-MpComputerStatus SmartAppControlState and the probe results.
# Observation only: a write that is refused is recorded verbatim; the original value is restored at the end.
# Output: OUT\sac-real.json (+ sac-real-*.json/.txt/.evtx)
$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')
Start-DiagScript -Name 'sac-real'

$citool = Join-Path $env:windir 'System32\CiTool.exe'
$regExe = Join-Path $env:windir 'System32\reg.exe'
$KEY = 'HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy'
$VAL = 'VerifiedAndReputablePolicyState'

$SAC = [ordered]@{
    started = (Get-IsoNow)
    matrix_os = $env:DIAG_MATRIX_OS
    native_ready = $false
    original_value = $null
    original_present = $null
    states = [ordered]@{}
    passes = [ordered]@{}
    notes = (New-Object System.Collections.Generic.List[string])
}

function Save-SacReal { Save-Json 'sac-real.json' $SAC 9 }

# objects of a ConvertFrom-Json graph that have any string property matching a regex
function Find-JsonObjectsByRegex {
    param([string]$JsonText, [string]$Pattern)
    $out = New-Object System.Collections.Generic.List[object]
    try {
        $j = ConvertFrom-Json $JsonText
        $stack = New-Object System.Collections.Generic.Stack[object]
        $stack.Push($j)
        while ($stack.Count -gt 0) {
            $n = $stack.Pop()
            if ($null -eq $n) { continue }
            if ($n -is [System.Management.Automation.PSCustomObject]) {
                $hit = $false
                foreach ($p in $n.PSObject.Properties) {
                    $v = $p.Value
                    if ($v -is [string]) { if ($v -match $Pattern) { $hit = $true } }
                    elseif (($v -is [System.Management.Automation.PSCustomObject]) -or ($v -is [System.Array])) { $stack.Push($v) }
                }
                if ($hit) { $out.Add($n) }
            } elseif ($n -is [System.Array]) {
                foreach ($x in $n) { $stack.Push($x) }
            }
        }
    } catch {
        $out.Add([ordered]@{ parse_error = $_.Exception.Message })
    }
    return $out.ToArray()
}

function Get-SacState {
    param([string]$Tag)
    $s = [ordered]@{ tag = $Tag; time = (Get-IsoNow); reg_rc = $null; reg_out = $null; reg_value = $null; reg_present = $null; citool_rc = $null; verified_and_reputable_names = @(); citool_matching_objects = @(); mp_state = $null; ci_state = $null; mode_by_citool_names = $null }
    try {
        $q = Invoke-Proc -File $regExe -Arguments ('query "{0}" /v {1}' -f $KEY, $VAL) -TimeoutSec 30
        $s['reg_rc'] = $q.rc
        $s['reg_out'] = ([string]$q.out + [string]$q.err).Trim()
        $m = [regex]::Match([string]$q.out, ($VAL + '\s+REG_DWORD\s+0x([0-9a-fA-F]+)'))
        if ($m.Success) { $s['reg_value'] = [Convert]::ToInt32($m.Groups[1].Value, 16); $s['reg_present'] = $true } else { $s['reg_present'] = $false }
    } catch { $s['reg_out'] = 'exception: ' + $_.Exception.Message }
    try {
        $c = Invoke-Proc -File $citool -Arguments '-lp -json' -TimeoutSec 120
        $s['citool_rc'] = $c.rc
        $ext = '.txt'
        $tt = ([string]$c.out).TrimStart()
        if ($tt.StartsWith('{') -or $tt.StartsWith('[')) { $ext = '.json' }
        Save-Text ('sac-real-citool-lp-' + $Tag + $ext) ([string]$c.out)
        if ($c.err) { Save-Text ('sac-real-citool-lp-' + $Tag + '.stderr.txt') ([string]$c.err) }
        $names = New-Object System.Collections.Generic.List[string]
        foreach ($mm in [regex]::Matches([string]$c.out, 'VerifiedAndReputable\w*')) { if (-not $names.Contains($mm.Value)) { $names.Add($mm.Value) } }
        $s['verified_and_reputable_names'] = $names.ToArray()
        if ($names.Contains('VerifiedAndReputableDesktopEvaluation')) { $s['mode_by_citool_names'] = 'evaluation policy listed' }
        elseif ($names.Contains('VerifiedAndReputableDesktop')) { $s['mode_by_citool_names'] = 'enforce policy listed' }
        else { $s['mode_by_citool_names'] = 'no VerifiedAndReputable policy listed' }
        $s['citool_matching_objects'] = @(Find-JsonObjectsByRegex ([string]$c.out) 'VerifiedAndReputable')
    } catch { $s['citool_rc'] = 'exception: ' + $_.Exception.Message }
    try {
        $mp = Invoke-JobWithTimeout { Get-MpComputerStatus -ErrorAction Stop } 60
        $s['mp_state'] = Select-Props $mp @('SmartAppControlState', 'SmartAppControlExpiration', 'AMRunningMode', 'RealTimeProtectionEnabled')
    } catch { $s['mp_state'] = 'unavailable: ' + $_.Exception.Message }
    $s['ci_state'] = Get-CodeIntegrityState
    $SAC['states'][$Tag] = $s
    Save-SacReal
    return $s
}

function Invoke-CiRefresh {
    param([string]$Tag)
    $r1 = Invoke-Proc -File $citool -Arguments '-r' -TimeoutSec 120
    Save-Text ('sac-real-citool-refresh-' + $Tag + '.txt') (Format-ProcText $r1)
    return [ordered]@{ rc = $r1.rc; out = $r1.out; err = $r1.err; start_error = $r1.startError }
}

# the registry value is 'dirty' from a successful write until the original state is restored
$script:Dirty = $false

function Restore-SacValue {
    param([string]$Why)
    $res = [ordered]@{ why = $Why; time = (Get-IsoNow); rc = $null; out = $null; err = $null; skipped = $null; refresh = $null }
    try {
        if (-not $script:Dirty) { $res['skipped'] = 'value not changed by us, or already restored'; return $res }
        if ($SAC['original_present']) {
            $rr = Invoke-Proc -File $regExe -Arguments ('add "{0}" /v {1} /t REG_DWORD /d {2} /f' -f $KEY, $VAL, $SAC['original_value']) -TimeoutSec 30
        } else {
            $rr = Invoke-Proc -File $regExe -Arguments ('delete "{0}" /v {1} /f' -f $KEY, $VAL) -TimeoutSec 30
        }
        $res['rc'] = $rr.rc
        $res['out'] = $rr.out
        $res['err'] = $rr.err
        if ($rr.rc -eq 0) { $script:Dirty = $false }
        $res['refresh'] = Invoke-CiRefresh ('restore-' + $Why)
    } catch {
        $res['skipped'] = 'exception: ' + $_.Exception.Message
    }
    return $res
}

function Invoke-SacPass {
    param([int]$Value)
    $pass = [ordered]@{ value = $Value; started = (Get-IsoNow); write = $null; refresh = $null; state_after = $null; probes = [ordered]@{}; events = $null; restore = $null; state_restored = $null }
    $SAC['passes'][[string]$Value] = $pass
    $t = (Get-Date).ToUniversalTime()
    $w = Invoke-Proc -File $regExe -Arguments ('add "{0}" /v {1} /t REG_DWORD /d {2} /f' -f $KEY, $VAL, $Value) -TimeoutSec 30
    $writeOk = ($w.rc -eq 0)
    $pass['write'] = [ordered]@{ rc = $w.rc; out = $w.out; err = $w.err; start_error = $w.startError; ok = $writeOk }
    Save-SacReal
    $pass['refresh'] = Invoke-CiRefresh ('set' + $Value)
    Start-Sleep -Seconds 10
    $after = Get-SacState ('after-set-' + $Value)
    $pass['state_after'] = $after

    if ($writeOk) {
        $script:Dirty = $true
        try {
            # probes (Add-Type was done before the value changed)
            foreach ($lab in @('control NEG (fresh unsigned exe)', 'installer 0.14.42')) {
                $path = $null
                foreach ($s in @($script:Subjects)) { if ([string]$s.label -eq $lab) { $path = [string]$s.path } }
                if ($path) { $pass['probes'][$lab] = Invoke-ProbeCreate $path } else { $pass['probes'][$lab] = 'subject path unknown' }
            }
            Start-Sleep -Seconds 3
            $ex = Export-CIEvtx -SinceUtc $t -FileName ('sac-real-' + $Value + '.evtx')
            $ev = Get-CIEvents -SinceUtc $t
            $cnt = [ordered]@{}
            foreach ($e in @($ev.events)) {
                $k = [string]$e.id
                if ($cnt.Contains($k)) { $cnt[$k] = [int]$cnt[$k] + 1 } else { $cnt[$k] = 1 }
            }
            $keep = New-Object System.Collections.Generic.List[object]
            foreach ($e in @($ev.events)) { if ((@(3076, 3077) -contains [int]$e.id) -and ($keep.Count -lt 300)) { $keep.Add($e) } }
            Save-Json ('sac-real-events-' + $Value + '.json') ([ordered]@{ note = $ev.note; counts_by_id = $cnt; events_3076_3077 = $keep.ToArray() }) 6
            $pass['events'] = [ordered]@{ evtx_rc = $ex.rc; note = $ev.note; counts_by_id = $cnt; file = ('sac-real-events-' + $Value + '.json') }
        } catch {
            $pass['probe_or_event_error'] = $_.Exception.Message
        } finally {
            # restore the original value whatever happened above
            $pass['restore'] = Restore-SacValue ('pass' + $Value)
            Start-Sleep -Seconds 5
            $pass['state_restored'] = Get-SacState ('restored-after-' + $Value)
        }
    } else {
        $pass['restore'] = 'nothing to restore: the registry write was refused'
        $pass['probes'] = 'skipped: the registry write was refused (no state change to observe)'
    }
    $pass['finished'] = (Get-IsoNow)
    Save-SacReal
}

try {
    Initialize-DiagNative
    Initialize-Screenshot
    $SAC['native_ready'] = $global:DiagNativeReady
    $SAC['native_error'] = $global:DiagNativeError
    $script:Subjects = @()
    $txt = Read-TextUtf8 (Join-Path $global:DiagOut 'subjects.json')
    if ($txt) {
        # NOT @(ConvertFrom-Json $txt): 5.1 emits the top-level JSON array as ONE pipeline object (array inside the array)
        $subjParsed = ConvertFrom-Json $txt
        $script:Subjects = @($subjParsed)
    } else {
        $SAC['notes'].Add('subjects.json missing: probes will be skipped')
    }
    Save-SacReal

    $init = Get-SacState 'initial'
    $SAC['original_present'] = [bool]$init['reg_present']
    $SAC['original_value'] = $init['reg_value']

    if ((Get-MinutesLeft) -gt 5) {
        Invoke-Safe 'sac-real pass value=1' { Invoke-SacPass 1 }
    } else {
        $SAC['notes'].Add('pass value=1 skipped: not enough job time left')
    }
    if ((Get-MinutesLeft) -gt 6) {
        Invoke-Safe 'sac-real pass value=2' { Invoke-SacPass 2 }
    } else {
        $SAC['notes'].Add('pass value=2 skipped: not enough job time left')
    }
} catch {
    Add-DiagError 'sac-real main' $_
} finally {
    try { $SAC['final_restore'] = Restore-SacValue 'final' } catch { }
    try { $SAC['final_state'] = Get-SacState 'final' } catch { }
    $SAC['still_dirty'] = $script:Dirty
    $SAC['finished'] = (Get-IsoNow)
    Save-SacReal
    Complete-DiagScript 'sac-real'
}
exit 0
