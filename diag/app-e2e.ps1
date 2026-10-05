# diag/app-e2e.ps1 -- the WHOLE 0.14.43 app on Windows, updated through its REAL Update button (windows-11-arm job only).
#   inputs   (workflow) the NSIS setup.exe that windows-build.yml built for the product branch (downloaded artifact) and the
#            manifest URL: CYS_UPDATE_MANIFEST_URL=https://raw.githubusercontent.com/<repo>/<sha>/diag/appe2e-manifest.json
#            (manifest version 0.14.99 whose url/signature are the PUBLIC 0.14.42 installer and its .sig: the signature is over the
#            file bytes, so the app verifies it whatever the version label says). So "0.14.43 receives update 0.14.99 and launches
#            the 0.14.42 installer": what is measured is the NEW execution path of 0.14.43, not what gets installed.
#   OFF      SAC off: install 0.14.43 (/S) -> start the app (WebView2 CDP + the manifest env) -> node cdp-update.mjs --mode ui clicks
#            Update -> "bin patch install" -> confirm "install" with real mouse events -> watch: download, temp installer, installer
#            process, app exit, version marker (0.14.42 expected), relaunch -> appe2e-off-verdict.json
#   ON       reinstall 0.14.43 -> start the app -> uipre (UI is up) -> checkpoint -> REAL SAC on (value 1 + CiTool -r, 3 states) ->
#            the same click flow -> expected: no installer, the app and its CDP session stay alive, a persistent notification
#            "installer launch blocked" (4551) is in the page, temp folder and attempt record are cleaned -> appe2e-on-verdict.json
#   finally  RESTORE SAC first (value 0 + CiTool -r + re-check), then cleanup, CodeIntegrity events (3077 naming an -updater- path),
#            verdict, appe2e-summary.txt (10 lines at most)
# ONE PowerShell process. Every Add-Type / module warm-up happens BEFORE SAC is turned on. While SAC is on only the signed node.exe
# and Windows system tools are started. OBSERVATION ONLY: no SAC / Defender bypass. The shared SAC switch helpers live in sac-lib.ps1.
# TEAM     (5th run, the LAST scene: after the restore and the cleanup, with SAC verified OFF again inside): the "create a team directly"
#          flow of the real UI. node cdp-update.mjs --mode uiteam clicks (toggle of the expert section -> team button -> confirm window ->
#          its execute button) and watches the page for up to 6 minutes; this script samples the processes (cysd.exe cys.exe bash.exe
#          python*.exe), the named pipes, the team daemon folder and its logs -> appe2e-team-verdict.json (PASS = the click flow started +
#          a new team tab stood + a new cysd.exe was created after the click + no failure notification). It runs AFTER the verdict and the
#          summary of the earlier scenes were written (a hang cannot take them away) and only ADDS: a "team" key and a TEAM line in
#          appe2e-verdict.json and two TEAM lines (11 and 12) appended to appe2e-summary.txt. The OFF / ON verdicts, the restore and
#          their order are unchanged. Time: DIAG_TEAM_EXTRA_MIN (workflow env, default 15) extra minutes on top of DIAG_JOB_LIMIT_MIN.
# SCENES   (README 7th section) the workflow runs this script once per scene (matrix) with APPE2E_SCENE:
#          fresh (or empty)  everything described above, unchanged - plus ONE read-only step right after the first install:
#                            install-facts (Get-InstallFacts of app-upgrade.ps1 -> appe2e-install-facts.json; never part of a verdict)
#          upgrade           Invoke-UpgradeMain of diag/app-upgrade.ps1 INSTEAD of the main below: a public older version is installed
#                            and running, the installer under test is applied on top of it the way that old app's updater does it,
#                            then the TEAM scene runs on the upgraded app. That scene never turns Smart App Control on.
#          The installer under test comes from the windows-build artifact (run id) or, when release_tag is set in
#          diag/appe2e-input.json, from that release (diag/fetch-release-asset.mjs; its report -> inputs.installer_source).
#          appe2e-summary.txt begins with ONE more line now ("installer source: ...": source, name, size, full sha256); the 10 lines
#          described above follow it unchanged (they are lines 2-11 of the file, the TEAM lines 12 and 13).
$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')
. (Join-Path $PSScriptRoot 'e2e-update.ps1')
. (Join-Path $PSScriptRoot 'sac-lib.ps1')
# functions only (Get-InstallFacts, Invoke-UpgradeMain ...). Loaded BEFORE the functions of this file, so a name defined in both would
# be the one of this file; a load problem (parse error) is kept as text and never stops the fresh scene.
$K_UPG_LOAD_ERROR = ''
try { . (Join-Path $PSScriptRoot 'app-upgrade.ps1') } catch { $K_UPG_LOAD_ERROR = 'diag/app-upgrade.ps1 could not be loaded: ' + $_.Exception.Message }
Start-DiagScript -Name 'app-e2e'

$K_CITOOL = Join-Path $env:windir 'System32\CiTool.exe'
$K_REG = Join-Path $env:windir 'System32\reg.exe'
$K_KEY = 'HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy'
$K_VAL = 'VerifiedAndReputablePolicyState'
$K_PFX = 'appe2e'
$K_JOB = 'app-e2e'
$K_NEG = 'control NEG (fresh unsigned exe)'
$K_PORT = 9333
$K_OFF_OBSERVE_SEC = 300
$K_ON_OBSERVE_SEC = 90
$K_ON_ALIVE_SEC = 60
$K_OFF_MAX_SEC = 900
# TEAM scene (the last scene; Invoke-TeamScene): observation window of the page, wait for the UI, watch cap, job minutes it needs
$K_TEAM_OBSERVE_SEC = 360
$K_TEAM_READY_SEC = 150
$K_TEAM_MAX_SEC = 780
$K_TEAM_NEED_MIN = 14

$RUN = [ordered]@{
    started = (Get-IsoNow)
    matrix_os = $env:DIAG_MATRIX_OS
    inputs = $null
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
}
$CTX = @{}
$CTX['subjects'] = @()
$CTX['driver_node'] = ''
$CTX['t_sac_on'] = $null
$CTX['t_restore_start'] = $null
$CTX['wv2_policy'] = $null
$CTX['installer'] = ''
$CTX['from_version'] = ''
$CTX['to_version'] = ''
$CTX['manifest_version'] = ''
$CTX['manifest_url'] = ''
$CTX['inputs_ok'] = $false
$CTX['off_install_ok'] = $false
$CTX['on_ready'] = $false
$CTX['off_cdp'] = $null
$CTX['off_obs'] = $null
$CTX['off_verdict'] = $null
$CTX['on_cdp'] = $null
$CTX['on_obs'] = $null
$CTX['on_verdict'] = $null
$CTX['app_proc'] = $null
$CTX['team_verdict'] = $null
$CTX['team_scene_utc'] = $null
# the registry value is 'touched' from the moment we try to write 1 until a restore was verified
$script:SacTouched = $false
# GITHUB_TOKEN is only for the checkpoint upload: keep it in a variable and take it out of the process environment so that no child
# process (the app under test, its installer, node drivers) inherits it; it is handed to node for the upload only.
$script:GhToken = [string]$env:GITHUB_TOKEN
Remove-Item -Path 'Env:\GITHUB_TOKEN' -ErrorAction SilentlyContinue

function Save-Run { Save-Json 'app-e2e.json' $RUN 9 }

# =========================================================================
# inputs: the installer of the product build, the manifest the app is pointed at
# =========================================================================
# (README 7th section, H1) where the installer under test came from: the report of diag/fetch-release-asset.mjs when the workflow
# downloaded a release asset (release_tag in diag/appe2e-input.json), else the windows-build artifact of the run id
function Get-InstallerSource {
    $tag = ([string]$env:APPE2E_RELEASE_TAG).Trim()
    $txt = Read-TextUtf8 (Join-Path $global:DiagOut 'appe2e-installer-source.json')
    if ((-not $txt) -and (-not $tag)) {
        return [ordered]@{ kind = 'windows-build-artifact'; run_id = [string]$env:APPE2E_RUN_ID; artifact = [string]$env:APPE2E_ARTIFACT }
    }
    $src = [ordered]@{ kind = 'release-asset'; tag = $tag; report_found = $false; ok = $null; error = $null }
    if (-not $txt) {
        $src['error'] = 'the download step left no report (appe2e-installer-source.json): it did not run or was stopped'
        return $src
    }
    try {
        $j = ConvertFrom-Json $txt
        $src['report_found'] = $true
        foreach ($k in @('ok', 'error', 'http_status', 'tag', 'release_id', 'draft', 'prerelease', 'release_name', 'asset_name', 'asset_id', 'size', 'sha256', 'digest', 'digest_match', 'asset_updated_at', 'saved_to', 'duplicates_note')) {
            $src[$k] = $j.$k
        }
    } catch {
        $src['error'] = 'the report of the release download could not be read: ' + $_.Exception.Message
    }
    return $src
}

# one line for the summaries: the source, then name, size and the full sha256 of the installer file that was found
function Get-InstallerSourceLine {
    $t = 'installer source: not known'
    try {
        $in = $RUN['inputs']
        $src = $null
        if ($in -is [System.Collections.IDictionary]) { $src = $in['installer_source'] }
        $where = 'not recorded'
        if ($src -is [System.Collections.IDictionary]) {
            if ([string]$src['kind'] -eq 'release-asset') {
                $v = @{}
                foreach ($k in @('tag', 'release_id', 'draft', 'asset_id', 'digest_match', 'ok')) {
                    $v[$k] = 'unknown'
                    if ($null -ne $src[$k]) { $v[$k] = [string]$src[$k] }
                }
                $where = ('release {0} (release id {1}, draft={2}, asset id {3}, digest match={4}, download ok={5})' -f $v['tag'], $v['release_id'], $v['draft'], $v['asset_id'], $v['digest_match'], $v['ok'])
                if ($src['duplicates_note']) { $where = $where + ' [' + [string]$src['duplicates_note'] + ']' }
                if ($src['error']) { $where = $where + ' [error: ' + [string]$src['error'] + ']' }
                if ($src.Contains('file_sha256_same') -and ($src['file_sha256_same'] -eq $false)) { $where = $where + ' [WARNING: the file on disk does not have the sha256 the download report recorded]' }
            } else {
                $where = ('windows-build artifact {0} of run {1}' -f $src['artifact'], $src['run_id'])
            }
        }
        $file = 'no installer file'
        if (($in -is [System.Collections.IDictionary]) -and ($in['installer'] -is [System.Collections.IDictionary])) {
            $ii = $in['installer']
            $file = ('{0}, {1} bytes, sha256 {2}' -f $ii['name'], $ii['size'], $ii['sha256'])
        }
        $t = ('installer source: {0}; {1}' -f $where, $file)
    } catch {
        $t = 'installer source: the line could not be built: ' + $_.Exception.Message
    }
    return $t
}

function Get-AppInputs {
    $in = [ordered]@{
        run_id = [string]$env:APPE2E_RUN_ID
        artifact = [string]$env:APPE2E_ARTIFACT
        installer_dir = [string]$env:APPE2E_INSTALLER_DIR
        manifest_url = [string]$env:APPE2E_MANIFEST_URL
        commit = [string]$env:GITHUB_SHA
        no_input = $false
        installer = $null
        installer_ok = $false
        from_version = $null
        manifest = $null
        problems = (New-Object System.Collections.Generic.List[string])
        release_tag = ([string]$env:APPE2E_RELEASE_TAG).Trim()
        gate_why = ([string]$env:APPE2E_GATE_WHY).Trim()
        installer_source = $null
    }
    $RUN['inputs'] = $in
    try { $in['installer_source'] = Get-InstallerSource } catch { }
    if ($in['gate_why']) { $in['problems'].Add('appe2e-gate refused diag/appe2e-input.json: ' + [string]$in['gate_why']) }
    $rid = [int64]0
    [void][int64]::TryParse($in['run_id'], [ref]$rid)
    if (($rid -le 0) -and (-not $in['release_tag'])) {
        $in['no_input'] = $true
        $in['problems'].Add('no input: diag/appe2e-input.json has windows_build_run_id 0 (the master fills in the run number of windows-build.yml before pushing)')
        return
    }
    $dir = [string]$in['installer_dir']
    if ((-not $dir) -or (-not (Test-Path -LiteralPath $dir))) {
        if ($in['release_tag']) { $in['problems'].Add('release ' + [string]$in['release_tag'] + ': the installer was not downloaded (inputs.installer_source and appe2e-installer-source.json say why)') }
        $in['problems'].Add('the downloaded artifact folder is missing: ' + $dir + ' (download-artifact failed? wrong run id or artifact name?)')
        return
    }
    $exe = ''
    $cands = @(Get-ChildItem -LiteralPath $dir -Recurse -File -Filter '*.exe' -ErrorAction SilentlyContinue)
    foreach ($c in $cands) {
        if (($exe -eq '') -and ($c.Name -match 'setup')) { $exe = [string]$c.FullName }
    }
    if (($exe -eq '') -and ($cands.Count -gt 0)) { $exe = [string]$cands[0].FullName }
    if ($exe -eq '') {
        $in['problems'].Add('no *.exe in the downloaded artifact folder ' + $dir)
        return
    }
    $inst = [ordered]@{ path = $exe; name = (Split-Path -Leaf $exe); size = (Get-Item -LiteralPath $exe).Length; sha256 = (Get-Sha256 $exe); signature = $null; file_version = $null; product_version = $null }
    $inst['signature'] = (Get-SigRecord $exe).status
    try {
        $vi = (Get-Item -LiteralPath $exe).VersionInfo
        $inst['file_version'] = [string]$vi.FileVersion
        $inst['product_version'] = [string]$vi.ProductVersion
    } catch { }
    $in['installer'] = $inst
    try {
        $rs = $in['installer_source']
        if (($rs -is [System.Collections.IDictionary]) -and ([string]$rs['kind'] -eq 'release-asset') -and $rs['sha256']) { $rs['file_sha256_same'] = [bool]([string]$rs['sha256'] -eq [string]$inst['sha256']) }
    } catch { }
    $fromVer = ''
    $m = [regex]::Match([string]$inst['name'], '(\d+\.\d+\.\d+)')
    if ($m.Success) { $fromVer = $m.Groups[1].Value }
    if ($fromVer -eq '') {
        $m2 = [regex]::Match([string]$inst['product_version'], '^(\d+\.\d+\.\d+)')
        if ($m2.Success) { $fromVer = $m2.Groups[1].Value }
    }
    if ($fromVer -eq '') {
        $in['problems'].Add('the version of the installer could not be read from its name or version resource')
        return
    }
    $in['from_version'] = $fromVer
    $in['installer_ok'] = $true
    $CTX['installer'] = $exe
    $CTX['from_version'] = $fromVer
    $CTX['manifest_url'] = [string]$in['manifest_url']
}

