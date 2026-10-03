# diag/p1-assets.ps1 -- download release installers, verify SHA256, extract with 7-Zip, build controls,
# and write the subject list (OUT\subjects.json) used by sacrules.ps1 / sac-real.ps1.
# Output: OUT\p1-assets.json, OUT\subjects.json. Big files stay in WORK (never published).
$ErrorActionPreference = 'Continue'
. (Join-Path $PSScriptRoot 'lib.ps1')
Start-DiagScript -Name 'p1-assets'

$repo = 'idoforgod/cys-terminal'
$versions = @('0.14.37', '0.14.41', '0.14.42')
$P1 = [ordered]@{
    started = (Get-IsoNow)
    repo = $repo
    versions = [ordered]@{}
    tools = [ordered]@{}
    controls = [ordered]@{}
    bundled_sig_summary = [ordered]@{}
    subjects_count = 0
    notes = (New-Object System.Collections.Generic.List[string])
}
$subjects = New-Object System.Collections.Generic.List[object]
$subjDir = Join-Path $global:DiagWork 'subj'
$ctlDir = Join-Path $global:DiagWork 'ctl'
foreach ($d in @($subjDir, $ctlDir)) {
    if (-not (Test-Path -LiteralPath $d)) { New-Item -ItemType Directory -Path $d -Force | Out-Null }
}

function Save-P1 {
    Save-Json 'p1-assets.json' $P1 9
}

function Save-Subjects {
    Save-Json 'subjects.json' $subjects.ToArray() 4
}

# Register one subject (hash + signature) and return its record.
function Add-Subject {
    param([string]$Label, [string]$Path, [string]$Kind, [string]$Needle = '', [switch]$NoSig)
    $e = [ordered]@{ label = $Label; path = $Path; sha256 = $null; size = 0; sig = 'missing'; sig_subject = $null; kind = $Kind; needle = $Needle }
    try {
        if (Test-Path -LiteralPath $Path) {
            $e['size'] = (Get-Item -LiteralPath $Path).Length
            $e['sha256'] = Get-Sha256 $Path
            if ($NoSig) {
                $e['sig'] = 'not checked (time budget)'
            } else {
                $s = Get-SigRecord $Path
                $e['sig'] = $s.status
                $e['sig_subject'] = $s.subject
            }
        }
    } catch {
        $e['sig'] = 'error: ' + $_.Exception.Message
    }
    $subjects.Add($e)
    return $e
}

function Copy-Subject {
    param([string]$Source, [string]$UniqueName)
    $dest = Join-Path $subjDir $UniqueName
    try {
        Copy-Item -LiteralPath $Source -Destination $dest -Force -ErrorAction Stop
        return $dest
    } catch {
        Write-Log ('Copy-Subject failed {0}: {1}' -f $UniqueName, $_.Exception.Message) 'WARN'
        return $null
    }
}

# Extract an installer with 7z. Names = optional file filters.
function Expand-With7z {
    param([string]$SevenZip, [string]$Archive, [string]$Dest, [string[]]$Names = @())
    New-Item -ItemType Directory -Path $Dest -Force | Out-Null
    $a = ('x -y -bd "-o{0}" "{1}"' -f $Dest, $Archive)
    foreach ($n in $Names) { $a = $a + ' ' + $n }
    $x = Invoke-Proc -File $SevenZip -Arguments $a -TimeoutSec 900
    $tail = [string]$x.out
    if ($tail.Length -gt 1500) { $tail = $tail.Substring($tail.Length - 1500) }
    $errt = [string]$x.err
    if ($errt.Length -gt 1500) { $errt = $errt.Substring($errt.Length - 1500) }
    return [ordered]@{ rc = $x.rc; ms = $x.ms; timedOut = $x.timedOut; startError = $x.startError; out_tail = $tail; err_tail = $errt }
}

# Find files by name (case-insensitive) anywhere below a folder.
function Find-FilesByName {
    param([string]$Root, [string]$Name)
    try {
        return @(Get-ChildItem -LiteralPath $Root -Recurse -File -Filter $Name -ErrorAction SilentlyContinue)
    } catch { return @() }
}

