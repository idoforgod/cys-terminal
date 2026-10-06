# diag/app-upgrade.ps1 -- LIBRARY of diag/app-e2e.ps1 (dot-sourced there; constants and functions only, there is no main in this file).
#   Get-InstallFacts     read-only facts about what an installer left behind (both scenes; never part of a verdict): the uninstall
#                        entry, shortcuts, the install folder (files / bytes), the three executables, "--version" of cys.exe / cysd.exe,
#                        the bundled bash / python. Scene fresh: once, right after the first install (appe2e-install-facts.json).
#   Invoke-UpgradeMain   scene UPGRADE (APPE2E_SCENE=upgrade; app-e2e.ps1 calls it INSTEAD of its own main = scene fresh):
#     a PUBLIC older version (APPE2E_UPGRADE_FROM, default 0.14.42) is installed with /S and started WITHOUT any manifest override
#     (its real endpoint), then the version under test is put on top of it the way that old app's own updater does it:
#       mode emulate  the harness copies the installer under test to %TEMP%\cys-<to>-updater-diag<6>\cys-<to>-installer.exe and starts
#                     it with ShellExecuteW(open, <file>, "/P /R /UPDATE /ARGS") - the call of tauri-plugin-updater 2.10.1 - and then
#                     ends ONLY the app process (the plugin calls exit(0) right after ShellExecuteW). The daemon is not touched: it
#                     survives, exactly as with the plugin (the app's own daemon hand-over never runs on the NSIS path).
#       mode updater  the old app updates ITSELF from the public endpoint: node cdp-update.mjs --mode attach reads check_update (its
#                     version must be APPE2E_UPGRADE_EXPECT, else "the public endpoint does not offer it"), then --mode update invokes
#                     install_update {force:true}.
#     Then: marker / failure file / file versions, the app the installer started again (version, UI), the meeting of the NEW app with
#     the OLD daemon (150 s: does the app rotate it by itself, or does it show its version-skew badge), leftovers (*.prev* / *.new.exe),
#     the pack before and after, and - on that same upgraded app, as it is - the TEAM scene of app-e2e.ps1 (Invoke-TeamScene).
#   steps   prepare, upg-audit-on, upg-base-installer, upg-install-base, upg-facts-base, upg-start-base, upg-base-state, upg-apply,
#           upg-after, upg-facts-after, upg-proc-audit, team, upg-second-start, upg-verdict, upg-summary, upg-cleanup
#   UPG-2   (records and NOTES only, the PASS / FAIL rules are as before) the processes the upgrade and the relaunched app start
#           (Security 4688 / 4689 with command lines and exit status -> appe2e-upg-proc-audit.{json,txt}, -end.{json,txt}; when the
#           audit cannot be used: diag/proc-poll.ps1 -> appe2e-upg-proc-poll.json, no exit status), every notification of the upgraded
#           app during a page watch of at least 45 s, and the marker / stamp files of the product at four moments. Three summary lines.
#   UPG-3   (records and NOTES only) the older app has to be SETTLED before the upgrade (stamp and onboarding marker at its version,
#           no pack staging, no init-pack / restore running: up to 90 s more); the stamp record carries .gui-onboarded and
#           .gui-onboard-attempts; after the TEAM scene step upg-second-start starts the upgraded app once more and files what
#           it runs (init-pack with / without --no-install-hook, daemon install), its notifications and the stamp files.
#   files   appe2e-upgrade-verdict.json, appe2e-upgrade-summary.txt (18 lines at most), appe2e-verdict.json + app-e2e.json
#           (verdict = { result, upgrade, team, reasons }), appe2e-upg-*.{json,txt,png}, appe2e-install-facts-{base,after}.json
#   verdict PASS only when everything holds: base install + base app start (else NOT-MEASURABLE), the marker reached the target and
#           the app came back, marker / no failure file / the three file versions, the new app's version and UI, TEAM PASS.
#           FAIL = the product did something wrong. NOT-MEASURABLE = the harness could not measure. NOTE = recorded, never changes
#           the verdict (the old daemon was not rotated in the window / the badge showed, the base app was not fully up, leftovers).
#           A watch of step 6 that the harness itself gave up (job time, an error in its loop: apply_observe.end_kind = harness) is
#           NOT-MEASURABLE, and what step 7 read right then is recorded but not judged (the installer may still have been running).
# CONTRACT: everything of app-e2e.ps1 is used as it is there ($RUN $CTX $K_PORT, Save-Run, Get-AppInputs, Install-CysFile,
# Reset-AppState, Start-CysApp, Invoke-NodePre, Test-AppInstallerProc, Test-TeamAppAlive, Get-TeamPipes, Format-TeamCmd,
# Invoke-TeamScene, Save-TeamNotRun, Get-TeamSummaryLines, Get-InstallerSourceLine ...). $CTX['from_version'] keeps its meaning there:
# the version of the installer UNDER TEST (Invoke-TeamScene reuses a live app whose marker equals it). The older version is
# $CTX['upg_base_version']. OBSERVATION ONLY: no Smart App Control / Defender bypass, and this scene never turns SAC on.
# Windows PowerShell 5.1, ASCII only.

$K_UPG_PFX = 'appe2e-upg'
$K_UPG_REPO = 'idoforgod/cys-terminal'
$K_UPG_BASE_READY_SEC = 120
$K_UPG_BASE_TICK_SEC = 3
$K_UPG_APPLY_MAX_SEC = 300
$K_UPG_OBS_SEC = 150
$K_UPG_OBS_TICK_SEC = 5
$K_UPG_INSTALLER_ARGS = '/P /R /UPDATE /ARGS'
# UPG-2 (README 7th section, "UPG-2"): the page is polled every second and for at least 45 s after the upgrade (notifications come
# and go in the first seconds); process creation / termination auditing (Security 4688 / 4689) is switched on for this scene only.
# Subcategory GUIDs (the names are localised, the GUIDs are not; "auditpol /list /subcategory:* /v" prints them):
# Detailed Tracking > Process Creation {0CCE922B-...}, Process Termination {0CCE922C-...}.
$K_UPG_OBS_MIN_SEC = 45
$K_UPG_OBS_PAGE_TICK_SEC = 1
$K_UPG_AUDIT_CREATE = '{0CCE922B-69AE-11D9-BED3-505054503030}'
$K_UPG_AUDIT_EXIT = '{0CCE922C-69AE-11D9-BED3-505054503030}'
$K_UPG_AUDIT_REGKEY = 'HKLM\Software\Microsoft\Windows\CurrentVersion\Policies\System\Audit'
$K_UPG_AUDIT_REGVAL = 'ProcessCreationIncludeCmdLine_Enabled'
$K_UPG_POLL_SEC = 30
$K_UPG_POLL_MS = 200
# UPG-3: the older app has to be past its own first-run work before the upgrade is applied (settled: up to 90 s more, every 2 s);
# after the TEAM scene the upgraded app is started a second time (a record: does the product retry its onboarding, does it work)
$K_UPG_SETTLE_SEC = 90
$K_UPG_SETTLE_TICK_SEC = 2
$K_UPG_SECOND_OBS_SEC = 45
$K_UPG_SECOND_MIN_SEC = 30
$K_UPG_SECOND_NEED_MIN = 4

# =========================================================================
# small readers
# =========================================================================
# the numeric file version of an exe as "a.b.c" (VS_FIXEDFILEINFO - the numbers the installer hook's own oracle compares); '' when unreadable
function Get-UpgFileVersion3 {
    param([string]$Path)
    try {
        if ((-not $Path) -or (-not (Test-Path -LiteralPath $Path))) { return '' }
        $vi = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($Path)
        return ('{0}.{1}.{2}' -f [int]$vi.FileMajorPart, [int]$vi.FileMinorPart, [int]$vi.FileBuildPart)
    } catch { return '' }
}

function Get-UpgExeRecord {
    param([string]$Path, [bool]$WithHash = $false)
    $r = [ordered]@{ path = $Path; exists = $false; size = $null; file_version = $null; product_version = $null; version3 = $null; sha256 = $null }
    try {
        if (Test-Path -LiteralPath $Path) {
            $r['exists'] = $true
            $it = Get-Item -LiteralPath $Path
            $r['size'] = $it.Length
            $r['file_version'] = [string]$it.VersionInfo.FileVersion
            $r['product_version'] = [string]$it.VersionInfo.ProductVersion
            $r['version3'] = Get-UpgFileVersion3 $Path
            if ($WithHash) { $r['sha256'] = Get-Sha256 $Path }
        }
    } catch { $r['error'] = $_.Exception.Message }
    return $r
}

# a value one or two keys deep in nested dictionaries, $null when a level is missing (for report lines only: scalars)
function Get-UpgVal {
    param($Obj, [string]$K1, [string]$K2 = '')
    try {
        if (-not ($Obj -is [System.Collections.IDictionary])) { return $null }
        $v = $Obj[$K1]
        if ($K2 -eq '') { return $v }
        if (-not ($v -is [System.Collections.IDictionary])) { return $null }
        return $v[$K2]
    } catch { return $null }
}

# a value for a report line: its text, or the given word when there is none (not measured / not seen)
function Format-UpgVal {
    param($Value, [string]$IfNone = 'unknown')
    if ($null -eq $Value) { return $IfNone }
    $t = [string]$Value
    if ($t -eq '') { return $IfNone }
    return $t
}

function Read-UpgJson {
    param([string]$Name)
    $txt = Read-TextUtf8 (Join-Path $global:DiagOut $Name)
    if (-not $txt) { return $null }
    try { return (ConvertFrom-Json $txt) } catch { return $null }
}