# the manifest that the app will fetch: reachable, parsable, version above the installed one, and it is THIS commit's file
function Test-ManifestReachable {
    $m = [ordered]@{ url = [string]$CTX['manifest_url']; attempts = (New-Object System.Collections.Generic.List[object]); ok = $false; version = $null; platform_keys = @(); target_key = $null; installer_url = $null; installer_version = $null; signature_chars = $null; same_as_repo_copy = $null; error = $null }
    $RUN['inputs']['manifest'] = $m
    if (-not $m['url']) { $m['error'] = 'no manifest url'; return }
    $dest = Join-Path $global:DiagWork 'appe2e-manifest-downloaded.json'
    $got = $false
    for ($i = 1; $i -le 4; $i++) {
        $d = Invoke-Download ([string]$m['url']) $dest 60
        $m['attempts'].Add([ordered]@{ n = $i; ok = $d['ok']; rc = $d['rc']; size = $d['size']; err = (Limit-Text $d['err'] 200) })
        if ($d['ok']) { $got = $true; break }
        if ($i -lt 4) { Start-Sleep -Seconds 15 }
    }
    if (-not $got) { $m['error'] = 'the manifest url could not be downloaded (not pushed yet? wrong repository?)'; return }
    $txt = Read-TextUtf8 $dest
    try {
        $repoCopy = Read-TextUtf8 (Join-Path $global:DiagRoot 'appe2e-manifest.json')
        if (($null -ne $txt) -and ($null -ne $repoCopy)) {
            $m['same_as_repo_copy'] = [bool](($txt -replace "`r`n", "`n").Trim() -eq ($repoCopy -replace "`r`n", "`n").Trim())
        }
    } catch { }
    $j = $null
    try { $j = ConvertFrom-Json $txt } catch { $m['error'] = 'the manifest is not valid JSON: ' + $_.Exception.Message; return }
    $m['version'] = [string]$j.version
    $keys = New-Object System.Collections.Generic.List[string]
    foreach ($pp in @($j.platforms.PSObject.Properties)) { $keys.Add([string]$pp.Name) }
    $m['platform_keys'] = $keys.ToArray()
    $plat = $null
    foreach ($k in @('windows-x86_64-nsis', 'windows-x86_64')) {
        if (($null -eq $plat) -and ($keys.Contains($k))) { $plat = $j.platforms.PSObject.Properties[$k].Value; $m['target_key'] = $k }
    }
    if ($null -eq $plat) { $m['error'] = 'no windows-x86_64 platform entry in the manifest'; return }
    $m['installer_url'] = [string]$plat.url
    $m['signature_chars'] = ([string]$plat.signature).Length
    $mv = [regex]::Match([string]$m['installer_url'], '(\d+\.\d+\.\d+)_x64-setup')
    if ($mv.Success) { $m['installer_version'] = $mv.Groups[1].Value }
    $newer = $false
    try { $newer = ([version][string]$m['version'] -gt [version][string]$CTX['from_version']) } catch { }
    if (-not $newer) { $m['error'] = ('manifest version {0} is not newer than the installed {1}: the app would see no update' -f $m['version'], $CTX['from_version']); return }
    if (-not $m['installer_version']) { $m['error'] = 'the installer version could not be read from the manifest url'; return }
    $CTX['manifest_version'] = [string]$m['version']
    $CTX['to_version'] = [string]$m['installer_version']
    $m['ok'] = $true
}

# =========================================================================
# the installed app
# =========================================================================
function Install-CysFile {
    param([string]$Installer, [string]$ExpectVersion)
    $info = [ordered]@{ installer = $Installer; expect = $ExpectVersion; started = (Get-IsoNow); rc = $null; timed_out = $false; marker = $null; failure_file = $null; file_version = $null; killed = @(); ok = $false; error = $null }
    try {
        $p = Start-Process -FilePath $Installer -ArgumentList '/S' -PassThru -ErrorAction Stop
        $null = $p.Handle
        if (-not $p.WaitForExit(420000)) {
            $info['timed_out'] = $true
            Stop-ProcessTree -ProcessId $p.Id
        } else {
            $p.WaitForExit()
        }
        try { $info['rc'] = $p.ExitCode } catch { }
        Start-Sleep -Seconds 6
        $info['marker'] = Get-InstalledMarker
        $info['failure_file'] = Get-FailureMarker
        $info['file_version'] = Get-AppFileVersion
        $info['killed'] = @(Stop-ProcessesUnder (Get-InstallDir))
        $info['ok'] = [bool](([string]$info['marker']) -eq $ExpectVersion)
    } catch {
        $info['error'] = Format-ErrorText $_
    }
    $info['finished'] = (Get-IsoNow)
    return $info
}

# an installer process of the update flow, STRICTLY: the app's temp installer (%TEMP%\cys-<ver>-updater-<rand>\...; Win32_Process shows the 8.3
# spelling of the path), the NSIS update helper (~nsu.tmp) or a cys-named installer. Not the generic rule of e2e-update.ps1 (a setup-like name in
# a Temp folder): Edge's updater (MicrosoftEdgeUpdate.exe in ...\Microsoft\Temp\EU*.tmp) would match that one.
function Test-AppInstallerProc {
    param([string]$Name, [string]$Path)
    if ($Path -like '*\cys-*-updater-*\*') { return $true }
    if ($Path -like '*\~nsu.tmp\*') { return $true }
    if ($Name -match '^cys[-_].*(installer|setup)') { return $true }
    return $false
}

# the record the app writes right before it launches the installer: ~/.cys/.update-attempt.json (cleared when the launch is blocked)
function Get-AttemptPath { return (Join-Path $env:USERPROFILE '.cys\.update-attempt.json') }

function Read-AttemptFile {
    $o = [ordered]@{ path = (Get-AttemptPath); exists = $false; text = $null }
    try {
        if (Test-Path -LiteralPath $o['path']) {
            $o['exists'] = $true
            $o['text'] = Limit-Text (Read-FileShared $o['path']) 600
        }
    } catch { }
    return $o
}

# kill the app, its daemon and any installer of the update flow, delete the temp installer folders and the attempt record
function Reset-AppState {
    param([string]$Why)
    $r = [ordered]@{ why = $Why; time = (Get-IsoNow); killed_under_install_dir = @(); killed_webview2 = @(); killed_installers = @(); updater_dirs_removed = @(); attempt_before = $null }
    try {
        $r['killed_under_install_dir'] = @(Stop-ProcessesUnder (Get-InstallDir))
        # WebView2 runtime processes of the app (they live under Program Files, not under the install folder): a stale one could keep the
        # debugging port and answer for the next start of the app
        $wk = New-Object System.Collections.Generic.List[string]
        try {
            foreach ($wp in @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -Filter "Name = 'msedgewebview2.exe'" -ErrorAction Stop)) {
                $wc = [string]$wp.CommandLine
                if (($wc -like ('*--remote-debugging-port=' + $K_PORT + '*')) -or ($wc -like '*com.cysjavis.terminal*')) {
                    Stop-ProcessTree -ProcessId ([int]$wp.ProcessId)
                    $wk.Add(('msedgewebview2#{0}' -f $wp.ProcessId))
                }
            }
        } catch { }
        $r['killed_webview2'] = $wk.ToArray()
        $ks = New-Object System.Collections.Generic.List[string]
        foreach ($p in @(Get-InterestingProcesses)) {
            if (Test-AppInstallerProc ([string]$p.name) ([string]$p.path)) {
                Stop-ProcessTree -ProcessId ([int]$p.id)
                $ks.Add(('{0}#{1}' -f $p.name, $p.id))
            }
        }
        $r['killed_installers'] = $ks.ToArray()
        Start-Sleep -Seconds 2
        $rm = New-Object System.Collections.Generic.List[string]
        foreach ($d in @(Get-ChildItem -LiteralPath ([System.IO.Path]::GetTempPath()) -Directory -Filter 'cys-*-updater-*' -ErrorAction SilentlyContinue)) {
            try { Remove-Item -LiteralPath $d.FullName -Recurse -Force -ErrorAction Stop; $rm.Add([string]$d.Name) } catch { $rm.Add(([string]$d.Name) + ' (not removed: ' + $_.Exception.Message + ')') }
        }
        $r['updater_dirs_removed'] = $rm.ToArray()
        $r['attempt_before'] = Read-AttemptFile
        if ($r['attempt_before']['exists']) { try { Remove-Item -LiteralPath (Get-AttemptPath) -Force -ErrorAction Stop } catch { } }
    } catch {
        $r['error'] = $_.Exception.Message
    }
    return $r
}

# start the installed app with the WebView2 remote debugging port (env var + HKLM policy, see e2e-update.ps1) and the manifest env
function Start-CysApp {
    param([string]$Prefix, [int]$StartupWaitSec = 120)
    $r = [ordered]@{ prefix = $Prefix; started = (Get-IsoNow); ok = $false; pid = $null; cdp_ready = $false; cdp_wait_sec = $null; installed_marker = (Get-InstalledMarker); error = $null; webview2_runtime = $null }
    try {
        $installDir = Get-InstallDir
        $appExe = Join-Path $installDir 'cys-app.exe'
        if (-not (Test-Path -LiteralPath $appExe)) { $r['error'] = 'cys-app.exe not found in the install folder'; return $r }
        if ($null -eq $CTX['wv2_policy']) {
            $CTX['wv2_policy'] = Set-WebView2DebugPolicy -Port $K_PORT
            $RUN['webview2_policy'] = $CTX['wv2_policy']
        }
        $r['webview2_runtime'] = $(try { [string](Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -Name pv -ErrorAction Stop).pv } catch { 'unknown' })
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = ('--remote-debugging-port={0} --remote-allow-origins=*' -f $K_PORT)
        $env:CYS_UPDATE_MANIFEST_URL = [string]$CTX['manifest_url']
        $appProc = $null
        try {
            $appProc = Start-Process -FilePath $appExe -WorkingDirectory $installDir -PassThru -ErrorAction Stop
            try { $null = $appProc.Handle } catch { }
            $r['ok'] = $true
            $r['pid'] = $appProc.Id
            $CTX['app_proc'] = $appProc
        } catch {
            $r['error'] = Format-ErrorText $_
        }
        Remove-Item -Path 'Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS' -ErrorAction SilentlyContinue
        Remove-Item -Path 'Env:\CYS_UPDATE_MANIFEST_URL' -ErrorAction SilentlyContinue
        if (-not $r['ok']) { return $r }
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $exitedAt = $null
        while ($sw.Elapsed.TotalSeconds -lt $StartupWaitSec) {
            try {
                $resp = Invoke-WebRequest -Uri ('http://127.0.0.1:{0}/json/version' -f $K_PORT) -UseBasicParsing -TimeoutSec 3 -ErrorAction Stop
                if ($resp.StatusCode -eq 200) { $r['cdp_ready'] = $true; break }
            } catch { }
            try {
                if ($appProc.HasExited) {
                    if ($null -eq $exitedAt) { $exitedAt = [int]$sw.Elapsed.TotalSeconds }
                    if (([int]$sw.Elapsed.TotalSeconds - $exitedAt) -ge 20) { $r['error'] = ('the app process exited at {0} s' -f $exitedAt); break }
                }
            } catch { }
            Start-Sleep -Seconds 2
        }
        $r['cdp_wait_sec'] = [int]$sw.Elapsed.TotalSeconds
        if (-not $r['cdp_ready']) {
            if (-not $r['error']) { $r['error'] = ('CDP port {0} did not answer within {1} s' -f $K_PORT, $StartupWaitSec) }
            $r['windows'] = Get-VisibleWindowsText
            $r['screenshot'] = Save-Screenshot ($Prefix + '-nocdp.png')
            Save-Text ($Prefix + '-nocdp-processes.txt') (Get-ProcessListText)
        }
    } catch {
        $r['error'] = 'exception: ' + $_.Exception.Message
    }
    return $r
}

# =========================================================================
# the node driver (diag/cdp-update.mjs --mode uipre|ui)
# =========================================================================
function Get-NodeScript { return (Join-Path $global:DiagRoot 'cdp-update.mjs') }

# synchronous: attach + wait for the UI + one screenshot, nothing is clicked
function Invoke-NodePre {
    param([string]$Prefix, [string]$NodeExe)
    $r = [ordered]@{ prefix = $Prefix; node_exe = $NodeExe; rc = $null; attached = $false; ui_ready = $false; app_version = $null; update_button = $null; error = $null }
    try {
        $a = '"{0}" --mode uipre --port {1} --out "{2}" --prefix {3} --max-wait-sec 200 --ready-wait-sec 90' -f (Get-NodeScript), $K_PORT, $global:DiagOut, $Prefix
        $x = Invoke-Proc -File $NodeExe -Arguments $a -TimeoutSec 420
        $r['rc'] = $x['rc']
        Save-Text ($Prefix + '-cdp-stdout.txt') ((([string]$x['out']) + "`r`n" + ([string]$x['err'])))
        $cj = Read-CdpJson $Prefix
        if ($null -eq $cj) {
            $r['error'] = ($Prefix + '-cdp.json missing or unreadable')
        } else {
            $r['attached'] = [bool]$cj.attached
            $r['app_version'] = $cj.app_version
            $r['ui_ready'] = [bool]($cj.ui.ready.ok -eq $true)
            $r['update_button'] = $cj.ui.ready.snapshot.update_button
        }
    } catch {
        $r['error'] = 'exception: ' + $_.Exception.Message
    }
    return $r
}

# background: the click flow + the observation of the page; the PowerShell watch loop runs meanwhile
function Start-NodeUi {
    param([string]$Prefix, [string]$NodeExe, [int]$ObserveSec)
    $a = '"{0}" --mode ui --port {1} --out "{2}" --prefix {3} --max-wait-sec 300 --observe-sec {4} --ready-wait-sec 90 --panel-wait-sec 180 --modal-wait-sec 60' -f (Get-NodeScript), $K_PORT, $global:DiagOut, $Prefix, $ObserveSec
    $o = Join-Path $global:DiagOut ($Prefix + '-cdp-stdout.txt')
    $e = Join-Path $global:DiagOut ($Prefix + '-cdp-stderr.txt')
    $p = Start-Process -FilePath $NodeExe -ArgumentList $a -NoNewWindow -PassThru -RedirectStandardOutput $o -RedirectStandardError $e -ErrorAction Stop
    try { $null = $p.Handle } catch { }
    return $p
}

function Read-CdpJson {
    param([string]$Prefix)
    $txt = Read-TextUtf8 (Join-Path $global:DiagOut ($Prefix + '-cdp.json'))
    if (-not $txt) { return $null }
    try { return (ConvertFrom-Json $txt) } catch { return $null }
}