try {
    # ---- tools ------------------------------------------------------------
    $sevenZip = Find-Exe '7z.exe' @('C:\Program Files\7-Zip\7z.exe', 'C:\Program Files (x86)\7-Zip\7z.exe')
    $P1['tools']['7z'] = $sevenZip
    if (-not $sevenZip) { $P1['notes'].Add('7z.exe not found: extraction impossible, bundled-exe subjects will be missing') }
    $rustc = Find-Exe 'rustc.exe'
    $P1['tools']['rustc'] = $rustc
    Save-P1

    # ---- release installers -----------------------------------------------
    foreach ($ver in $versions) {
        $e = [ordered]@{
            version = $ver
            usable = $false
            installer = $null
            download = $null
            sums_download = $null
            size = 0
            sha256 = $null
            sums_expected = $null
            sums_match = $null
            sums_note = $null
            sig = $null
            extract = $null
        }
        $P1['versions'][$ver] = $e
        Invoke-Safe ('version ' + $ver) {
            $dir = Join-Path $global:DiagWork ('dl\' + $ver)
            $inst = Get-AssetInstaller $ver
            $name = 'cys_{0}_x64-setup.exe' -f $ver
            $base = 'https://github.com/' + $repo + '/releases/download/v' + $ver + '/'
            $e['installer'] = $inst
            $d1 = Invoke-Download ($base + $name) $inst 900
            $e['download'] = $d1
            $sumsPath = Join-Path $dir 'SHA256SUMS.txt'
            $d2 = Invoke-Download ($base + 'SHA256SUMS.txt') $sumsPath 120
            $e['sums_download'] = $d2
            if ($d1.ok) {
                $e['size'] = $d1.size
                $e['sha256'] = Get-Sha256 $inst
                $e['usable'] = $true
                $sumsTxt = Read-TextUtf8 $sumsPath
                if ($d2.ok -and $sumsTxt) {
                    $expected = $null
                    foreach ($line in ($sumsTxt -split "`n")) {
                        $m = [regex]::Match($line.Trim(), '^([0-9a-fA-F]{64})\s+\*?(.+)$')
                        if ($m.Success) {
                            $fn = $m.Groups[2].Value.Trim()
                            if (($fn -eq $name) -or $fn.EndsWith('/' + $name) -or $fn.EndsWith('\' + $name)) { $expected = $m.Groups[1].Value.ToLowerInvariant() }
                        }
                    }
                    $e['sums_expected'] = $expected
                    if ($expected) {
                        $e['sums_match'] = ($expected -eq $e['sha256'])
                        if (-not $e['sums_match']) {
                            $e['usable'] = $false
                            $e['sums_note'] = 'SHA256 MISMATCH vs SHA256SUMS.txt -> marked unusable'
                        }
                    } else {
                        $e['sums_note'] = 'no entry for ' + $name + ' in SHA256SUMS.txt (could not verify; kept usable)'
                    }
                } else {
                    $e['sums_note'] = 'SHA256SUMS.txt not downloaded (could not verify; kept usable)'
                }
                $e['sig'] = Get-SigRecord $inst
            } else {
                $e['sums_note'] = 'installer download failed'
            }
        }
        Save-P1
    }

    # ---- extraction ----------------------------------------------------------
    $rootExes = @('cys-app.exe', 'cys.exe', 'cysd.exe')
    foreach ($ver in $versions) {
        $e = $P1['versions'][$ver]
        if (-not $e['usable']) { continue }
        if (-not $sevenZip) { continue }
        Invoke-Safe ('extract ' + $ver) {
            $xdir = Join-Path $global:DiagWork ('x\' + $ver)
            $ex = [ordered]@{ dir = $xdir; mode = $null; first = $null; fallback = $null; root_exes = [ordered]@{}; exe_count = 0 }
            $e['extract'] = $ex
            if ($ver -eq '0.14.42') {
                $ex['mode'] = 'all'
                $ex['first'] = Expand-With7z $sevenZip $e['installer'] $xdir @()
            } else {
                $ex['mode'] = 'root exes only'
                $ex['first'] = Expand-With7z $sevenZip $e['installer'] $xdir $rootExes
            }
            $missing = New-Object System.Collections.Generic.List[string]
            foreach ($n in $rootExes) {
                if (-not (Test-Path -LiteralPath (Join-Path $xdir $n))) { $missing.Add($n) }
            }
            if ($missing.Count -gt 0 -and $ver -ne '0.14.42') {
                $full = Join-Path $global:DiagWork ('x\' + $ver + '-full')
                $ex['fallback'] = [ordered]@{ reason = ('missing at root: ' + ($missing -join ',')); extract = (Expand-With7z $sevenZip $e['installer'] $full @()); copied = @() }
                foreach ($n in $missing) {
                    $hit = @(Find-FilesByName $full $n) | Select-Object -First 1
                    if ($hit) {
                        Copy-Item -LiteralPath $hit.FullName -Destination (Join-Path $xdir $n) -Force
                        $ex['fallback']['copied'] += ($n + ' <- ' + $hit.FullName.Substring($full.Length))
                    }
                }
                try { Remove-Item -LiteralPath $full -Recurse -Force -ErrorAction SilentlyContinue } catch { }
            }
            foreach ($n in $rootExes) { $ex['root_exes'][$n] = (Test-Path -LiteralPath (Join-Path $xdir $n)) }
            $ex['exe_count'] = @(Get-ChildItem -LiteralPath $xdir -Recurse -File -Filter '*.exe' -ErrorAction SilentlyContinue).Count
        }
        Save-P1
    }

    # ---- controls ------------------------------------------------------------
    # NEG: an exe nobody else has (UUID inside the source) -> reputation unknown by construction.
    Invoke-Safe 'control NEG' {
        $c = [ordered]@{ path = $null; built_with = $null; uuid = $null; sha256 = $null; sig = $null; notes = (New-Object System.Collections.Generic.List[string]) }
        $P1['controls']['neg'] = $c
        $uuid = [guid]::NewGuid().ToString()
        $c['uuid'] = $uuid
        $negExe = Join-Path $ctlDir 'ctl-neg-fresh.exe'
        if ($rustc) {
            $rs = Join-Path $ctlDir 'ctl-neg.rs'
            Write-Utf8NoBom $rs ('fn main() { println!("diag-neg ' + $uuid + '"); }' + "`r`n")
            $b = Invoke-Proc -File $rustc -Arguments ('-O -C target-feature=+crt-static -o "{0}" "{1}"' -f $negExe, $rs) -TimeoutSec 300
            $c['notes'].Add(('rustc rc={0} err={1}' -f $b.rc, ([string]$b.err).Trim()))
            if (Test-Path -LiteralPath $negExe) { $c['built_with'] = 'rustc' }
        }
        if (-not (Test-Path -LiteralPath $negExe)) {
            $csc = Join-Path $env:windir 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
            if (Test-Path -LiteralPath $csc) {
                $cs = Join-Path $ctlDir 'ctl-neg.cs'
                Write-Utf8NoBom $cs ('class P { static void Main() { System.Console.WriteLine("diag-neg ' + $uuid + '"); } }' + "`r`n")
                $b2 = Invoke-Proc -File $csc -Arguments ('/nologo /out:"{0}" "{1}"' -f $negExe, $cs) -TimeoutSec 120
                $c['notes'].Add(('csc rc={0} out={1}' -f $b2.rc, ([string]$b2.out).Trim()))
                if (Test-Path -LiteralPath $negExe) { $c['built_with'] = 'csc (managed exe)' }
            } else {
                $c['notes'].Add('csc.exe not found')
            }
        }
        if (Test-Path -LiteralPath $negExe) {
            $c['path'] = $negExe
            $c['sha256'] = Get-Sha256 $negExe
            $c['sig'] = (Get-SigRecord $negExe).status
        } else {
            $c['notes'].Add('NEG control could not be built')
        }
    }
    Save-P1

    # POS-UNSIGNED: widely used unsigned installer (7-Zip) and the exes inside it.
    Invoke-Safe 'control POS-UNSIGNED' {
        $c = [ordered]@{ setup = $null; download = $null; sig = $null; suitable = $null; extract = $null; z_exe = $null; zfm_exe = $null; notes = (New-Object System.Collections.Generic.List[string]) }
        $P1['controls']['pos_unsigned'] = $c
        $zSetup = Join-Path $ctlDir '7z2501-x64.exe'
        $got = $false
        foreach ($u in @('https://www.7-zip.org/a/7z2501-x64.exe', 'https://github.com/ip7z/7zip/releases/download/25.01/7z2501-x64.exe')) {
            $d = Invoke-Download $u $zSetup 300
            $c['notes'].Add(('download {0} ok={1} rc={2} {3}' -f $u, $d.ok, $d.rc, ([string]$d.err).Trim()))
            if ($d.ok) { $got = $true; $c['download'] = $d; break }
        }
        if ($got) {
            $c['setup'] = $zSetup
            $sg = Get-SigRecord $zSetup
            $c['sig'] = $sg
            if ($sg.status -eq 'Valid') {
                $c['suitable'] = $false
                $c['notes'].Add('7-Zip installer is Authenticode-signed (status Valid): control UNSUITABLE as POS-UNSIGNED')
            } else {
                $c['suitable'] = $true
            }
            if ($sevenZip) {
                $zx = Join-Path $global:DiagWork 'x\7zip-setup'
                $c['extract'] = Expand-With7z $sevenZip $zSetup $zx @('7z.exe', '7zFM.exe')
                foreach ($n in @('7z.exe', '7zFM.exe')) {
                    $p = Join-Path $zx $n
                    if (-not (Test-Path -LiteralPath $p)) {
                        $full = Join-Path $global:DiagWork 'x\7zip-setup-full'
                        if (-not (Test-Path -LiteralPath $full)) { [void](Expand-With7z $sevenZip $zSetup $full @()) }
                        $hit = @(Find-FilesByName $full $n) | Select-Object -First 1
                        if ($hit) { Copy-Item -LiteralPath $hit.FullName -Destination $p -Force }
                    }
                }
                if (Test-Path -LiteralPath (Join-Path $zx '7z.exe')) { $c['z_exe'] = (Join-Path $zx '7z.exe') }
                if (Test-Path -LiteralPath (Join-Path $zx '7zFM.exe')) { $c['zfm_exe'] = (Join-Path $zx '7zFM.exe') }
            }
        } else {
            $c['notes'].Add('7-Zip installer could not be downloaded: POS-UNSIGNED control missing')
        }
    }
    Save-P1

    # POS-SIGNED: node.exe shipped inside 0.14.42.
    Invoke-Safe 'control POS-SIGNED' {
        $c = [ordered]@{ path = $null; sig = $null; notes = (New-Object System.Collections.Generic.List[string]) }
        $P1['controls']['pos_signed'] = $c
        $xdir = Join-Path $global:DiagWork 'x\0.14.42'
        $node = Join-Path $xdir 'runtime\node\node.exe'
        if (-not (Test-Path -LiteralPath $node)) {
            $hit = @(Find-FilesByName $xdir 'node.exe') | Select-Object -First 1
            if ($hit) { $node = $hit.FullName; $c['notes'].Add('node.exe found at non-default location') } else { $node = $null }
        }
        if ($node) {
            $c['path'] = $node
            $c['sig'] = Get-SigRecord $node
        } else {
            $c['notes'].Add('node.exe not found in the 0.14.42 extraction')
        }
    }
    Save-P1

    # ---- subjects ------------------------------------------------------------
    Invoke-Safe 'subjects: installers' {
        foreach ($ver in $versions) {
            $e = $P1['versions'][$ver]
            if (-not $e['usable']) { continue }
            $dest = Copy-Subject $e['installer'] ('inst-{0}.exe' -f $ver)
            if ($dest) { [void](Add-Subject ('installer ' + $ver) $dest 'installer' ('\subj\inst-{0}.exe' -f $ver)) }
        }
    }
    Save-Subjects

    Invoke-Safe 'subjects: app exes' {
        foreach ($ver in $versions) {
            $xdir = Join-Path $global:DiagWork ('x\' + $ver)
            foreach ($n in $rootExes) {
                $src = Join-Path $xdir $n
                if (Test-Path -LiteralPath $src) {
                    $uname = 'app-{0}-{1}' -f $ver, $n
                    $dest = Copy-Subject $src $uname
                    if ($dest) { [void](Add-Subject ('app ' + $ver + ' ' + $n) $dest 'app-main' ('\subj\' + $uname)) }
                }
            }
        }
    }
    Save-Subjects

    Invoke-Safe 'subjects: controls' {
        $neg = $P1['controls']['neg']
        if ($neg -and $neg['path']) {
            $dest = Copy-Subject $neg['path'] 'ctl-neg-fresh.exe'
            if ($dest) { [void](Add-Subject 'control NEG (fresh unsigned exe)' $dest 'control-neg' '\subj\ctl-neg-fresh.exe') }
        }
        $pu = $P1['controls']['pos_unsigned']
        if ($pu -and $pu['setup']) {
            $dest = Copy-Subject $pu['setup'] 'ctl-pos-7zip-setup.exe'
            if ($dest) { [void](Add-Subject 'control POS-UNSIGNED 7-Zip setup' $dest 'control-pos-unsigned' '\subj\ctl-pos-7zip-setup.exe') }
        }
        if ($pu -and $pu['z_exe']) {
            $dest = Copy-Subject $pu['z_exe'] 'ctl-pos-7z.exe'
            if ($dest) { [void](Add-Subject 'control POS-UNSIGNED 7z.exe' $dest 'control-pos-unsigned' '\subj\ctl-pos-7z.exe') }
        }
        if ($pu -and $pu['zfm_exe']) {
            $dest = Copy-Subject $pu['zfm_exe'] 'ctl-pos-7zFM.exe'
            if ($dest) { [void](Add-Subject 'control POS-UNSIGNED 7zFM.exe' $dest 'control-pos-unsigned' '\subj\ctl-pos-7zFM.exe') }
        }
        $ps = $P1['controls']['pos_signed']
        if ($ps -and $ps['path']) {
            $dest = Copy-Subject $ps['path'] 'ctl-signed-node.exe'
            if ($dest) { [void](Add-Subject 'control POS-SIGNED node.exe (0.14.42 runtime)' $dest 'control-signed' '\subj\ctl-signed-node.exe') }
        }
    }
    Save-Subjects
    Save-P1

    # every exe of the 0.14.42 extraction, at its extraction location (label = relative path)
    Invoke-Safe 'subjects: 0.14.42 bundled exes' {
        $xdir = Join-Path $global:DiagWork 'x\0.14.42'
        $sum = $P1['bundled_sig_summary']
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        if (Test-Path -LiteralPath $xdir) {
            $files = @(Get-ChildItem -LiteralPath $xdir -Recurse -File -Filter '*.exe' -ErrorAction SilentlyContinue | Sort-Object FullName)
            foreach ($f in $files) {
                $rel = $f.FullName.Substring($xdir.Length).TrimStart('\')
                $noSig = ($sw.Elapsed.TotalSeconds -gt 420)
                if ($noSig) { $P1['bundled_sig_truncated'] = 'signature checks stopped after 420 s; remaining subjects have sig=not checked' }
                $rec = Add-Subject ('bundled 0.14.42/' + ($rel -replace '\\', '/')) $f.FullName 'bundled-0.14.42' ('\x\0.14.42\' + $rel) -NoSig:$noSig
                $k = [string]$rec['sig']
                if ($sum.Contains($k)) { $sum[$k] = $sum[$k] + 1 } else { $sum[$k] = 1 }
            }
        }
    }
} catch {
    Add-DiagError 'p1-assets main' $_
} finally {
    $P1['subjects_count'] = $subjects.Count
    $P1['finished'] = (Get-IsoNow)
    Save-Subjects
    Save-P1
    Complete-DiagScript 'p1-assets'
}
exit 0