# =========================================================================
# install facts (README 7th section, H2-b): READ-ONLY, both scenes, never part of a verdict
# =========================================================================
# where an installer puts shortcuts: the start menu (this user, all users; searched with subfolders) and the desktop (this user, public)
function Get-UpgShortcutPlaces {
    $pl = New-Object System.Collections.Generic.List[object]
    $pl.Add([ordered]@{ kind = 'start_menu'; root = [string](Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'); recurse = $true })
    $pl.Add([ordered]@{ kind = 'start_menu'; root = [string](Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs'); recurse = $true })
    $pl.Add([ordered]@{ kind = 'desktop'; root = [string][System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::DesktopDirectory); recurse = $false })
    $pl.Add([ordered]@{ kind = 'desktop'; root = [string][System.Environment]::GetFolderPath([System.Environment+SpecialFolder]::CommonDesktopDirectory); recurse = $false })
    return $pl.ToArray()
}

function Get-InstallFacts {
    param([string]$Tag)
    $dir = Get-InstallDir
    $f = [ordered]@{
        tag = $Tag; time = (Get-IsoNow); install_dir = $dir; install_dir_exists = $false; marker = $null; failure_file_present = $null
        uninstall = $null; shortcuts = $null; files = $null; core = $null; cli = $null; bundled = $null
        ms = $null; errors = (New-Object System.Collections.Generic.List[string])
    }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        $f['install_dir_exists'] = [bool](Test-Path -LiteralPath $dir)
        $f['marker'] = Get-InstalledMarker
        $f['failure_file_present'] = [bool](Get-FailureMarker)
    } catch { $f['errors'].Add('marker: ' + $_.Exception.Message) }

    # 1. the uninstall entry. Tauri NSIS with installMode currentUser writes HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\<product
    #    name> (product name: cys); the two machine hives are read too, so that an entry in an unexpected place is seen
    try {
        $un = [ordered]@{ hkcu_entry_found = $false; display_version = $null; entries = @() }
        $f['uninstall'] = $un
        $list = New-Object System.Collections.Generic.List[object]
        foreach ($root in @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall')) {
            if (-not (Test-Path -LiteralPath $root)) { continue }
            foreach ($k in @(Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue)) {
                $p = $null
                try { $p = Get-ItemProperty -LiteralPath $k.PSPath -ErrorAction Stop } catch { continue }
                $kn = [string]$k.PSChildName
                $dn = [string]$p.DisplayName
                $us = [string]$p.UninstallString
                $il = [string]$p.InstallLocation
                $hit = [bool](($kn -ieq 'cys') -or ($dn -ieq 'cys') -or ($us -like '*\Local\cys\*') -or ($il -like '*\Local\cys*'))
                if (-not $hit) { continue }
                $hive = $root.Substring(0, 4)
                $list.Add([ordered]@{ hive = $hive; root = $root; key = $kn; DisplayName = $dn; DisplayVersion = [string]$p.DisplayVersion; UninstallString = $us; InstallLocation = $il; Publisher = [string]$p.Publisher; DisplayIcon = [string]$p.DisplayIcon; EstimatedSize = $p.EstimatedSize })
                if (($hive -eq 'HKCU') -and (-not $un['hkcu_entry_found'])) {
                    $un['hkcu_entry_found'] = $true
                    $un['display_version'] = [string]$p.DisplayVersion
                }
            }
        }
        $un['entries'] = $list.ToArray()
    } catch { $f['errors'].Add('uninstall: ' + $_.Exception.Message) }

    # 2. shortcuts: start menu (this user + all users) and desktop. The target is read through WScript.Shell (loading a .lnk changes nothing)
    try {
        $sc = [ordered]@{ start_menu_present = $false; desktop_present = $false; start_menu = @(); desktop = @() }
        $f['shortcuts'] = $sc
        $wsh = $null
        try { $wsh = New-Object -ComObject WScript.Shell -ErrorAction Stop } catch { }
        $found = @{ start_menu = (New-Object System.Collections.Generic.List[object]); desktop = (New-Object System.Collections.Generic.List[object]) }
        foreach ($pl in @(Get-UpgShortcutPlaces)) {
            if ($null -eq $pl) { continue }
            $kind = [string]$pl['kind']
            $root = [string]$pl['root']
            if ((-not $root) -or (-not (Test-Path -LiteralPath $root))) { continue }
            $links = @()
            if ($pl['recurse']) { $links = @(Get-ChildItem -LiteralPath $root -Recurse -File -Filter '*.lnk' -ErrorAction SilentlyContinue) }
            else { $links = @(Get-ChildItem -LiteralPath $root -File -Filter '*.lnk' -ErrorAction SilentlyContinue) }
            foreach ($l in $links) {
                if ([string]$l.BaseName -notlike 'cys*') { continue }
                $target = ''
                if ($null -ne $wsh) { try { $target = [string]$wsh.CreateShortcut($l.FullName).TargetPath } catch { } }
                $found[$kind].Add([ordered]@{ path = [string]$l.FullName; target = $target })
            }
        }
        $sc['start_menu'] = $found['start_menu'].ToArray()
        $sc['desktop'] = $found['desktop'].ToArray()
        $sc['start_menu_present'] = [bool]($found['start_menu'].Count -gt 0)
        $sc['desktop_present'] = [bool]($found['desktop'].Count -gt 0)
    } catch { $f['errors'].Add('shortcuts: ' + $_.Exception.Message) }

    # 3. the install folder: files and bytes (it is also the state folder of the base daemon, so the numbers move while the app runs)
    try {
        $fi = [ordered]@{ count = $null; bytes = $null; ms = $null; via = $null; error = $null; top_level = @() }
        $f['files'] = $fi
        if ($f['install_dir_exists']) {
            $sf = [System.Diagnostics.Stopwatch]::StartNew()
            $cnt = 0
            $bytes = [int64]0
            try {
                $di = New-Object System.IO.DirectoryInfo($dir)
                foreach ($x in $di.EnumerateFiles('*', [System.IO.SearchOption]::AllDirectories)) {
                    $cnt++
                    $bytes = $bytes + [int64]$x.Length
                }
                $fi['via'] = 'DirectoryInfo.EnumerateFiles'
            } catch {
                # a file went away or a folder was locked in the middle: count again the slow, forgiving way
                $fi['error'] = $_.Exception.Message
                $cnt = 0
                $bytes = [int64]0
                foreach ($y in @(Get-ChildItem -LiteralPath $dir -Recurse -File -Force -ErrorAction SilentlyContinue)) {
                    $cnt++
                    $bytes = $bytes + [int64]$y.Length
                }
                $fi['via'] = 'Get-ChildItem'
            }
            $fi['count'] = $cnt
            $fi['bytes'] = $bytes
            $fi['ms'] = [int]$sf.ElapsedMilliseconds
            $tl = New-Object System.Collections.Generic.List[string]
            foreach ($t in @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue | Sort-Object Name)) {
                if ($tl.Count -lt 80) { $tl.Add(('{0}{1}' -f [string]$t.Name, $(if ($t.PSIsContainer) { '\' } else { '' }))) }
            }
            $fi['top_level'] = $tl.ToArray()
        }
    } catch { $f['errors'].Add('files: ' + $_.Exception.Message) }

    # 4. the three executables (size, version resource, sha256)
    try {
        $core = [ordered]@{}
        foreach ($n in @('cys-app.exe', 'cys.exe', 'cysd.exe')) { $core[$n] = Get-UpgExeRecord (Join-Path $dir $n) $true }
        $f['core'] = $core
    } catch { $f['errors'].Add('core: ' + $_.Exception.Message) }

    # 5. "--version" of the CLI and of the daemon binary (15 s cap each). Neither call starts or contacts a daemon - product code:
    #    cys.exe   src/bin/cys.rs: the Cli struct has #[command(name = "cys", version, ...)] and main() calls Cli::parse() BEFORE
    #              run(cli.command); clap prints "cys <version>" and exits inside parse(), so run() - where the daemon autostart
    #              lives - is never reached (the same text at tag v0.14.42 and in 0.14.43).
    #    cysd.exe  src/bin/cysd/main.rs: main() first matches parse_cysd_args(args): "--version" | "-V" => println!(cysd_version_line())
    #              and exit(0); only NO argument starts the daemon. That parser exists at tags v0.14.41 and v0.14.42 and in 0.14.43;
    #              v0.14.37 has no argument parsing at all (any argument started the daemon there), so cysd.exe is asked only when
    #              its file version is 0.14.41 or newer.
    try {
        $cli = [ordered]@{}
        $f['cli'] = $cli
        foreach ($n in @('cys.exe', 'cysd.exe')) {
            $p = Join-Path $dir $n
            $c = [ordered]@{ path = $p; ran = $false; skipped = $null; rc = $null; timed_out = $null; start_error = $null; ms = $null; out = $null; err = $null }
            $cli[$n] = $c
            if (-not (Test-Path -LiteralPath $p)) { $c['skipped'] = 'the file is missing'; continue }
            if ($n -eq 'cysd.exe') {
                $newEnough = $false
                try { $newEnough = [bool]([version](Get-UpgFileVersion3 $p) -ge [version]'0.14.41') } catch { }
                if (-not $newEnough) { $c['skipped'] = 'cysd.exe is older than 0.14.41 (or its version is unreadable): it has no argument parser, any argument would start the daemon'; continue }
            }
            $x = Invoke-Proc -File $p -Arguments '--version' -TimeoutSec 15
            $c['ran'] = $true
            $c['rc'] = $x['rc']
            $c['timed_out'] = $x['timedOut']
            $c['start_error'] = $x['startError']
            $c['ms'] = $x['ms']
            $c['out'] = Limit-Text (([string]$x['out']).Trim()) 400
            $c['err'] = Limit-Text (([string]$x['err']).Trim()) 400
        }
    } catch { $f['errors'].Add('cli: ' + $_.Exception.Message) }

    # 6. the bundled runtime: presence and size; "--version" (exit code + first line) of ONE bash and ONE python - the first that exists
    try {
        $bl = New-Object System.Collections.Generic.List[object]
        $ran = @{}
        foreach ($rel in @('runtime\git\bin\bash.exe', 'runtime\git\usr\bin\bash.exe', 'runtime\git\cmd\git.exe', 'runtime\python\python3.exe', 'runtime\python\python.exe', 'runtime\node\node.exe')) {
            $p = Join-Path $dir $rel
            $b = [ordered]@{ rel = $rel; exists = [bool](Test-Path -LiteralPath $p); size = $null; version_rc = $null; version_out = $null; version_timed_out = $null; version_ms = $null }
            if ($b['exists']) { try { $b['size'] = (Get-Item -LiteralPath $p).Length } catch { } }
            $kind = ''
            if ($rel -like '*\bash.exe') { $kind = 'bash' }
            if ($rel -like '*\python*.exe') { $kind = 'python' }
            if ($b['exists'] -and $kind -and (-not $ran.ContainsKey($kind))) {
                $ran[$kind] = $true
                $x = Invoke-Proc -File $p -Arguments '--version' -TimeoutSec 15
                $b['version_rc'] = $x['rc']
                $b['version_timed_out'] = $x['timedOut']
                $b['version_ms'] = $x['ms']
                $first = ((([string]$x['out']) + ([string]$x['err'])).Trim() -split "`r?`n")[0]
                $b['version_out'] = Limit-Text ([string]$first) 200
            }
            $bl.Add($b)
        }
        $f['bundled'] = $bl.ToArray()
    } catch { $f['errors'].Add('bundled: ' + $_.Exception.Message) }

    $sw.Stop()
    $f['ms'] = [int]$sw.ElapsedMilliseconds
    return $f
}

# =========================================================================
# scene UPGRADE: parameters, the older (public) installer
# =========================================================================
# the upgrade parameters the gate job read from diag/appe2e-input.json (workflow env); the gate already checked their form
function Get-UpgParams {
    $p = [ordered]@{
        scene = ([string]$env:APPE2E_SCENE).Trim()
        from = ([string]$env:APPE2E_UPGRADE_FROM).Trim()
        mode = ([string]$env:APPE2E_UPGRADE_MODE).Trim().ToLowerInvariant()
        expect = ([string]$env:APPE2E_UPGRADE_EXPECT).Trim()
        problems = (New-Object System.Collections.Generic.List[string])
    }
    if (-not $p['from']) { $p['from'] = '0.14.42' }
    if (-not $p['mode']) { $p['mode'] = 'emulate' }
    if ([string]$p['from'] -notmatch '^\d+\.\d+\.\d+$') { $p['problems'].Add('APPE2E_UPGRADE_FROM is not a version: ' + [string]$p['from']) }
    if (@('emulate', 'updater') -notcontains [string]$p['mode']) { $p['problems'].Add('APPE2E_UPGRADE_MODE is neither emulate nor updater: ' + [string]$p['mode']) }
    if ($p['expect'] -and ([string]$p['expect'] -notmatch '^\d+\.\d+\.\d+$')) { $p['problems'].Add('APPE2E_UPGRADE_EXPECT is not a version: ' + [string]$p['expect']) }
    if (([string]$p['mode'] -eq 'updater') -and (-not $p['expect'])) { $p['problems'].Add('updater mode needs APPE2E_UPGRADE_EXPECT (the version the public endpoint must offer)') }
    return $p
}

# the PUBLIC installer of the older version: the file p1-assets.ps1 downloaded and compared with SHA256SUMS.txt (WORK\dl\<ver>\), else
# the public release asset, at most 3 tries (size and sha256 are recorded; compared with the SHA256SUMS value when p1-assets has one)
function Get-UpgBaseInstaller {
    param([string]$Version)
    $b = [ordered]@{ version = $Version; source = $null; path = $null; exists = $false; usable = $false; size = $null; sha256 = $null; signature = $null; p1 = $null; url = $null; attempts = (New-Object System.Collections.Generic.List[object]); sha256_expected = $null; sha256_match = $null; error = $null }
    try {
        $path = Get-AssetInstaller $Version
        $b['path'] = $path
        try {
            $j = Read-UpgJson 'p1-assets.json'
            if ($null -ne $j) {
                $e = $j.versions.PSObject.Properties[$Version]
                if ($null -ne $e) {
                    $v = $e.Value
                    $b['p1'] = [ordered]@{ usable = $v.usable; size = $v.size; sha256 = $v.sha256; sums_expected = $v.sums_expected; sums_match = $v.sums_match; sums_note = $v.sums_note }
                    if ($v.sums_expected) { $b['sha256_expected'] = [string]$v.sums_expected }
                }
            }
        } catch { }
        if ((Test-Path -LiteralPath $path) -and (Test-AssetUsable $Version)) {
            $b['source'] = 'p1-assets (WORK\dl, compared with SHA256SUMS.txt there)'
        } else {
            $b['source'] = 'download (public release asset)'
            $b['url'] = ('https://github.com/{0}/releases/download/v{1}/cys_{1}_x64-setup.exe' -f $K_UPG_REPO, $Version)
            for ($i = 1; $i -le 3; $i++) {
                $d = Invoke-Download ([string]$b['url']) $path 900
                $b['attempts'].Add([ordered]@{ n = $i; ok = $d['ok']; rc = $d['rc']; size = $d['size']; ms = $d['ms']; err = (Limit-Text $d['err'] 200) })
                if ($d['ok']) { break }
                if ($i -lt 3) { Start-Sleep -Seconds 10 }
            }
        }
        if (Test-Path -LiteralPath $path) {
            $b['exists'] = $true
            $b['size'] = (Get-Item -LiteralPath $path).Length
            $b['sha256'] = Get-Sha256 $path
            $b['signature'] = (Get-SigRecord $path).status
            $b['usable'] = [bool]([int64]$b['size'] -gt 1048576)
            if ($b['sha256_expected']) {
                $b['sha256_match'] = [bool]([string]$b['sha256'] -eq [string]$b['sha256_expected'])
                if (-not $b['sha256_match']) {
                    $b['usable'] = $false
                    $b['error'] = 'the sha256 of the file is not the one of SHA256SUMS.txt'
                }
            }
            if ((-not $b['usable']) -and (-not $b['error'])) { $b['error'] = 'the file is too small to be an installer' }
        } else {
            $b['error'] = 'the installer file is not there (p1-assets did not leave it and the download failed)'
        }
    } catch {
        $b['error'] = 'exception: ' + $_.Exception.Message
    }
    return $b
}

# =========================================================================
# scene UPGRADE: what runs, what is on disk
# =========================================================================
# one process table read: the app, the daemons, the CLI, installers of the update flow and everything that runs from the install folder
function Get-UpgProcs {
    $res = New-Object System.Collections.Generic.List[object]
    $ok = $true
    try {
        $inst = (Get-InstallDir).TrimEnd('\') + '\'
        foreach ($p in @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 30 -ErrorAction Stop)) {
            $n = [string]$p.Name
            $path = [string]$p.ExecutablePath
            $hit = $false
            if (@('cys-app.exe', 'cysd.exe', 'cys.exe') -contains $n.ToLowerInvariant()) { $hit = $true }
            elseif (Test-AppInstallerProc $n $path) { $hit = $true }
            elseif ($path -and $path.StartsWith($inst, [System.StringComparison]::OrdinalIgnoreCase)) { $hit = $true }
            if (-not $hit) { continue }
            $created = $null
            try {
                if ($null -ne $p.CreationDate) { $created = ConvertTo-IsoUtc $p.CreationDate }
            } catch { }
            $res.Add([pscustomobject]@{ name = $n; id = [int]$p.ProcessId; ppid = [int]$p.ParentProcessId; path = $path; created = $created; cmd = (Format-TeamCmd ([string]$p.CommandLine) 300) })
        }
    } catch {
        $ok = $false
    }
    return [pscustomobject]@{ ok = $ok; rows = $res.ToArray() }
}

# the daemon processes among those rows: pid, parent, creation time, image path and the file version of that image. After the
# installer's rename swap an OLD daemon keeps running from cysd.prev*.exe (src-tauri/nsis-hooks.nsh rules L2 / L3: the locked cysd.exe
# is renamed to a prev slot, the new one takes its name); a daemon started afterwards runs from the new cysd.exe.
function Get-UpgDaemonRows {
    param($Rows)
    $out = New-Object System.Collections.Generic.List[object]
    foreach ($r in @($Rows)) {
        if ($null -eq $r) { continue }
        $leaf = ''
        try {
            if ($r.path) { $leaf = [System.IO.Path]::GetFileName([string]$r.path) }
        } catch { }
        $isDaemon = [bool](([string]$r.name -match '^cysd(\.prev\d*)?\.exe$') -or ($leaf -match '^cysd(\.prev\d*)?\.exe$'))
        if (-not $isDaemon) { continue }
        $out.Add([ordered]@{ id = [int]$r.id; ppid = [int]$r.ppid; created = $r.created; name = [string]$r.name; path = [string]$r.path; image = $leaf; image_version = (Get-UpgFileVersion3 ([string]$r.path)) })
    }
    return $out.ToArray()
}

function Format-UpgDaemons {
    param($Daemons)
    $parts = New-Object System.Collections.Generic.List[string]
    foreach ($d in @($Daemons)) {
        if ($null -eq $d) { continue }
        $parts.Add(('pid {0} (parent {1}, created {2}, image {3} v{4})' -f $d['id'], $d['ppid'], $d['created'], $d['image'], $d['image_version']))
    }
    return ($parts.ToArray() -join '; ')
}

# the pack folder of the base daemon: %USERPROFILE%\.cys\pack (src/pack.rs pack_dir(); PACK_VERSION_FILE ".pack-version" is written
# last = the commit marker of a pack, PACK_STATE_FILE ".pack-state.json" is its channel / base record)
function Get-UpgPackState {
    $dir = Join-Path $env:USERPROFILE '.cys\pack'
    $o = [ordered]@{ time = (Get-IsoNow); dir = $dir; exists = $false; files = 0; pack_version = $null; pack_state = $null; error = $null }
    try {
        if (Test-Path -LiteralPath $dir) {
            $o['exists'] = $true
            try { $o['files'] = [System.IO.Directory]::GetFiles($dir, '*', [System.IO.SearchOption]::AllDirectories).Length } catch { $o['error'] = 'count: ' + $_.Exception.Message }
            $pv = Join-Path $dir '.pack-version'
            if (Test-Path -LiteralPath $pv) { $o['pack_version'] = (Limit-Text (([string](Read-FileShared $pv)).Trim()) 100) }
            $ps = Join-Path $dir '.pack-state.json'
            if (Test-Path -LiteralPath $ps) { $o['pack_state'] = (Limit-Text (([string](Read-FileShared $ps)).Trim()) 1500) }
        }
    } catch { $o['error'] = $_.Exception.Message }
    return $o
}

# what the installer's lock-tolerant placement can leave in the install folder: <name>.prev* (an image that was renamed away while it was
# in use - the NEW daemon removes those at its start, nsis-hooks.nsh rule L6) and <bin>.new.exe (a staged new image)
function Get-UpgLeftovers {
    param([string]$Tag)
    $o = [ordered]@{ tag = $Tag; time = (Get-IsoNow); prev_count = 0; new_exe_count = 0; names = @(); error = $null }
    try {
        $dir = Get-InstallDir
        $names = New-Object System.Collections.Generic.List[string]
        $prevCnt = 0
        $newCnt = 0
        foreach ($file in [System.IO.Directory]::GetFiles($dir, '*', [System.IO.SearchOption]::AllDirectories)) {
            $leaf = [System.IO.Path]::GetFileName($file)
            $isPrev = [bool]($leaf -like '*.prev*')
            $isNew = [bool]($leaf -like '*.new.exe')
            if ($isPrev) { $prevCnt++ }
            if ($isNew) { $newCnt++ }
            if (($isPrev -or $isNew) -and ($names.Count -lt 40)) { $names.Add(([string]$file).Substring($dir.Length).TrimStart('\')) }
        }
        $o['prev_count'] = $prevCnt
        $o['new_exe_count'] = $newCnt
        $o['names'] = $names.ToArray()
    } catch { $o['error'] = $_.Exception.Message }
    return $o
}

# names (only names) at the top of %USERPROFILE%\.cys
function Get-UpgCysHomeNames {
    $o = New-Object System.Collections.Generic.List[string]
    try {
        $h = Join-Path $env:USERPROFILE '.cys'
        if (Test-Path -LiteralPath $h) {
            foreach ($t in @(Get-ChildItem -LiteralPath $h -Force -ErrorAction SilentlyContinue | Sort-Object Name)) {
                if ($o.Count -lt 120) { $o.Add(('{0}{1}' -f [string]$t.Name, $(if ($t.PSIsContainer) { '\' } else { '' }))) }
            }
        }
    } catch { }
    return $o.ToArray()
}

# =========================================================================
# scene UPGRADE: the node driver (diag/cdp-update.mjs) in its read-only modes
# =========================================================================
# synchronous: --mode version | attach | uiobs. Returns rc; the result files are <prefix>-cdp.json / -cdp-after.json / -obs-facts.json
function Invoke-UpgNode {
    param([string]$Prefix, [string]$NodeExe, [string]$Mode, [string]$Extra = '', [int]$TimeoutSec = 240)
    $r = [ordered]@{ prefix = $Prefix; mode = $Mode; rc = $null; timed_out = $null; error = $null }
    try {
        $a = '"{0}" --mode {1} --port {2} --out "{3}" --prefix {4} --max-wait-sec 120 --ready-wait-sec 30' -f (Get-NodeScript), $Mode, $K_PORT, $global:DiagOut, $Prefix
        if ($Extra) { $a = $a + ' ' + $Extra }
        $x = Invoke-Proc -File $NodeExe -Arguments $a -TimeoutSec $TimeoutSec
        $r['rc'] = $x['rc']
        $r['timed_out'] = $x['timedOut']
        Save-Text ($Prefix + '-cdp-stdout.txt') ((([string]$x['out']) + "`r`n" + ([string]$x['err'])))
    } catch {
        $r['error'] = 'exception: ' + $_.Exception.Message
    }
    return $r
}

# background: the PowerShell side samples the machine meanwhile (same form as Start-NodeTeam of app-e2e.ps1)
function Start-UpgNode {
    param([string]$Prefix, [string]$NodeExe, [string]$Mode, [string]$Extra = '')
    $a = '"{0}" --mode {1} --port {2} --out "{3}" --prefix {4}' -f (Get-NodeScript), $Mode, $K_PORT, $global:DiagOut, $Prefix
    if ($Extra) { $a = $a + ' ' + $Extra }
    $o = Join-Path $global:DiagOut ($Prefix + '-cdp-stdout.txt')
    $e = Join-Path $global:DiagOut ($Prefix + '-cdp-stderr.txt')
    $p = Start-Process -FilePath $NodeExe -ArgumentList $a -NoNewWindow -PassThru -RedirectStandardOutput $o -RedirectStandardError $e -ErrorAction Stop
    try { $null = $p.Handle } catch { }
    return $p
}

# one poll of the page in the observe-only mode (--obs-sec 0): status bar text, badge, toasts, the product's read-only daemon_status
function Get-UpgPageOnce {
    param([string]$Prefix, [string]$NodeExe)
    $o = [ordered]@{ prefix = $Prefix; rc = $null; attached = $null; app_version = $null; daemon_info = $null; daemon_version = $null; daemon_pid = $null; daemon_surface_count = $null; badge_seen = $null; badge_text = $null; toasts = @(); error = $null }
    try {
        if (-not $NodeExe) { $o['error'] = 'node.exe was not found'; return $o }
        $x = Invoke-UpgNode -Prefix $Prefix -NodeExe $NodeExe -Mode 'uiobs' -Extra '--obs-sec 0' -TimeoutSec 200
        $o['rc'] = $x['rc']
        if ($x['error']) { $o['error'] = $x['error'] }
        $j = Read-UpgJson ($Prefix + '-obs-facts.json')
        if ($null -eq $j) {
            if (-not $o['error']) { $o['error'] = ($Prefix + '-obs-facts.json missing or unreadable') }
            return $o
        }
        $o['attached'] = [bool]$j.attached
        $o['app_version'] = $j.app_version
        $f = $j.facts
        if ($null -ne $f) {
            $o['daemon_info'] = $f.daemon_info_last
            $o['daemon_version'] = $f.daemon_version_last
            $o['daemon_pid'] = $f.daemon_pid_last
            $o['daemon_surface_count'] = $f.daemon_surface_count_last
            $o['badge_seen'] = [bool]$f.skew_badge_seen
            $o['badge_text'] = $f.skew_badge_text
            $ts = New-Object System.Collections.Generic.List[string]
            foreach ($t in @($f.toasts_seen)) {
                if ($null -eq $t) { continue }
                $ts.Add(('[{0}] {1} :: {2}' -f [string]$t.cls, [string]$t.name, (Limit-Text ([string]$t.detail) 300)))
            }
            $o['toasts'] = $ts.ToArray()
        }
    } catch {
        $o['error'] = 'exception: ' + $_.Exception.Message
    }
    return $o
}

# =========================================================================
# scene UPGRADE step 5: the state of the older app before the upgrade (recorded; being "not ready" is a NOTE, never a failure)
# =========================================================================
function Get-UpgBaseState {
    param([string]$NodeExe)
    $pfx = $K_UPG_PFX + '-base-state'
    $s = [ordered]@{
        started = (Get-IsoNow); ticks = 0; waited_sec = $null; ready = $false; end_reason = $null
        daemon_first_seen_sec = $null; pack_first_ready_sec = $null
        daemons = @(); procs_under_install_dir = @(); pipes = $null; pack = $null; cys_home = @(); screenshot = $null; page = $null
    }
    $tl = Join-Path $global:DiagOut ($pfx + '-timeline.txt')
    try { Add-Utf8NoBom $tl (('# base state: waiting (every {0} s, at most {1} s) for a cysd.exe of the install folder AND a committed pack (.pack-version); start {2}' -f $K_UPG_BASE_TICK_SEC, $K_UPG_BASE_READY_SEC, (Get-IsoNow)) + "`r`n") } catch { }
    $t0 = Get-Date
    try {
        while ($true) {
            $el = [int]((Get-Date) - $t0).TotalSeconds
            $tab = Get-UpgProcs
            $dm = @(Get-UpgDaemonRows $tab.rows)
            $pk = Get-UpgPackState
            $pipes = Get-TeamPipes
            $haveDaemon = [bool]($dm.Count -gt 0)
            $havePack = [bool]($pk['pack_version'])
            if ($haveDaemon -and ($null -eq $s['daemon_first_seen_sec'])) { $s['daemon_first_seen_sec'] = $el }
            if ($havePack -and ($null -eq $s['pack_first_ready_sec'])) { $s['pack_first_ready_sec'] = $el }
            $line = '[{0,4}s {1}] cysd=[{2}] pipes=[{3}] pack: files={4} version={5}' -f $el, (Get-IsoNow), (Format-UpgDaemons $dm), (@($pipes['cys']) -join ' '), $pk['files'], $pk['pack_version']
            try { Add-Utf8NoBom $tl ($line + "`r`n") } catch { }
            $s['ticks'] = [int]$s['ticks'] + 1
            $s['daemons'] = $dm
            $s['pipes'] = $pipes
            $s['pack'] = $pk
            $s['waited_sec'] = $el
            if ($haveDaemon -and $havePack) { $s['ready'] = $true; $s['end_reason'] = 'a daemon runs and the pack is committed'; break }
            if ($el -ge $K_UPG_BASE_READY_SEC) { $s['end_reason'] = ('not ready within {0} s (daemon running={1}, pack committed={2})' -f $K_UPG_BASE_READY_SEC, $haveDaemon, $havePack); break }
            if ((Get-MinutesLeft) -lt 12) { $s['end_reason'] = 'job time budget: the wait was cut short'; break }
            Start-Sleep -Seconds $K_UPG_BASE_TICK_SEC
        }
    } catch {
        $s['end_reason'] = 'wait loop exception: ' + $_.Exception.Message
        Add-DiagError 'Get-UpgBaseState' $_
    }
    try { Add-Utf8NoBom $tl (('# end: {0}' -f $s['end_reason']) + "`r`n") } catch { }
    # UPG-3: "ready" is not yet "settled". A PC on which the older version has been in use is past that app's own first-run work:
    # the stamp (.last-app-version) and the onboarding marker (.gui-onboarded) carry its version, no pack staging folder is left and
    # no "cys.exe init-pack" / "cys.exe restore" is running. Wait for that - up to 90 s more, every 2 s. Not settled = a NOTE.
    $s['settled'] = $false
    $s['settled_after_sec'] = $null
    $s['settle_ticks'] = 0
    $s['settle_missing'] = @()
    $s['settle_stamps'] = $null
    $s['settle_proc_check_ok'] = $null
    $baseVer = [string]$CTX['upg_base_version']
    if ($s['ready']) {
        $ts = Get-Date
        try {
            while ($true) {
                $el2 = [int]((Get-Date) - $ts).TotalSeconds
                $stp = Get-UpgStamps 'settle'
                $busy = New-Object System.Collections.Generic.List[string]
                $procOk = $true
                try {
                    foreach ($cp in @(Get-CimInstance -ClassName Win32_Process -OperationTimeoutSec 15 -Filter "Name = 'cys.exe'" -ErrorAction Stop)) {
                        if ($null -eq $cp) { continue }
                        $ca = Get-UpgCmdArgs ([string]$cp.CommandLine)
                        if ($ca -match '(^|\s)(init-pack|restore)(\s|$)') { $busy.Add(('cys.exe#{0} {1}' -f $cp.ProcessId, (Limit-Text $ca 60))) }
                    }
                } catch { $procOk = $false }
                $miss = New-Object System.Collections.Generic.List[string]
                if ([string]$stp['last_app_version'] -ne $baseVer) { $miss.Add(('.last-app-version is "{0}"' -f (Format-UpgVal $stp['last_app_version'] 'absent'))) }
                if ([string]$stp['gui_onboarded'] -ne $baseVer) { $miss.Add(('.gui-onboarded is "{0}"' -f (Format-UpgVal $stp['gui_onboarded'] 'absent'))) }
                if (@($stp['staging']).Count -gt 0) { $miss.Add('pack staging left: ' + (@($stp['staging']) -join ',')) }
                if ($busy.Count -gt 0) { $miss.Add('running: ' + ($busy.ToArray() -join ', ')) }
                $s['settle_ticks'] = [int]$s['settle_ticks'] + 1
                $s['settle_stamps'] = $stp
                $s['settle_proc_check_ok'] = $procOk
                try { Add-Utf8NoBom $tl (('[settle +{0,3}s {1}] {2} busy=[{3}] missing={4}' -f $el2, (Get-IsoNow), (Format-UpgStamps $stp), ($busy.ToArray() -join ', '), $miss.Count) + "`r`n") } catch { }
                if ($miss.Count -eq 0) { $s['settled'] = $true; $s['settled_after_sec'] = $el2; $s['settle_missing'] = @(); break }
                if ($el2 -ge $K_UPG_SETTLE_SEC) { $s['settle_missing'] = $miss.ToArray(); break }
                if ((Get-MinutesLeft) -lt 12) { $miss.Add('the wait was cut short (job time)'); $s['settle_missing'] = $miss.ToArray(); break }
                $s['settle_missing'] = $miss.ToArray()
                Start-Sleep -Seconds $K_UPG_SETTLE_TICK_SEC
            }
        } catch {
            $s['settle_missing'] = @('settle loop exception: ' + $_.Exception.Message)
            Add-DiagError 'Get-UpgBaseState settle' $_
        }
        # the state the upgrade starts from is the one after this wait
        try {
            $tab2 = Get-UpgProcs
            $s['daemons'] = @(Get-UpgDaemonRows $tab2.rows)
            $s['pack'] = Get-UpgPackState
        } catch { }
    } else {
        $s['settle_missing'] = @('the readiness wait itself did not complete')
    }
    try { Add-Utf8NoBom $tl (('# settled={0} after +{1} s; missing: {2}' -f $s['settled'], (Format-UpgVal $s['settled_after_sec'] '-'), (@($s['settle_missing']) -join '; ')) + "`r`n") } catch { }
    # the records: processes of the install folder, pipes, pack, the names in the .cys folder, the desktop, the page
    try {
        $pl = New-Object System.Collections.Generic.List[object]
        foreach ($p in @(Get-ProcessesUnder -Dir (Get-InstallDir))) {
            if ($null -eq $p) { continue }
            $pl.Add([ordered]@{ name = [string]$p.Name; id = [int]$p.Id; ppid = [int]$p.ParentId; path = [string]$p.Path; cmd = (Format-TeamCmd ([string]$p.Cmd) 300) })
        }
        $s['procs_under_install_dir'] = $pl.ToArray()
    } catch { }
    try { $s['cys_home'] = @(Get-UpgCysHomeNames) } catch { }
    try { Save-Text ($pfx + '-processes.txt') (Get-ProcessListText) } catch { }
    try { $s['screenshot'] = Save-Screenshot ($pfx + '-screen.png') } catch { }
    try { $s['page'] = Get-UpgPageOnce -Prefix ($K_UPG_PFX + '-base-obs') -NodeExe $NodeExe } catch { }
    $s['finished'] = (Get-IsoNow)
    return $s
}

# =========================================================================
# scene UPGRADE step 6, mode emulate: start the installer under test the way the old app's updater plugin does, then the app is gone
# =========================================================================
function Start-UpgEmulate {
    param([string]$Installer, [string]$ToVersion, [int]$AppPid, [string]$ExpectSha256 = '')
    $e = [ordered]@{
        mode = 'emulate'; source_installer = $Installer; temp_dir = $null; temp_installer = $null; copy_ok = $false; copy_sha256 = $null; copy_sha256_same = $null
        args = $K_UPG_INSTALLER_ARGS; launch_api = $null; launch_cwd = $null; launch = $null; launch_ok = $false; launched_at = $null
        app_pid = $AppPid; kill_delay_ms = 300; app_alive_at_kill = $null; app_killed = $null; app_kill_error = $null; killed_at = $null; error = $null
    }
    try {
        if ((-not $Installer) -or (-not (Test-Path -LiteralPath $Installer))) { $e['error'] = 'the installer under test is missing: ' + $Installer; return $e }
        # the plugin's shape: %TEMP%\<app>-<version>-updater-<random>\<app>-<version>-installer.exe (Test-AppInstallerProc knows that path)
        $rand = [guid]::NewGuid().ToString('N').Substring(0, 6)
        $dir = Join-Path ([System.IO.Path]::GetTempPath()) ('cys-{0}-updater-diag{1}' -f $ToVersion, $rand)
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        $dest = Join-Path $dir ('cys-{0}-installer.exe' -f $ToVersion)
        Copy-Item -LiteralPath $Installer -Destination $dest -Force -ErrorAction Stop
        $e['temp_dir'] = $dir
        $e['temp_installer'] = $dest
        $e['copy_ok'] = [bool](Test-Path -LiteralPath $dest)
        $e['copy_sha256'] = Get-Sha256 $dest
        if ($ExpectSha256) { $e['copy_sha256_same'] = [bool]([string]$e['copy_sha256'] -eq $ExpectSha256) }
        if ((-not $e['copy_ok']) -or ($e['copy_sha256_same'] -eq $false)) { $e['error'] = 'the copy of the installer in the temp folder is not the installer under test'; return $e }

        # ShellExecuteW(NULL, "open", <temp installer>, "/P /R /UPDATE /ARGS", NULL, SW_SHOW): tauri-plugin-updater 2.10.1, updater.rs
        # install_inner() - install_mode.nsis_args() is ["/P", "/R"] for the default mode "passive" (the product's tauri.conf.json sets none),
        # then "/UPDATE" "/ARGS" and the app's own arguments (none here) - followed at once by std::process::exit(0).
        # Two things the installer inherits from the old app when the plugin is the caller are given here the same way: the debugging flag
        # in the environment, and the current directory (lpDirectory is NULL in the plugin's call, so the installer starts in the app's
        # current directory - the install folder: Start-CysApp starts the app there, as the Start menu shortcut does).
        $instDir = Get-InstallDir
        $oldCwd = [System.Environment]::CurrentDirectory
        $e['launch_cwd'] = $instDir
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = ('--remote-debugging-port={0} --remote-allow-origins=*' -f $K_PORT)
        try {
            if ($global:DiagNativeReady) {
                $e['launch_api'] = 'ShellExecuteW(open, SW_SHOW) [DiagNative.ShellExecStart]'
                try { [System.Environment]::CurrentDirectory = $instDir } catch { $e['launch_cwd'] = $oldCwd }
                $call = [DiagNative]::ShellExecStart($dest, [string]$e['args'], 20000)
                $e['launch'] = ConvertTo-CallRecord $call
                $e['launch_ok'] = [bool]$call.Ok
                if (-not $e['launch_ok']) { $e['error'] = ('ShellExecuteW did not start the installer: ret={0} last_error={1} ({2}) timed_out={3}' -f $call.Ret, $call.LastError, ([string]$call.LastErrorText).Trim(), $call.TimedOut) }
            } else {
                $e['launch_api'] = 'Start-Process (the native helper is not available: ' + [string]$global:DiagNativeError + ')'
                $sp = Start-Process -FilePath $dest -ArgumentList ([string]$e['args']) -WorkingDirectory $instDir -PassThru -ErrorAction Stop
                $e['launch'] = [ordered]@{ process_id = $sp.Id }
                $e['launch_ok'] = $true
            }
        } finally {
            try { [System.Environment]::CurrentDirectory = $oldCwd } catch { }
            Remove-Item -Path 'Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS' -ErrorAction SilentlyContinue
        }
        $e['launched_at'] = (Get-IsoNow)
        if (-not $e['launch_ok']) { return $e }

        # the plugin's exit(0): THIS ONE process ends - no tree kill. The daemon is a child of the app and must stay alive, as it does
        # after the real updater (the installer hook itself only ever ends cys-app.exe, without /T: nsis-hooks.nsh rule L1).
        Start-Sleep -Milliseconds ([int]$e['kill_delay_ms'])
        $alive = $false
        if ($AppPid -gt 0) {
            try {
                $gp = Get-Process -Id $AppPid -ErrorAction Stop
                $alive = [bool]((-not $gp.HasExited) -and ([string]$gp.ProcessName -ieq 'cys-app'))
            } catch { $alive = $false }
        }
        $e['app_alive_at_kill'] = [bool]$alive
        if ($alive) {
            try {
                Stop-Process -Id $AppPid -Force -ErrorAction Stop
                $e['app_killed'] = $true
            } catch {
                $e['app_killed'] = $false
                $e['app_kill_error'] = $_.Exception.Message
            }
        }
        $e['killed_at'] = (Get-IsoNow)
    } catch {
        $e['error'] = 'exception: ' + $_.Exception.Message
    }
    return $e
}

# =========================================================================
# scene UPGRADE step 6: the watch (both modes) - temp installer, installer process, the old app going away, the version marker,
# the app coming back. Every 2 s -> appe2e-upg-timeline.txt. Success = the marker reached the target AND a new cys-app.exe runs.
# =========================================================================
function Watch-UpgApply {
    param([string]$Mode, [int]$BasePid, [string]$FromVersion, [string]$ToVersion, $NodeProc, [int]$MaxSec, [datetime]$ApplyStart)
    $pfx = $K_UPG_PFX
    $obs = [ordered]@{
        mode = $Mode; started = (Get-IsoNow); ended = $null; end_reason = $null; end_kind = $null; success = $false; ticks = 0
        node_exited_at = $null; node_exit_code = $null
        base_app_pid = $BasePid; base_app_gone_at = $null; base_app_gone_sec = $null
        installer_seen = $false; installer_first_seen_at = $null; installer_first_seen_sec = $null; installer_first_seen = $null; installer_last_seen_sec = $null
        updater_dirs_seen = @(); updater_dir_max_bytes = 0
        marker_initial = $null; marker_last = $null; marker_changed_at = $null; marker_changed_sec = $null; marker_reached_at = $null; marker_reached_sec = $null
        file_version_last = $null; failure_file_seen = $false; failure_file_text = $null
        new_app_seen = $false; new_app_first_seen_at = $null; new_app_first_seen_sec = $null; new_app_pid = $null
        daemons_first = @(); daemons_last = @()
        installer_alive_at_end = $null; windows_at_end = $null
        screenshots = (New-Object System.Collections.Generic.List[object])
    }
    $tl = Join-Path $global:DiagOut ($pfx + '-timeline.txt')
    try { Add-Utf8NoBom $tl (('# upgrade watch start {0} mode={1} marker={2} target={3} old app pid={4}' -f (Get-IsoNow), $Mode, (Get-InstalledMarker), $ToVersion, $BasePid) + "`r`n") } catch { }
    $t0 = Get-Date
    $applyIso = ConvertTo-IsoUtc $ApplyStart
    $lastActivity = $t0
    $nodeExitAt = $null
    $baseGoneAt = $null
    $markerReachedAt = $null
    $failSeenAt = $null
    $installerAlive = $false
    $installerEver = $false
    $shotInst = $false
    $shotGone = $false
    $stuckShots = 0
    $lastStuckShotAt = $t0
    $lastBytes = [int64]-1
    $lastDirSample = $t0.AddSeconds(-60)
    $lastDirCount = [int]-2
    $seenDirs = New-Object System.Collections.Generic.List[string]
    $firstTick = $true
    $reason = $null
    # end_kind: 'success', 'product' (what was seen is how the upgrade went) or 'harness' (the watch itself could not go on: no verdict on the product)
    $kind = 'product'
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
            $tab = Get-UpgProcs
            $rows = @($tab.rows)
            $marker = Get-InstalledMarker
            $fv = Get-AppFileVersion
            $upd = @(Get-UpdaterTempInfo)
            $failTxt = Get-FailureMarker

            $baseAlive = $false
            $newApp = $null
            $installerAlive = $false
            $installerProc = $null
            foreach ($p in $rows) {
                if ($null -eq $p) { continue }
                $isApp = [bool](([string]$p.name -ieq 'cys-app.exe') -and ((-not $p.path) -or ([string]$p.path -like '*\Local\cys\*')))
                if ($isApp) {
                    # a process created after the upgrade began is a NEW app even if Windows gave it the old pid again
                    $bornAfter = [bool]($p.created -and ([string]::CompareOrdinal([string]$p.created, [string]$applyIso) -ge 0))
                    if (([int]$p.id -eq $BasePid) -and (-not $bornAfter)) { $baseAlive = $true }
                    elseif ($null -eq $newApp) { $newApp = $p }
                }
                if (Test-AppInstallerProc ([string]$p.name) ([string]$p.path)) {
                    $installerAlive = $true
                    if ($null -eq $installerProc) { $installerProc = $p }
                }
            }
            # the CIM list can fail or lag: look again with Get-Process before the app is counted as gone / not back
            if ((-not $tab.ok) -or ((-not $baseAlive) -and ($null -eq $newApp))) {
                try {
                    foreach ($g in @(Get-Process -Name 'cys-app' -ErrorAction SilentlyContinue)) {
                        $gpath = ''
                        try { $gpath = [string]$g.Path } catch { }
                        if ($gpath -and ($gpath -notlike '*\Local\cys\*')) { continue }
                        $gBornAfter = $false
                        try { $gBornAfter = [bool]($g.StartTime.ToUniversalTime() -ge $ApplyStart.ToUniversalTime()) } catch { }
                        if (([int]$g.Id -eq $BasePid) -and (-not $gBornAfter)) { $baseAlive = $true }
                        elseif ($null -eq $newApp) { $newApp = [pscustomobject]@{ name = 'cys-app.exe'; id = [int]$g.Id; ppid = 0; path = $gpath; created = $null; cmd = '' } }
                    }
                } catch { }
            }
            $dm = @(Get-UpgDaemonRows $rows)
            if ($firstTick) {
                $firstTick = $false
                $obs['marker_initial'] = $marker
                $obs['daemons_first'] = $dm
            }
            $obs['daemons_last'] = $dm
            $obs['marker_last'] = $marker
            $obs['file_version_last'] = $fv

            $bytes = [int64]0
            foreach ($u in $upd) {
                $bytes = $bytes + [int64]$u.bytes
                if (-not $seenDirs.Contains([string]$u.name)) { $seenDirs.Add([string]$u.name) }
            }
            if ($bytes -gt [int64]$obs['updater_dir_max_bytes']) { $obs['updater_dir_max_bytes'] = $bytes }
            if ($bytes -ne $lastBytes) { $lastActivity = $now; $lastBytes = $bytes }
            # an installer that extracts or removes files changes the file count of the install folder; one that waits on a dialog changes nothing
            if ((($now - $lastDirSample).TotalSeconds) -ge 10) {
                $lastDirSample = $now
                $dc = -1
                try { $dc = [System.IO.Directory]::GetFiles((Get-InstallDir), '*', [System.IO.SearchOption]::AllDirectories).Length } catch { }
                if (($dc -ge 0) -and ($dc -ne $lastDirCount)) { $lastDirCount = $dc; $lastActivity = $now }
            }
            if ($failTxt -and (-not $obs['failure_file_seen'])) {
                $obs['failure_file_seen'] = $true
                $obs['failure_file_text'] = Limit-Text $failTxt 1500
                $failSeenAt = $now
                $lastActivity = $now
            }
            if ((-not $baseAlive) -and ($null -eq $baseGoneAt)) {
                $baseGoneAt = $now
                $obs['base_app_gone_at'] = (Get-IsoNow)
                $obs['base_app_gone_sec'] = $elapsed
                $lastActivity = $now
            }
            if ($installerAlive) {
                $obs['installer_last_seen_sec'] = $elapsed
                if (-not $installerEver) {
                    $installerEver = $true
                    $obs['installer_seen'] = $true
                    $obs['installer_first_seen_at'] = (Get-IsoNow)
                    $obs['installer_first_seen_sec'] = $elapsed
                    $obs['installer_first_seen'] = [ordered]@{ name = $installerProc.name; id = $installerProc.id; ppid = $installerProc.ppid; path = $installerProc.path; cmd = $installerProc.cmd }
                    $lastActivity = $now
                }
            }
            if ($marker -and $FromVersion -and ($marker -ne $FromVersion) -and ($null -eq $obs['marker_changed_at'])) {
                $obs['marker_changed_at'] = (Get-IsoNow)
                $obs['marker_changed_sec'] = $elapsed
                $lastActivity = $now
            }
            if ($ToVersion -and ($marker -eq $ToVersion) -and ($null -eq $markerReachedAt)) {
                $markerReachedAt = $now
                $obs['marker_reached_at'] = (Get-IsoNow)
                $obs['marker_reached_sec'] = $elapsed
                $lastActivity = $now
            }
            if (($null -ne $newApp) -and (-not $obs['new_app_seen'])) {
                $obs['new_app_seen'] = $true
                $obs['new_app_first_seen_at'] = (Get-IsoNow)
                $obs['new_app_first_seen_sec'] = $elapsed
                $obs['new_app_pid'] = [int]$newApp.id
                $lastActivity = $now
            }

            $procText = (($rows | ForEach-Object { '{0}#{1}' -f $_.name, $_.id }) -join ' ')
            $updText = (($upd | ForEach-Object { '{0}:{1}B[{2}]' -f $_.name, $_.bytes, $_.files }) -join ' ')
            $newAppText = '-'
            if ($null -ne $newApp) { $newAppText = [string]$newApp.id }
            $line = '[{0,5}s {1}] marker={2} appver={3} old_app={4} new_app={5} installer={6} failurefile={7} procs=[{8}] updater=[{9}]' -f $elapsed, (Get-IsoNow), $marker, $fv, $baseAlive, $newAppText, $installerAlive, [bool]$failTxt, $procText, $updText
            try { Add-Utf8NoBom $tl ($line + "`r`n") } catch { }
            $obs['ticks'] = [int]$obs['ticks'] + 1

            # desktop screenshots: the installer window, the moment without an app, an installer that seems to wait
            if ($installerAlive -and (-not $shotInst)) {
                $shotInst = $true
                $obs['screenshots'].Add((Save-Screenshot ($pfx + '-screen-installer.png')))
            }
            if (($null -ne $baseGoneAt) -and ($null -eq $newApp) -and (-not $shotGone) -and ((($now - $baseGoneAt).TotalSeconds) -ge 3)) {
                $shotGone = $true
                $obs['screenshots'].Add((Save-Screenshot ($pfx + '-screen-app-gone.png')))
            }
            if ($installerAlive -and ($null -eq $markerReachedAt) -and ($stuckShots -lt 3) -and ((($now - $lastStuckShotAt).TotalSeconds) -ge 60)) {
                $stuckShots++
                $lastStuckShotAt = $now
                $obs['screenshots'].Add((Save-Screenshot ('{0}-screen-installer-wait{1}.png' -f $pfx, $stuckShots)))
            }

            # end conditions
            if (($null -ne $markerReachedAt) -and ($null -ne $newApp)) { $obs['success'] = $true; $kind = 'success'; $reason = 'success: the version marker reached the target and a new cys-app.exe is running'; break }
            if (($null -ne $markerReachedAt) -and ((($now - $markerReachedAt).TotalSeconds) -ge 90)) { $reason = 'the version marker reached the target but cys-app.exe did not come back within 90 s'; break }
            if (($null -ne $failSeenAt) -and (-not $installerAlive) -and ((($now - $failSeenAt).TotalSeconds) -ge 10)) { $reason = 'the installer left cys-install-failure.txt and ended: the install failed'; break }
            if (($null -ne $baseGoneAt) -and (-not $installerAlive) -and ($null -eq $markerReachedAt) -and ((($now - $baseGoneAt).TotalSeconds) -ge 120) -and ((($now - $lastActivity).TotalSeconds) -ge 60)) { $reason = 'no installer process and the marker is not at the target 120 s after the old app went away'; break }
            if ($installerAlive -and ($null -eq $markerReachedAt) -and ((($now - $lastActivity).TotalSeconds) -ge 180)) { $reason = 'an installer process is alive but nothing changed for 180 s (waiting on a dialog?)'; break }
            if (($null -ne $nodeExitAt) -and $baseAlive -and (-not $installerEver) -and ((($now - $nodeExitAt).TotalSeconds) -ge 30)) { $reason = 'the node driver ended and the old app is still running 30 s later: no update started'; break }
            if ($elapsed -ge $MaxSec) { $reason = ('max watch seconds reached ({0})' -f $MaxSec); break }
            if ((Get-MinutesLeft) -lt 4) { $kind = 'harness'; $reason = 'job time budget exhausted'; break }
            Start-Sleep -Seconds 2
        }
    } catch {
        $kind = 'harness'
        $reason = 'watch loop exception: ' + $_.Exception.Message
        Add-DiagError 'Watch-UpgApply' $_
    }
    $obs['end_reason'] = $reason
    $obs['end_kind'] = $kind
    $obs['ended'] = (Get-IsoNow)
    $obs['updater_dirs_seen'] = $seenDirs.ToArray()
    $obs['installer_alive_at_end'] = [bool]$installerAlive
    try { $obs['windows_at_end'] = Get-VisibleWindowsText } catch { }
    $obs['screenshots'].Add((Save-Screenshot ($pfx + '-screen-end.png')))
    try { Add-Utf8NoBom $tl (('# end: {0}' -f $reason) + "`r`n") } catch { }
    Save-Json ($pfx + '-observe.json') $obs 6
    return $obs
}

# =========================================================================
# scene UPGRADE step 7 (c): the NEW app meets the OLD daemon. The page is watched by node (--mode uiobs: status bar, badge, toasts,
# the product's read-only daemon_status); this side samples the daemon processes meanwhile -> appe2e-upg-daemon-timeline.txt
# =========================================================================
function Watch-UpgDaemons {
    param($NodeProc, [int]$MaxSec)
    $w = [ordered]@{ started = (Get-IsoNow); ended = $null; end_reason = $null; ticks = 0; node_exit_code = $null; first = @(); last = @(); samples = (New-Object System.Collections.Generic.List[object]); stamp_changes = (New-Object System.Collections.Generic.List[object]) }
    $lastStampText = ''
    $tl = Join-Path $global:DiagOut ($K_UPG_PFX + '-daemon-timeline.txt')
    try { Add-Utf8NoBom $tl (('# daemon watch start {0} (every {1} s until the page observer ends, at most {2} s)' -f (Get-IsoNow), $K_UPG_OBS_TICK_SEC, $MaxSec) + "`r`n") } catch { }
    $t0 = Get-Date
    $nodeExitAt = $null
    $firstTick = $true
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
                    try { $w['node_exit_code'] = $NodeProc.ExitCode } catch { }
                }
            }
            $tab = Get-UpgProcs
            $rows = @($tab.rows)
            $dm = @(Get-UpgDaemonRows $rows)
            $apps = New-Object System.Collections.Generic.List[string]
            foreach ($p in $rows) {
                if ($null -eq $p) { continue }
                if ([string]$p.name -ieq 'cys-app.exe') { $apps.Add(('cys-app.exe#{0}' -f $p.id)) }
            }
            $pipes = Get-TeamPipes
            # UPG-2: the marker / stamp files at every tick; a change is filed with its time
            $stp = Get-UpgStamps 'daemon-watch'
            $stpText = Format-UpgStamps $stp
            if ($stpText -ne $lastStampText) {
                $lastStampText = $stpText
                $stp['sec'] = $elapsed
                if ($w['stamp_changes'].Count -lt 40) { $w['stamp_changes'].Add($stp) }
            }
            $line = '[{0,4}s {1}] cysd=[{2}] app=[{3}] pipes=[{4}] {5}' -f $elapsed, (Get-IsoNow), (Format-UpgDaemons $dm), ($apps.ToArray() -join ' '), (@($pipes['cys']) -join ' '), $stpText
            try { Add-Utf8NoBom $tl ($line + "`r`n") } catch { }
            if ($firstTick) {
                $firstTick = $false
                $w['first'] = $dm
            }
            $w['last'] = $dm
            if ($w['samples'].Count -lt 60) { $w['samples'].Add([ordered]@{ sec = $elapsed; time = (Get-IsoNow); daemons = $dm; app = $apps.ToArray(); pipes = @($pipes['cys']) }) }
            $w['ticks'] = [int]$w['ticks'] + 1
            if ($null -ne $nodeExitAt) { $reason = 'the page observer ended'; break }
            if ($null -eq $NodeProc) { $reason = 'no page observer: one sample'; break }
            if ($elapsed -ge $MaxSec) { $reason = ('max watch seconds reached ({0})' -f $MaxSec); break }
            if ((Get-TeamMinutesLeft) -lt ($K_TEAM_NEED_MIN + 1)) { $reason = 'job time budget: the watch was cut short to leave room for the TEAM scene'; break }
            # sleep one tick, but notice the end of the observer within a second
            for ($i = 0; $i -lt $K_UPG_OBS_TICK_SEC; $i++) {
                Start-Sleep -Seconds 1
                $gone = $false
                try { $gone = $NodeProc.HasExited } catch { $gone = $true }
                if ($gone) { break }
            }
        }
    } catch {
        $reason = 'watch loop exception: ' + $_.Exception.Message
        Add-DiagError 'Watch-UpgDaemons' $_
    }
    $w['end_reason'] = $reason
    $w['ended'] = (Get-IsoNow)
    try { Add-Utf8NoBom $tl (('# end: {0}' -f $reason) + "`r`n") } catch { }
    return $w
}

# what happened to the daemon: the page facts of node (<prefix>-obs-facts.json) + the process table of this side
function Get-UpgDaemonResult {
    param($Watch, $ObsFile, [string]$ToVersion, $BeforePids)
    $d = [ordered]@{
        pids_before = @(); pids_after = @(); new_pids = @(); old_alive_at_end = @(); images_at_end = @()
        page_attached = $null; page_polls = $null; page_reads_ok = $null; page_end_reason = $null
        daemon_version_first = $null; daemon_version_last = $null; daemon_pid_first = $null; daemon_pid_last = $null
        daemon_reached_target = $null; daemon_reached_target_sec = $null
        daemon_rotated_auto = $null; rotated_evidence = 'none'
        skew_badge_seen = $null; skew_badge_first_sec = $null; skew_badge_present_at_end = $null; skew_badge_text = $null; skew_badge_title = $null
        skew_notice_seen = $null; skew_notice_detail = $null
        daemon_info_last = $null; app_version_cmd = $null; toasts_seen = @()
        toasts = @(); update_error_toast_seen = $null; update_error_toast = $null; update_error_toast_sec = $null; restore_done_toast_seen = $null; restore_done_toast = $null
        status_bar_pid = $null; status_bar_pid_differs = $null; obs_min_sec = $null
    }
    $before = New-Object System.Collections.Generic.List[int]
    foreach ($b in @($BeforePids)) {
        if ($null -ne $b) { $before.Add([int]$b) }
    }
    $after = New-Object System.Collections.Generic.List[int]
    $newPids = New-Object System.Collections.Generic.List[int]
    $oldAlive = New-Object System.Collections.Generic.List[int]
    $images = New-Object System.Collections.Generic.List[string]
    $newImageOk = $false
    if ($Watch -is [System.Collections.IDictionary]) {
        foreach ($r in @($Watch['last'])) {
            if ($null -eq $r) { continue }
            $id = [int]$r['id']
            $after.Add($id)
            if ($before.Contains($id)) {
                $oldAlive.Add($id)
            } else {
                $newPids.Add($id)
                if ([string]$r['image_version'] -eq $ToVersion) { $newImageOk = $true }
            }
            $images.Add(('pid {0}: {1} v{2}' -f $id, [string]$r['image'], [string]$r['image_version']))
        }
    }
    $d['pids_before'] = $before.ToArray()
    $d['pids_after'] = $after.ToArray()
    $d['new_pids'] = $newPids.ToArray()
    $d['old_alive_at_end'] = $oldAlive.ToArray()
    $d['images_at_end'] = $images.ToArray()
    $f = $null
    if ($null -ne $ObsFile) {
        $f = $ObsFile.facts
        $d['page_attached'] = [bool]$ObsFile.attached
        $d['page_end_reason'] = [string]$ObsFile.end_reason
    }
    $readsOk = 0
    if ($null -ne $f) {
        try { $readsOk = [int]$f.daemon_reads_ok } catch { $readsOk = 0 }
        $d['page_polls'] = $f.polls
        $d['page_reads_ok'] = $readsOk
        $d['daemon_version_first'] = $f.daemon_version_first
        $d['daemon_version_last'] = $f.daemon_version_last
        $d['daemon_pid_first'] = $f.daemon_pid_first
        $d['daemon_pid_last'] = $f.daemon_pid_last
        if ($null -ne $f.daemon_reached_expect) { $d['daemon_reached_target'] = [bool]$f.daemon_reached_expect }
        if ($null -ne $f.daemon_reached_expect_el_ms) { $d['daemon_reached_target_sec'] = [math]::Round(([double]$f.daemon_reached_expect_el_ms) / 1000, 1) }
        $d['skew_badge_seen'] = [bool]$f.skew_badge_seen
        if ($null -ne $f.skew_badge_first_el_ms) { $d['skew_badge_first_sec'] = [math]::Round(([double]$f.skew_badge_first_el_ms) / 1000, 1) }
        $d['skew_badge_present_at_end'] = [bool]$f.skew_badge_present_at_end
        $d['skew_badge_text'] = [string]$f.skew_badge_text
        $d['skew_badge_title'] = [string]$f.skew_badge_title
        $d['skew_notice_seen'] = [bool]$f.skew_notice_seen
        $d['skew_notice_detail'] = [string]$f.skew_notice_detail
        $d['daemon_info_last'] = $f.daemon_info_last
        $d['app_version_cmd'] = $f.app_version_cmd
        $ts = New-Object System.Collections.Generic.List[string]
        foreach ($t in @($f.toasts_seen)) {
            if ($null -eq $t) { continue }
            $ts.Add(('[{0}] {1} :: {2}' -f [string]$t.cls, [string]$t.name, (Limit-Text ([string]$t.detail) 300)))
        }
        $d['toasts_seen'] = $ts.ToArray()
        # UPG-2: every notification as a record (when first seen, when gone, there at the start / at the end) and the two that matter
        $tl2 = New-Object System.Collections.Generic.List[object]
        foreach ($t in @($f.toasts_seen)) {
            if ($null -eq $t) { continue }
            $gone = $null
            if ($null -ne $t.gone_el_ms) { $gone = [math]::Round(([double]$t.gone_el_ms) / 1000, 1) }
            $tl2.Add([ordered]@{ kind = [string]$t.cls; title = [string]$t.name; text = (Limit-Text ([string]$t.detail) 400); first_sec = [math]::Round(([double]$t.first_el_ms) / 1000, 1); gone_sec = $gone; at_start = [bool]$t.present_at_start; at_end = [bool]$t.present_at_end; polls = $t.polls })
        }
        $d['toasts'] = $tl2.ToArray()
        if ($null -ne $f.update_error_toast_seen) {
            $d['update_error_toast_seen'] = [bool]$f.update_error_toast_seen
            $d['update_error_toast'] = [string]$f.update_error_toast_detail
            if ($null -ne $f.update_error_toast_first_el_ms) { $d['update_error_toast_sec'] = [math]::Round(([double]$f.update_error_toast_first_el_ms) / 1000, 1) }
        }
        if ($null -ne $f.restore_done_toast_seen) {
            $d['restore_done_toast_seen'] = [bool]$f.restore_done_toast_seen
            $d['restore_done_toast'] = [string]$f.restore_done_toast_detail
        }
        $d['status_bar_pid'] = $f.status_bar_pid
        if ($null -ne $f.status_bar_pid_differs) { $d['status_bar_pid_differs'] = [bool]$f.status_bar_pid_differs }
        if ($null -ne $ObsFile.obs_min_ms) { $d['obs_min_sec'] = [math]::Round(([double]$ObsFile.obs_min_ms) / 1000, 0) }
    }
    # nothing is clicked by this harness in the window, so a daemon that changed did so by the app's own doing
    if (($d['daemon_reached_target'] -eq $true) -and ($oldAlive.Count -gt 0) -and ($newPids.Count -eq 0)) {
        $d['daemon_rotated_auto'] = $false
        $d['rotated_evidence'] = 'the daemon of before the upgrade is still the only one and it already reports the target version (the same version on both sides)'
    } elseif ($d['daemon_reached_target'] -eq $true) {
        $d['daemon_rotated_auto'] = $true
        $d['rotated_evidence'] = 'the app''s own daemon_status reports the target version'
    } elseif ($readsOk -gt 0) {
        $d['daemon_rotated_auto'] = $false
        $d['rotated_evidence'] = 'the app''s own daemon_status still reports ' + [string]$d['daemon_version_last']
    } elseif (($before.Count -gt 0) -and ($oldAlive.Count -eq 0) -and $newImageOk) {
        $d['daemon_rotated_auto'] = $true
        $d['rotated_evidence'] = 'process table only (the page could not be asked): every daemon of before the upgrade is gone and a cysd.exe of the target version runs'
    } elseif ($oldAlive.Count -gt 0) {
        $d['daemon_rotated_auto'] = $false
        $d['rotated_evidence'] = 'process table only (the page could not be asked): a daemon of before the upgrade is still running'
    } else {
        $d['rotated_evidence'] = 'unknown: the page could not be asked and the process table is not conclusive'
    }
    return $d
}

# =========================================================================
# UPG-2: marker / stamp files of the product (records, never part of a verdict)
#   %USERPROFILE%\.cys\.pending-restore   src-tauri/src/main.rs pending_restore_path(): written before a daemon hand-over, removed after
#                                         a successful "cys init-pack" of maybe_apply_pending_update
#   %USERPROFILE%\.cys\.last-app-version  last_app_version_path(): the app version whose pack was applied (written after init-pack)
#   %USERPROFILE%\.cys\pack\.pack-version the commit marker of the pack
#   %USERPROFILE%\.cys\pack.prev          src/pack.rs pack_prev_dir(): "<pack_dir>.prev", the one-generation rollback copy (atomic_swap)
#   %USERPROFILE%\.cys\.pack-staging*     init_staging_dir(): ".pack-staging-init-<pid>" beside the pack (pack-update uses ".pack-staging")
#   %USERPROFILE%\.cys\.gui-onboarded     gui_onboarded_path(): the app version whose GUI onboarding (cys init-pack + cys daemon install,
#                                         maybe_windows_onboard) succeeded; anything else makes the next app start run the onboarding again
#   %USERPROFILE%\.cys\.gui-onboard-attempts  {"version","reason","attempts"}: onboarding tries that could not be judged
# =========================================================================
function Get-UpgStamps {
    param([string]$Tag)
    $h = Join-Path $env:USERPROFILE '.cys'
    $o = [ordered]@{ tag = $Tag; time = (Get-IsoNow); pending_restore = $null; last_app_version = $null; pack_version = $null; pack_prev = $null; staging = @(); gui_onboarded = $null; gui_onboard_attempts_present = $null; gui_onboard_attempts = $null; error = $null }
    try {
        $go = Join-Path $h '.gui-onboarded'
        if (Test-Path -LiteralPath $go) { $o['gui_onboarded'] = (Limit-Text (([string](Read-FileShared $go)).Trim()) 60) }
        $ga = Join-Path $h '.gui-onboard-attempts'
        $o['gui_onboard_attempts_present'] = [bool](Test-Path -LiteralPath $ga)
        if ($o['gui_onboard_attempts_present']) { $o['gui_onboard_attempts'] = (Limit-Text (([string](Read-FileShared $ga)).Trim()) 200) }
        $o['pending_restore'] = [bool](Test-Path -LiteralPath (Join-Path $h '.pending-restore'))
        $lv = Join-Path $h '.last-app-version'
        if (Test-Path -LiteralPath $lv) { $o['last_app_version'] = (Limit-Text (([string](Read-FileShared $lv)).Trim()) 60) }
        $pv = Join-Path $h 'pack\.pack-version'
        if (Test-Path -LiteralPath $pv) { $o['pack_version'] = (Limit-Text (([string](Read-FileShared $pv)).Trim()) 60) }
        $o['pack_prev'] = [bool](Test-Path -LiteralPath (Join-Path $h 'pack.prev'))
        $st = New-Object System.Collections.Generic.List[string]
        if (Test-Path -LiteralPath $h) {
            foreach ($d in @(Get-ChildItem -LiteralPath $h -Force -Filter '.pack-staging*' -ErrorAction SilentlyContinue)) {
                if ($null -ne $d) { $st.Add([string]$d.Name) }
            }
        }
        $o['staging'] = $st.ToArray()
    } catch { $o['error'] = $_.Exception.Message }
    return $o
}

function Format-UpgStamps {
    param($S)
    if (-not ($S -is [System.Collections.IDictionary])) { return 'not read' }
    return ('stamp={0} pending-restore={1} pack={2} pack.prev={3} staging=[{4}] gui-onboarded={5} onboard-attempts={6}' -f (Format-UpgVal $S['last_app_version'] 'none'), $S['pending_restore'], (Format-UpgVal $S['pack_version'] 'none'), $S['pack_prev'], (@($S['staging']) -join ','), (Format-UpgVal $S['gui_onboarded'] 'none'), (Format-UpgVal $S['gui_onboard_attempts'] 'none'))
}

# =========================================================================
# UPG-2: process creation / termination auditing for this scene (observation only; everything is put back in upg-cleanup)
#   on      auditpol /set /subcategory:{GUID} /success:enable for Process Creation and Process Termination, and the policy value
#           ProcessCreationIncludeCmdLine_Enabled = 1 (4688 then carries the command line). The values of before are recorded.
#   test    a cmd.exe that exits with 7 and carries a random word in its command line must show up as 4688 (+ command line) and
#           4689 (Status 0x7). Only then the audit counts as working; else the fallback (diag/proc-poll.ps1) is used.
#   read    Get-WinEvent Security 4688 / 4689. 4688: NewProcessId, ProcessId (= the creator), NewProcessName, CommandLine,
#           ParentProcessName. 4689: ProcessId, ProcessName, Status. The ids are hex strings ("0x1a2c").
# =========================================================================
function ConvertFrom-UpgHex {
    param([string]$Text)
    try {
        $t = ([string]$Text).Trim()
        if ($t -eq '') { return $null }
        if ($t -match '^0[xX]([0-9a-fA-F]+)$') { return [Convert]::ToInt64($Matches[1], 16) }
        if ($t -match '^\d+$') { return [int64]$t }
    } catch { }
    return $null
}

function Invoke-UpgSysTool {
    param([string]$Exe, [string]$Arguments, [int]$TimeoutSec = 30)
    $x = Invoke-Proc -File (Join-Path $env:windir ('System32\' + $Exe)) -Arguments $Arguments -TimeoutSec $TimeoutSec
    return [ordered]@{ exe = $Exe; args = $Arguments; rc = $x['rc']; timed_out = $x['timedOut']; start_error = $x['startError']; out = (Limit-Text (([string]$x['out']).Trim()) 1500); err = (Limit-Text (([string]$x['err']).Trim()) 600) }
}

# "auditpol /get /subcategory:{GUID} /r" prints CSV: Machine Name,Policy Target,Subcategory,Subcategory GUID,Inclusion Setting,Exclusion Setting
function Get-UpgAuditSetting {
    param([string]$Guid)
    $r = Invoke-UpgSysTool 'auditpol.exe' ('/get /subcategory:' + $Guid + ' /r')
    $r['setting'] = $null
    $r['success'] = $null
    try {
        foreach ($line in (([string]$r['out']) -split "`r?`n")) {
            if ($line.ToUpperInvariant().Contains($Guid.ToUpperInvariant())) {
                $f = $line.Split(',')
                if ($f.Count -ge 5) {
                    $r['setting'] = [string]$f[4]
                    $r['success'] = [bool]([string]$f[4] -match 'Success')
                }
            }
        }
    } catch { }
    return $r
}

# the Security events 4688 / 4689 since a moment, oldest first, as plain records
function Read-UpgAuditEvents {
    param([datetime]$Since, [int]$MaxEvents = 40000)
    $res = [ordered]@{ ok = $false; error = $null; since = (ConvertTo-IsoUtc $Since); read = 0; events = @() }
    $raw = @()
    try {
        $raw = @(Get-WinEvent -FilterHashtable @{ LogName = 'Security'; Id = 4688, 4689; StartTime = $Since } -MaxEvents $MaxEvents -ErrorAction Stop)
        $res['ok'] = $true
    } catch {
        if ([string]$_.FullyQualifiedErrorId -like 'NoMatchingEventsFound*') { $res['ok'] = $true } else { $res['error'] = $_.Exception.Message }
        $raw = @()
    }
    $list = New-Object System.Collections.Generic.List[object]
    for ($i = $raw.Count - 1; $i -ge 0; $i--) {
        $ev = $raw[$i]
        if ($null -eq $ev) { continue }
        try {
            $d = [ordered]@{}
            $x = [xml]$ev.ToXml()
            foreach ($n in @($x.Event.EventData.Data)) {
                if ($null -ne $n) { $d[[string]$n.Name] = [string]$n.InnerText }
            }
            $id = [int]$ev.Id
            $e = [ordered]@{ time = (ConvertTo-IsoUtc $ev.TimeCreated); id = $id; kind = $null; pid = $null; ppid = $null; image = $null; cmd = $null; parent_image = $null; status = $null; status_dec = $null; raw = $d }
            if ($id -eq 4688) {
                $e['kind'] = 'create'
                $e['pid'] = ConvertFrom-UpgHex ([string]$d['NewProcessId'])
                $e['ppid'] = ConvertFrom-UpgHex ([string]$d['ProcessId'])
                $e['image'] = [string]$d['NewProcessName']
                $e['cmd'] = [string]$d['CommandLine']
                $e['parent_image'] = [string]$d['ParentProcessName']
            } else {
                $e['kind'] = 'exit'
                $e['pid'] = ConvertFrom-UpgHex ([string]$d['ProcessId'])
                $e['image'] = [string]$d['ProcessName']
                $e['status'] = [string]$d['Status']
                $e['status_dec'] = ConvertFrom-UpgHex ([string]$d['Status'])
            }
            $list.Add($e)
        } catch { }
    }
    $res['read'] = $list.Count
    $res['events'] = $list.ToArray()
    return $res
}

function Enable-UpgProcAudit {
    $a = [ordered]@{ started = (Get-IsoNow); works = $false; cmdline_works = $false; exit_status_works = $false; before = [ordered]@{}; set = [ordered]@{}; after = [ordered]@{}; reg_before = $null; reg_before_value = $null; reg_set = $null; log_before = $null; log_max_before = $null; log_set = $null; selftest = $null; error = $null; restored = $null }
    try {
        foreach ($g in @($K_UPG_AUDIT_CREATE, $K_UPG_AUDIT_EXIT)) { $a['before'][$g] = Get-UpgAuditSetting $g }
        $q = Invoke-UpgSysTool 'reg.exe' ('query "' + $K_UPG_AUDIT_REGKEY + '" /v ' + $K_UPG_AUDIT_REGVAL)
        $a['reg_before'] = $q
        $m = [regex]::Match([string]$q['out'], 'REG_DWORD\s+0x([0-9a-fA-F]+)')
        if (($q['rc'] -eq 0) -and $m.Success) { $a['reg_before_value'] = [Convert]::ToInt64($m.Groups[1].Value, 16) }
        $a['reg_set'] = Invoke-UpgSysTool 'reg.exe' ('add "' + $K_UPG_AUDIT_REGKEY + '" /v ' + $K_UPG_AUDIT_REGVAL + ' /t REG_DWORD /d 1 /f')
        foreach ($g in @($K_UPG_AUDIT_CREATE, $K_UPG_AUDIT_EXIT)) { $a['set'][$g] = Invoke-UpgSysTool 'auditpol.exe' ('/set /subcategory:' + $g + ' /success:enable') }
        foreach ($g in @($K_UPG_AUDIT_CREATE, $K_UPG_AUDIT_EXIT)) { $a['after'][$g] = Get-UpgAuditSetting $g }
        # the Security log: 20 MB by default; every process of the machine is logged from now on, so give it room
        $gl = Invoke-UpgSysTool 'wevtutil.exe' 'gl Security'
        $a['log_before'] = $gl
        $m2 = [regex]::Match([string]$gl['out'], 'maxSize:\s*(\d+)')
        if ($m2.Success) {
            $a['log_max_before'] = [int64]$m2.Groups[1].Value
            if ([int64]$a['log_max_before'] -lt 134217728) { $a['log_set'] = Invoke-UpgSysTool 'wevtutil.exe' 'sl Security /ms:134217728' }
        }
        # the test: one process with a known command line and a known exit code
        $word = 'diag-audit-' + [guid]::NewGuid().ToString('N').Substring(0, 10)
        $t0 = (Get-Date).AddSeconds(-3)
        $st = [ordered]@{ word = $word; rc = $null; tries = 0; create_seen = $false; cmdline_seen = $false; exit_seen = $false; exit_status = $null; events_read = $null; read_error = $null }
        $a['selftest'] = $st
        $x = Invoke-Proc -File (Join-Path $env:windir 'System32\cmd.exe') -Arguments ('/c "echo ' + $word + '>nul & exit 7"') -TimeoutSec 20
        $st['rc'] = $x['rc']
        for ($n = 1; $n -le 5; $n++) {
            $st['tries'] = $n
            Start-Sleep -Seconds 1
            $r = Read-UpgAuditEvents -Since $t0 -MaxEvents 5000
            $st['events_read'] = $r['read']
            $st['read_error'] = $r['error']
            $tp = $null
            foreach ($e in @($r['events'])) {
                if ($null -eq $e) { continue }
                if (($e['kind'] -eq 'create') -and ([string]$e['image'] -like '*\cmd.exe')) {
                    if ([string]$e['cmd'] -like ('*' + $word + '*')) { $st['create_seen'] = $true; $st['cmdline_seen'] = $true; $tp = $e['pid'] }
                    elseif (($null -eq $tp) -and ([int64]$e['ppid'] -eq [int64]$PID)) { $st['create_seen'] = $true; $tp = $e['pid'] }
                }
                if (($e['kind'] -eq 'exit') -and ($null -ne $tp) -and ($e['pid'] -eq $tp) -and ([string]$e['image'] -like '*\cmd.exe')) { $st['exit_seen'] = $true; $st['exit_status'] = $e['status'] }
            }
            if ($st['create_seen'] -and $st['exit_seen']) { break }
        }
        $a['works'] = [bool]$st['create_seen']
        $a['cmdline_works'] = [bool]$st['cmdline_seen']
        $a['exit_status_works'] = [bool]($st['exit_seen'] -and ((ConvertFrom-UpgHex ([string]$st['exit_status'])) -eq 7))
    } catch {
        $a['error'] = 'exception: ' + $_.Exception.Message
    }
    $a['finished'] = (Get-IsoNow)
    return $a
}

# put back what Enable-UpgProcAudit changed (the log size stays: a bigger log does no harm and shrinking a filled log can fail)
function Restore-UpgProcAudit {
    param($Setup)
    $r = [ordered]@{ time = (Get-IsoNow); auditpol = [ordered]@{}; registry = $null; note = $null }
    try {
        if (-not ($Setup -is [System.Collections.IDictionary])) { $r['note'] = 'the audit was never set up: nothing to put back'; return $r }
        foreach ($g in @($K_UPG_AUDIT_CREATE, $K_UPG_AUDIT_EXIT)) {
            $b = $Setup['before'][$g]
            if (($b -is [System.Collections.IDictionary]) -and ($b['success'] -eq $false)) {
                $r['auditpol'][$g] = Invoke-UpgSysTool 'auditpol.exe' ('/set /subcategory:' + $g + ' /success:disable')
            } else {
                $r['auditpol'][$g] = 'left as it is (success auditing was already on before, or the value of before could not be read)'
            }
        }
        if ($null -eq $Setup['reg_before_value']) {
            $r['registry'] = Invoke-UpgSysTool 'reg.exe' ('delete "' + $K_UPG_AUDIT_REGKEY + '" /v ' + $K_UPG_AUDIT_REGVAL + ' /f')
        } else {
            $r['registry'] = Invoke-UpgSysTool 'reg.exe' ('add "' + $K_UPG_AUDIT_REGKEY + '" /v ' + $K_UPG_AUDIT_REGVAL + ' /t REG_DWORD /d ' + [string]$Setup['reg_before_value'] + ' /f')
        }
    } catch { $r['note'] = 'exception: ' + $_.Exception.Message }
    return $r
}

# the file name of an image path as the event wrote it (a plain text split: the path is data, not a file of this machine)
function Get-UpgLeaf {
    param([string]$Image)
    if (-not $Image) { return '' }
    return [string](($Image -split '[\\/]')[-1])
}

function Test-UpgAuditImage {
    param([string]$Image, [string]$InstallPrefix)
    if (-not $Image) { return $false }
    $leaf = Get-UpgLeaf $Image
    if ($Image.StartsWith($InstallPrefix, [System.StringComparison]::OrdinalIgnoreCase)) { return $true }
    if (@('cys-app.exe', 'cys.exe', 'cysd.exe') -contains $leaf.ToLowerInvariant()) { return $true }
    if (Test-AppInstallerProc $leaf $Image) { return $true }
    return $false
}

# keep the processes of the product (install folder, updater temp folder, installer) and everything they started, in time order;
# a creation and its termination are matched by (pid, image, order in time) because Windows hands a pid out again
function Select-UpgAuditTree {
    param($Events)
    $kept = New-Object System.Collections.Generic.List[object]
    $alive = @{}
    $prefix = (Get-InstallDir).TrimEnd('\') + '\'
    foreach ($e in @($Events)) {
        if ($null -eq $e) { continue }
        $img = [string]$e['image']
        $mine = Test-UpgAuditImage $img $prefix
        $k = [string]$e['pid']
        if ($e['kind'] -eq 'create') {
            $pk = [string]$e['ppid']
            if ($mine -or $alive.ContainsKey($pk)) {
                $e['why'] = $(if ($mine) { 'product image' } else { 'started by ' + [string]$alive[$pk] })
                $alive[$k] = $img
                $kept.Add($e)
            }
        } else {
            if ($alive.ContainsKey($k) -and ([string]$alive[$k] -ieq $img)) {
                $alive.Remove($k)
                $kept.Add($e)
            } elseif ($mine) {
                $e['why'] = 'product image (its creation is older than the window)'
                $kept.Add($e)
            }
        }
        if ($kept.Count -ge 8000) { break }
    }
    return $kept.ToArray()
}

# creation + termination -> one record per process
function ConvertTo-UpgProcList {
    param($Kept)
    $open = @{}
    $done = New-Object System.Collections.Generic.List[object]
    foreach ($e in @($Kept)) {
        if ($null -eq $e) { continue }
        $k = [string]$e['pid']
        if ($e['kind'] -eq 'create') {
            if ($open.ContainsKey($k)) { $done.Add($open[$k]); $open.Remove($k) }
            $leaf = Get-UpgLeaf ([string]$e['image'])
            $open[$k] = [ordered]@{ pid = $e['pid']; ppid = $e['ppid']; image = [string]$e['image']; leaf = $leaf; cmd = [string]$e['cmd']; parent_image = [string]$e['parent_image']; start = [string]$e['time']; end = $null; seconds = $null; exit_status = $null; exit_status_dec = $null; source = 'audit' }
        } elseif ($open.ContainsKey($k) -and ([string]$open[$k]['image'] -ieq [string]$e['image'])) {
            $p = $open[$k]
            $p['end'] = [string]$e['time']
            $p['seconds'] = Get-UpgSeconds ([string]$p['start']) ([string]$p['end'])
            $p['exit_status'] = $e['status']
            $p['exit_status_dec'] = $e['status_dec']
            $done.Add($p)
            $open.Remove($k)
        }
    }
    foreach ($k in @($open.Keys)) { $done.Add($open[$k]) }
    return @($done.ToArray() | Sort-Object { [string]$_['start'] })
}

# the arguments of a command line without the program ("C:\...\cys.exe" init-pack --no-install-hook -> init-pack --no-install-hook)
function Get-UpgCmdArgs {
    param([string]$Cmd)
    $m = [regex]::Match(([string]$Cmd).Trim(), '^(?:"[^"]*"|\S+)\s*(.*)$')
    if ($m.Success) { return [string]$m.Groups[1].Value }
    return ''
}

# the table the summary line comes from: the cys.exe processes after the relaunch, init-pack among them, the new daemon, restore / drain
function Get-UpgInitPackTable {
    param($Procs, [string]$RelaunchIso, $BeforeDaemonPids, [string]$Source, [string]$NowIso)
    $t = [ordered]@{ source = $Source; relaunch_at = $RelaunchIso; cys_processes = @(); init_pack_count = 0; init_pack_onboarding = 0; init_pack_no_install_hook = 0; init_pack = @(); init_pack_overlap = $null; init_pack_nonzero = 0; exit_status_known = $false; new_daemon_started_at = $null; new_daemon_pid = $null; restore = @(); drain = @(); daemon_install = @(); line = $null; parts = '' }
    $from = ''
    try { if ($RelaunchIso) { $from = ConvertTo-IsoUtc (([datetime]::Parse($RelaunchIso, [System.Globalization.CultureInfo]::InvariantCulture, [System.Globalization.DateTimeStyles]::RoundtripKind)).AddSeconds(-10)) } } catch { $from = '' }
    $before = New-Object System.Collections.Generic.List[string]
    foreach ($b in @($BeforeDaemonPids)) { if ($null -ne $b) { $before.Add([string]$b) } }
    $cys = New-Object System.Collections.Generic.List[object]
    $ip = New-Object System.Collections.Generic.List[object]
    $rs = New-Object System.Collections.Generic.List[object]
    $dr = New-Object System.Collections.Generic.List[object]
    $di = New-Object System.Collections.Generic.List[object]
    foreach ($p in @($Procs)) {
        if ($null -eq $p) { continue }
        if ($from -and ([string]::CompareOrdinal([string]$p['start'], $from) -lt 0)) { continue }
        $leaf = ([string]$p['leaf']).ToLowerInvariant()
        if (($leaf -eq 'cysd.exe') -and ($null -eq $t['new_daemon_started_at']) -and (-not $before.Contains([string]$p['pid']))) {
            $t['new_daemon_started_at'] = [string]$p['start']
            $t['new_daemon_pid'] = $p['pid']
        }
        if ($leaf -ne 'cys.exe') { continue }
        $args1 = Get-UpgCmdArgs ([string]$p['cmd'])
        $row = [ordered]@{ pid = $p['pid']; ppid = $p['ppid']; parent_image = $p['parent_image']; args = (Limit-Text $args1 300); start = $p['start']; end = $p['end']; seconds = $p['seconds']; exit_status = $p['exit_status']; exit_status_dec = $p['exit_status_dec'] }
        $cys.Add($row)
        if ($args1 -match '(^|\s)init-pack(\s|$)') {
            # UPG-3: the two callers differ by one flag: "init-pack" alone is the GUI onboarding (maybe_windows_onboard),
            # "init-pack --no-install-hook" is maybe_apply_pending_update (app setup / after a daemon rotation)
            if ($args1 -match '--no-install-hook') { $row['kind'] = 'no-install-hook'; $t['init_pack_no_install_hook'] = [int]$t['init_pack_no_install_hook'] + 1 }
            else { $row['kind'] = 'onboarding'; $t['init_pack_onboarding'] = [int]$t['init_pack_onboarding'] + 1 }
            $ip.Add($row)
        }
        if ($args1 -match '(^|\s)daemon\s+install(\s|$)') { $di.Add($row) }
        if ($args1 -match '(^|\s)restore(\s|$)') { $rs.Add($row) }
        if ($args1 -match '(^|\s)drain(\s|$)') { $dr.Add($row) }
    }
    $t['cys_processes'] = $cys.ToArray()
    $t['init_pack'] = $ip.ToArray()
    $t['init_pack_count'] = $ip.Count
    $t['restore'] = $rs.ToArray()
    $t['drain'] = $dr.ToArray()
    $t['daemon_install'] = $di.ToArray()
    # overlap: two intervals [start, end] meet; a process without a seen end is taken as running until now
    $ov = $false
    for ($i = 0; $i -lt $ip.Count; $i++) {
        for ($j = $i + 1; $j -lt $ip.Count; $j++) {
            $a1 = [string]$ip[$i]['start']; $a2 = [string]$ip[$i]['end']; if (-not $a2) { $a2 = $NowIso }
            $b1 = [string]$ip[$j]['start']; $b2 = [string]$ip[$j]['end']; if (-not $b2) { $b2 = $NowIso }
            if (([string]::CompareOrdinal($a1, $b2) -lt 0) -and ([string]::CompareOrdinal($b1, $a2) -lt 0)) { $ov = $true }
        }
    }
    if ($ip.Count -ge 2) { $t['init_pack_overlap'] = [bool]$ov } elseif ($ip.Count -eq 1) { $t['init_pack_overlap'] = $false }
    $parts = New-Object System.Collections.Generic.List[string]
    foreach ($r in $ip.ToArray()) {
        if ($null -ne $r['exit_status_dec']) {
            $t['exit_status_known'] = $true
            if ([int64]$r['exit_status_dec'] -ne 0) { $t['init_pack_nonzero'] = [int]$t['init_pack_nonzero'] + 1 }
        }
        $s1 = [string]$r['start']; if ($s1.Length -ge 23) { $s1 = $s1.Substring(11, 12) }
        $e1 = [string]$r['end']; if ($e1.Length -ge 23) { $e1 = $e1.Substring(11, 12) } elseif (-not $e1) { $e1 = '?' }
        $parts.Add(('[pid {0} "{1}" {2}..{3} exit={4}]' -f $r['pid'], (Limit-Text ([string]$r['args']) 60), $s1, $e1, (Format-UpgVal $r['exit_status'] 'unknown')))
    }
    $t['parts'] = ($parts.ToArray() -join ' ')
    $nd = [string]$t['new_daemon_started_at']; if ($nd.Length -ge 23) { $nd = $nd.Substring(11, 12) } elseif (-not $nd) { $nd = 'not seen' }
    $t['line'] = ('init-pack after the upgrade ({0}): {1} process(es) {2} overlap={3}; new daemon started at {4}; restore={5} drain={6} daemon install={7}; cys.exe processes after the relaunch={8}' -f $Source, $ip.Count, $t['parts'], (Format-UpgVal $t['init_pack_overlap'] 'n/a'), $nd, $rs.Count, $dr.Count, $di.Count, $cys.Count)
    return $t
}

# the audit of the window [since, now]: files <name>.json (the kept events, raw fields + decimal pids) and <name>.txt (one line each)
function Save-UpgProcAudit {
    param([datetime]$Since, [string]$Name)
    $o = [ordered]@{ name = $Name; time = (Get-IsoNow); since = (ConvertTo-IsoUtc $Since); ok = $false; error = $null; events_read = 0; events_kept = 0; processes = @() }
    try {
        $r = Read-UpgAuditEvents -Since $Since
        $o['ok'] = [bool]$r['ok']
        $o['error'] = $r['error']
        $o['events_read'] = $r['read']
        $kept = @(Select-UpgAuditTree $r['events'])
        $o['events_kept'] = $kept.Count
        Save-Json ($Name + '.json') ([ordered]@{ since = $o['since']; read = $r['read']; kept = $kept.Count; error = $r['error']; events = $kept }) 6
        $sb = New-Object System.Text.StringBuilder
        [void]$sb.Append(('# Security 4688 (create) / 4689 (exit) since {0}: {1} read, {2} kept (product images and what they started)' -f $o['since'], $r['read'], $kept.Count) + "`r`n")
        foreach ($e in $kept) {
            if ($e['kind'] -eq 'create') { [void]$sb.Append(('{0} CREATE pid={1} parent={2} [{3}] image={4} cmd={5}' -f $e['time'], $e['pid'], $e['ppid'], $e['parent_image'], $e['image'], (Format-TeamCmd ([string]$e['cmd']) 600)) + "`r`n") }
            else { [void]$sb.Append(('{0} EXIT   pid={1} status={2} image={3}' -f $e['time'], $e['pid'], $e['status'], $e['image']) + "`r`n") }
        }
        Save-Text ($Name + '.txt') $sb.ToString()
        $o['processes'] = @(ConvertTo-UpgProcList $kept)
    } catch {
        $o['error'] = 'exception: ' + $_.Exception.Message
    }
    return $o
}

# the fallback when the audit does not work: diag/proc-poll.ps1 in a second PowerShell, started the moment the new app is seen
function Start-UpgProcPoll {
    param([string]$Name = 'proc-poll')
    $f = Join-Path $global:DiagOut ($K_UPG_PFX + '-' + $Name + '.json')
    $ps = Join-Path $env:windir 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $a = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}" -Seconds {1} -IntervalMs {2} -OutFile "{3}"' -f (Join-Path $PSScriptRoot 'proc-poll.ps1'), $K_UPG_POLL_SEC, $K_UPG_POLL_MS, $f
    $p = Start-Process -FilePath $ps -ArgumentList $a -WindowStyle Hidden -PassThru -ErrorAction Stop
    try { $null = $p.Handle } catch { }
    return [ordered]@{ started = (Get-IsoNow); pid = $p.Id; file = $f; process = $p }
}

function Read-UpgProcPoll {
    param([string]$Name = 'proc-poll')
    $o = [ordered]@{ found = $false; samples = $null; rows = 0; processes = @(); error = $null }
    try {
        $j = Read-UpgJson ($K_UPG_PFX + '-' + $Name + '.json')
        if ($null -eq $j) { $o['error'] = 'the result file of the poll is missing'; return $o }
        $o['found'] = $true
        $o['samples'] = $j.samples
        $list = New-Object System.Collections.Generic.List[object]
        foreach ($r in @($j.rows)) {
            if ($null -eq $r) { continue }
            # the times as the same ISO text the audit records carry (whatever type the JSON reader gave them)
            $c1 = [string](ConvertTo-IsoUtc $r.created)
            $l1 = [string](ConvertTo-IsoUtc $r.last_seen)
            $list.Add([ordered]@{ pid = $r.pid; ppid = $r.ppid; image = [string]$r.path; leaf = [string]$r.name; cmd = [string]$r.cmd; parent_image = $null; start = $c1; end = $l1; seconds = (Get-UpgSeconds $c1 $l1); exit_status = $null; exit_status_dec = $null; source = 'poll' })
        }
        $o['rows'] = $list.Count
        $o['processes'] = @($list.ToArray() | Sort-Object { [string]$_['start'] })
    } catch { $o['error'] = 'exception: ' + $_.Exception.Message }
    return $o
}

# =========================================================================
# UPG-3: the SECOND start of the upgraded app (a record; it never changes PASS / FAIL)
#   Why: right after an upgrade the product runs "cys.exe init-pack" up to three times at once (the GUI onboarding of setup, the
#   apply of maybe_apply_pending_update, and that one again after the daemon rotation). The loser varies with timing. When the
#   onboarding call loses, %USERPROFILE%\.cys\.gui-onboarded is not written and the product runs its onboarding again on the NEXT start.
#   What: after the TEAM scene - whose own cleanup has ended every process of the install folder and removed the WebView2 debug
#   policy (Invoke-TeamScene: Reset-AppState 'after-team', Remove-WebView2DebugPolicy), so this is normally a COLD start; when an app
#   is still there, only its cys-app.exe main process is ended (the daemons stay, as when a user closes the window) - the app is
#   started the way the scene starts it (Start-CysApp, no manifest override, the policy set again), its version is read (uipre),
#   the page is watched for at least 30 s (uiobs: toasts, daemon_status, status bar), the process audit of that window is read
#   (or the fallback poll), and the stamp files are read before and after.
# =========================================================================
# the page facts of one uiobs run (<prefix>-obs-facts.json) as a small record
function Get-UpgObsNotes {
    param($ObsFile)
    $o = [ordered]@{ found = $false; attached = $null; app_version = $null; polls = $null; end_reason = $null; obs_min_sec = $null; daemon_version_last = $null; daemon_pid_last = $null; daemon_info_last = $null; status_bar_pid = $null; status_bar_pid_differs = $null; update_error_toast_seen = $null; update_error_toast = $null; update_error_toast_sec = $null; restore_done_toast_seen = $null; toasts = @(); toast_text = 'not observed' }
    try {
        if ($null -eq $ObsFile) { return $o }
        $o['found'] = $true
        $o['attached'] = [bool]$ObsFile.attached
        $o['app_version'] = $ObsFile.app_version
        $o['end_reason'] = [string]$ObsFile.end_reason
        if ($null -ne $ObsFile.obs_min_ms) { $o['obs_min_sec'] = [math]::Round(([double]$ObsFile.obs_min_ms) / 1000, 0) }
        $f = $ObsFile.facts
        if ($null -eq $f) { return $o }
        $o['polls'] = $f.polls
        $o['daemon_version_last'] = $f.daemon_version_last
        $o['daemon_pid_last'] = $f.daemon_pid_last
        $o['daemon_info_last'] = $f.daemon_info_last
        $o['status_bar_pid'] = $f.status_bar_pid
        if ($null -ne $f.status_bar_pid_differs) { $o['status_bar_pid_differs'] = [bool]$f.status_bar_pid_differs }
        if ($null -ne $f.update_error_toast_seen) {
            $o['update_error_toast_seen'] = [bool]$f.update_error_toast_seen
            $o['update_error_toast'] = [string]$f.update_error_toast_detail
            if ($null -ne $f.update_error_toast_first_el_ms) { $o['update_error_toast_sec'] = [math]::Round(([double]$f.update_error_toast_first_el_ms) / 1000, 1) }
        }
        if ($null -ne $f.restore_done_toast_seen) { $o['restore_done_toast_seen'] = [bool]$f.restore_done_toast_seen }
        $tl = New-Object System.Collections.Generic.List[object]
        $tx = New-Object System.Collections.Generic.List[string]
        foreach ($t in @($f.toasts_seen)) {
            if ($null -eq $t) { continue }
            $gone = $null
            if ($null -ne $t.gone_el_ms) { $gone = [math]::Round(([double]$t.gone_el_ms) / 1000, 1) }
            $first = [math]::Round(([double]$t.first_el_ms) / 1000, 1)
            $tl.Add([ordered]@{ kind = [string]$t.cls; title = [string]$t.name; text = (Limit-Text ([string]$t.detail) 400); first_sec = $first; gone_sec = $gone; at_start = [bool]$t.present_at_start; at_end = [bool]$t.present_at_end; polls = $t.polls })
            $tx.Add(('[{0}] {1} :: {2} (+{3} s{4})' -f [string]$t.cls, [string]$t.name, (Limit-Text ([string]$t.detail) 100), $first, $(if ($null -ne $gone) { ', gone at +' + [string]$gone + ' s' } else { '' })))
        }
        $o['toasts'] = $tl.ToArray()
        if ($tx.Count -gt 0) { $o['toast_text'] = ($tx.ToArray() -join ' || ') } else { $o['toast_text'] = 'none' }
    } catch { $o['error'] = $_.Exception.Message }
    return $o
}

function Invoke-UpgSecondStart {
    $up = $RUN['upgrade']
    $to = [string]$CTX['upg_to_version']
    $pfx = $K_UPG_PFX + '-second'
    $ss = [ordered]@{
        ran = $false; skipped = $null; started = (Get-IsoNow); finished = $null; to_version = $to; cold_start = $null; t0 = $null
        before = [ordered]@{ app_running = $null; app_pids = @(); cdp = $null; daemons = @(); note = $null }
        stop = $null; start = $null; pre = $null; app_version = $null; ui_ready = $null; obs = $null; audit = $null; poll = $null; source = $null; table = $null
        stamps_before = $null; stamps_after = $null; gui_onboarded_before = $null; gui_onboarded_after = $null
        line = $null; notes = (New-Object System.Collections.Generic.List[string]); error = $null
    }
    $up['second_start'] = $ss
    try {
        if (-not $CTX['upg_marker_reached']) { $ss['skipped'] = 'the upgrade did not put the target version on disk'; return $ss }
        $left = [math]::Round([double](Get-TeamMinutesLeft), 1)
        if ($left -lt $K_UPG_SECOND_NEED_MIN) { $ss['skipped'] = ('fewer than {0} minutes of job time are left ({1})' -f $K_UPG_SECOND_NEED_MIN, $left); return $ss }
        $node = Find-Exe 'node.exe'
        if (-not $node) { $ss['skipped'] = 'node.exe was not found'; return $ss }

        # ---- a) what runs now; the stamp files; only the app's main process is ended
        $stB = Get-UpgStamps 'second-before'
        $ss['stamps_before'] = $stB
        $ss['gui_onboarded_before'] = $stB['gui_onboarded']
        $tab = Get-UpgProcs
        $dmB = @(Get-UpgDaemonRows $tab.rows)
        $ss['before']['daemons'] = $dmB
        $beforeIds = New-Object System.Collections.Generic.List[int]
        foreach ($d0 in $dmB) { if ($null -ne $d0) { $beforeIds.Add([int]$d0['id']) } }
        $appIds = New-Object System.Collections.Generic.List[int]
        foreach ($g in @(Get-Process -Name 'cys-app' -ErrorAction SilentlyContinue)) {
            if ($null -eq $g) { continue }
            $gp = ''
            try { $gp = [string]$g.Path } catch { }
            if ((-not $gp) -or ($gp -like '*\Local\cys\*')) { $appIds.Add([int]$g.Id) }
        }
        $alive0 = Test-TeamAppAlive
        $ss['before']['app_running'] = [bool]($appIds.Count -gt 0)
        $ss['before']['app_pids'] = $appIds.ToArray()
        $ss['before']['cdp'] = [bool]$alive0['cdp']
        $ss['cold_start'] = [bool](($appIds.Count -eq 0) -and ($dmB.Count -eq 0))
        if ($appIds.Count -eq 0) {
            $ss['before']['note'] = ('no cys-app.exe was running and {0} daemon(s) were (the cleanup of the TEAM scene ends the app and the daemons): this second start is {1}' -f $dmB.Count, $(if ($dmB.Count -eq 0) { 'a cold start' } else { 'a start with a daemon already there' }))
        } else {
            $stop = [ordered]@{ pids = $appIds.ToArray(); stopped = (New-Object System.Collections.Generic.List[string]); gone = $false; gone_after_sec = $null; port_free = $false }
            $ss['stop'] = $stop
            foreach ($id in $appIds.ToArray()) {
                try { Stop-Process -Id $id -Force -ErrorAction Stop; $stop['stopped'].Add(('cys-app.exe#{0}' -f $id)) } catch { $stop['stopped'].Add(('cys-app.exe#{0} NOT stopped: {1}' -f $id, $_.Exception.Message)) }
            }
            # the WebView2 processes of that app end by themselves; wait until the app is gone and the debugging port is silent
            $tw = Get-Date
            while (((Get-Date) - $tw).TotalSeconds -lt 20) {
                $al = Test-TeamAppAlive
                if ((-not $al['process']) -and (-not $stop['gone'])) { $stop['gone'] = $true; $stop['gone_after_sec'] = [int]((Get-Date) - $tw).TotalSeconds }
                $portUp = $false
                try {
                    $resp = Invoke-WebRequest -Uri ('http://127.0.0.1:{0}/json/version' -f $K_PORT) -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
                    if ($resp.StatusCode -eq 200) { $portUp = $true }
                } catch { $portUp = $false }
                if ($stop['gone'] -and (-not $portUp)) { $stop['port_free'] = $true; break }
                Start-Sleep -Seconds 1
            }
        }
        Save-Run

        # ---- b) the start: the same way the scene starts the app (no manifest override; the debug policy is set again)
        $t0 = Get-Date
        $ss['t0'] = ConvertTo-IsoUtc $t0
        $poll = $null
        if (-not $CTX['upg_audit_ok']) {
            try { $poll = Start-UpgProcPoll -Name 'second-proc-poll' } catch { $poll = $null; Add-DiagError 'second start: Start-UpgProcPoll' $_ }
        }
        $prevPolicyRec = $RUN['webview2_policy']
        try { $null = Remove-WebView2DebugPolicy $CTX['wv2_policy'] } catch { }
        $CTX['wv2_policy'] = $null
        $CTX['manifest_url'] = ''
        $st = Start-CysApp $pfx
        $RUN['webview2_policy_second'] = $RUN['webview2_policy']
        $RUN['webview2_policy'] = $prevPolicyRec
        $ss['start'] = $st
        $ss['ran'] = $true
        Save-Run
        if ($st['cdp_ready']) {
            $pre = Invoke-NodePre -Prefix ($pfx + '-pre') -NodeExe $node
            $ss['pre'] = $pre
            $ss['app_version'] = $pre['app_version']
            $ss['ui_ready'] = [bool]$pre['ui_ready']
            $x = Invoke-UpgNode -Prefix $pfx -NodeExe $node -Mode 'uiobs' -Extra ('--obs-sec {0} --obs-interval-sec {1} --obs-min-sec {2} --obs-expect-daemon {3}' -f $K_UPG_SECOND_OBS_SEC, $K_UPG_OBS_PAGE_TICK_SEC, $K_UPG_SECOND_MIN_SEC, $to) -TimeoutSec 240
            $ob = Get-UpgObsNotes (Read-UpgJson ($pfx + '-obs-facts.json'))
            $ob['rc'] = $x['rc']
            $ss['obs'] = $ob
            if ((-not $ss['app_version']) -and $ob['app_version']) { $ss['app_version'] = $ob['app_version'] }
            # 0.14.44 scene on this live (second-started) app: usage button, Feed, office tab, bridge, doctor line
            try { if (Get-Command -Name 'Invoke-W44Scene' -CommandType Function -ErrorAction SilentlyContinue) { $ss['w44'] = Invoke-W44Scene 'w44-upg' $false } } catch { Add-DiagError 'second start w44 scene' $_ }
            try { if (Get-Command -Name 'Invoke-W44Scene2' -CommandType Function -ErrorAction SilentlyContinue) { $ss['w44b'] = Invoke-W44Scene2 'w44b-upg' } } catch { Add-DiagError 'second start w44 scene2' $_ }
        } else {
            $ss['notes'].Add(('the app did not come up at the second start (no answer on the debugging port): {0}' -f [string]$st['error']))
            try { $null = Save-Screenshot ($pfx + '-screen-nocdp.png') } catch { }
        }
        try { $null = Save-Screenshot ($pfx + '-screen-end.png') } catch { }

        # ---- c) the processes of this window: the audit, or the fallback poll
        $procs = @()
        if ($CTX['upg_audit_ok']) {
            $a = Save-UpgProcAudit -Since $t0.AddSeconds(-5) -Name ($pfx + '-proc-audit')
            $procs = @($a['processes'])
            $ss['audit'] = [ordered]@{ time = $a['time']; since = $a['since']; ok = $a['ok']; error = $a['error']; events_read = $a['events_read']; events_kept = $a['events_kept']; processes = $procs.Count }
            $ss['source'] = 'Security 4688/4689'
        } elseif ($null -ne $poll) {
            $tw2 = Get-Date
            while (((Get-Date) - $tw2).TotalSeconds -lt ($K_UPG_POLL_SEC + 20)) {
                $gone2 = $true
                try { $gone2 = [bool]$poll['process'].HasExited } catch { $gone2 = $true }
                if ($gone2) { break }
                Start-Sleep -Seconds 1
            }
            $rp = Read-UpgProcPoll -Name 'second-proc-poll'
            $procs = @($rp['processes'])
            $ss['poll'] = [ordered]@{ started = $poll['started']; found = $rp['found']; samples = $rp['samples']; rows = $rp['rows']; error = $rp['error'] }
            $ss['source'] = 'fallback poll, no exit status'
        } else {
            $ss['source'] = 'none'
        }
        if ($ss['source'] -ne 'none') { $ss['table'] = Get-UpgInitPackTable -Procs $procs -RelaunchIso ([string]$ss['t0']) -BeforeDaemonPids $beforeIds.ToArray() -Source ([string]$ss['source']) -NowIso (Get-IsoNow) }

        # ---- d) the stamp files after it
        $stA = Get-UpgStamps 'second-after'
        $ss['stamps_after'] = $stA
        $ss['gui_onboarded_after'] = $stA['gui_onboarded']

        # ---- e) NOTES (never a FAIL)
        $tb = $ss['table']
        if (($tb -is [System.Collections.IDictionary]) -and $tb['exit_status_known'] -and ([int]$tb['init_pack_nonzero'] -gt 0)) { $ss['notes'].Add(('{0} of the {1} "cys.exe init-pack" process(es) of the second start ended with a non-zero exit status: {2}' -f $tb['init_pack_nonzero'], $tb['init_pack_count'], [string]$tb['parts'])) }
        if (($ss['obs'] -is [System.Collections.IDictionary]) -and ($ss['obs']['update_error_toast_seen'] -eq $true)) { $ss['notes'].Add(('the update-error notification was shown at the second start: "{0}"' -f [string]$ss['obs']['update_error_toast'])) }
        if ($to -and ([string]$stA['gui_onboarded'] -ne $to)) { $ss['notes'].Add(('.gui-onboarded is still "{0}" after the second start (the app is {1}): the product will run its onboarding again on the next start' -f (Format-UpgVal $stA['gui_onboarded'] 'absent'), $to)) }
    } catch {
        $ss['error'] = 'exception: ' + $_.Exception.Message
        Add-DiagError 'Invoke-UpgSecondStart' $_
    }
    $ss['finished'] = (Get-IsoNow)
    return $ss
}

# the one summary line of the second start
function Format-UpgSecondStart {
    param($S)
    if (-not ($S -is [System.Collections.IDictionary])) { return 'second start: not run' }
    if ($S['skipped']) { return 'second start: not run (' + [string]$S['skipped'] + ')' }
    $tb = $S['table']
    $ipText = 'processes not read'
    if ($tb -is [System.Collections.IDictionary]) { $ipText = ('cys.exe init-pack {0} (onboarding {1}, --no-install-hook {2}){3}; daemon install={4}; cys.exe processes={5}' -f $tb['init_pack_count'], $tb['init_pack_onboarding'], $tb['init_pack_no_install_hook'], $(if ($tb['parts']) { ' ' + [string]$tb['parts'] } else { '' }), @($tb['daemon_install']).Count, @($tb['cys_processes']).Count) }
    $ob = $S['obs']
    return ('second start ({0}; {1}): app version {2}, CDP ready={3}, UI ready={4}; {5}; update-error toast seen={6}; .gui-onboarded {7} -> {8}; stamp {9} -> {10}; toasts: {11}{12}' -f $(if ($null -eq $S['cold_start']) { 'the state before it was not read' } elseif ($S['cold_start']) { 'cold start' } else { 'not a cold start: app running before=' + [string]$S['before']['app_running'] + ', daemons before=' + [string](@($S['before']['daemons']).Count) }), (Format-UpgVal $S['source'] 'no process source'), (Format-UpgVal $S['app_version'] 'not read'), (Get-UpgVal $S 'start' 'cdp_ready'), (Format-UpgVal $S['ui_ready']), $ipText, (Format-UpgVal (Get-UpgVal $ob 'update_error_toast_seen')), (Format-UpgVal $S['gui_onboarded_before'] 'none'), (Format-UpgVal $S['gui_onboarded_after'] 'none'), (Format-UpgVal (Get-UpgVal $S 'stamps_before' 'last_app_version') 'none'), (Format-UpgVal (Get-UpgVal $S 'stamps_after' 'last_app_version') 'none'), (Format-UpgVal (Get-UpgVal $ob 'toast_text') 'not observed'), $(if ($S['error']) { ' [' + [string]$S['error'] + ']' } else { '' }))
}

# =========================================================================
# scene UPGRADE: verdict. FAIL = the product did something wrong; NOT-MEASURABLE = the harness could not measure; NOTE = recorded only
# =========================================================================
function Get-UpgVerdict {
    $up = $RUN['upgrade']
    $fl = New-Object System.Collections.Generic.List[string]
    $nm = New-Object System.Collections.Generic.List[string]
    $nt = New-Object System.Collections.Generic.List[string]
    $facts = [ordered]@{}
    $from = [string]$CTX['upg_base_version']
    $to = [string]$CTX['upg_to_version']
    $mode = [string]$CTX['upg_mode']
    foreach ($x in $up['failures'].ToArray()) { $fl.Add([string]$x) }
    foreach ($x in $up['not_measurable'].ToArray()) { $nm.Add([string]$x) }
    foreach ($x in $up['notes'].ToArray()) { $nt.Add([string]$x) }
    # a measuring step that stopped on an exception could not measure (fact steps and the base-state record never decide)
    foreach ($sn in @('prepare', 'upg-base-installer', 'upg-install-base', 'upg-start-base', 'upg-apply', 'upg-after')) {
        if ($RUN['steps'].Contains($sn)) {
            $st = $RUN['steps'][$sn]
            if (($st -is [System.Collections.IDictionary]) -and ($st['ok'] -eq $false)) { $nm.Add(('step {0} stopped on an error: {1}' -f $sn, (Limit-Text ([string]$st['error']) 300))) }
        }
    }
    foreach ($sn in @('upg-base-state', 'upg-facts-base', 'upg-facts-after')) {
        if ($RUN['steps'].Contains($sn)) {
            $st = $RUN['steps'][$sn]
            if (($st -is [System.Collections.IDictionary]) -and ($st['ok'] -eq $false)) { $nt.Add(('step {0} (a record, not a test) stopped on an error: {1}' -f $sn, (Limit-Text ([string]$st['error']) 200))) }
        }
    }
    $facts['mode'] = $mode
    $facts['from_version'] = $from
    $facts['to_version'] = $to
    $facts['base_install_ok'] = [bool]$CTX['upg_base_install_ok']
    $facts['base_start_ok'] = [bool]$CTX['upg_base_start_ok']
    $facts['apply_started'] = [bool]$CTX['upg_apply_started']
    $facts['apply_ok'] = [bool]$CTX['upg_apply_ok']

    # step 6: the marker reached the target and the app came back (its failure text was filed by the step itself)
    $watchCut = $false
    $obs = $up['apply_observe']
    if ($obs -is [System.Collections.IDictionary]) {
        $facts['marker_reached_sec'] = $obs['marker_reached_sec']
        $facts['new_app_first_seen_sec'] = $obs['new_app_first_seen_sec']
        $facts['installer_seen'] = [bool]$obs['installer_seen']
        $facts['watch_end'] = [string]$obs['end_reason']
        $facts['watch_end_kind'] = [string]$obs['end_kind']
        # the step files this itself; these two lines only speak when it could not (it stopped on an error after the watch)
        if (-not $obs['success']) {
            $watchCut = [bool]([string]$obs['end_kind'] -eq 'harness')
            if ($watchCut -and ($nm.Count -eq 0)) { $nm.Add('the upgrade watch was given up by the harness: ' + [string]$obs['end_reason']) }
            if ((-not $watchCut) -and ($fl.Count -eq 0)) { $fl.Add('the upgrade did not complete: ' + [string]$obs['end_reason']) }
        }
    } elseif (($fl.Count -eq 0) -and ($nm.Count -eq 0)) {
        $nm.Add('the upgrade was never applied (see steps in app-e2e.json)')
    }

    # step 7 (a) what is on disk, (b) the app that came back
    $af = $up['after']
    if ($af -is [System.Collections.IDictionary]) {
        $facts['marker_after'] = $af['marker']
        $facts['failure_file_absent'] = $af['failure_file_absent']
        $facts['file_versions'] = $af['file_versions']
        $facts['app_version_after'] = $af['app_version']
        $facts['ui_ready_after'] = $af['ui_ready']
        if ($watchCut) {
            # the harness stopped watching before the upgrade was over: what the disk and the app looked like right then is recorded
            # (after, facts) but is not a result of the upgrade
            $nm.Add('the checks after the upgrade are not judged: the watch was given up while the upgrade may still have been running')
        } elseif ($af['a_done']) {
            if (-not $af['marker_ok']) { $fl.Add(('the version marker is "{0}" after the upgrade (expected {1})' -f [string]$af['marker'], $to)) }
            if (-not $af['failure_file_absent']) { $fl.Add('the installer left cys-install-failure.txt: ' + (Limit-Text (([string]$af['failure_file']) -replace '\s+', ' ') 300)) }
            if (-not $af['file_versions_ok']) {
                $fvs = New-Object System.Collections.Generic.List[string]
                if ($af['file_versions'] -is [System.Collections.IDictionary]) {
                    foreach ($k in @($af['file_versions'].Keys)) { $fvs.Add(('{0}={1}' -f $k, (Format-UpgVal $af['file_versions'][$k] 'unreadable'))) }
                }
                $fl.Add(('not every executable has the file version {0}: {1}' -f $to, ($fvs.ToArray() -join ', ')))
            }
        } else {
            $nm.Add('the checks of the disk after the upgrade (marker, failure file, file versions) did not complete (see step upg-after)')
        }
        $al = $af['app_alive']
        $pre = $af['pre']
        if ($watchCut) {
            $facts['app_checks_judged'] = $false
        } elseif (-not ($al -is [System.Collections.IDictionary])) {
            $nm.Add('whether the upgraded app runs was not checked (see step upg-after)')
        } elseif (-not $al['process']) {
            if ($CTX['upg_apply_ok']) { $fl.Add('the app that the installer started is not running any more') }
        } elseif (-not ($pre -is [System.Collections.IDictionary])) {
            $nm.Add('the upgraded app runs but its debugging port did not answer: version and UI could not be read')
        } elseif (-not $pre['attached']) {
            $nm.Add('the CDP driver could not attach to the upgraded app: ' + [string]$pre['error'])
        } else {
            if (-not $af['app_version_ok']) { $fl.Add(('the upgraded app reports version "{0}" (expected {1})' -f [string]$af['app_version'], $to)) }
            if (-not $af['ui_ready']) { $fl.Add('the UI of the upgraded app did not come up (its Update button was not in the page within the wait)') }
        }
    } elseif ($CTX['upg_apply_started']) {
        $nm.Add('the checks after the upgrade did not run (see step upg-after)')
    }

    # notes: the daemon, leftovers, the pack
    $dn = $up['daemon']
    if ($dn -is [System.Collections.IDictionary]) {
        $facts['daemon_rotated_auto'] = $dn['daemon_rotated_auto']
        $facts['daemon_version_last'] = $dn['daemon_version_last']
        $facts['skew_badge_seen'] = $dn['skew_badge_seen']
        if ($dn['daemon_rotated_auto'] -eq $true) {
            if ($dn['skew_badge_seen'] -eq $true) { $nt.Add(('the version-skew badge was on the screen for a while before the daemon was rotated: "{0}"' -f [string]$dn['skew_badge_text'])) }
        } elseif ($dn['daemon_rotated_auto'] -eq $false) {
            $nt.Add(('the OLD daemon was NOT replaced by the new app within {0} s ({1}); badge seen={2}{3}; notice toast seen={4}; daemon images at the end: {5}' -f $K_UPG_OBS_SEC, [string]$dn['rotated_evidence'], (Format-UpgVal $dn['skew_badge_seen']), $(if ($dn['skew_badge_text']) { ' "' + [string]$dn['skew_badge_text'] + '"' } else { '' }), (Format-UpgVal $dn['skew_notice_seen']), (@($dn['images_at_end']) -join ' | ')))
        } else {
            $nt.Add('whether the old daemon was replaced could not be determined (' + [string]$dn['rotated_evidence'] + ')')
        }
        # UPG-2: the notifications after the upgrade are facts; the two below are NOTES, never a FAIL
        $facts['toasts_after_upgrade'] = $dn['toasts']
        $facts['update_error_toast_seen'] = $dn['update_error_toast_seen']
        $facts['restore_done_toast_seen'] = $dn['restore_done_toast_seen']
        $facts['status_bar_pid_differs_from_daemon_status'] = $dn['status_bar_pid_differs']
        if ($dn['update_error_toast_seen'] -eq $true) { $nt.Add(('the upgraded app showed the update-error notification at +{0} s of the page watch: "{1}"' -f (Format-UpgVal $dn['update_error_toast_sec']), [string]$dn['update_error_toast'])) }
        if ($dn['status_bar_pid_differs'] -eq $true) { $nt.Add(('the status bar still shows daemon pid {0} at the end while daemon_status answers pid {1} (the text is not rewritten after the rotation)' -f (Format-UpgVal $dn['status_bar_pid']), (Format-UpgVal $dn['daemon_pid_last']))) }
    } elseif ($CTX['upg_apply_ok']) {
        $nt.Add('the meeting of the new app with the old daemon was not observed (see step upg-after)')
    }
    # UPG-2: the process audit and the stamp files are records
    $pa = $up['proc_audit']
    if ($pa -is [System.Collections.IDictionary]) {
        $facts['proc_audit_source'] = $pa['source']
        $tb = $pa['table']
        if ($tb -is [System.Collections.IDictionary]) {
            $facts['init_pack_count'] = $tb['init_pack_count']
            $facts['init_pack_overlap'] = $tb['init_pack_overlap']
            $facts['init_pack_nonzero_exits'] = $(if ($tb['exit_status_known']) { $tb['init_pack_nonzero'] } else { $null })
            $facts['init_pack'] = $tb['init_pack']
            $facts['new_daemon_started_at'] = $tb['new_daemon_started_at']
            if ($tb['exit_status_known'] -and ([int]$tb['init_pack_nonzero'] -gt 0)) { $nt.Add(('{0} of the {1} "cys.exe init-pack" process(es) after the upgrade ended with a non-zero exit status (overlap={2}; see appe2e-upg-proc-audit.txt)' -f $tb['init_pack_nonzero'], $tb['init_pack_count'], (Format-UpgVal $tb['init_pack_overlap'] 'n/a'))) }
        }
        if ($pa['note']) { $nt.Add('process audit: ' + [string]$pa['note']) }
    }
    $sx = $up['stamps']
    if ($sx -is [System.Collections.IDictionary]) { $facts['stamps'] = $sx['summary'] }
    # UPG-3: the second start is a record too; its findings are NOTES
    $s2 = $up['second_start']
    if ($s2 -is [System.Collections.IDictionary]) {
        $facts['second_start'] = [ordered]@{ ran = $s2['ran']; skipped = $s2['skipped']; cold_start = $s2['cold_start']; app_version = $s2['app_version']; gui_onboarded_before = $s2['gui_onboarded_before']; gui_onboarded_after = $s2['gui_onboarded_after']; init_pack_count = (Get-UpgVal $s2 'table' 'init_pack_count'); init_pack_onboarding = (Get-UpgVal $s2 'table' 'init_pack_onboarding'); init_pack_nonzero = (Get-UpgVal $s2 'table' 'init_pack_nonzero'); daemon_install = $(if ($s2['table'] -is [System.Collections.IDictionary]) { @($s2['table']['daemon_install']).Count } else { $null }); update_error_toast_seen = (Get-UpgVal $s2 'obs' 'update_error_toast_seen'); line = $s2['line'] }
        if ($s2['skipped']) { $nt.Add('second start: not run (' + [string]$s2['skipped'] + ')') }
        foreach ($x in @($s2['notes'].ToArray())) { if ($x) { $nt.Add('second start: ' + [string]$x) } }
        if ($s2['error']) { $nt.Add('second start: the step stopped on an error: ' + [string]$s2['error']) }
    }
    $lo = $up['leftovers']
    if ($lo -is [System.Collections.IDictionary]) {
        foreach ($lk in @('after_observation', 'at_end')) {
            if ($lo.Contains($lk) -and ($lo[$lk] -is [System.Collections.IDictionary])) {
                $l1 = $lo[$lk]
                $facts['leftovers_' + $lk] = ('prev={0} new.exe={1}' -f $l1['prev_count'], $l1['new_exe_count'])
                if ((([int]$l1['prev_count']) + ([int]$l1['new_exe_count'])) -gt 0) { $nt.Add(('leftovers in the install folder ({0}): {1} *.prev* file(s), {2} *.new.exe file(s): {3}' -f $lk, $l1['prev_count'], $l1['new_exe_count'], (Limit-Text ((@($l1['names']) -join ', ')) 400))) }
            }
        }
    }
    $pkB = $up['pack']['before']
    $pkA = $up['pack']['after']
    if (($pkB -is [System.Collections.IDictionary]) -and ($pkA -is [System.Collections.IDictionary])) {
        $facts['pack'] = ('files {0} -> {1}, version {2} -> {3}' -f $pkB['files'], $pkA['files'], $pkB['pack_version'], $pkA['pack_version'])
    }

    # TEAM on the upgraded app
    $tv = $CTX['team_verdict']
    $team = 'NOT_RUN'
    $teamLine = 'TEAM NOT_RUN: the scene did not run'
    if ($tv -is [System.Collections.IDictionary]) {
        $team = [string]$tv['result']
        $teamLine = 'TEAM ' + $team + ': ' + (@($tv['reasons']) -join '; ')
    }
    $reused = $null
    try { $reused = $RUN['team']['app']['reused'] } catch { }
    $facts['team_app_reused'] = $reused
    if (($team -ne 'NOT_RUN') -and ($reused -ne $true)) { $nt.Add('the TEAM scene did not use the upgraded app as it was: it started the app again (team.app in app-e2e.json says why)') }

    $upgrade = 'PASS'
    if ($fl.Count -gt 0) { $upgrade = 'FAIL' } elseif ($nm.Count -gt 0) { $upgrade = 'NOT-MEASURABLE' }
    $result = 'PASS'
    if (($upgrade -eq 'FAIL') -or ($team -eq 'FAIL')) { $result = 'FAIL' }
    elseif (($upgrade -ne 'PASS') -or ($team -ne 'PASS')) { $result = 'NOT-MEASURABLE' }
    $all = New-Object System.Collections.Generic.List[string]
    if ($upgrade -eq 'PASS') { $all.Add(('UPGRADE PASS: the installed {0} app was upgraded to {1} ({2}): the marker and the three executables are {1}, no failure file, the app came back, reports {1} and its UI is up' -f $from, $to, $mode)) }
    foreach ($x in $fl.ToArray()) { $all.Add('UPGRADE FAIL: ' + $x) }
    foreach ($x in $nm.ToArray()) { $all.Add('NOT MEASURED: ' + $x) }
    $all.Add($teamLine)
    foreach ($x in $nt.ToArray()) { $all.Add('NOTE ' + $x) }
    return [ordered]@{ result = $result; upgrade = $upgrade; team = $team; mode = $mode; from_version = $from; to_version = $to; failures = $fl.ToArray(); not_measurable = $nm.ToArray(); notes = $nt.ToArray(); reasons = $all.ToArray(); facts = $facts }
}

# =========================================================================
# scene UPGRADE: summary, 18 lines at most (14 + the three of UPG-2 + the second start of UPG-3) -> appe2e-upgrade-summary.txt
# =========================================================================
function Write-UpgSummary {
    $lines = New-Object System.Collections.Generic.List[string]
    $up = $RUN['upgrade']
    $from = [string]$CTX['upg_base_version']
    $to = [string]$CTX['upg_to_version']
    $mode = [string]$CTX['upg_mode']
    try {
        $sha = [string]$env:GITHUB_SHA
        $lines.Add(('app-upgrade on {0}: commit {1}; mode {2}; {3} -> {4}; SAC enforced at the start={5}' -f $env:DIAG_MATRIX_OS, $sha.Substring(0, [math]::Min(8, $sha.Length)), $mode, $from, $to, $RUN['sac_initially_enforced']))
    } catch { $lines.Add('line 1 (run) could not be built: ' + $_.Exception.Message) }
    try {
        $t2 = Get-InstallerSourceLine
        if ($mode -eq 'updater') { $t2 = 'installer under test: none is handed over in updater mode (the old app downloads from its real endpoint); ' + $t2 }
        $lines.Add($t2)
    } catch { $lines.Add('line 2 (installer under test) could not be built: ' + $_.Exception.Message) }
    try {
        $b = $up['base_installer']
        $t3 = 'older installer: not prepared'
        if ($b -is [System.Collections.IDictionary]) { $t3 = ('older installer {0}: {1}; {2} bytes, sha256 {3}, same as SHA256SUMS.txt={4}, signature {5}, usable={6}{7}' -f $b['version'], $b['source'], $b['size'], $b['sha256'], $b['sha256_match'], $b['signature'], $b['usable'], $(if ($b['error']) { ' [' + [string]$b['error'] + ']' } else { '' })) }
        $lines.Add($t3)
    } catch { $lines.Add('line 3 (older installer) could not be built: ' + $_.Exception.Message) }
    try {
        $bi = $up['base_install']
        $t4 = 'older version install: not run'
        if ($bi -is [System.Collections.IDictionary]) {
            $t4 = ('install of {0} with /S: rc={1} marker={2} ok={3} failure file={4} ({5} s)' -f $from, $bi['rc'], $bi['marker'], $bi['ok'], [bool]$bi['failure_file'], (Get-UpgVal $up 'timings' 'base_install_sec'))
            $bs = $up['base_start']
            $bp = $up['base_pre']
            if ($bs -is [System.Collections.IDictionary]) { $t4 = $t4 + ('; app started={0} CDP ready={1} in {2} s (WebView2 runtime {3})' -f $bs['ok'], $bs['cdp_ready'], $bs['cdp_wait_sec'], $bs['webview2_runtime']) }
            if ($bp -is [System.Collections.IDictionary]) { $t4 = $t4 + ('; attached={0} via {1}, app version {2}, UI ready={3}' -f $bp['attached'], $bp['via'], $bp['app_version'], $bp['ui_ready']) }
        }
        $lines.Add($t4)
    } catch { $lines.Add('line 4 (older version install and start) could not be built: ' + $_.Exception.Message) }
    try {
        $st = $up['base_state']
        $t5 = 'state before the upgrade: not recorded'
        if ($st -is [System.Collections.IDictionary]) {
            $pg = $st['page']
            $pgText = ''
            if ($pg -is [System.Collections.IDictionary]) { $pgText = ('; page: status bar "{0}", daemon_status version {1} pid {2} surfaces {3}' -f $pg['daemon_info'], $pg['daemon_version'], $pg['daemon_pid'], $pg['daemon_surface_count']) }
            $setText = 'not checked'
            if ($st.Contains('settled')) {
                if ($st['settled']) { $setText = ('True after +{0} s' -f $st['settled_after_sec']) }
                else { $setText = 'False (' + (Limit-Text ((@($st['settle_missing']) -join '; ')) 300) + ')' }
            }
            $t5 = ('state before the upgrade: ready={0} after {1} s ({2}); settled={8}; daemons: {3}; pipes: {4}; pack: {5} files, version {6}{7}' -f $st['ready'], $st['waited_sec'], $st['end_reason'], (Format-UpgDaemons $st['daemons']), (@(Get-UpgVal $st 'pipes' 'cys') -join ','), (Get-UpgVal $st 'pack' 'files'), (Get-UpgVal $st 'pack' 'pack_version'), $pgText, $setText)
        }
        $lines.Add($t5)
    } catch { $lines.Add('line 5 (state before the upgrade) could not be built: ' + $_.Exception.Message) }
    try {
        $ap = $up['apply']
        $t6 = 'upgrade: not applied'
        if ($ap -is [System.Collections.IDictionary]) {
            if ($ap['skipped']) {
                $t6 = 'upgrade: not applied (' + [string]$ap['skipped'] + ')'
            } elseif ($ap['emulate'] -is [System.Collections.IDictionary]) {
                $em = $ap['emulate']
                $lr = ''
                if ($em['launch'] -is [System.Collections.IDictionary]) { $lr = (' ret={0} ok={1} last_error={2}' -f (Get-UpgVal $em 'launch' 'ret'), (Get-UpgVal $em 'launch' 'ok'), (Get-UpgVal $em 'launch' 'last_error')) }
                $t6 = ('upgrade (emulate): {0} "{1}" via {2}{3}; copy is the installer under test={4}; old app pid {5} alive at the kill={6}, ended by the harness={7}{8}' -f $em['temp_installer'], $em['args'], $em['launch_api'], $lr, $em['copy_sha256_same'], $em['app_pid'], (Format-UpgVal $em['app_alive_at_kill'] 'not reached'), (Format-UpgVal $em['app_killed'] 'False'), $(if ($em['error']) { ' [' + [string]$em['error'] + ']' } else { '' }))
            } elseif ($ap['updater'] -is [System.Collections.IDictionary]) {
                $ud = $ap['updater']
                $oc = ''
                if ($ud['cdp'] -is [System.Collections.IDictionary]) { $oc = ('; install_update outcome: {0}; progress events {1}' -f (Get-UpgVal $ud 'cdp' 'install_update_outcome'), (Get-UpgVal $ud 'cdp' 'progress_count')) }
                $t6 = ('upgrade (updater): check_update of the {0} app offers "{1}" (expected {2}, gate ok={3}){4}{5}' -f $from, $ud['offered'], $ud['expected'], $ud['gate_ok'], $oc, $(if ($ap['why']) { ' [' + [string]$ap['why'] + ']' } else { '' }))
            }
        }
        $lines.Add($t6)
    } catch { $lines.Add('line 6 (how the upgrade was applied) could not be built: ' + $_.Exception.Message) }
    try {
        $ob = $up['apply_observe']
        $t7 = 'upgrade watch: not run'
        if ($ob -is [System.Collections.IDictionary]) {
            $ipText = 'no installer process was seen'
            if ($ob['installer_first_seen'] -is [System.Collections.IDictionary]) { $ipText = ('installer process at +{0} s ({1})' -f $ob['installer_first_seen_sec'], [string]$ob['installer_first_seen']['path']) }
            $goneText = 'the old app never went away'
            if ($null -ne $ob['base_app_gone_sec']) { $goneText = ('old app gone at +{0} s' -f $ob['base_app_gone_sec']) }
            $mkText = ('marker {0} -> {1}, target {2} NOT reached' -f $ob['marker_initial'], $ob['marker_last'], $to)
            if ($null -ne $ob['marker_reached_sec']) { $mkText = ('marker {0} -> {1}, target reached at +{2} s' -f $ob['marker_initial'], $ob['marker_last'], $ob['marker_reached_sec']) }
            $naText = 'no new cys-app.exe'
            if ($null -ne $ob['new_app_pid']) { $naText = ('new app pid {0} at +{1} s' -f $ob['new_app_pid'], $ob['new_app_first_seen_sec']) }
            $t7 = ('upgrade watch: success={0}; {1}; {2}; {3}; {4}; failure file seen={5}; end: {6}' -f $ob['success'], $ipText, $goneText, $mkText, $naText, $ob['failure_file_seen'], $ob['end_reason'])
        }
        $lines.Add($t7)
    } catch { $lines.Add('line 7 (upgrade watch) could not be built: ' + $_.Exception.Message) }
    try {
        $af = $up['after']
        $t8 = 'after the upgrade: not checked'
        if ($af -is [System.Collections.IDictionary]) {
            $fvs = New-Object System.Collections.Generic.List[string]
            if ($af['file_versions'] -is [System.Collections.IDictionary]) {
                foreach ($k in @($af['file_versions'].Keys)) { $fvs.Add(('{0} {1}' -f $k, (Format-UpgVal $af['file_versions'][$k] 'unreadable'))) }
            }
            $t8 = ('after the upgrade: marker {0} (ok={1}); failure file absent={2}; file versions: {3} (ok={4}); app process={5} CDP={6}; app version by CDP {7} (ok={8}); UI ready={9}' -f $af['marker'], $af['marker_ok'], $af['failure_file_absent'], ($fvs.ToArray() -join ', '), $af['file_versions_ok'], (Get-UpgVal $af 'app_alive' 'process'), (Get-UpgVal $af 'app_alive' 'cdp'), (Format-UpgVal $af['app_version'] 'not read'), $af['app_version_ok'], $af['ui_ready'])
        }
        $lines.Add($t8)
    } catch { $lines.Add('line 8 (after the upgrade) could not be built: ' + $_.Exception.Message) }
    try {
        $dn = $up['daemon']
        $t9 = 'daemon: the meeting of the new app with the old daemon was not observed'
        if ($dn -is [System.Collections.IDictionary]) {
            $t9 = ('daemon: pids before [{0}] -> after [{1}]; rotated by the app itself={2} ({3}); daemon_status version {4} -> {5}{6}; version-skew badge seen={7}{8}, present at the end={9}; notice toast seen={10}; images at the end: {11}; page polls {12}, end: {13}' -f (@($dn['pids_before']) -join ','), (@($dn['pids_after']) -join ','), (Format-UpgVal $dn['daemon_rotated_auto']), $dn['rotated_evidence'], (Format-UpgVal $dn['daemon_version_first']), (Format-UpgVal $dn['daemon_version_last']), $(if ($null -ne $dn['daemon_reached_target_sec']) { ' (target at +' + [string]$dn['daemon_reached_target_sec'] + ' s)' } else { '' }), (Format-UpgVal $dn['skew_badge_seen']), $(if ($dn['skew_badge_text']) { ' "' + [string]$dn['skew_badge_text'] + '"' } else { '' }), (Format-UpgVal $dn['skew_badge_present_at_end']), (Format-UpgVal $dn['skew_notice_seen']), (@($dn['images_at_end']) -join ' | '), (Format-UpgVal $dn['page_polls'] '0'), (Format-UpgVal $dn['page_end_reason'] 'no result of the page observer'))
        }
        $lines.Add($t9)
    } catch { $lines.Add('line 9 (daemon) could not be built: ' + $_.Exception.Message) }
    # UPG-2: three more lines (records): notifications, init-pack processes, stamp files
    try {
        $dn = $up['daemon']
        $tA = 'notifications after the upgrade: not observed'
        if ($dn -is [System.Collections.IDictionary]) {
            $tp = New-Object System.Collections.Generic.List[string]
            foreach ($x in @($dn['toasts'])) {
                if (-not ($x -is [System.Collections.IDictionary])) { continue }
                $tp.Add(('[{0}] {1} :: {2} (+{3} s{4}{5})' -f $x['kind'], $x['title'], (Limit-Text ([string]$x['text']) 120), $x['first_sec'], $(if ($x['at_start']) { ', there at the start' } else { '' }), $(if ($null -ne $x['gone_sec']) { ', gone at +' + [string]$x['gone_sec'] + ' s' } else { ', still there at the end' })))
            }
            $tA = ('notifications after the upgrade ({0} in a watch of at least {1} s): update-error (init-pack) seen={2}; restore done seen={3}; status bar pid differs from daemon_status={4} | {5}' -f $tp.Count, (Format-UpgVal $dn['obs_min_sec']), (Format-UpgVal $dn['update_error_toast_seen']), (Format-UpgVal $dn['restore_done_toast_seen']), (Format-UpgVal $dn['status_bar_pid_differs']), $(if ($tp.Count -gt 0) { ($tp.ToArray() -join ' || ') } else { 'none' }))
        }
        $lines.Add($tA)
    } catch { $lines.Add('line (notifications) could not be built: ' + $_.Exception.Message) }
    try {
        $pa = $up['proc_audit']
        $tB = 'init-pack after the upgrade: the process audit did not run'
        if ($pa -is [System.Collections.IDictionary]) {
            if (($pa['table'] -is [System.Collections.IDictionary]) -and $pa['table']['line']) { $tB = [string]$pa['table']['line'] }
            else { $tB = 'init-pack after the upgrade: no table (' + (Format-UpgVal $pa['note'] 'see proc_audit in app-e2e.json') + ')' }
            $tB = $tB + ('; audit works={0} command line={1} exit status={2}' -f (Get-UpgVal $pa 'setup' 'works'), (Get-UpgVal $pa 'setup' 'cmdline_works'), (Get-UpgVal $pa 'setup' 'exit_status_works'))
        }
        $lines.Add($tB)
    } catch { $lines.Add('line (init-pack) could not be built: ' + $_.Exception.Message) }
    try {
        $sx = $up['stamps']
        $tC = 'stamp: not read'
        if (($sx -is [System.Collections.IDictionary]) -and $sx['summary']) { $tC = [string]$sx['summary'] }
        $lines.Add($tC)
    } catch { $lines.Add('line (stamp) could not be built: ' + $_.Exception.Message) }
    try { $lines.Add((Format-UpgSecondStart $up['second_start'])) } catch { $lines.Add('line (second start) could not be built: ' + $_.Exception.Message) }
    try {
        $parts = New-Object System.Collections.Generic.List[string]
        $lo = $up['leftovers']
        foreach ($lk in @('right_after', 'after_observation', 'at_end')) {
            if ($lo.Contains($lk) -and ($lo[$lk] -is [System.Collections.IDictionary])) { $parts.Add(('{0}: {1} *.prev*, {2} *.new.exe' -f $lk, $lo[$lk]['prev_count'], $lo[$lk]['new_exe_count'])) }
        }
        $pkText = 'pack: not compared'
        $pkB = $up['pack']['before']
        $pkA = $up['pack']['after']
        if (($pkB -is [System.Collections.IDictionary]) -and ($pkA -is [System.Collections.IDictionary])) { $pkText = ('pack: {0} files version {1} -> {2} files version {3}' -f $pkB['files'], $pkB['pack_version'], $pkA['files'], $pkA['pack_version']) }
        $lines.Add(('leftovers in the install folder: {0}; {1}' -f $(if ($parts.Count -gt 0) { ($parts.ToArray() -join ' | ') } else { 'not counted' }), $pkText))
    } catch { $lines.Add('line 10 (leftovers and pack) could not be built: ' + $_.Exception.Message) }
    try {
        $fparts = New-Object System.Collections.Generic.List[string]
        foreach ($fk in @('base', 'after')) {
            $ff = $up['install_facts'][$fk]
            if ($ff -is [System.Collections.IDictionary]) {
                $cv = ''
                if ($ff['cli'] -is [System.Collections.IDictionary]) {
                    foreach ($cn in @('cys.exe', 'cysd.exe')) {
                        $cc = $ff['cli'][$cn]
                        if ($cc -is [System.Collections.IDictionary]) { $cv = $cv + (' {0} --version: rc={1} "{2}"{3};' -f $cn, $cc['rc'], $cc['out'], $(if ($cc['skipped']) { ' skipped: ' + [string]$cc['skipped'] } else { '' })) }
                    }
                }
                $fparts.Add(('[{0}] uninstall entry DisplayVersion {1}, start menu shortcut={2}, desktop shortcut={3}, {4} files / {5} bytes,{6}' -f $fk, (Format-UpgVal (Get-UpgVal $ff 'uninstall' 'display_version') 'not found'), (Get-UpgVal $ff 'shortcuts' 'start_menu_present'), (Get-UpgVal $ff 'shortcuts' 'desktop_present'), (Get-UpgVal $ff 'files' 'count'), (Get-UpgVal $ff 'files' 'bytes'), $cv))
            }
        }
        $lines.Add(('install facts (records, not tests): {0}' -f $(if ($fparts.Count -gt 0) { ($fparts.ToArray() -join ' ') } else { 'not collected' })))
    } catch { $lines.Add('line 11 (install facts) could not be built: ' + $_.Exception.Message) }
    try {
        $tm = $up['timings']
        $tv5 = New-Object System.Collections.Generic.List[string]
        foreach ($tk in @('base_install_sec', 'apply_to_marker_sec', 'marker_to_relaunch_sec', 'apply_to_relaunch_sec', 'daemon_target_sec')) {
            if ($null -ne $tm[$tk]) { $tv5.Add(([string]$tm[$tk]) + ' s') } else { $tv5.Add('not measured') }
        }
        $lines.Add(('times: install of the older version {0}; upgrade start -> marker {1}; marker -> app back {2}; upgrade start -> app back {3}; daemon at the target version {4} after the page watch began' -f $tv5[0], $tv5[1], $tv5[2], $tv5[3], $tv5[4]))
    } catch { $lines.Add('line 12 (times) could not be built: ' + $_.Exception.Message) }
    try {
        $tl = @(Get-TeamSummaryLines)
        $lines.Add([string]$tl[0])
    } catch { $lines.Add('line 13 (TEAM) could not be built: ' + $_.Exception.Message) }
    try {
        $vd = $up['verdict']
        $vText = 'VERDICT: not decided'
        if ($vd -is [System.Collections.IDictionary]) {
            $vText = ('VERDICT: {0} (upgrade {1}, team {2})' -f $vd['result'], $vd['upgrade'], $vd['team'])
            foreach ($x in @($vd['failures'])) { if ($x) { $vText = $vText + ' | FAIL: ' + [string]$x } }
            foreach ($x in @($vd['not_measurable'])) { if ($x) { $vText = $vText + ' | NOT MEASURED: ' + [string]$x } }
            foreach ($x in @($vd['notes'])) { if ($x) { $vText = $vText + ' | NOTE: ' + [string]$x } }
        }
        $lines.Add($vText)
    } catch { $lines.Add('line 14 (verdict) could not be built: ' + $_.Exception.Message) }
    $out = New-Object System.Collections.Generic.List[string]
    foreach ($l in $lines.ToArray()) {
        $one = [regex]::Replace([string]$l, '[\r\n]+', ' ')
        $out.Add((Limit-Text $one 1500))
    }
    Save-Text 'appe2e-upgrade-summary.txt' (($out.ToArray()) -join "`r`n")
    foreach ($l in $out.ToArray()) { Write-Log ('SUMMARY ' + $l) }
}

function Get-UpgSeconds {
    param([string]$FromIso, [string]$ToIso)
    try {
        if ((-not $FromIso) -or (-not $ToIso)) { return $null }
        $a = [datetime]::Parse($FromIso, [System.Globalization.CultureInfo]::InvariantCulture, [System.Globalization.DateTimeStyles]::RoundtripKind)
        $b = [datetime]::Parse($ToIso, [System.Globalization.CultureInfo]::InvariantCulture, [System.Globalization.DateTimeStyles]::RoundtripKind)
        return [math]::Round(($b.ToUniversalTime() - $a.ToUniversalTime()).TotalSeconds, 1)
    } catch { return $null }
}

# =========================================================================
# scene UPGRADE: main (app-e2e.ps1 calls this INSTEAD of its own main when APPE2E_SCENE=upgrade; one PowerShell process)
# Every step is an Invoke-Step (a failing step never stops the next ones) and saves app-e2e.json as it goes.
# =========================================================================
function Invoke-UpgradeMain {
    $CTX['script_start_utc'] = (Get-Date).ToUniversalTime()
    $RUN['scene'] = 'upgrade'
    $RUN['upgrade'] = [ordered]@{
        scene = 'upgrade'; params = $null; mode = $null; base_version = $null; to_version = $null
        failures = (New-Object System.Collections.Generic.List[string])
        not_measurable = (New-Object System.Collections.Generic.List[string])
        notes = (New-Object System.Collections.Generic.List[string])
        base_installer = $null; base_reset = $null; base_install = $null; base_start = $null; base_pre = $null; base_state = $null
        apply = $null; apply_observe = $null; after = $null; daemon = $null; app_evidence = $null
        leftovers = [ordered]@{}; pack = [ordered]@{}; install_facts = [ordered]@{}; timings = [ordered]@{}
        proc_audit = $null; stamps = [ordered]@{ points = [ordered]@{}; summary = $null }; second_start = $null
        verdict = $null
    }
    $CTX['upg_audit_ok'] = $false
    $CTX['upg_poll'] = $null
    foreach ($k in @('upg_params_ok', 'upg_base_installer_ok', 'upg_base_install_ok', 'upg_base_start_ok', 'upg_apply_started', 'upg_apply_ok', 'upg_marker_reached')) { $CTX[$k] = $false }
    $CTX['upg_base_version'] = ''
    $CTX['upg_to_version'] = ''
    $CTX['upg_mode'] = ''
    $CTX['upg_daemon_pids_before'] = @()
    Write-Log 'scene UPGRADE (APPE2E_SCENE=upgrade): diag/app-upgrade.ps1'
    try {
        # ---------------------------------------------------------------- 1. prepare: native helper, SAC must be OFF, inputs
        Invoke-Step 'prepare' {
            $up = $RUN['upgrade']
            $prep = [ordered]@{}
            $RUN['prepare'] = $prep
            Initialize-DiagNative
            $prep['native_ready'] = $global:DiagNativeReady
            $prep['native_error'] = $global:DiagNativeError
            Initialize-Screenshot
            $prep['language_mode'] = Get-LangMode
            try {
                $nt = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -ErrorAction Stop
                $prep['os_product'] = ('{0} {1} build {2}.{3}' -f $nt.ProductName, $nt.DisplayVersion, $nt.CurrentBuild, $nt.UBR)
            } catch { }
            $prep['node_exe'] = Find-Exe 'node.exe'
            # this scene never switches Smart App Control: it has to be OFF already (else what is measured is not the plain upgrade)
            $init = Get-SacRealState 'initial'
            $RUN['sac_initially_enforced'] = $init['enforced']
            # the installer under test (windows-build artifact or release asset; updater mode needs none) and the upgrade parameters
            Get-AppInputs
            $in = $RUN['inputs']
            $par = Get-UpgParams
            $up['params'] = $par
            $mode = [string]$par['mode']
            $from = [string]$par['from']
            $to = [string]$par['expect']
            foreach ($pp in $par['problems'].ToArray()) { $up['not_measurable'].Add([string]$pp) }
            if ($init['enforced']) { $up['not_measurable'].Add('Smart App Control is enforced at the start of the run: the upgrade scene needs it OFF and never switches it') }
            if (-not $prep['node_exe']) { $up['not_measurable'].Add('node.exe was not found: the app cannot be observed') }
            if ($mode -eq 'emulate') {
                if (-not $in['installer_ok']) {
                    $up['not_measurable'].Add('emulate mode needs the installer under test and there is none: ' + (@($in['problems'].ToArray()) -join '; '))
                } else {
                    $iv = [string]$CTX['from_version']
                    if (-not $to) {
                        $to = $iv
                    } elseif ($to -ne $iv) {
                        $up['not_measurable'].Add(('upgrade_expect {0} is not the version of the installer under test ({1})' -f $to, $iv))
                    }
                }
            } else {
                # updater mode: the old app downloads the installer itself. The "version under test" that the TEAM scene compares the
                # marker with is the expected one.
                $CTX['from_version'] = $to
                $in['note'] = 'updater mode: no installer under test is needed (the old app downloads from its real endpoint)'
            }
            if ($to -and ($to -eq $from)) { $up['notes'].Add(('the older version and the target version are the same ({0}): this is a re-install on top, not an upgrade' -f $to)) }
            $up['mode'] = $mode
            $up['base_version'] = $from
            $up['to_version'] = $to
            $CTX['upg_mode'] = $mode
            $CTX['upg_base_version'] = $from
            $CTX['upg_to_version'] = $to
            # no manifest override in this scene: Start-CysApp then starts the app without CYS_UPDATE_MANIFEST_URL (its real endpoint)
            $CTX['manifest_url'] = ''
            $CTX['inputs_ok'] = [bool]($up['not_measurable'].Count -eq 0)
            $CTX['upg_params_ok'] = [bool]$CTX['inputs_ok']
            Write-Log ('upgrade parameters: mode={0} from={1} to={2} usable={3}' -f $mode, $from, $to, $CTX['upg_params_ok'])
            Save-Run
        }

        # ---------------------------------------------------------------- UPG-2: process auditing on (a record; a failure only means the fallback)
        if ($CTX['upg_params_ok']) {
            Invoke-Step 'upg-audit-on' {
                $pa = [ordered]@{ setup = $null; source = $null; note = $null; first = $null; table = $null; poll = $null; end = $null; restore = $null }
                $RUN['upgrade']['proc_audit'] = $pa
                $pa['setup'] = Enable-UpgProcAudit
                $CTX['upg_audit_ok'] = [bool]($pa['setup']['works'] -and $pa['setup']['cmdline_works'])
                if (-not $CTX['upg_audit_ok']) { $pa['note'] = ('Security auditing of process creation is not usable here (events seen={0}, command line={1}{2}): the fallback poll is used, it has no exit status' -f $pa['setup']['works'], $pa['setup']['cmdline_works'], $(if ($pa['setup']['error']) { ', ' + [string]$pa['setup']['error'] } else { '' })) }
                Save-Run
            }
        }

        # ---------------------------------------------------------------- 2. the public installer of the older version
        if ($CTX['upg_params_ok']) {
            Invoke-Step 'upg-base-installer' {
                $up = $RUN['upgrade']
                $b = Get-UpgBaseInstaller ([string]$CTX['upg_base_version'])
                $up['base_installer'] = $b
                $CTX['upg_base_installer_ok'] = [bool]$b['usable']
                if (-not $b['usable']) { $up['not_measurable'].Add(('the public {0} installer could not be obtained: {1}' -f $CTX['upg_base_version'], [string]$b['error'])) }
                Save-Run
            }
        }

        # ---------------------------------------------------------------- 3. install it (/S) + install facts
        if ($CTX['upg_base_installer_ok']) {
            if ((Get-MinutesLeft) -gt 18) {
                Invoke-Step 'upg-install-base' {
                    $up = $RUN['upgrade']
                    $from = [string]$CTX['upg_base_version']
                    $up['base_reset'] = Reset-AppState 'upg-before-base'
                    $sw = [System.Diagnostics.Stopwatch]::StartNew()
                    $ins = Install-CysFile -Installer ([string]$up['base_installer']['path']) -ExpectVersion $from
                    $up['timings']['base_install_sec'] = [math]::Round($sw.Elapsed.TotalSeconds, 1)
                    $up['base_install'] = $ins
                    $ok = [bool]($ins['ok'] -and (-not $ins['failure_file']))
                    $CTX['upg_base_install_ok'] = $ok
                    if (-not $ok) { $up['not_measurable'].Add(('the public {0} installer did not install: rc={1} marker={2} failure file={3} error={4}' -f $from, $ins['rc'], $ins['marker'], [bool]$ins['failure_file'], [string]$ins['error'])) }
                    Save-Run
                }
            } else {
                $RUN['upgrade']['not_measurable'].Add('not enough job time left to install the older version')
            }
        }
        if ($CTX['upg_base_install_ok']) {
            Invoke-Step 'upg-facts-base' {
                $ff = Get-InstallFacts 'upgrade-older-version-installed'
                $RUN['upgrade']['install_facts']['base'] = $ff
                Save-Json 'appe2e-install-facts-base.json' $ff 7
                Save-Run
            }

            # ------------------------------------------------------------ 4. start the older app (real endpoint), attach, version
            Invoke-Step 'upg-start-base' {
                $up = $RUN['upgrade']
                $from = [string]$CTX['upg_base_version']
                $CTX['manifest_url'] = ''
                $st = Start-CysApp ($K_UPG_PFX + '-base')
                $up['base_start'] = $st
                Save-Run
                $node = Find-Exe 'node.exe'
                $pre = [ordered]@{ attached = $false; ui_ready = $false; app_version = $null; via = $null; uipre = $null; version_fallback = $null }
                $up['base_pre'] = $pre
                if ($st['cdp_ready'] -and $node) {
                    # the UI of the older version has the same ready signal the uipre mode waits for (the Update button #btn-update is part
                    # of ui/index.html at tag v0.14.42 too); if that mode still does not get an attach + version, the version-only mode is used
                    $u1 = Invoke-NodePre -Prefix ($K_UPG_PFX + '-base-pre') -NodeExe $node
                    $pre['uipre'] = $u1
                    $pre['attached'] = [bool]$u1['attached']
                    $pre['ui_ready'] = [bool]$u1['ui_ready']
                    $pre['app_version'] = $u1['app_version']
                    $pre['via'] = 'uipre'
                    if (-not ($pre['attached'] -and $pre['app_version'])) {
                        $v1 = Invoke-UpgNode -Prefix ($K_UPG_PFX + '-base') -NodeExe $node -Mode 'version' -TimeoutSec 240
                        $vj = Read-UpgJson ($K_UPG_PFX + '-base-cdp-after.json')
                        $fb = [ordered]@{ rc = $v1['rc']; attached = $false; app_version = $null }
                        if ($null -ne $vj) {
                            $fb['attached'] = [bool]$vj.attached
                            $fb['app_version'] = $vj.app_version
                        }
                        $pre['version_fallback'] = $fb
                        if ($fb['attached']) {
                            $pre['attached'] = $true
                            $pre['app_version'] = $fb['app_version']
                            $pre['via'] = 'version'
                        }
                    }
                }
                $ok = [bool]($st['cdp_ready'] -and $pre['attached'] -and ([string]$pre['app_version'] -eq $from))
                $CTX['upg_base_start_ok'] = $ok
                if (-not $ok) {
                    $up['not_measurable'].Add(('the older app {0} could not be started and read: started={1} CDP ready={2} attached={3} app version seen="{4}" ({5})' -f $from, $st['ok'], $st['cdp_ready'], $pre['attached'], [string]$pre['app_version'], [string]$st['error']))
                } elseif (-not $pre['ui_ready']) {
                    $up['notes'].Add('the UI ready signal (#btn-update) was not seen on the older app; attach and version were read')
                }
                Save-Run
            }
        }

        if ($CTX['upg_base_start_ok']) {
            # ------------------------------------------------------------ 5. the state before the upgrade (a record; not ready = NOTE)
            Invoke-Step 'upg-base-state' {
                $up = $RUN['upgrade']
                $bs = Get-UpgBaseState (Find-Exe 'node.exe')
                $up['base_state'] = $bs
                $up['pack']['before'] = $bs['pack']
                try { if (Get-Command -Name 'Get-W44BridgeRows' -CommandType Function -ErrorAction SilentlyContinue) { $up['w44_bridge_base'] = Get-W44BridgeRows 'base-state-before-upgrade' } } catch { }
                try { if (Get-Command -Name 'Invoke-W44DoctorProbe' -CommandType Function -ErrorAction SilentlyContinue) { $up['w44_doctor_base'] = Invoke-W44DoctorProbe 'base-0.14.43' 300 } } catch { }
                if (-not $bs['ready']) { $up['notes'].Add(('the older app was not fully up before the upgrade ({0}); the upgrade was applied anyway' -f [string]$bs['end_reason'])) }
                elseif (-not $bs['settled']) { $up['notes'].Add(('the upgrade was applied while the older app was still in its first-run work: {0}' -f (Limit-Text ((@($bs['settle_missing']) -join '; ')) 400))) }
                Save-Run
            }

            # ------------------------------------------------------------ 6. apply the upgrade and watch it
            Invoke-Step 'upg-apply' {
                $up = $RUN['upgrade']
                $mode = [string]$CTX['upg_mode']
                $from = [string]$CTX['upg_base_version']
                $to = [string]$CTX['upg_to_version']
                $ap = [ordered]@{ mode = $mode; started = (Get-IsoNow); finished = $null; skipped = $null; why = $null; app_pid = $null; daemons_before = @(); emulate = $null; updater = $null; ok = $false }
                $up['apply'] = $ap
                if ((Get-MinutesLeft) -lt 10) {
                    $ap['skipped'] = 'not enough job time left'
                    $up['not_measurable'].Add('not enough job time left to apply the upgrade')
                    return
                }
                $appPid = 0
                try { $appPid = [int]$up['base_start']['pid'] } catch { $appPid = 0 }
                $ap['app_pid'] = $appPid
                $tab = Get-UpgProcs
                $dmBefore = @(Get-UpgDaemonRows $tab.rows)
                $ap['daemons_before'] = $dmBefore
                $ids = New-Object System.Collections.Generic.List[int]
                foreach ($d0 in $dmBefore) {
                    if ($null -ne $d0) { $ids.Add([int]$d0['id']) }
                }
                $CTX['upg_daemon_pids_before'] = $ids.ToArray()
                $null = Save-Screenshot ($K_UPG_PFX + '-screen-before-apply.png')
                try { if (Get-Command -Name 'Get-W44BridgeRows' -CommandType Function -ErrorAction SilentlyContinue) { $up['w44_bridge_before_apply'] = Get-W44BridgeRows 'before-apply' } } catch { }
                $up['stamps']['points']['before_apply'] = Get-UpgStamps 'before-apply'
                $nodeProc = $null
                $go = $false
                $applyStart = Get-Date
                $up['timings']['apply_started_at'] = ConvertTo-IsoUtc $applyStart
                if ($mode -eq 'emulate') {
                    $em = Start-UpgEmulate -Installer ([string]$CTX['installer']) -ToVersion $to -AppPid $appPid -ExpectSha256 ([string](Get-UpgVal $RUN['inputs'] 'installer' 'sha256'))
                    $ap['emulate'] = $em
                    $go = [bool]$em['launch_ok']
                    if (-not $go) {
                        $ap['why'] = 'the installer under test could not be started by the harness: ' + [string]$em['error']
                        $up['not_measurable'].Add([string]$ap['why'])
                        # what the desktop shows at that moment (a dialog of Windows?) - a record
                        try { $ap['launch_failed_windows'] = Get-VisibleWindowsText } catch { }
                        try { $null = Save-Screenshot ($K_UPG_PFX + '-screen-launch-failed.png') } catch { }
                    }
                } else {
                    $node = Find-Exe 'node.exe'
                    $ud = [ordered]@{ check = $null; offered = $null; expected = $to; gate_ok = $false; node_pid = $null; cdp = $null }
                    $ap['updater'] = $ud
                    # the gate: what does the old app's own check_update - against its REAL endpoint - offer? (--mode attach stops before install_update)
                    $c1 = Invoke-UpgNode -Prefix ($K_UPG_PFX + '-check') -NodeExe $node -Mode 'attach' -TimeoutSec 320
                    $cj = Read-UpgJson ($K_UPG_PFX + '-check-cdp.json')
                    $offered = ''
                    $cerr = ''
                    $cAttached = $false
                    $cValue = $null
                    if ($null -ne $cj) {
                        $cAttached = [bool]$cj.attached
                        $cValue = $cj.check_update
                        $cerr = [string]$cj.check_update_error
                        if ($null -ne $cValue) { $offered = [string]$cValue.version }
                    }
                    $ud['check'] = [ordered]@{ rc = $c1['rc']; attached = $cAttached; check_update = $cValue; check_update_error = $cerr }
                    $ud['offered'] = $offered
                    if (-not $cAttached) {
                        $ap['why'] = 'the CDP driver could not attach to the older app to read check_update'
                        $up['not_measurable'].Add([string]$ap['why'])
                    } elseif ($offered -ne $to) {
                        $ap['why'] = ('the public endpoint does not offer {0}: check_update of the {1} app answers version "{2}"{3}' -f $to, $from, $offered, $(if ($cerr) { ' (error: ' + $cerr + ')' } else { '' }))
                        $up['failures'].Add([string]$ap['why'])
                    } else {
                        $ud['gate_ok'] = $true
                        $applyStart = Get-Date
                        $up['timings']['apply_started_at'] = ConvertTo-IsoUtc $applyStart
                        $nodeProc = Start-UpgNode -Prefix ($K_UPG_PFX + '-apply') -NodeExe $node -Mode 'update' -Extra '--max-wait-sec 300'
                        $ud['node_pid'] = $nodeProc.Id
                        $go = $true
                    }
                }
                Save-Run
                if ($go) {
                    $CTX['upg_apply_started'] = $true
                    $obs = Watch-UpgApply -Mode $mode -BasePid $appPid -FromVersion $from -ToVersion $to -NodeProc $nodeProc -MaxSec $K_UPG_APPLY_MAX_SEC -ApplyStart $applyStart
                    if ($null -ne $nodeProc) {
                        try {
                            if (-not $nodeProc.HasExited) { Stop-ProcessTree -ProcessId $nodeProc.Id }
                        } catch { }
                    }
                    # UPG-2: the stamp files the moment the relaunch is known; and, without a working audit, the fallback poll right now
                    $up['stamps']['points']['after_relaunch'] = Get-UpgStamps 'after-relaunch'
                    if ($obs['success'] -and (-not $CTX['upg_audit_ok'])) {
                        try { $CTX['upg_poll'] = Start-UpgProcPoll } catch { $CTX['upg_poll'] = $null; Add-DiagError 'Start-UpgProcPoll' $_ }
                    }
                    $up['apply_observe'] = $obs
                    $ap['ok'] = [bool]$obs['success']
                    $CTX['upg_apply_ok'] = [bool]$obs['success']
                    $CTX['upg_marker_reached'] = [bool]([string](Get-InstalledMarker) -eq $to)
                    $up['timings']['apply_to_marker_sec'] = Get-UpgSeconds ([string]$up['timings']['apply_started_at']) ([string]$obs['marker_reached_at'])
                    $up['timings']['apply_to_relaunch_sec'] = Get-UpgSeconds ([string]$up['timings']['apply_started_at']) ([string]$obs['new_app_first_seen_at'])
                    $up['timings']['marker_to_relaunch_sec'] = Get-UpgSeconds ([string]$obs['marker_reached_at']) ([string]$obs['new_app_first_seen_at'])
                    if ($mode -ne 'emulate') {
                        $aj = Read-UpgJson ($K_UPG_PFX + '-apply-cdp.json')
                        if ($null -ne $aj) { $ap['updater']['cdp'] = [ordered]@{ attached = [bool]$aj.attached; check_update = $aj.check_update; install_update_outcome = [string]$aj.install_update_outcome; socket_closed_at = [string]$aj.socket_closed_at; progress_count = $aj.events.progress_count } }
                    }
                    if (-not $obs['success']) {
                        $why = ('the upgrade from {0} to {1} did not complete: {2}' -f $from, $to, [string]$obs['end_reason'])
                        $ap['why'] = $why
                        # a watch that the harness had to give up (job time, an exception in the loop) says nothing about the product
                        if ([string]$obs['end_kind'] -eq 'harness') { $up['not_measurable'].Add($why) } else { $up['failures'].Add($why) }
                    }
                }
                $ap['finished'] = (Get-IsoNow)
                Save-Run
            }
        }

        if ($CTX['upg_apply_started']) {
            # ------------------------------------------------------------ 7. after: disk (a), the app that came back (b), the daemon (c), leftovers (d), pack (e)
            Invoke-Step 'upg-after' {
                $up = $RUN['upgrade']
                $to = [string]$CTX['upg_to_version']
                $af = [ordered]@{ started = (Get-IsoNow); marker = $null; marker_ok = $false; failure_file = $null; failure_file_absent = $false; file_versions = [ordered]@{}; file_versions_ok = $false; a_done = $false; a_ok = $false; app_alive = $null; cdp_wait_sec = $null; pre = $null; app_version = $null; app_version_ok = $false; ui_ready = $false; b_ok = $false }
                $up['after'] = $af
                try { if (Get-Command -Name 'Get-W44BridgeRows' -CommandType Function -ErrorAction SilentlyContinue) { $up['w44_bridge_right_after'] = Get-W44BridgeRows 'upg-after-start' } } catch { }
                # (d) leftovers, right after the installer
                $up['leftovers']['right_after'] = Get-UpgLeftovers 'right-after'
                # (a) what is on disk
                $af['marker'] = Get-InstalledMarker
                $af['marker_ok'] = [bool]([string]$af['marker'] -eq $to)
                $ft = Get-FailureMarker
                if ($ft) { $af['failure_file'] = Limit-Text $ft 1500 }
                $af['failure_file_absent'] = [bool](-not $ft)
                $allSame = $true
                foreach ($n in @('cys-app.exe', 'cys.exe', 'cysd.exe')) {
                    $v3 = Get-UpgFileVersion3 (Join-Path (Get-InstallDir) $n)
                    $af['file_versions'][$n] = $v3
                    if ($v3 -ne $to) { $allSame = $false }
                }
                $af['file_versions_ok'] = [bool]$allSame
                $af['a_ok'] = [bool]($af['marker_ok'] -and $af['failure_file_absent'] -and $af['file_versions_ok'])
                $af['a_done'] = $true
                Save-Run
                # (b) the app the installer started again: version and UI, read over CDP (nothing is clicked). The process is there
                # before its WebView2 answers on the debugging port: wait for the port (as Start-CysApp does for an app it starts itself)
                $alive = Test-TeamAppAlive
                $tPort = Get-Date
                while (-not $alive['cdp']) {
                    $waited = ((Get-Date) - $tPort).TotalSeconds
                    if ($waited -ge 120) { break }
                    if ((-not $alive['process']) -and ($waited -ge 20)) { break }
                    Start-Sleep -Seconds 2
                    $alive = Test-TeamAppAlive
                }
                $af['app_alive'] = $alive
                $af['cdp_wait_sec'] = [int]((Get-Date) - $tPort).TotalSeconds
                $node = Find-Exe 'node.exe'
                if ($alive['process'] -and $alive['cdp'] -and $node) {
                    $pre = Invoke-NodePre -Prefix ($K_UPG_PFX + '-after-pre') -NodeExe $node
                    $af['pre'] = $pre
                    $af['app_version'] = $pre['app_version']
                    $af['app_version_ok'] = [bool]([string]$pre['app_version'] -eq $to)
                    $af['ui_ready'] = [bool]$pre['ui_ready']
                    $af['b_ok'] = [bool]($pre['attached'] -and $af['app_version_ok'] -and $af['ui_ready'])
                    Save-Run
                    # (c) the new app meets the old daemon: node watches the page, this side the processes
                    if ((Get-TeamMinutesLeft) -gt ($K_TEAM_NEED_MIN + 2)) {
                        # an observation: a problem in it is a NOTE and never takes (a) / (b) or the TEAM scene with it
                        try {
                            $np = Start-UpgNode -Prefix ($K_UPG_PFX + '-after') -NodeExe $node -Mode 'uiobs' -Extra ('--max-wait-sec 120 --ready-wait-sec 30 --obs-sec {0} --obs-interval-sec {1} --obs-min-sec {2} --obs-expect-daemon {3}' -f $K_UPG_OBS_SEC, $K_UPG_OBS_PAGE_TICK_SEC, $K_UPG_OBS_MIN_SEC, $to)
                            $w = Watch-UpgDaemons -NodeProc $np -MaxSec ($K_UPG_OBS_SEC + 90)
                            try {
                                if (-not $np.HasExited) { Stop-ProcessTree -ProcessId $np.Id }
                            } catch { }
                            $of = Read-UpgJson ($K_UPG_PFX + '-after-obs-facts.json')
                            $dn = Get-UpgDaemonResult -Watch $w -ObsFile $of -ToVersion $to -BeforePids $CTX['upg_daemon_pids_before']
                            $dn['watch'] = [ordered]@{ started = $w['started']; ended = $w['ended']; end_reason = $w['end_reason']; ticks = $w['ticks']; node_exit_code = $w['node_exit_code']; first = $w['first']; last = $w['last'] }
                            $up['stamps']['watch_changes'] = $w['stamp_changes'].ToArray()
                            $up['daemon'] = $dn
                            $up['timings']['daemon_target_sec'] = $dn['daemon_reached_target_sec']
                            Save-Json ($K_UPG_PFX + '-daemon-watch.json') $w 7
                        } catch {
                            Add-DiagError 'upg-after daemon observation' $_
                            $up['notes'].Add('the daemon observation stopped on an error: ' + $_.Exception.Message)
                        }
                    } else {
                        $up['notes'].Add('the daemon observation was skipped: the job time left is kept for the TEAM scene')
                    }
                }
                # (d) leftovers at the end of the observation, (e) the pack after the upgrade, the desktop, the app's files
                $up['stamps']['points']['after_observation'] = Get-UpgStamps 'after-observation'
                $up['leftovers']['after_observation'] = Get-UpgLeftovers 'after-observation'
                try { if (Get-Command -Name 'Invoke-W44DoctorProbe' -CommandType Function -ErrorAction SilentlyContinue) { $up['w44_doctor_after'] = Invoke-W44DoctorProbe 'after-upgrade-0.14.44' 300 } } catch { }
                try { if (Get-Command -Name 'Get-W44BridgeRows' -CommandType Function -ErrorAction SilentlyContinue) { $up['w44_bridge_after_observation'] = Get-W44BridgeRows 'after-observation'; $up['w44_log_lines_after_observation'] = Get-W44LogLines 'office-bridge|bridge' 60 } } catch { }
                $up['pack']['after'] = Get-UpgPackState
                try { $null = Save-Screenshot ($K_UPG_PFX + '-screen-after.png') } catch { }
                try { $up['app_evidence'] = Save-AppEvidence $K_UPG_PFX } catch { }
                try { Save-AppEventLog $K_UPG_PFX $CTX['script_start_utc'] } catch { }
                $af['finished'] = (Get-IsoNow)
                Save-Run
            }
            Invoke-Step 'upg-facts-after' {
                $ff = Get-InstallFacts 'upgrade-after'
                $RUN['upgrade']['install_facts']['after'] = $ff
                Save-Json 'appe2e-install-facts-after.json' $ff 7
                Save-Run
            }
            # UPG-2: which processes the upgrade and the relaunched app started (Security 4688 / 4689 from 30 s before the upgrade
            # began), or the fallback poll. A record: the init-pack line of the summary comes from here.
            Invoke-Step 'upg-proc-audit' {
                $up = $RUN['upgrade']
                $pa = $up['proc_audit']
                if (-not ($pa -is [System.Collections.IDictionary])) { $pa = [ordered]@{ setup = $null; source = $null; note = 'the audit was not set up'; first = $null; table = $null; poll = $null; end = $null; restore = $null }; $up['proc_audit'] = $pa }
                $relaunch = [string](Get-UpgVal $up 'apply_observe' 'new_app_first_seen_at')
                $since = (Get-Date).AddMinutes(-10)
                try { if ($up['timings']['apply_started_at']) { $since = ([datetime]::Parse([string]$up['timings']['apply_started_at'], [System.Globalization.CultureInfo]::InvariantCulture, [System.Globalization.DateTimeStyles]::RoundtripKind)).ToLocalTime().AddSeconds(-30) } } catch { }
                $CTX['upg_audit_since'] = $since
                $procs = @()
                if ($CTX['upg_audit_ok']) {
                    $first = Save-UpgProcAudit -Since $since -Name ($K_UPG_PFX + '-proc-audit')
                    $procs = @($first['processes'])
                    $pa['first'] = [ordered]@{ time = $first['time']; since = $first['since']; ok = $first['ok']; error = $first['error']; events_read = $first['events_read']; events_kept = $first['events_kept']; processes = $procs.Count }
                    $pa['source'] = 'audit'
                    if (-not $first['ok']) { $pa['note'] = 'the Security log could not be read: ' + [string]$first['error'] }
                } elseif ($null -ne $CTX['upg_poll']) {
                    # the poll runs for a fixed time: wait for its end (it is short), then read its file
                    $pp = $CTX['upg_poll']['process']
                    $tw = Get-Date
                    while (((Get-Date) - $tw).TotalSeconds -lt ($K_UPG_POLL_SEC + 20)) {
                        $gone = $true
                        try { $gone = [bool]$pp.HasExited } catch { $gone = $true }
                        if ($gone) { break }
                        Start-Sleep -Seconds 1
                    }
                    $rp = Read-UpgProcPoll
                    $procs = @($rp['processes'])
                    $pa['poll'] = [ordered]@{ started = $CTX['upg_poll']['started']; found = $rp['found']; samples = $rp['samples']; rows = $rp['rows']; error = $rp['error'] }
                    $pa['source'] = 'poll'
                } else {
                    $pa['source'] = 'none'
                    if (-not $pa['note']) { $pa['note'] = 'neither the audit nor the fallback poll ran' }
                }
                if ($pa['source'] -ne 'none') {
                    $src = $(if ($pa['source'] -eq 'audit') { 'Security 4688/4689' } else { 'fallback poll every ' + [string]$K_UPG_POLL_MS + ' ms for ' + [string]$K_UPG_POLL_SEC + ' s, no exit status' })
                    $pa['table'] = Get-UpgInitPackTable -Procs $procs -RelaunchIso $relaunch -BeforeDaemonPids $CTX['upg_daemon_pids_before'] -Source $src -NowIso (Get-IsoNow)
                }
                Save-Run
            }
        }

        # ---------------------------------------------------------------- 8. TEAM on the upgraded app, as it is (Invoke-TeamScene reuses a
        # live app with an answering CDP port whose marker equals $CTX['from_version'] = the target version; else its record says so)
        if ($CTX['upg_marker_reached']) {
            Invoke-Step 'team' { Invoke-TeamScene }
        } else {
            Invoke-Step 'team' { Save-TeamNotRun 'the upgrade did not put the target version on disk: the TEAM scene on the upgraded app was not run' }
        }
    } catch {
        Add-DiagError 'app-upgrade main' $_
    } finally {
        # ---------------------------------------------------------------- UPG-3: the second start of the upgraded app (a record)
        # after the TEAM scene, before the verdict; whatever happens in it stays inside it (Invoke-Step + its own try / catch)
        Invoke-Step 'upg-second-start' {
            $s2 = Invoke-UpgSecondStart
            try { $s2['line'] = Format-UpgSecondStart $s2 } catch { }
            Save-Run
        }
        # ---------------------------------------------------------------- 9. verdict  10. summary  11. cleanup
        Invoke-Step 'upg-verdict' {
            $up = $RUN['upgrade']
            # what is left on disk at the end (the TEAM scene's own cleanup stopped every process of the install folder; the second
            # start of UPG-3 then started the app once more - no installer runs in it)
            $up['leftovers']['at_end'] = Get-UpgLeftovers 'at-end'
            # UPG-2: the stamp files at the end, their one line, and the audit once more over the whole scene (a second pair of files)
            try {
                $sx = $up['stamps']
                $sx['points']['at_end'] = Get-UpgStamps 'at-end'
                $b0 = $sx['points']['before_apply']
                $e0 = $sx['points']['at_end']
                $to0 = [string]$CTX['upg_to_version']
                $relaunch0 = [string](Get-UpgVal $up 'apply_observe' 'new_app_first_seen_at')
                $seen = 'the stamp never showed ' + $to0
                $all0 = New-Object System.Collections.Generic.List[object]
                foreach ($k0 in @('after_relaunch')) { if ($sx['points'][$k0] -is [System.Collections.IDictionary]) { $all0.Add($sx['points'][$k0]) } }
                foreach ($c0 in @($sx['watch_changes'])) { if ($c0 -is [System.Collections.IDictionary]) { $all0.Add($c0) } }
                foreach ($k0 in @('after_observation', 'at_end')) { if ($sx['points'][$k0] -is [System.Collections.IDictionary]) { $all0.Add($sx['points'][$k0]) } }
                foreach ($s0 in $all0.ToArray()) {
                    if ($to0 -and ([string]$s0['last_app_version'] -eq $to0)) {
                        $seen = ('first seen at {0} in sample "{1}", {2} s after the new app was seen' -f $s0['time'], $s0['tag'], (Format-UpgVal (Get-UpgSeconds $relaunch0 ([string]$s0['time'])) '?'))
                        break
                    }
                }
                $sx['summary'] = ('stamp (.last-app-version) {0} -> {1} ({2}); .pending-restore before={3} right after the relaunch={4} at the end={5}; pack version {6} -> {7}; pack.prev at the end={8}; staging leftovers at the end=[{9}]; .gui-onboarded before={10} right after the relaunch={11} after the page watch={12} at the end={13}; onboard-attempts at the end={14}' -f (Format-UpgVal (Get-UpgVal $b0 'last_app_version') 'none'), (Format-UpgVal (Get-UpgVal $e0 'last_app_version') 'none'), $seen, (Format-UpgVal (Get-UpgVal $b0 'pending_restore')), (Format-UpgVal (Get-UpgVal $sx['points'] 'after_relaunch' 'pending_restore')), (Format-UpgVal (Get-UpgVal $e0 'pending_restore')), (Format-UpgVal (Get-UpgVal $b0 'pack_version') 'none'), (Format-UpgVal (Get-UpgVal $e0 'pack_version') 'none'), (Format-UpgVal (Get-UpgVal $e0 'pack_prev')), (@(Get-UpgVal $e0 'staging') -join ','), (Format-UpgVal (Get-UpgVal $b0 'gui_onboarded') 'none'), (Format-UpgVal (Get-UpgVal $sx['points'] 'after_relaunch' 'gui_onboarded') 'none'), (Format-UpgVal (Get-UpgVal $sx['points'] 'after_observation' 'gui_onboarded') 'none'), (Format-UpgVal (Get-UpgVal $e0 'gui_onboarded') 'none'), (Format-UpgVal (Get-UpgVal $e0 'gui_onboard_attempts') 'none'))
            } catch { Add-DiagError 'upg stamps summary' $_ }
            try {
                $pa = $up['proc_audit']
                if (($pa -is [System.Collections.IDictionary]) -and $CTX['upg_audit_ok'] -and ($null -ne $CTX['upg_audit_since'])) {
                    $e1 = Save-UpgProcAudit -Since $CTX['upg_audit_since'] -Name ($K_UPG_PFX + '-proc-audit-end')
                    $pa['end'] = [ordered]@{ time = $e1['time']; ok = $e1['ok']; error = $e1['error']; events_read = $e1['events_read']; events_kept = $e1['events_kept']; processes = @($e1['processes']).Count }
                }
            } catch { Add-DiagError 'upg proc audit at the end' $_ }
            $v = $null
            try {
                $v = Get-UpgVerdict
            } catch {
                # the verdict files must exist whatever happens: say that the verdict itself could not be computed
                Add-DiagError 'Get-UpgVerdict' $_
                $vw = 'the verdict could not be computed (' + $_.Exception.Message + '): read app-e2e.json'
                $v = [ordered]@{ result = 'NOT-MEASURABLE'; upgrade = 'NOT-MEASURABLE'; team = 'NOT_RUN'; mode = [string]$CTX['upg_mode']; from_version = [string]$CTX['upg_base_version']; to_version = [string]$CTX['upg_to_version']; failures = @(); not_measurable = @($vw); notes = @(); reasons = @('NOT MEASURED: ' + $vw); facts = [ordered]@{} }
            }
            $up['verdict'] = $v
            $RUN['measurable'] = [bool]([string]$v['upgrade'] -ne 'NOT-MEASURABLE')
            Save-Json 'appe2e-upgrade-verdict.json' $v 8
            # the standard files carry the result too: appe2e-verdict.json and the verdict key of app-e2e.json
            $RUN['verdict'] = [ordered]@{ result = $v['result']; upgrade = $v['upgrade']; team = $v['team']; reasons = $v['reasons'] }
            Save-Json 'appe2e-verdict.json' $RUN['verdict'] 5
            Save-Run
        }
        Invoke-Step 'upg-summary' { Write-UpgSummary }
        Invoke-Step 'upg-cleanup' {
            $cl = [ordered]@{ node_killed = @(); reset = $null; webview2_policy_removed = $null }
            $RUN['cleanup'] = $cl
            # node drivers that are still running (never started from inside the install folder)
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
            $cl['reset'] = Reset-AppState 'upg-final'
            try { $cl['webview2_policy_removed'] = Remove-WebView2DebugPolicy $CTX['wv2_policy'] } catch { }
            # UPG-2: the audit settings back to what they were
            try {
                $pa = $RUN['upgrade']['proc_audit']
                if (($pa -is [System.Collections.IDictionary]) -and ($pa['setup'] -is [System.Collections.IDictionary])) {
                    $pa['restore'] = Restore-UpgProcAudit $pa['setup']
                    $cl['proc_audit_restored'] = $true
                }
            } catch { }
        }
        $RUN['finished'] = (Get-IsoNow)
        Save-Run
        Complete-DiagScript 'app-e2e'
    }
}