# =========================================================================
# the PowerShell side of the observation (processes, temp installer, marker, attempt record, desktop screenshots)
# =========================================================================
function Watch-AppRun {
    param([string]$Prefix, [string]$RunMode, $NodeProc, [string]$InitialMarker, [string]$TargetMarker, [int]$MaxSec)
    $obs = [ordered]@{
        run_mode = $RunMode; started = (Get-IsoNow); ended = $null; end_reason = $null; ticks = 0
        click_at = $null; node_exited_at = $null; node_exit_code = $null
        app_alive_first_at = $null; app_gone_at = $null; app_relaunched = $false
        alive_samples_after_click = 0; dead_samples_after_click = 0; last_alive_after_click_sec = $null; first_dead_after_click_sec = $null
        installer_seen = $false; installer_first_seen_at = $null; installer_first_seen = $null
        updater_dirs_seen = @(); updater_dir_first_seen_at = $null; updater_dir_max_bytes = 0; updater_dirs_at_end = @()
        attempt_seen = $false; attempt_first_seen_at = $null; attempt_present_at_end = $null; attempt_text = $null
        installer_alive_at_end = $null; windows_at_end = $null
        marker_initial = $InitialMarker; marker_last = $null; marker_changed_at = $null; marker_reached_target_at = $null; file_version_last = $null
        screenshots = (New-Object System.Collections.Generic.List[object])
    }
    $tl = Join-Path $global:DiagOut ($Prefix + '-timeline.txt')
    try { Add-Utf8NoBom $tl (('# timeline start {0} mode={1} marker={2} target={3}' -f (Get-IsoNow), $RunMode, $InitialMarker, $TargetMarker) + "`r`n") } catch { }
    $t0 = Get-Date
    $lastActivity = $t0
    $clickTime = $null
    $nodeExitAt = $null
    $appEverAlive = $false
    $appGone = $false
    $appGoneAt = $null
    $prevAppAlive = $false
    $markerReachedAt = $null
    $shotInst = $false
    $shotGone = $false
    $shotC5 = $false
    $shotC30 = $false
    $shotC60 = $false
    $stuckShots = 0
    $lastStuckShotAt = $t0
    $lastBytes = [int64]-1
    $reason = $null
    $installerAlive = $false
    $lastDirSample = $t0.AddSeconds(-60)
    $lastDirCount = [int]-2
    $seenDirs = New-Object System.Collections.Generic.List[string]
    try {
        while ($true) {
            $now = Get-Date
            $elapsed = [int]($now - $t0).TotalSeconds
            if (($null -ne $NodeProc) -and ($null -eq $nodeExitAt)) {
                $exited = $false
                try { $exited = $NodeProc.HasExited } catch { $exited = $true }
                if ($exited) {
                    $nodeExitAt = $now
                    $lastActivity = $now
                    $obs['node_exited_at'] = ConvertTo-IsoUtc $now
                    try { $obs['node_exit_code'] = $NodeProc.ExitCode } catch { }
                }
            }
            # the moment of the install click, from the node result (regex: the file is rewritten while we read it)
            if ($null -eq $clickTime) {
                $ptxt = [string](Read-FileShared (Join-Path $global:DiagOut ($Prefix + '-cdp.json')))
                $pm = [regex]::Match($ptxt, '"clicked_install_at"\s*:\s*"([^"]+)"')
                if ($pm.Success) {
                    try {
                        $clickTime = [datetime]::Parse($pm.Groups[1].Value, [System.Globalization.CultureInfo]::InvariantCulture, [System.Globalization.DateTimeStyles]::RoundtripKind).ToUniversalTime()
                        $obs['click_at'] = $pm.Groups[1].Value
                        $lastActivity = $now
                    } catch { }
                }
            }
            $sinceClick = -1.0
            if ($null -ne $clickTime) { $sinceClick = ($now.ToUniversalTime() - $clickTime).TotalSeconds }

            $procs = @(Get-InterestingProcesses)
            $marker = Get-InstalledMarker
            $fv = Get-AppFileVersion
            $upd = @(Get-UpdaterTempInfo)
            $attemptNow = [bool](Test-Path -LiteralPath (Get-AttemptPath))

            $appAlive = $false
            $installerAlive = $false
            $installerProc = $null
            foreach ($p in $procs) {
                if (([string]$p.name -ieq 'cys-app.exe') -and ([string]$p.path -like '*\Local\cys\*')) { $appAlive = $true }
                if (Test-AppInstallerProc ([string]$p.name) ([string]$p.path)) {
                    $installerAlive = $true
                    if ($null -eq $installerProc) { $installerProc = $p }
                }
            }
            # the CIM list can come back empty on an error: look again with Get-Process before the app is counted as gone
            if (-not $appAlive) {
                try {
                    foreach ($g in @(Get-Process -Name 'cys-app' -ErrorAction SilentlyContinue)) {
                        $gpath = ''
                        try { $gpath = [string]$g.Path } catch { }
                        if ((-not $gpath) -or ($gpath -like '*\Local\cys\*')) { $appAlive = $true }
                    }
                } catch { }
            }
            $bytes = [int64]0
            foreach ($u in $upd) {
                $bytes = $bytes + [int64]$u.bytes
                if (-not $seenDirs.Contains([string]$u.name)) { $seenDirs.Add([string]$u.name) }
            }
            if (($upd.Count -gt 0) -and ($null -eq $obs['updater_dir_first_seen_at'])) { $obs['updater_dir_first_seen_at'] = (Get-IsoNow) }
            if ($bytes -gt [int64]$obs['updater_dir_max_bytes']) { $obs['updater_dir_max_bytes'] = $bytes }
            $obs['marker_last'] = $marker
            $obs['file_version_last'] = $fv
            if ($attemptNow -and (-not $obs['attempt_seen'])) {
                $obs['attempt_seen'] = $true
                $obs['attempt_first_seen_at'] = (Get-IsoNow)
                $obs['attempt_text'] = (Read-AttemptFile)['text']
            }

            if ($bytes -ne $lastBytes) { $lastActivity = $now; $lastBytes = $bytes }
            if ($appAlive -ne $prevAppAlive) { $lastActivity = $now }
            # an installer that extracts or removes files changes the file count of the install folder; one that waits on a dialog changes nothing
            if ((($now - $lastDirSample).TotalSeconds) -ge 10) {
                $lastDirSample = $now
                $dc = -1
                try { $dc = [System.IO.Directory]::GetFiles((Get-InstallDir), '*', [System.IO.SearchOption]::AllDirectories).Length } catch { }
                if (($dc -ge 0) -and ($dc -ne $lastDirCount)) { $lastDirCount = $dc; $lastActivity = $now }
            }
            if ($appAlive) {
                if (-not $appEverAlive) { $appEverAlive = $true; $obs['app_alive_first_at'] = (Get-IsoNow) }
                if ($appGone) { $obs['app_relaunched'] = $true }
            } else {
                if ($appEverAlive -and (-not $appGone)) {
                    $appGone = $true
                    $appGoneAt = $now
                    $obs['app_gone_at'] = (Get-IsoNow)
                }
            }
            $prevAppAlive = $appAlive
            if ($installerAlive -and (-not $obs['installer_seen'])) {
                $obs['installer_seen'] = $true
                $obs['installer_first_seen_at'] = (Get-IsoNow)
                $obs['installer_first_seen'] = [ordered]@{ name = $installerProc.name; id = $installerProc.id; ppid = $installerProc.ppid; path = $installerProc.path; cmd = $installerProc.cmd }
                $lastActivity = $now
            }
            if ($marker -and $InitialMarker -and ($marker -ne $InitialMarker) -and ($null -eq $obs['marker_changed_at'])) { $obs['marker_changed_at'] = (Get-IsoNow); $lastActivity = $now }
            if ($TargetMarker -and ($marker -eq $TargetMarker) -and ($null -eq $markerReachedAt)) { $markerReachedAt = $now; $obs['marker_reached_target_at'] = (Get-IsoNow); $lastActivity = $now }
            if ($sinceClick -ge 0) {
                if ($appAlive) {
                    $obs['alive_samples_after_click'] = [int]$obs['alive_samples_after_click'] + 1
                    $obs['last_alive_after_click_sec'] = [int]$sinceClick
                } else {
                    $obs['dead_samples_after_click'] = [int]$obs['dead_samples_after_click'] + 1
                    if ($null -eq $obs['first_dead_after_click_sec']) { $obs['first_dead_after_click_sec'] = [int]$sinceClick }
                }
            }

            $procText = (($procs | ForEach-Object { '{0}#{1}' -f $_.name, $_.id }) -join ' ')
            $updText = (($upd | ForEach-Object { '{0}:{1}B[{2}]' -f $_.name, $_.bytes, $_.files }) -join ' ')
            $line = '[{0,5}s {1}] click+{2} marker={3} appver={4} cysapp={5} installer={6} attempt={7} procs=[{8}] updater=[{9}]' -f $elapsed, (Get-IsoNow), [int]$sinceClick, $marker, $fv, $appAlive, $installerAlive, $attemptNow, $procText, $updText
            try { Add-Utf8NoBom $tl ($line + "`r`n") } catch { }
            $obs['ticks'] = [int]$obs['ticks'] + 1

            # desktop screenshots (the CDP screenshots of the page are taken by the node driver)
            if ($installerAlive -and (-not $shotInst)) {
                $shotInst = $true
                $obs['screenshots'].Add((Save-Screenshot ($Prefix + '-screen-installer.png')))
            }
            if ($appGone -and (-not $shotGone) -and ((($now - $appGoneAt).TotalSeconds) -ge 3)) {
                $shotGone = $true
                $obs['screenshots'].Add((Save-Screenshot ($Prefix + '-screen-app-gone.png')))
            }
            if (($RunMode -eq 'on') -and ($sinceClick -ge 0)) {
                if ((-not $shotC5) -and ($sinceClick -ge 5)) { $shotC5 = $true; $obs['screenshots'].Add((Save-Screenshot ($Prefix + '-screen-click5s.png'))) }
                if ((-not $shotC30) -and ($sinceClick -ge 30)) { $shotC30 = $true; $obs['screenshots'].Add((Save-Screenshot ($Prefix + '-screen-click30s.png'))) }
                if ((-not $shotC60) -and ($sinceClick -ge 60)) { $shotC60 = $true; $obs['screenshots'].Add((Save-Screenshot ($Prefix + '-screen-click60s.png'))) }
            }
            if (($RunMode -eq 'off') -and $appGone -and $installerAlive -and ($null -eq $markerReachedAt) -and ($stuckShots -lt 4) -and ((($now - $lastStuckShotAt).TotalSeconds) -ge 60)) {
                $stuckShots++
                $lastStuckShotAt = $now
                $obs['screenshots'].Add((Save-Screenshot ('{0}-screen-installer-wait{1}.png' -f $Prefix, $stuckShots)))
            }

            # end conditions
            if ($RunMode -eq 'off') {
                if (($null -ne $markerReachedAt) -and $appAlive) { $reason = 'success: the version marker reached the target and cys-app.exe is running again'; break }
                if (($null -ne $markerReachedAt) -and ((($now - $markerReachedAt).TotalSeconds) -ge 90)) { $reason = 'the version marker reached the target but cys-app.exe did not come back within 90 s'; break }
                if ($appGone -and (-not $installerAlive) -and ($null -eq $markerReachedAt) -and ((($now - $appGoneAt).TotalSeconds) -ge 120)) { $reason = 'no installer process any more and the marker is not at the target 120 s after the app exited'; break }
                if ($appGone -and $installerAlive -and ($null -eq $markerReachedAt) -and ((($now - $lastActivity).TotalSeconds) -ge 180)) { $reason = 'an installer process is alive but nothing changed for 180 s (waiting on a dialog?)'; break }
                if (($null -ne $nodeExitAt) -and (-not $appGone) -and ((($now - $nodeExitAt).TotalSeconds) -ge 30)) { $reason = 'the node driver ended and the app is still running 30 s later: no update started'; break }
            } else {
                # the node driver observes the page for K_ON_OBSERVE_SEC after the click and writes its facts at the END of that window:
                # wait for it (never kill it early); the app liveness keeps being sampled here meanwhile
                if (($null -ne $nodeExitAt) -and ($sinceClick -lt 0) -and ((($now - $nodeExitAt).TotalSeconds) -ge 10)) { $reason = 'the node driver ended without an install click'; break }
                if (($null -ne $nodeExitAt) -and ($sinceClick -ge 0) -and ((($now - $nodeExitAt).TotalSeconds) -ge 5)) { $reason = 'the node driver finished its observation window after the click'; break }
                if (($sinceClick -ge 0) -and ($sinceClick -ge ($K_ON_OBSERVE_SEC + 45))) { $reason = 'the observation window after the install click elapsed (the node driver is still running)'; break }
            }
            if ($elapsed -ge $MaxSec) { $reason = ('max watch seconds reached ({0})' -f $MaxSec); break }
            if ((Get-MinutesLeft) -lt 4) { $reason = 'job time budget exhausted'; break }
            Start-Sleep -Seconds 2
        }
    } catch {
        $reason = 'watch loop exception: ' + $_.Exception.Message
        Add-DiagError ('Watch-AppRun ' + $Prefix) $_
    }
    $obs['end_reason'] = $reason
    $obs['ended'] = (Get-IsoNow)
    $obs['updater_dirs_seen'] = $seenDirs.ToArray()
    try { $obs['updater_dirs_at_end'] = @(Get-UpdaterTempInfo | ForEach-Object { [string]$_.name }) } catch { }
    try { $obs['attempt_present_at_end'] = [bool](Test-Path -LiteralPath (Get-AttemptPath)) } catch { }
    $obs['installer_alive_at_end'] = [bool]$installerAlive
    try { $obs['windows_at_end'] = Get-VisibleWindowsText } catch { }
    $obs['screenshots'].Add((Save-Screenshot ($Prefix + '-screen-end.png')))
    try { Add-Utf8NoBom $tl (('# end: {0}' -f $reason) + "`r`n") } catch { }
    Save-Json ($Prefix + '-observe.json') $obs 6
    return $obs
}

# =========================================================================
# SAC on (same switch and the same 3-state confirmation as sacreal-e2e.ps1 / product-launch.ps1)
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
        $RUN['notes'].Add('REAL Smart App Control could not be confirmed ON (states: appe2e-state-after-on-*.json): measurable=false, the update is not clicked')
    } else {
        $RUN['measurable'] = $true
    }
    Save-Run
}

# which node.exe may be started while SAC is on: the runner's (setup-node, signed) unless a CreateProcessW probe (no code runs) is blocked,
# then the signed node.exe of the cys runtime (control POS-SIGNED of subjects.json). Returns the path or ''.
function Select-AppDriverNode {
    $rec = [ordered]@{ runner_node = $null; runner_probe = $null; fallback_probe = $null; chosen = '' }
    $RUN['driver_node'] = $rec
    $runner = Find-Exe 'node.exe'
    $rec['runner_node'] = $runner
    if ($runner) {
        $probe = Invoke-ProbeCreate $runner
        $rec['runner_probe'] = $probe
        if (-not (Test-ProbeBlocked $probe)) {
            $rec['chosen'] = $runner
            $CTX['driver_node'] = $runner
            return [string]$runner
        }
    }
    foreach ($s in @($CTX['subjects'])) {
        if ([string]$s.kind -ne 'control-signed') { continue }
        $p2 = Invoke-ProbeCreate ([string]$s.path)
        if ($p2['ok'] -eq $true) {
            $rec['fallback_probe'] = $p2
            $rec['chosen'] = [string]$s.path
            $CTX['driver_node'] = [string]$s.path
            $RUN['notes'].Add('the runner node.exe is blocked (or missing) under SAC: the CDP driver uses the signed node.exe of the cys runtime: ' + [string]$s.path)
            return [string]$s.path
        }
    }
    $RUN['notes'].Add('no node.exe may be started under SAC: the UI flow cannot be driven')
    return ''
}

# =========================================================================
# verdicts (the node and PowerShell facts -> PASS / PARTIAL / FAIL with reasons)
# =========================================================================
function Get-OffVerdict {
    param($Cj, $Obs, $Install, [string]$ToVersion)
    $reasons = New-Object System.Collections.Generic.List[string]
    $f = [ordered]@{}
    $result = 'FAIL'
    if ($null -eq $Cj) {
        $reasons.Add('no result of the CDP driver (appe2e-off-cdp.json is missing or unreadable)')
    } elseif (-not [bool]$Cj.attached) {
        $reasons.Add('the CDP driver could not attach to the app: ' + [string]$Cj.error_summary)
    } else {
        $ui = $Cj.ui
        $btn = [bool](($ui.button_flow_complete -eq $true) -and ($ui.fallback_invoke -ne $true))
        $started = [bool]($Obs['installer_seen'] -or ($null -ne $Obs['marker_changed_at']))
        $exited = [bool](($null -ne $Obs['app_gone_at']) -or ($ui.observe.socket_closed -eq $true))
        $reached = [bool]($null -ne $Obs['marker_reached_target_at'])
        $relaunched = [bool]$Obs['app_relaunched']
        $f['app_version_seen_by_cdp'] = [string]$Cj.app_version
        $f['button_click'] = $btn
        $f['fallback_invoke'] = [bool]($ui.fallback_invoke -eq $true)
        $f['flow_error'] = [string]$ui.flow_error
        $f['confirm_title'] = [string]$ui.confirm.title
        $f['confirm_names_sac'] = [bool]($ui.confirm.sac_note_in_body -eq $true)
        $f['download_progress_events'] = $Cj.events.progress_count
        $f['updater_dir_max_bytes'] = $Obs['updater_dir_max_bytes']
        $f['updater_dirs_seen'] = $Obs['updater_dirs_seen']
        $f['installer_process_seen'] = [bool]$Obs['installer_seen']
        $f['installer_process'] = $Obs['installer_first_seen']
        $f['app_exit_seen'] = $exited
        $f['cdp_socket_closed_at'] = [string]$ui.observe.socket_closed_at
        $f['marker_initial'] = $Obs['marker_initial']
        $f['marker_last'] = $Obs['marker_last']
        $f['marker_reached_target'] = $reached
        $f['target_marker'] = $ToVersion
        $f['app_relaunched'] = $relaunched
        $f['attempt_record_seen'] = [bool]$Obs['attempt_seen']
        $f['installer_alive_at_end'] = [bool]$Obs['installer_alive_at_end']
        $f['windows_at_end'] = [string]$Obs['windows_at_end']
        $f['watch_end'] = [string]$Obs['end_reason']
        if (-not $started) {
            $reasons.Add('no installer process was seen and the version marker did not change: the update never reached the installer (download or launch failed; see the timeline and the page states)')
        } elseif (-not $exited) {
            $reasons.Add('the installer started but the app did not exit')
        } else {
            if (-not $btn) { $reasons.Add('the update was started by invoke(install_update), NOT by a button click: ' + [string]$ui.flow_error) }
            if (-not $reached) {
                $stall = ''
                if ($Obs['installer_alive_at_end'] -eq $true) { $stall = ' - an installer process was still running at the end (probably waiting on a dialog; visible windows: ' + (Limit-Text ([string]$Obs['windows_at_end']) 300) + ')' }
                $reasons.Add(('the installer started and the app exited, but the version marker did not change to {0} (end of the watch: {1}): the install did not complete{2}' -f $ToVersion, [string]$Obs['end_reason'], $stall))
            }
            elseif (-not $relaunched) { $reasons.Add('the version marker changed but cys-app.exe did not come back') }
            if ($reasons.Count -eq 0) {
                $result = 'PASS'
                $reasons.Add('the real Update button flow downloaded the installer, started it, the app exited, the version marker changed and the app came back')
            } else {
                $result = 'PARTIAL'
            }
        }
    }
    return [ordered]@{ result = $result; reasons = $reasons.ToArray(); facts = $f }
}

function Get-OnVerdict {
    param($Cj, $Obs, [string]$FromVersion)
    $reasons = New-Object System.Collections.Generic.List[string]
    $notes = New-Object System.Collections.Generic.List[string]
    $f = [ordered]@{}
    $result = 'FAIL'
    if ($RUN['measurable'] -ne $true) {
        if ($null -eq $RUN['sac_on']) { $reasons.Add('SAC was never turned on (the UI was not ready or there was no time): nothing was measured') }
        else { $reasons.Add('SAC could not be confirmed ON: not measurable') }
    } elseif ($null -eq $Cj) {
        $reasons.Add('no result of the CDP driver (appe2e-on-cdp.json is missing or unreadable)')
    } elseif (-not [bool]$Cj.attached) {
        $reasons.Add('the CDP driver could not attach to the app under SAC: ' + [string]$Cj.error_summary)
    } else {
        $ui = $Cj.ui
        $btn = [bool](($ui.button_flow_complete -eq $true) -and ($ui.fallback_invoke -ne $true))
        $bt = $ui.blocked_toast
        $toastFound = [bool]($bt.found -eq $true)
        $has4551 = [bool]($bt.has_4551 -eq $true)
        $socketClosed = [bool]($ui.observe.socket_closed -eq $true)
        $aliveSec = 0
        if ($null -ne $Obs['last_alive_after_click_sec']) { $aliveSec = [int]$Obs['last_alive_after_click_sec'] }
        $dead = [int]$Obs['dead_samples_after_click']
        $aliveOk = [bool]((-not $socketClosed) -and ($dead -eq 0) -and ($aliveSec -ge $K_ON_ALIVE_SEC))
        $left = @($Obs['updater_dirs_at_end'])
        $f['app_version_seen_by_cdp'] = [string]$Cj.app_version
        $f['button_click'] = $btn
        $f['fallback_invoke'] = [bool]($ui.fallback_invoke -eq $true)
        $f['flow_error'] = [string]$ui.flow_error
        $f['confirm_title'] = [string]$ui.confirm.title
        $f['confirm_names_smart_app_control'] = [bool]($ui.confirm.sac_note_in_body -eq $true)
        $f['blocked_toast_found'] = $toastFound
        $f['blocked_toast_name'] = [string]$bt.name
        $f['blocked_toast_has_4551'] = $has4551
        $f['blocked_toast_says_app_not_closed'] = [bool]($bt.has_kept_phrase -eq $true)
        $f['blocked_toast_detail'] = [string]$bt.detail
        $f['cdp_socket_closed'] = $socketClosed
        $f['app_alive_seconds_after_click'] = $aliveSec
        $f['app_dead_samples_after_click'] = $dead
        $f['installer_process_seen'] = [bool]$Obs['installer_seen']
        $f['installer_process'] = $Obs['installer_first_seen']
        $f['updater_dirs_seen'] = $Obs['updater_dirs_seen']
        $f['updater_dirs_left_at_end'] = $left
        $f['attempt_record_seen_while_running'] = [bool]$Obs['attempt_seen']
        $f['attempt_record_present_at_end'] = $Obs['attempt_present_at_end']
        $f['marker_initial'] = $Obs['marker_initial']
        $f['marker_last'] = $Obs['marker_last']
        $f['download_progress_events'] = $Cj.events.progress_count
        $f['watch_end'] = [string]$Obs['end_reason']
        if (-not $toastFound) { $reasons.Add('no "installer launch blocked" notification was found in the page') }
        elseif (-not $has4551) { $reasons.Add('the notification does not mention error 4551') }
        if (-not $aliveOk) { $reasons.Add(('the app did not stay alive for {0} s after the click (alive {1} s, dead samples {2}, CDP socket closed={3})' -f $K_ON_ALIVE_SEC, $aliveSec, $dead, $socketClosed)) }
        if ($Obs['installer_seen']) { $reasons.Add('an installer process was started although SAC is on') }
        if ($left.Count -gt 0) { $reasons.Add('the temp installer folder was not cleaned up: ' + ($left -join ', ')) }
        if ($Obs['attempt_present_at_end'] -eq $true) { $reasons.Add('the update attempt record (.update-attempt.json) was not cleared after the blocked launch') }
        if (([string]$Obs['marker_last']) -ne $FromVersion) { $reasons.Add(('the version marker is {0} (expected the unchanged {1})' -f [string]$Obs['marker_last'], $FromVersion)) }
        if (-not $btn) { $notes.Add('the update was started by invoke(install_update), NOT by a button click: ' + [string]$ui.flow_error) }
        if (-not [bool]($ui.confirm.sac_note_in_body -eq $true)) { $notes.Add('NOTE the confirm window text does not mention Smart App Control (the pre-install notice of the product)') }
        if (-not $Obs['attempt_seen']) { $notes.Add('NOTE the attempt record was never seen while the update ran (path assumption: %USERPROFILE%\.cys\.update-attempt.json)') }
        if ($reasons.Count -eq 0) {
            if ($btn) {
                $result = 'PASS'
                $reasons.Add('under real SAC the blocked update left the app running, no installer started, the notification named 4551, the temp folder and the attempt record were cleaned up')
            } else {
                $result = 'PARTIAL'
            }
        }
    }
    foreach ($n in $notes.ToArray()) { $reasons.Add($n) }
    return [ordered]@{ result = $result; reasons = $reasons.ToArray(); facts = $f }
}

# =========================================================================
# CodeIntegrity events of the SAC window (read AFTER the restore from the persisted log; the part from the start of the restore on is left out)
# =========================================================================
function Save-AppEvents {
    if (-not $CTX['t_sac_on']) { return }
    if ((Get-MinutesLeft) -lt 2) {
        $RUN['notes'].Add('CodeIntegrity event collection skipped: job time budget nearly exhausted')
        return
    }
    Start-Sleep -Seconds 2
    $fe = [ordered]@{ started = (Get-IsoNow); evtx_rc = $null; note = $null; counts_by_id = $null; events_from_restore_start_left_out = 0; installer_path_event_count = 0; installer_path_events = @(); cys_app_block_event_count = 0; finished = $null }
    $RUN['events'] = $fe
    $ex = Export-CIEvtx -SinceUtc $CTX['t_sac_on'] -FileName 'appe2e-ci.evtx'
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
    Save-EventsJson $ev 'appe2e-events.json'
    $fe['note'] = $ev.note
    $fe['counts_by_id'] = Get-IdCounts $ev.events
    $bo = Save-BlockEvents $ev.events 'appe2e-3077.json'
    $hits = New-Object System.Collections.Generic.List[object]
    $appHits = 0
    try {
        foreach ($b in @($bo['block_events_3076_3077'])) {
            $pj = (@($b['file_paths']) -join ' ')
            $pn = (@($b['process_names']) -join ' ')
            if ($pn -match '(?i)cys-app\.exe') { $appHits++ }
            if ($pj -match '-updater-') {
                $hits.Add([ordered]@{ id = $b['id']; time = $b['time']; marked = 'path contains -updater-'; policy_names = $b['policy_names']; file_paths = $b['file_paths']; process_names = $b['process_names']; status = $b['status'] })
            }
        }
    } catch {
        $fe['error'] = 'marking the installer-path events failed: ' + $_.Exception.Message
    }
    $fe['installer_path_event_count'] = $hits.Count
    $fe['installer_path_events'] = $hits.ToArray()
    $fe['cys_app_block_event_count'] = $appHits
    $fe['finished'] = (Get-IsoNow)
}

# =========================================================================
# summary: 10 lines at most
# =========================================================================
function Get-ClickText {
    param($Cj)
    $parts = New-Object System.Collections.Generic.List[string]
    try {
        $cl = $Cj.ui.clicks
        for ($i = 0; $i -lt @($cl).Count; $i++) {
            $c = @($cl)[$i]
            $parts.Add(('{0}={1}{2}' -f [string]$c.label, [string]$c.method, $(if ($c.ok -eq $true) { '' } else { '(failed)' })))
        }
    } catch { }
    return ($parts.ToArray() -join '; ')
}

function Write-AppSummary {
    $lines = New-Object System.Collections.Generic.List[string]
    # (README 7th section, H1) head line: where the installer under test came from, its name, size and full sha256
    $lines.Add((Get-InstallerSourceLine))
    try {
        $in = $RUN['inputs']
        $instText = 'no installer'
        $mfText = 'n/a'
        if ($in -is [System.Collections.IDictionary]) {
            if ($in['installer'] -is [System.Collections.IDictionary]) {
                $ii = $in['installer']
                $instText = ('{0} ({1} bytes, sha256 {2}, signature {3})' -f $ii['name'], $ii['size'], ([string]$ii['sha256']).Substring(0, [math]::Min(12, ([string]$ii['sha256']).Length)), $ii['signature'])
            }
            if ($in['manifest'] -is [System.Collections.IDictionary]) {
                $mm = $in['manifest']
                $mfText = ('ok={0} version {1} -> installs {2}, same as the repo copy={3}{4}' -f $mm['ok'], $mm['version'], $mm['installer_version'], $mm['same_as_repo_copy'], $(if ($mm['error']) { ' [' + [string]$mm['error'] + ']' } else { '' }))
            }
            if ($in['no_input']) { $instText = 'NO INPUT: ' + (@($in['problems'].ToArray()) -join '; ') }
        }
        $lines.Add(('app-e2e on {0}: run {1}, commit {2}; installer {3}; manifest {4}; measurable={5}' -f $env:DIAG_MATRIX_OS, $env:APPE2E_RUN_ID, ([string]$env:GITHUB_SHA).Substring(0, [math]::Min(8, ([string]$env:GITHUB_SHA).Length)), $instText, $mfText, $RUN['measurable']))
    } catch { $lines.Add('line 1 (inputs) could not be built: ' + $_.Exception.Message) }
    try {
        $oi = $RUN['off_install']
        $line2 = 'OFF: install of the product build: not run'
        if ($oi -is [System.Collections.IDictionary]) {
            $line2 = ('OFF: install of {0} with /S: rc={1} marker={2} ok={3} file version {4}' -f $CTX['from_version'], $oi['rc'], $oi['marker'], $oi['ok'], $oi['file_version'])
            $os = $RUN['off_start']
            if ($os -is [System.Collections.IDictionary]) { $line2 = $line2 + ('; app started={0} CDP ready={1} (WebView2 runtime {2})' -f $os['ok'], $os['cdp_ready'], $os['webview2_runtime']) }
        }
        $lines.Add($line2)
    } catch { $lines.Add('line 2 (OFF install) could not be built: ' + $_.Exception.Message) }
    try {
        $cj = $CTX['off_cdp']
        $t3 = 'OFF UI flow: no result'
        if ($null -ne $cj) {
            $t3 = ('OFF UI flow: button flow complete={0}, fallback invoke={1}; clicks: {2}; confirm window "{3}"; error: {4}' -f [bool]($cj.ui.button_flow_complete -eq $true), [bool]($cj.ui.fallback_invoke -eq $true), (Get-ClickText $cj), [string]$cj.ui.confirm.title, [string]$cj.ui.flow_error)
        }
        $lines.Add($t3)
    } catch { $lines.Add('line 3 (OFF UI flow) could not be built: ' + $_.Exception.Message) }
    try {
        $ov = $CTX['off_verdict']
        $t4 = 'OFF result: not run'
        if ($ov -is [System.Collections.IDictionary]) {
            $ff = $ov['facts']
            $t4 = ('OFF result {0}: download dir max {1} MB, installer process={2} ({3}), app exit={4}, marker {5} -> {6} (target {7}), relaunched={8}; {9}' -f $ov['result'], [int](([double]$ff['updater_dir_max_bytes']) / 1048576), $ff['installer_process_seen'], [string]$ff['installer_process'].path, $ff['app_exit_seen'], $ff['marker_initial'], $ff['marker_last'], $ff['target_marker'], $ff['app_relaunched'], (@($ov['reasons']) -join '; '))
        }
        $lines.Add($t4)
    } catch { $lines.Add('line 4 (OFF result) could not be built: ' + $_.Exception.Message) }
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
            $lines.Add('SAC on: the switch was never made (see app-e2e.json notes / steps)')
        }
    } catch { $lines.Add('line 5 (SAC on) could not be built: ' + $_.Exception.Message) }
    try {
        $cj2 = $CTX['on_cdp']
        $t6 = 'ON UI flow: no result'
        if ($null -ne $cj2) {
            $t6 = ('ON UI flow: button flow complete={0}, fallback invoke={1}; clicks: {2}; confirm window "{3}" names Smart App Control={4}; error: {5}' -f [bool]($cj2.ui.button_flow_complete -eq $true), [bool]($cj2.ui.fallback_invoke -eq $true), (Get-ClickText $cj2), [string]$cj2.ui.confirm.title, [bool]($cj2.ui.confirm.sac_note_in_body -eq $true), [string]$cj2.ui.flow_error)
        }
        $lines.Add($t6)
    } catch { $lines.Add('line 6 (ON UI flow) could not be built: ' + $_.Exception.Message) }
    try {
        $nv = $CTX['on_verdict']
        $t7 = 'ON block result: not run'
        if (($nv -is [System.Collections.IDictionary]) -and (@('NOT_RUN', 'SKIPPED') -contains [string]$nv['result'])) {
            $t7 = 'ON block result: not run (' + (@($nv['reasons']) -join '; ') + ')'
        } elseif ($nv -is [System.Collections.IDictionary]) {
            $fn = $nv['facts']
            $t7 = ('ON block result {0}: notification found={1} "{2}" has 4551={3}; app alive {4} s after the click (dead samples {5}, CDP socket closed={6}); installer process seen={7}; temp dirs left={8}; attempt record present at end={9}; marker {10}' -f $nv['result'], $fn['blocked_toast_found'], $fn['blocked_toast_name'], $fn['blocked_toast_has_4551'], $fn['app_alive_seconds_after_click'], $fn['app_dead_samples_after_click'], $fn['cdp_socket_closed'], $fn['installer_process_seen'], (@($fn['updater_dirs_left_at_end']) -join ','), $fn['attempt_record_present_at_end'], $fn['marker_last'])
        }
        $lines.Add($t7)
    } catch { $lines.Add('line 7 (ON block result) could not be built: ' + $_.Exception.Message) }
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
                $evText = ('{0} CodeIntegrity 3076/3077 event(s) name an -updater- path (blocked processes: {1}); {2} block event(s) by cys-app.exe' -f $ev['installer_path_event_count'], ($pn.ToArray() -join ' | '), $ev['cys_app_block_event_count'])
            }
        }
        $lines.Add('events: ' + $evText)
    } catch { $lines.Add('line 8 (events) could not be built: ' + $_.Exception.Message) }
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
            $lines.Add('restore: not recorded (see app-e2e.json)')
        }
    } catch { $lines.Add('line 9 (restore) could not be built: ' + $_.Exception.Message) }
    try {
        $vd = $RUN['verdict']
        $vText = 'VERDICT: not decided'
        if ($vd -is [System.Collections.IDictionary]) { $vText = ('VERDICT: {0} - {1}' -f $vd['result'], (@($vd['reasons']) -join '; ')) }
        if ($RUN['notes'].Count -gt 0) { $vText = $vText + ' | notes: ' + (($RUN['notes'].ToArray()) -join ' | ') }
        $lines.Add($vText)
    } catch { $lines.Add('line 10 (verdict) could not be built: ' + $_.Exception.Message) }
    $out = New-Object System.Collections.Generic.List[string]
    foreach ($l in $lines.ToArray()) {
        $one = [regex]::Replace([string]$l, '[\r\n]+', ' ')
        $out.Add((Limit-Text $one 1200))
    }
    Save-Text 'appe2e-summary.txt' (($out.ToArray()) -join "`r`n")
    foreach ($l in $out.ToArray()) { Write-Log ('SUMMARY ' + $l) }
}

# a short record of a Watch-AppRun result for app-e2e.json (the full one is in <prefix>-observe.json)
function Get-ObsBrief {
    param($Obs)
    if ($null -eq $Obs) { return $null }
    return [ordered]@{
        end_reason = $Obs['end_reason']; click_at = $Obs['click_at']; ticks = $Obs['ticks']
        node_exited_at = $Obs['node_exited_at']; node_exit_code = $Obs['node_exit_code']
        installer_seen = $Obs['installer_seen']; installer_first_seen = $Obs['installer_first_seen']
        updater_dirs_seen = $Obs['updater_dirs_seen']; updater_dir_max_bytes = $Obs['updater_dir_max_bytes']; updater_dirs_at_end = $Obs['updater_dirs_at_end']
        app_gone_at = $Obs['app_gone_at']; app_relaunched = $Obs['app_relaunched']
        alive_samples_after_click = $Obs['alive_samples_after_click']; dead_samples_after_click = $Obs['dead_samples_after_click']; last_alive_after_click_sec = $Obs['last_alive_after_click_sec']
        attempt_seen = $Obs['attempt_seen']; attempt_present_at_end = $Obs['attempt_present_at_end']
        installer_alive_at_end = $Obs['installer_alive_at_end']; windows_at_end = $Obs['windows_at_end']
        marker_initial = $Obs['marker_initial']; marker_last = $Obs['marker_last']; marker_changed_at = $Obs['marker_changed_at']; marker_reached_target_at = $Obs['marker_reached_target_at']
    }
}

# =========================================================================
# TEAM scene (5th run, LAST scene): "create a team directly" on the real UI of the app, SAC OFF.
#   gates      the SAC restore is verified (fresh state check: not enforced), the inputs are usable, enough job time is left
#   app        a live 0.14.43 app with the CDP port is used as it is; otherwise (the normal case: the cleanup step killed it) the app is
#              started again (Reset-AppState, marker check / reinstall, the WebView2 debug policy again, Start-CysApp)
#   observe    node cdp-update.mjs --mode uiteam presses the buttons (real mouse clicks) and watches the page for up to 6 minutes; this
#              side samples processes (cysd.exe cys.exe bash.exe sh.exe python*.exe: pid / parent / command line / creation time), the
#              named pipes and the team daemon folders every 2 s, before and after tables, log tails, app files
#   verdict    appe2e-team-verdict.json: PASS = the click flow started AND a new team tab stood AND a new cysd.exe process started after
#              the click AND no failure notification; the stage order / seconds / notices / seats / bash are reference facts only
# Nothing here changes the OFF / ON scenes, the restore or their verdicts: the scene runs after them and only ADDS a verdict key and lines.
# =========================================================================
function Get-TeamMinutesLeft {
    $x = 15
    $n = 0
    if ([int]::TryParse([string]$env:DIAG_TEAM_EXTRA_MIN, [ref]$n)) {
        if ($n -ge 0) { $x = $n }
    }
    return ((Get-MinutesLeft) + $x)
}

function Format-TeamCmd {
    param([string]$Cmd, [int]$Max = 400)
    $c = [regex]::Replace([string]$Cmd, '(?i)(token|password|passwd|secret|apikey|api-key|authorization)([=:\s]+)\S+', '$1$2<redacted>')
    return (Limit-Text $c $Max)
}

# the processes the team flow is about: the app, the daemons (base + team), the CLI, MSYS bash / sh and the pythons
function Get-TeamProcs {
    $res = New-Object System.Collections.Generic.List[object]
    try {
        $all = Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop
        foreach ($p in $all) {
            $n = [string]$p.Name
            if ($n -match '^(cysd|cys|cys-app|bash|sh|python[0-9.]*|pythonw[0-9.]*)\.exe$') {
                $created = $null
                try {
                    if ($null -ne $p.CreationDate) { $created = ConvertTo-IsoUtc $p.CreationDate }
                } catch { }
                $res.Add([pscustomobject]@{ name = $n; id = [int]$p.ProcessId; ppid = [int]$p.ParentProcessId; path = [string]$p.ExecutablePath; cmd = (Format-TeamCmd ([string]$p.CommandLine) 400); created = $created })
            }
        }
    } catch { }
    return $res.ToArray()
}

# named pipes whose name contains cys (the base daemon: cys, a team daemon: cys-dept-<name>)
function Get-TeamPipes {
    $o = [ordered]@{ ok = $false; total = 0; cys = @(); error = $null }
    try {
        $names = New-Object System.Collections.Generic.List[string]
        $files = [System.IO.Directory]::GetFiles('\\.\pipe\')
        foreach ($f in $files) {
            $nm = [string]$f
            $i = $nm.LastIndexOf('\')
            if ($i -ge 0) { $nm = $nm.Substring($i + 1) }
            if ($nm -match 'cys') { $names.Add($nm) }
        }
        $o['total'] = @($files).Count
        $o['cys'] = $names.ToArray()
        $o['ok'] = $true
    } catch {
        $o['error'] = $_.Exception.Message
        # fallback: the same list through cmd.exe (a pipe name with a character .NET refuses as a path makes GetFiles throw)
        try {
            $dr = Invoke-Proc -File $env:ComSpec -Arguments '/d /c dir /b \\.\pipe\' -TimeoutSec 20
            $names2 = New-Object System.Collections.Generic.List[string]
            foreach ($ln in ([string]$dr['out'] -split "`r?`n")) {
                $tn = $ln.Trim()
                if (($tn -ne '') -and ($tn -match 'cys')) { $names2.Add($tn) }
            }
            $o['cys'] = $names2.ToArray()
            $o['ok'] = $true
            $o['via'] = 'cmd dir /b'
        } catch { }
    }
    return $o
}

# team daemon folders: %LOCALAPPDATA%\cys\cys-dept-<name> (cys-dept dept_logdir on Windows: cysd.log, formation.log, ...)
function Get-TeamDeptDirs {
    $res = New-Object System.Collections.Generic.List[object]
    try {
        foreach ($d in @(Get-ChildItem -LiteralPath (Get-InstallDir) -Directory -Filter 'cys-dept-*' -ErrorAction SilentlyContinue)) {
            $files = New-Object System.Collections.Generic.List[string]
            foreach ($f in @(Get-ChildItem -LiteralPath $d.FullName -File -ErrorAction SilentlyContinue)) {
                $files.Add(('{0}:{1}B' -f $f.Name, $f.Length))
            }
            $res.Add([pscustomobject]@{ name = [string]$d.Name; files = ($files.ToArray() -join ',') })
        }
    } catch { }
    return $res.ToArray()
}

function Read-FileTailShared {
    param([string]$Path, [int]$MaxBytes = 49152)
    try {
        if (-not (Test-Path -LiteralPath $Path)) { return '' }
        $fs = New-Object System.IO.FileStream($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
        try {
            $len = [int64]$fs.Length
            $start = [int64]0
            if ($len -gt $MaxBytes) { $start = $len - $MaxBytes }
            [void]$fs.Seek($start, [System.IO.SeekOrigin]::Begin)
            $cnt = [int]($len - $start)
            $buf = New-Object byte[] $cnt
            $read = 0
            while ($read -lt $cnt) {
                $got = $fs.Read($buf, $read, $cnt - $read)
                if ($got -le 0) { break }
                $read = $read + $got
            }
            $txt = [System.Text.Encoding]::UTF8.GetString($buf, 0, $read)
            if ($start -gt 0) { $txt = '...[head cut]' + $txt }
            return $txt
        } finally { $fs.Dispose() }
    } catch { return ('[read failed: ' + $_.Exception.Message + ']') }
}

# which bash can cys-dept use: the bundled MSYS bash of the app (runtime\git) and what the runner has on its PATH (static facts)
function Get-TeamBashStatic {
    $o = [ordered]@{ install_dir = (Get-InstallDir); bundled = @(); runner_path_hits = @(); where_bash = $null; path_entries = @() }
    try {
        $inst = Get-InstallDir
        $b = New-Object System.Collections.Generic.List[object]
        foreach ($rel in @('runtime\git\bin\bash.exe', 'runtime\git\usr\bin\bash.exe', 'runtime\git\cmd\git.exe', 'runtime\python\python.exe', 'runtime\python\python3.exe', 'runtime\node\node.exe')) {
            $pth = Join-Path $inst $rel
            $ex = [bool](Test-Path -LiteralPath $pth)
            $sz = $null
            if ($ex) {
                try { $sz = (Get-Item -LiteralPath $pth).Length } catch { }
            }
            $b.Add([ordered]@{ rel = $rel; exists = $ex; size = $sz })
        }
        $o['bundled'] = $b.ToArray()
    } catch {
        $o['bundled_error'] = $_.Exception.Message
    }
    try {
        $hits = New-Object System.Collections.Generic.List[string]
        foreach ($c in @(Get-Command 'bash.exe' -All -ErrorAction SilentlyContinue)) { $hits.Add([string]$c.Source) }
        $o['runner_path_hits'] = $hits.ToArray()
    } catch { }
    try {
        $w = Invoke-Proc -File (Join-Path $env:windir 'System32\where.exe') -Arguments 'bash' -TimeoutSec 20
        $o['where_bash'] = [ordered]@{ rc = $w['rc']; out = (Limit-Text ([string]$w['out']) 600) }
    } catch { }
    try {
        $pe = New-Object System.Collections.Generic.List[string]
        foreach ($e in @(([string]$env:PATH).Split(';'))) {
            if (($e -match 'git|bash|msys|system32$') -and ($pe.Count -lt 12)) { $pe.Add($e) }
        }
        $o['path_entries'] = $pe.ToArray()
    } catch { }
    return $o
}

# where a bash.exe that was seen running comes from
function Get-TeamBashKind {
    param([string]$Path)
    $p = [string]$Path
    if (-not $p) { return 'unknown (no path)' }
    $rt = Join-Path (Get-InstallDir) 'runtime'
    if ($p.StartsWith($rt, [System.StringComparison]::OrdinalIgnoreCase) -or ($p -like '*\Local\cys\runtime\*')) { return 'bundled (install dir runtime)' }
    if ($p -like '*\Git\*') { return 'runner Git for Windows' }
    if ($p -like '*\System32\bash.exe') { return 'System32 bash.exe (WSL launcher)' }
    return 'other'
}

# a live 0.14.43 app: a cys-app.exe process of the install folder AND an answering CDP port
function Test-TeamAppAlive {
    $r = [ordered]@{ process = $false; cdp = $false; marker = (Get-InstalledMarker) }
    try {
        foreach ($g in @(Get-Process -Name 'cys-app' -ErrorAction SilentlyContinue)) {
            $gp = ''
            try { $gp = [string]$g.Path } catch { }
            if ((-not $gp) -or ($gp -like '*\Local\cys\*')) { $r['process'] = $true }
        }
    } catch { }
    if ($r['process']) {
        try {
            $resp = Invoke-WebRequest -Uri ('http://127.0.0.1:{0}/json/version' -f $K_PORT) -UseBasicParsing -TimeoutSec 3 -ErrorAction Stop
            if ($resp.StatusCode -eq 200) { $r['cdp'] = $true }
        } catch { }
    }
    return $r
}

# background: the click flow + the observation of the page (node); this process samples the machine meanwhile
function Start-NodeTeam {
    param([string]$Prefix, [string]$NodeExe, [int]$ObserveSec, [int]$ReadySec)
    $a = '"{0}" --mode uiteam --port {1} --out "{2}" --prefix {3} --max-wait-sec 300 --team-observe-sec {4} --team-ready-wait-sec {5}' -f (Get-NodeScript), $K_PORT, $global:DiagOut, $Prefix, $ObserveSec, $ReadySec
    $o = Join-Path $global:DiagOut ($Prefix + '-cdp-stdout.txt')
    $e = Join-Path $global:DiagOut ($Prefix + '-cdp-stderr.txt')
    $p = Start-Process -FilePath $NodeExe -ArgumentList $a -NoNewWindow -PassThru -RedirectStandardOutput $o -RedirectStandardError $e -ErrorAction Stop
    try { $null = $p.Handle } catch { }
    return $p
}

function Read-TeamFacts {
    param([string]$Prefix)
    $txt = Read-TextUtf8 (Join-Path $global:DiagOut ($Prefix + '-facts.json'))
    if (-not $txt) { return $null }
    try { return (ConvertFrom-Json $txt) } catch { return $null }
}

# the machine side of the observation: processes, pipes, team daemon folders every 2 s until the node driver ends
function Watch-TeamRun {
    param([string]$Prefix, $NodeProc, [int]$MaxSec)
    $obs = [ordered]@{
        started = (Get-IsoNow); ended = $null; end_reason = $null; ticks = 0
        click_at = $null; node_exited_at = $null; node_exit_code = $null
        procs_seen = (New-Object System.Collections.Generic.List[object])
        pipes_first_seen = [ordered]@{}
        pipes_last = $null
        dept_dirs_last = @()
        new_cysd = @()
        windows_at_end = $null
    }
    $tl = Join-Path $global:DiagOut ($Prefix + '-timeline.txt')
    try { Add-Utf8NoBom $tl (('# TEAM scene watch start {0} (sampled every 2 s; a line is written when the process set or the cys pipes change, and every 30 s)' -f (Get-IsoNow)) + "`r`n") } catch { }
    $t0 = Get-Date
    $seen = @{}
    $clickIso = $null
    $lastSig = ''
    $lastLine = $t0.AddSeconds(-60)
    $lastDirs = $t0.AddSeconds(-60)
    $nodeExitAt = $null
    $reason = $null
    try {
        while ($true) {
            $now = Get-Date
            $elapsed = [int]($now - $t0).TotalSeconds
            if (($null -ne $NodeProc) -and ($null -eq $nodeExitAt)) {
                $exited = $false
                try { $exited = $NodeProc.HasExited } catch { $exited = $true }
                if ($exited) {
                    $nodeExitAt = $now
                    $obs['node_exited_at'] = ConvertTo-IsoUtc $now
                    try { $obs['node_exit_code'] = $NodeProc.ExitCode } catch { }
                }
            }
            if ($null -eq $clickIso) {
                $ltxt = [string](Read-FileShared (Join-Path $global:DiagOut ($Prefix + '-live.json')))
                $lm = [regex]::Match($ltxt, '"team_clicked_at"\s*:\s*"([^"]+)"')
                if ($lm.Success) {
                    $clickIso = $lm.Groups[1].Value
                    $obs['click_at'] = $clickIso
                }
            }
            $rows = @(Get-TeamProcs)
            $parts = New-Object System.Collections.Generic.List[string]
            foreach ($p in $rows) {
                $parts.Add(('{0}#{1}' -f $p.name, $p.id))
                $k = [string]$p.id
                if ($seen.ContainsKey($k)) {
                    $seen[$k]['last_seen'] = (Get-IsoNow)
                } elseif (($p.name -ieq 'cysd.exe') -or ($p.name -ieq 'bash.exe') -or ($obs['procs_seen'].Count -lt 800)) {
                    # cys.exe / sh.exe / python rows are capped (cys ping loops make hundreds of short-lived processes); daemons and bash never are
                    $rec = [ordered]@{ name = $p.name; id = $p.id; ppid = $p.ppid; path = $p.path; cmd = $p.cmd; created = $p.created; first_seen = (Get-IsoNow); last_seen = (Get-IsoNow); after_click = $null }
                    $seen[$k] = $rec
                    $obs['procs_seen'].Add($rec)
                }
            }
            $pipes = Get-TeamPipes
            $obs['pipes_last'] = $pipes
            foreach ($pn in @($pipes['cys'])) {
                $pkey = [string]$pn
                if (-not $obs['pipes_first_seen'].Contains($pkey)) { $obs['pipes_first_seen'][$pkey] = (Get-IsoNow) }
            }
            if ((($now - $lastDirs).TotalSeconds) -ge 10) {
                $lastDirs = $now
                $obs['dept_dirs_last'] = @(Get-TeamDeptDirs)
            }
            $sig = (($parts.ToArray() -join ' ') + ' | ' + (@($pipes['cys']) -join ' '))
            if (($sig -ne $lastSig) -or ((($now - $lastLine).TotalSeconds) -ge 30)) {
                $lastSig = $sig
                $lastLine = $now
                $since = -1
                if ($clickIso) {
                    try { $since = [int](($now.ToUniversalTime() - [datetime]::Parse($clickIso, [System.Globalization.CultureInfo]::InvariantCulture, [System.Globalization.DateTimeStyles]::RoundtripKind).ToUniversalTime()).TotalSeconds) } catch { }
                }
                $line = '[{0,5}s {1}] click+{2} procs=[{3}] pipes=[{4}]' -f $elapsed, (Get-IsoNow), $since, ($parts.ToArray() -join ' '), (@($pipes['cys']) -join ' ')
                try { Add-Utf8NoBom $tl ($line + "`r`n") } catch { }
            }
            $obs['ticks'] = [int]$obs['ticks'] + 1
            if (($null -ne $nodeExitAt) -and ((($now - $nodeExitAt).TotalSeconds) -ge 3)) { $reason = 'the node driver ended'; break }
            if ($elapsed -ge $MaxSec) { $reason = ('max watch seconds reached ({0})' -f $MaxSec); break }
            if ((Get-TeamMinutesLeft) -lt 3) { $reason = 'job time budget exhausted'; break }
            Start-Sleep -Seconds 2
        }
    } catch {
        $reason = 'watch loop exception: ' + $_.Exception.Message
        Add-DiagError ('Watch-TeamRun ' + $Prefix) $_
    }
    $obs['end_reason'] = $reason
    $obs['ended'] = (Get-IsoNow)
    # a team daemon = a cysd.exe of the install folder that was CREATED after the execute click
    $inst = Get-InstallDir
    $newOnes = New-Object System.Collections.Generic.List[object]
    foreach ($rec in $obs['procs_seen']) {
        if ($clickIso -and $rec['created']) {
            $rec['after_click'] = [bool]([string]::CompareOrdinal([string]$rec['created'], [string]$clickIso) -ge 0)
        }
        $recPath = [string]$rec['path']
        $inInstall = ($recPath.StartsWith($inst, [System.StringComparison]::OrdinalIgnoreCase) -or ($recPath -like '*\Local\cys\*'))
        if (([string]$rec['name'] -ieq 'cysd.exe') -and $inInstall -and ($rec['after_click'] -eq $true)) {
            $newOnes.Add($rec)
        }
    }
    $obs['new_cysd'] = $newOnes.ToArray()
    try { $obs['windows_at_end'] = Get-VisibleWindowsText } catch { }
    try { Add-Utf8NoBom $tl (('# end: {0}' -f $reason) + "`r`n") } catch { }
    Save-Json ($Prefix + '-observe.json') $obs 7
    return $obs
}

# the logs of the team daemon(s) (cysd.log, formation.log ...: tails), the registry of the app, the team pack folders
function Save-TeamLogs {
    param([string]$Prefix)
    $o = [ordered]@{ dirs = @(); copied = @(); tails = [ordered]@{}; registry = $null; catalog_present = $null; packs = @(); error = $null }
    try {
        $copied = New-Object System.Collections.Generic.List[string]
        $dirs = @(Get-ChildItem -LiteralPath (Get-InstallDir) -Directory -Filter 'cys-dept-*' -ErrorAction SilentlyContinue)
        $dn = New-Object System.Collections.Generic.List[string]
        foreach ($d in $dirs) {
            $dn.Add([string]$d.Name)
            foreach ($f in @(Get-ChildItem -LiteralPath $d.FullName -File -Filter '*.log' -ErrorAction SilentlyContinue)) {
                $tail = Read-FileTailShared $f.FullName 49152
                Save-Text ('{0}-{1}-{2}-tail.txt' -f $Prefix, $d.Name, $f.Name) $tail
                $copied.Add(('{0}\{1} ({2} bytes)' -f $d.Name, $f.Name, $f.Length))
                $o['tails'][('{0}\{1}' -f $d.Name, $f.Name)] = $tail.Substring([math]::Max(0, $tail.Length - 1500))
            }
        }
        $o['dirs'] = $dn.ToArray()
        $o['copied'] = $copied.ToArray()
        $cysHome = Join-Path $env:USERPROFILE '.cys'
        $reg = Join-Path $cysHome 'depts.json'
        if (Test-Path -LiteralPath $reg) {
            Save-Text ($Prefix + '-depts.json') (Limit-Text (Read-FileShared $reg) 60000)
            $o['registry'] = 'saved'
        } else {
            $o['registry'] = 'absent'
        }
        $o['catalog_present'] = [bool](Test-Path -LiteralPath (Join-Path $cysHome 'dept-catalog.json'))
        $packs = New-Object System.Collections.Generic.List[string]
        foreach ($pd in @(Get-ChildItem -LiteralPath $cysHome -Directory -Filter 'pack-dept-*' -ErrorAction SilentlyContinue)) { $packs.Add([string]$pd.Name) }
        $o['packs'] = $packs.ToArray()
    } catch {
        $o['error'] = $_.Exception.Message
    }
    return $o
}

function Save-TeamNotRun {
    param([string]$Why)
    $tv = [ordered]@{ result = 'NOT_RUN'; reasons = @($Why); facts = [ordered]@{}; reference = $null; ps_reference = $null }
    $CTX['team_verdict'] = $tv
    try { $RUN['team']['skipped'] = $Why } catch { }
    Save-Json 'appe2e-team-verdict.json' $tv 6
    Write-Log ('TEAM scene not run: ' + $Why) 'WARN'
}

# PASS = the click flow started AND a new team tab stood AND a new cysd.exe started after the click AND no failure notification.
# Facts of the node driver (appe2e-team-facts.json) + facts of this side (processes). Everything else is reference.
function Get-TeamVerdict {
    param($Facts, $Obs, $PsRef, $Cj)
    $reasons = New-Object System.Collections.Generic.List[string]
    $f = [ordered]@{}
    $nodeOk = $false
    $ref = $null
    if ($null -eq $Facts) {
        $reasons.Add('the CDP driver left no facts file (appe2e-team-facts.json): the flow could not be observed')
        if ($null -ne $Cj) {
            $reasons.Add(('CDP driver result: attached={0} app_version={1} error_summary={2}' -f [string]$Cj.attached, [string]$Cj.app_version, [string]$Cj.error_summary))
        } else {
            $reasons.Add('the CDP driver left no result file either (appe2e-team-cdp.json): node did not start or died at once (see appe2e-team-cdp-stderr.txt)')
        }
    } else {
        $nodeOk = [bool]($Facts.node_ok -eq $true)
        $ref = $Facts.reference
        $fa = $Facts.facts
        foreach ($m in @($Facts.node_missing)) {
            if ($m) { $reasons.Add([string]$m) }
        }
        $f['team_clicked_at'] = [string]$Facts.team_clicked_at
        $f['app_version_seen_by_cdp'] = [string]$Facts.app_version
        $f['ready_ok'] = [bool]($fa.ready_ok -eq $true)
        $f['toggle_click_method'] = [string]$fa.toggle_click_method
        $f['entry_click_ok'] = [bool]($fa.entry_click_ok -eq $true)
        $f['entry_click_method'] = [string]$fa.entry_click_method
        $f['menu_seen'] = [bool]($fa.menu_seen -eq $true)
        $f['confirm_seen'] = [bool]($fa.confirm_seen -eq $true)
        $f['confirm_title'] = [string]$fa.confirm_title
        $f['confirm_click_ok'] = [bool]($fa.confirm_click_ok -eq $true)
        $f['confirm_click_method'] = [string]$fa.confirm_click_method
        $f['any_dom_click'] = [bool]($fa.any_dom_click -eq $true)
        $f['flow_started'] = [bool]($fa.flow_started -eq $true)
        $f['pending_seen'] = [bool]($fa.pending_seen -eq $true)
        $f['tab_created'] = [bool]($fa.tab_created -eq $true)
        $f['tab_ms'] = $fa.tab_ms
        $f['tab_name'] = [string]$fa.tab_name
        $f['tab_count_before'] = $fa.tab_count_before
        $f['tab_count_after'] = $fa.tab_count_after
        $f['failure_alert_count'] = $fa.failure_alert_count
        $f['failure_alerts'] = $fa.failure_alerts
        $f['post_creation_alert_count'] = $fa.post_creation_alert_count
        $f['stage_source'] = [string]$fa.stage_source
        $f['observe_end_reason'] = [string]$fa.observe_end_reason
        $f['flow_error'] = [string]$fa.flow_error
    }
    $newCysd = @($Obs['new_cysd'])
    $daemonNew = [bool]($newCysd.Count -gt 0)
    $f['new_cysd_count'] = $newCysd.Count
    $nc = New-Object System.Collections.Generic.List[object]
    foreach ($r in $newCysd) { $nc.Add([ordered]@{ id = $r['id']; ppid = $r['ppid']; created = $r['created']; first_seen = $r['first_seen']; path = $r['path'] }) }
    $f['new_cysd'] = $nc.ToArray()
    $f['click_at_seen_by_powershell'] = [string]$Obs['click_at']
    $f['node_exit_code'] = $Obs['node_exit_code']
    $f['watch_end'] = [string]$Obs['end_reason']
    if (-not $daemonNew) {
        $cs = New-Object System.Collections.Generic.List[string]
        foreach ($r in @($Obs['procs_seen'])) {
            if ([string]$r['name'] -ieq 'cysd.exe') { $cs.Add(('pid {0} created {1} after_click={2}' -f $r['id'], $r['created'], $r['after_click'])) }
        }
        $reasons.Add(('no new cysd.exe process of the install folder was created after the execute click (click seen at {0}; cysd.exe rows seen: {1})' -f [string]$Obs['click_at'], ($cs.ToArray() -join '; ')))
    }
    $result = 'FAIL'
    if ($nodeOk -and $daemonNew) {
        $result = 'PASS'
        $reasons.Add('the button flow started by clicks, a new team tab stood (waiting screen -> real tab), a new cysd.exe was created after the click and no failure notification was shown')
        if ($f['pending_seen'] -ne $true) { $reasons.Add('NOTE the waiting screen was not seen before the tab stood (selector or timing: read appe2e-team-ui-timeline.txt)') }
        if ($f['any_dom_click'] -eq $true) { $reasons.Add('NOTE at least one click was a DOM click (the element was not hit-testable), not a mouse event: see ui.clicks in appe2e-team-cdp.json') }
        if ([int]$f['post_creation_alert_count'] -gt 0) { $reasons.Add(('NOTE {0} failure-like notification(s) (watchdog / health) appeared AFTER the tab stood (reference only, not counted): see reference.post_creation_notices' -f [int]$f['post_creation_alert_count'])) }
        $deptPipe = $false
        if ($PsRef -is [System.Collections.IDictionary]) {
            foreach ($np in @($PsRef['new_pipes'])) {
                if ([string]$np -match '^cys-dept-') { $deptPipe = $true }
            }
        }
        if (-not $deptPipe) { $reasons.Add('NOTE no new cys-dept-* named pipe was listed after the scene (check that the new cysd.exe is the team daemon: ps_reference.pipes_first_seen, appe2e-team-timeline.txt)') }
    }
    return [ordered]@{ result = $result; reasons = $reasons.ToArray(); facts = $f; reference = $ref; ps_reference = $PsRef }
}

# the TEAM lines of the summary (after the 10 lines of the earlier scenes, which are unchanged)
function Get-TeamSummaryLines {
    $out = New-Object System.Collections.Generic.List[string]
    $tv = $CTX['team_verdict']
    if (-not ($tv -is [System.Collections.IDictionary])) {
        $out.Add('TEAM NOT_RUN: the scene did not complete (see steps.team in app-e2e.json)')
        return $out.ToArray()
    }
    if ([string]$tv['result'] -eq 'NOT_RUN') {
        $out.Add('TEAM NOT_RUN: ' + (@($tv['reasons']) -join '; '))
        return $out.ToArray()
    }
    $f = $tv['facts']
    $r = $tv['reference']
    $pr = $tv['ps_reference']
    $l1 = 'TEAM ' + [string]$tv['result'] + ': ' + (@($tv['reasons']) -join '; ')
    $l1 = $l1 + ' | clicks: toggle=' + [string]$f['toggle_click_method'] + ' entry=' + [string]$f['entry_click_method'] + ' confirm=' + [string]$f['confirm_click_method'] + ' (a DOM click was used: ' + [string]$f['any_dom_click'] + ')'
    $l1 = $l1 + ' | confirm window "' + [string]$f['confirm_title'] + '" | waiting screen seen=' + [string]$f['pending_seen']
    $l1 = $l1 + ' | new tab=' + [string]$f['tab_created'] + ' (' + [string]$f['tab_ms'] + ' ms; tabs ' + [string]$f['tab_count_before'] + ' -> ' + [string]$f['tab_count_after'] + ')'
    $l1 = $l1 + ' | failure notifications=' + [string]$f['failure_alert_count'] + ' | new cysd.exe=' + [string]$f['new_cysd_count']
    $out.Add($l1)
    $l2 = 'TEAM observed:'
    if ($null -ne $r) {
        $l2 = $l2 + ' stages (' + [string]$r.stage_source + '): ' + [string]$r.stage_line + ' | ' + [string]$r.tab_line + ' | notices after the tab: ' + [string]$r.formation_line + ' | seats at the end: ' + [string]$r.seats_line + ' | alarm ids seen: ' + [string]$r.alarm_id_count
    } else {
        $l2 = $l2 + ' no reference facts from the CDP driver'
    }
    if ($pr -is [System.Collections.IDictionary]) {
        $l2 = $l2 + ' | bash: ' + [string]$pr['bash_summary'] + ' | new pipes: ' + ((@($pr['new_pipes'])) -join ',') + ' | python/bash/cys processes seen: ' + [string]$pr['proc_rows_seen']
        if (([string]$tv['result'] -ne 'PASS') -and ($pr['daemon_log_tails'] -is [System.Collections.IDictionary])) {
            foreach ($tk in @($pr['daemon_log_tails'].Keys)) {
                $tt = [string]$pr['daemon_log_tails'][$tk]
                $l2 = $l2 + ' | log tail ' + [string]$tk + ': ' + $tt.Substring([math]::Max(0, $tt.Length - 300))
            }
        }
    }
    $out.Add($l2)
    return $out.ToArray()
}

# the TEAM result as one more key and one more line of appe2e-verdict.json (result / off / on and their reasons are NOT touched)
function Add-TeamToVerdict {
    $tv = $CTX['team_verdict']
    $res = 'NOT_RUN'
    $line = 'TEAM NOT_RUN: the scene did not complete (see steps.team in app-e2e.json)'
    if ($tv -is [System.Collections.IDictionary]) {
        $res = [string]$tv['result']
        $line = 'TEAM ' + $res + ': ' + (@($tv['reasons']) -join '; ')
    }
    if (-not ($RUN['verdict'] -is [System.Collections.IDictionary])) { return }
    $RUN['verdict']['team'] = $res
    $RUN['verdict']['reasons'] = @($RUN['verdict']['reasons']) + @($line)
    Save-Json 'appe2e-verdict.json' $RUN['verdict'] 5
}

# the TEAM lines appended to appe2e-summary.txt (the 10 lines of the earlier scenes were written before and are not rewritten)
function Add-TeamSummary {
    $sum = Join-Path $global:DiagOut 'appe2e-summary.txt'
    $add = New-Object System.Collections.Generic.List[string]
    try {
        foreach ($tl in @(Get-TeamSummaryLines)) {
            $one = [regex]::Replace([string]$tl, '[\r\n]+', ' ')
            $add.Add((Limit-Text $one 1200))
        }
    } catch {
        $add.Add('TEAM lines could not be built: ' + $_.Exception.Message)
    }
    foreach ($l in $add.ToArray()) { Write-Log ('SUMMARY ' + $l) }
    try { Add-Utf8NoBom $sum (("`r`n" + ($add.ToArray() -join "`r`n"))) } catch { }
}

function Invoke-TeamScene {
    $pfx = 'appe2e-team'
    $T = [ordered]@{ started = (Get-IsoNow); skipped = $null; gates = $null; app = $null; node = $null; before = $null; observe = $null; after = $null; logs = $null; evidence = $null; cleanup = $null; error = $null; finished = $null }
    $RUN['team'] = $T
    $CTX['team_scene_utc'] = (Get-Date).ToUniversalTime()
    $prepared = $false
    try {
        # ---- gates: what the harness needs before it can measure at all (a miss is NOT_RUN, not a product failure) ----
        if (-not $CTX['inputs_ok']) { Save-TeamNotRun 'the inputs are not usable (no installer of the product build): nothing to run'; return }
        if ((Get-TeamMinutesLeft) -lt $K_TEAM_NEED_MIN) { Save-TeamNotRun ('not enough job time left ({0} min, needs {1})' -f (Get-TeamMinutesLeft), $K_TEAM_NEED_MIN); return }
        $chk = Get-SacRealState 'before-team'
        $T['gates'] = [ordered]@{ sac_enforced = $chk['enforced']; sac_registry_value = $chk['registry']['value']; minutes_left = (Get-TeamMinutesLeft) }
        if ($chk['enforced']) { Save-TeamNotRun 'Smart App Control is still enforced after the restore: the scene needs it OFF'; return }
        $node = Find-Exe 'node.exe'
        if (-not $node) { Save-TeamNotRun 'node.exe was not found'; return }
        $prepared = $true

        # ---- the app: a live 0.14.43 app as it is, else (normal case: the cleanup step killed it) started again ----
        $alive = Test-TeamAppAlive
        $reuse = [bool]($alive['process'] -and $alive['cdp'] -and ([string]$alive['marker'] -eq [string]$CTX['from_version']))
        $T['app'] = [ordered]@{ alive_check = $alive; reused = $reuse; reset = $null; reinstall = $null; start = $null }
        if (-not $reuse) {
            $T['app']['reset'] = Reset-AppState 'before-team'
            if ((Get-InstalledMarker) -ne [string]$CTX['from_version']) {
                if ((Get-TeamMinutesLeft) -lt ($K_TEAM_NEED_MIN + 3)) { Save-TeamNotRun 'the installed app is not the product build and there is no time to reinstall it'; return }
                $T['app']['reinstall'] = Install-CysFile -Installer ([string]$CTX['installer']) -ExpectVersion ([string]$CTX['from_version'])
                if (-not $T['app']['reinstall']['ok']) { Save-TeamNotRun ('the reinstall of the product build did not leave the version marker at {0}' -f $CTX['from_version']); return }
            }
            # the cleanup step removed the WebView2 debug policy: set it again (Start-CysApp sets it when the state is empty)
            $prevPolicyRec = $RUN['webview2_policy']
            $CTX['wv2_policy'] = $null
            $st = Start-CysApp $pfx
            # Start-CysApp files its policy record under the field of the OFF scene: keep that one and file this one under its own key
            $RUN['webview2_policy_team'] = $RUN['webview2_policy']
            $RUN['webview2_policy'] = $prevPolicyRec
            $T['app']['start'] = $st
            if (-not $st['cdp_ready']) { Save-TeamNotRun ('the app did not answer on the CDP port: {0}' -f [string]$st['error']); return }
        }

        # ---- before: processes, pipes, daemon folders, bash facts ----
        $T['before'] = [ordered]@{ time = (Get-IsoNow); procs = @(Get-TeamProcs); pipes = (Get-TeamPipes); dept_dirs = @(Get-TeamDeptDirs); bash_static = (Get-TeamBashStatic) }
        Save-Text ($pfx + '-procs-before.txt') (Get-ProcessListText)
        Save-Text ($pfx + '-pipes-before.txt') ((@($T['before']['pipes']['cys']) -join "`r`n"))

        # ---- the scene: node presses the buttons and watches the page; this side watches the machine ----
        $T['node'] = [ordered]@{ node_exe = $node; started = (Get-IsoNow) }
        $nodeProc = Start-NodeTeam -Prefix $pfx -NodeExe $node -ObserveSec $K_TEAM_OBSERVE_SEC -ReadySec $K_TEAM_READY_SEC
        $obs = Watch-TeamRun -Prefix $pfx -NodeProc $nodeProc -MaxSec $K_TEAM_MAX_SEC
        try { if (-not $nodeProc.HasExited) { Stop-ProcessTree -ProcessId $nodeProc.Id } } catch { }

        # ---- after: tables, tails, files ----
        $T['after'] = [ordered]@{ time = (Get-IsoNow); procs = @(Get-TeamProcs); pipes = (Get-TeamPipes); dept_dirs = @(Get-TeamDeptDirs) }
        Save-Text ($pfx + '-procs-after.txt') (Get-ProcessListText)
        Save-Text ($pfx + '-pipes-after.txt') ((@($T['after']['pipes']['cys']) -join "`r`n"))
        $null = Save-Screenshot ($pfx + '-screen-end.png')
        $T['logs'] = Save-TeamLogs $pfx
        $T['evidence'] = Save-AppEvidence $pfx
        try { Save-AppEventLog $pfx $CTX['team_scene_utc'] } catch { }
        $T['observe'] = [ordered]@{ end_reason = $obs['end_reason']; click_at = $obs['click_at']; ticks = $obs['ticks']; node_exited_at = $obs['node_exited_at']; node_exit_code = $obs['node_exit_code']; new_cysd = @($obs['new_cysd']).Count; windows_at_end = $obs['windows_at_end'] }

        # ---- reference facts of this side (bash that ran, new pipes) and the verdict ----
        $bashRows = New-Object System.Collections.Generic.List[object]
        $kinds = @{}
        $procRows = 0
        foreach ($rec in $obs['procs_seen']) {
            if (@('bash.exe', 'sh.exe', 'python.exe', 'python3.exe', 'pythonw.exe', 'cys.exe') -contains ([string]$rec['name']).ToLowerInvariant()) { $procRows++ }
            if ([string]$rec['name'] -ieq 'bash.exe') {
                $kd = Get-TeamBashKind ([string]$rec['path'])
                $bashRows.Add([ordered]@{ id = $rec['id']; ppid = $rec['ppid']; kind = $kd; path = $rec['path']; cmd = $rec['cmd']; created = $rec['created']; first_seen = $rec['first_seen']; last_seen = $rec['last_seen'] })
                if ($kinds.ContainsKey($kd)) { $kinds[$kd] = [int]$kinds[$kd] + 1 } else { $kinds[$kd] = 1 }
            }
        }
        $kt = New-Object System.Collections.Generic.List[string]
        foreach ($kk in @($kinds.Keys)) { $kt.Add(('{0} x{1}' -f $kk, $kinds[$kk])) }
        $bashSummary = 'no bash.exe was seen running'
        if ($kt.Count -gt 0) { $bashSummary = ($kt.ToArray() -join ', ') }
        $newPipes = New-Object System.Collections.Generic.List[string]
        foreach ($pn in @($T['after']['pipes']['cys'])) {
            if (@($T['before']['pipes']['cys']) -notcontains [string]$pn) { $newPipes.Add([string]$pn) }
        }
        $psRef = [ordered]@{ bash_summary = $bashSummary; bash_rows = $bashRows.ToArray(); bash_static = $T['before']['bash_static']; new_pipes = $newPipes.ToArray(); pipes_first_seen = $obs['pipes_first_seen']; proc_rows_seen = $procRows; dept_dirs_after = $T['after']['dept_dirs']; daemon_log_tails = $T['logs']['tails'] }
        $facts = Read-TeamFacts $pfx
        $cjTeam = $null
        if ($null -eq $facts) { $cjTeam = Read-CdpJson $pfx }
        $tv = Get-TeamVerdict -Facts $facts -Obs $obs -PsRef $psRef -Cj $cjTeam
        $CTX['team_verdict'] = $tv
        Save-Json 'appe2e-team-verdict.json' $tv 8
    } catch {
        $T['error'] = Format-ErrorText $_
        Add-DiagError 'team scene' $_
        if ($null -eq $CTX['team_verdict']) { Save-TeamNotRun ('the scene stopped on an exception: ' + $_.Exception.Message) }
    } finally {
        if ($prepared) {
            $cl = [ordered]@{ node_killed = @(); reset = $null; webview2_policy_removed = $null }
            $T['cleanup'] = $cl
            try {
                $nk = New-Object System.Collections.Generic.List[string]
                foreach ($np in @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -Filter "Name = 'node.exe'" -ErrorAction Stop)) {
                    if ([string]$np.CommandLine -like '*cdp-update.mjs*') {
                        Stop-ProcessTree -ProcessId ([int]$np.ProcessId)
                        $nk.Add(('node#{0}' -f $np.ProcessId))
                    }
                }
                $cl['node_killed'] = $nk.ToArray()
            } catch { }
            try { $cl['reset'] = Reset-AppState 'after-team' } catch { }
            try { $cl['webview2_policy_removed'] = Remove-WebView2DebugPolicy $CTX['wv2_policy'] } catch { }
        }
        $T['finished'] = (Get-IsoNow)
    }
}

# =========================================================================
# scene switch (README 7th section): APPE2E_SCENE=upgrade runs the UPGRADE scene of diag/app-upgrade.ps1 and ends here.
# Every other value (fresh, empty) falls through to the main below.
# =========================================================================
if (([string]$env:APPE2E_SCENE).Trim().ToLowerInvariant() -eq 'upgrade') {
    if (Get-Command -Name 'Invoke-UpgradeMain' -CommandType Function -ErrorAction SilentlyContinue) {
        Invoke-UpgradeMain
    } else {
        # the library did not load (syntax-check.txt of this job names the parse error): say so in the standard result files
        $why = 'the UPGRADE scene could not run: ' + $(if ($K_UPG_LOAD_ERROR) { $K_UPG_LOAD_ERROR } else { 'Invoke-UpgradeMain is not defined (diag/app-upgrade.ps1 missing?)' })
        Write-Log $why 'ERROR'
        $RUN['notes'].Add($why)
        $RUN['measurable'] = $false
        $RUN['verdict'] = [ordered]@{ result = 'NOT-MEASURABLE'; upgrade = 'NOT-MEASURABLE'; team = 'NOT_RUN'; reasons = @('NOT MEASURED: ' + $why) }
        Save-Json 'appe2e-verdict.json' $RUN['verdict'] 5
        # the same keys as the verdict file of the scene itself (Get-UpgVerdict), so that a reader finds one shape
        $uv = [ordered]@{ result = 'NOT-MEASURABLE'; upgrade = 'NOT-MEASURABLE'; team = 'NOT_RUN'; mode = ([string]$env:APPE2E_UPGRADE_MODE).Trim(); from_version = ([string]$env:APPE2E_UPGRADE_FROM).Trim(); to_version = ([string]$env:APPE2E_UPGRADE_EXPECT).Trim(); failures = @(); not_measurable = @($why); notes = @(); reasons = @('NOT MEASURED: ' + $why); facts = [ordered]@{} }
        Save-Json 'appe2e-upgrade-verdict.json' $uv 5
        Save-Text 'appe2e-upgrade-summary.txt' ('VERDICT: NOT-MEASURABLE - ' + $why)
        $RUN['finished'] = (Get-IsoNow)
        Save-Run
        Complete-DiagScript 'app-e2e'
    }
    exit 0
}

# =========================================================================
# main
# =========================================================================
$CTX['script_start_utc'] = (Get-Date).ToUniversalTime()
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
        try { $null = Invoke-WebRequest -Uri 'http://127.0.0.1:9/' -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop } catch { }
        try { $null = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -ErrorAction Stop } catch { }
        $prep['language_mode'] = Get-LangMode
        try {
            $nt = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -ErrorAction Stop
            $prep['os_product'] = ('{0} {1} build {2}.{3}' -f $nt.ProductName, $nt.DisplayVersion, $nt.CurrentBuild, $nt.UBR)
        } catch { }
        $prep['node_exe'] = Find-Exe 'node.exe'
        Initialize-Subjects
        $prep['subjects_loaded'] = @($CTX['subjects']).Count
        # the CodeIntegrity/Operational log is 1 MB by default: raise it to 128 MB before the SAC window
        $we = Join-Path $env:windir 'System32\wevtutil.exe'
        $sl = Invoke-Proc -File $we -Arguments 'sl Microsoft-Windows-CodeIntegrity/Operational /ms:134217728' -TimeoutSec 30
        $prep['ci_log_size_set_rc'] = $sl['rc']
        # baseline: SAC must be OFF now (value 0, nothing enforced)
        $init = Get-SacRealState 'initial'
        $RUN['sac_initially_enforced'] = $init['enforced']
        if ($init['enforced']) { $RUN['notes'].Add('SAC was ALREADY enforced at the start of the run: the SAC-off baseline premise does not hold') }
        # the inputs of this run (installer artifact, manifest)
        Get-AppInputs
        $in = $RUN['inputs']
        if ($in['installer_ok']) { Test-ManifestReachable }
        $mf = $in['manifest']
        $CTX['inputs_ok'] = [bool]($in['installer_ok'] -and ($mf -is [System.Collections.IDictionary]) -and ($mf['ok'] -eq $true))
        if (-not $CTX['inputs_ok']) {
            foreach ($pr in $in['problems'].ToArray()) { $RUN['notes'].Add([string]$pr) }
            if (($mf -is [System.Collections.IDictionary]) -and $mf['error']) { $RUN['notes'].Add('manifest: ' + [string]$mf['error']) }
        }
        Save-Run
    }

    if ($CTX['inputs_ok']) {
        # ---------------------------------------------------------------- OFF: the success path
        if ((Get-MinutesLeft) -gt 25) {
            Invoke-Step 'install-off' {
                $RUN['reset_before_off'] = Reset-AppState 'before-off'
                $RUN['off_install'] = Install-CysFile -Installer ([string]$CTX['installer']) -ExpectVersion ([string]$CTX['from_version'])
                $CTX['off_install_ok'] = [bool]($RUN['off_install']['ok'])
                if (-not $CTX['off_install_ok']) { $RUN['notes'].Add(('the installer of the product build did not leave the version marker at {0} (marker: {1})' -f $CTX['from_version'], $RUN['off_install']['marker'])) }
                Save-Run
            }
            # (README 7th section, H2-b) read-only facts about what the installer left behind: a record, never part of a verdict
            Invoke-Step 'install-facts' {
                $RUN['install_facts'] = Get-InstallFacts 'fresh-after-first-install'
                Save-Json 'appe2e-install-facts.json' $RUN['install_facts'] 7
            }
        } else {
            $RUN['notes'].Add('install-off skipped: not enough job time left')
        }
        if ($CTX['off_install_ok']) {
            Invoke-Step 'off-run' {
                $prefix = 'appe2e-off'
                $st = Start-CysApp $prefix
                $RUN['off_start'] = $st
                Save-Run
                $node = Find-Exe 'node.exe'
                if ((-not $st['cdp_ready']) -or (-not $node)) {
                    $RUN['notes'].Add('OFF run not driven: the app did not answer on the CDP port or node.exe is missing')
                    return
                }
                $initial = Get-InstalledMarker
                $nodeProc = Start-NodeUi -Prefix $prefix -NodeExe $node -ObserveSec $K_OFF_OBSERVE_SEC
                $obs = Watch-AppRun -Prefix $prefix -RunMode 'off' -NodeProc $nodeProc -InitialMarker $initial -TargetMarker ([string]$CTX['to_version']) -MaxSec $K_OFF_MAX_SEC
                try { if (-not $nodeProc.HasExited) { Stop-ProcessTree -ProcessId $nodeProc.Id } } catch { }
                $CTX['off_obs'] = $obs
                $RUN['off_observe'] = Get-ObsBrief $obs
                $cj = Read-CdpJson $prefix
                $CTX['off_cdp'] = $cj
                $RUN['off_attempt_after'] = Read-AttemptFile
                $RUN['off_app_evidence'] = Save-AppEvidence $prefix
                try { Save-AppEventLog $prefix $CTX['script_start_utc'] } catch { }
                $ov = Get-OffVerdict -Cj $cj -Obs $obs -Install $RUN['off_install'] -ToVersion ([string]$CTX['to_version'])
                $CTX['off_verdict'] = $ov
                Save-Json 'appe2e-off-verdict.json' $ov 7
                $RUN['off_verdict'] = [ordered]@{ result = $ov['result']; reasons = $ov['reasons'] }
                Save-Run
            }
        }

        # ---------------------------------------------------------------- ON: the blocked path under REAL SAC
        $onTimeOk = ((Get-MinutesLeft) -gt 14)
        if ($onTimeOk) {
            Invoke-Step 'reinstall' {
                $RUN['reset_before_on'] = Reset-AppState 'before-on'
                $RUN['on_install'] = Install-CysFile -Installer ([string]$CTX['installer']) -ExpectVersion ([string]$CTX['from_version'])
                $CTX['on_install_ok'] = [bool]($RUN['on_install']['ok'])
                if (-not $CTX['on_install_ok']) { $RUN['notes'].Add(('the second install of the product build did not leave the version marker at {0} (marker: {1})' -f $CTX['from_version'], $RUN['on_install']['marker'])) }
                Save-Run
            }
            if ($CTX['on_install_ok']) {
                Invoke-Step 'on-pre' {
                    $st = Start-CysApp 'appe2e-on'
                    $RUN['on_start'] = $st
                    $node = Find-Exe 'node.exe'
                    if ($st['cdp_ready'] -and $node) {
                        $pre = Invoke-NodePre -Prefix 'appe2e-on-pre' -NodeExe $node
                        $RUN['on_pre'] = $pre
                        $CTX['on_ready'] = [bool]($pre['attached'] -and $pre['ui_ready'])
                    }
                    if (-not $CTX['on_ready']) { $RUN['notes'].Add('the app UI was not ready before SAC: SAC is not turned on') }
                    Save-Run
                }
            }
        } else {
            $RUN['notes'].Add('ON path skipped: not enough job time left')
        }

        # everything measured so far is safe on a results branch BEFORE the policy changes
        Invoke-Step 'checkpoint-pre-sac' { $null = Publish-Checkpoint 'pre-sac' }

        if ($CTX['on_ready'] -and ((Get-MinutesLeft) -gt 10)) {
            Invoke-Step 'sac-on' { Invoke-SacOn }
        } elseif ($CTX['on_ready']) {
            $RUN['notes'].Add('SAC-on skipped: not enough job time left')
        }

        Invoke-Step 'on-run' {
            if ($RUN['measurable'] -ne $true) {
                $RUN['notes'].Add('ON run not driven: SAC is not confirmed on')
                return
            }
            $node = Select-AppDriverNode
            if (-not $node) { return }
            # SAC must still be enforced right before the click
            $chk = Get-SacRealState 'before-click'
            $RUN['on_sac_enforced_before_click'] = [bool]$chk['enforced']
            if (-not $chk['enforced']) { $RUN['notes'].Add('SAC was NOT enforced any more right before the click: the ON result is not valid for SAC on') }
            $prefix = 'appe2e-on'
            $initial = Get-InstalledMarker
            $nodeProc = Start-NodeUi -Prefix $prefix -NodeExe $node -ObserveSec $K_ON_OBSERVE_SEC
            $obs = Watch-AppRun -Prefix $prefix -RunMode 'on' -NodeProc $nodeProc -InitialMarker $initial -TargetMarker '' -MaxSec 600
            try { if (-not $nodeProc.HasExited) { Stop-ProcessTree -ProcessId $nodeProc.Id } } catch { }
            $CTX['on_obs'] = $obs
            $RUN['on_observe'] = Get-ObsBrief $obs
            $CTX['on_cdp'] = Read-CdpJson $prefix
            Save-Run
        }
    }
} catch {
    Add-DiagError 'app-e2e main' $_
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
            $rs['neg_control_run'] = [ordered]@{ rc = $nr['rc']; timed_out = $nr['timedOut']; start_error = $nr['startError']; stdout = (Limit-Text $nr['out'] 300) }
            $rs['neg_control_runs_again'] = [bool](($nr['rc'] -eq 0) -and ([string]$nr['out'] -match 'diag-neg'))
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
    Invoke-Step 'cleanup' {
        $cl = [ordered]@{ node_killed = @(); reset = $null; webview2_policy_removed = $null }
        $RUN['cleanup'] = $cl
        # evidence of the ON run needs the app files: collect before everything is killed
        try { $RUN['on_app_evidence'] = Save-AppEvidence 'appe2e-on' } catch { }
        try { Save-AppEventLog 'appe2e-on' $CTX['script_start_utc'] } catch { }
        # node drivers that are still running (never started from inside the install dir)
        $nk = New-Object System.Collections.Generic.List[string]
        try {
            foreach ($np2 in @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -Filter "Name = 'node.exe'" -ErrorAction Stop)) {
                if ([string]$np2.CommandLine -like '*cdp-update.mjs*') {
                    Stop-ProcessTree -ProcessId ([int]$np2.ProcessId)
                    $nk.Add(('node#{0}' -f $np2.ProcessId))
                }
            }
        } catch { }
        $cl['node_killed'] = $nk.ToArray()
        $cl['reset'] = Reset-AppState 'final'
        try { $cl['webview2_policy_removed'] = Remove-WebView2DebugPolicy $CTX['wv2_policy'] } catch { }
    }
    Invoke-Step 'events' { Save-AppEvents }
    Invoke-Step 'verdict' {
        $inp = $RUN['inputs']
        $offV = $CTX['off_verdict']
        if ($null -eq $offV) { $offV = [ordered]@{ result = 'NOT_RUN'; reasons = @('the OFF run did not happen (see the notes)'); facts = [ordered]@{} } }
        $obsOn = $CTX['on_obs']
        if ($null -eq $obsOn) { $obsOn = [ordered]@{} }
        if ($CTX['inputs_ok']) {
            $onV = Get-OnVerdict -Cj $CTX['on_cdp'] -Obs $obsOn -FromVersion ([string]$CTX['from_version'])
        } else {
            $onV = [ordered]@{ result = 'NOT_RUN'; reasons = @('the inputs are not usable (see the notes): nothing was run'); facts = [ordered]@{} }
        }
        $ev = $RUN['events']
        if (($onV['result'] -eq 'PASS') -and ($ev -is [System.Collections.IDictionary]) -and ($null -ne $ev['finished']) -and ([int]$ev['installer_path_event_count'] -eq 0)) {
            $onV['reasons'] = @($onV['reasons']) + @('NOTE no CodeIntegrity 3076/3077 event names an -updater- installer path (the event may carry no path, or the blocked launch is logged differently)')
        }
        $CTX['on_verdict'] = $onV
        Save-Json 'appe2e-on-verdict.json' $onV 7
        $all = New-Object System.Collections.Generic.List[string]
        $overall = 'FAIL'
        if ((($null -ne $inp) -and ($inp['no_input'] -eq $true))) {
            $overall = 'SKIPPED'
            $all.Add('no input: diag/appe2e-input.json windows_build_run_id is 0')
        } elseif (-not $CTX['inputs_ok']) {
            $all.Add('the inputs are not usable: ' + (@($RUN['notes'].ToArray()) -join ' | '))
        } else {
            if (($offV['result'] -eq 'PASS') -and ($onV['result'] -eq 'PASS')) { $overall = 'PASS' }
            elseif (($offV['result'] -eq 'FAIL') -or ($offV['result'] -eq 'NOT_RUN') -or ($onV['result'] -eq 'FAIL')) { $overall = 'FAIL' }
            else { $overall = 'PARTIAL' }
            $all.Add(('OFF {0}: {1}' -f $offV['result'], (@($offV['reasons']) -join '; ')))
            $all.Add(('ON {0}: {1}' -f $onV['result'], (@($onV['reasons']) -join '; ')))
        }
        if ($RUN['restore_ok'] -ne $true) { $all.Add('NOTE the restore could not be verified (see restore)') }
        $RUN['verdict'] = [ordered]@{ result = $overall; off = $offV['result']; on = $onV['result']; reasons = $all.ToArray() }
        Save-Json 'appe2e-verdict.json' $RUN['verdict'] 5
    }
    Invoke-Step 'summary' { Write-AppSummary }
    # the TEAM scene: LAST - after the verdict and the summary of the earlier scenes are already on disk (a hang or a crash of this scene
    # cannot take them away), after the restore and the cleanup; it checks SAC again. The report step then ADDS a team key + line to
    # appe2e-verdict.json and two TEAM lines to appe2e-summary.txt (the 10 lines written above stay byte for byte as they are).
    Invoke-Step 'team' { Invoke-TeamScene }
    Invoke-Step 'team-report' { Add-TeamSummary; Add-TeamToVerdict }
    $RUN['finished'] = (Get-IsoNow)
    Save-Run
    Complete-DiagScript 'app-e2e'
}
exit 0
