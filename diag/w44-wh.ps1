# diag/w44-wh.ps1 -- run the product's `wh_windows` test executable and a real cysd under an elevated and a standard-user token. ASCII only, PS 5.1.
$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'
. (Join-Path $PSScriptRoot 'lib.ps1')
Start-DiagScript 'w44-wh'
$OUT = $global:DiagOut
$PUB = 'C:\w44pub'
$BIN = Join-Path $PUB 'bin'
& icacls.exe $PUB /grant 'Everyone:(OI)(CI)M' | Out-Null
$R = [ordered]@{ os = [string]$env:DIAG_MATRIX_OS; files = @(); tests = [ordered]@{}; daemon = [ordered]@{}; errors = @() }
function SaveR { Save-Json 'w44-wh-results.json' $R 8 }
$testExe = Join-Path $BIN 'cysd-test.exe'
$R['test_exe_exists'] = Test-Path -LiteralPath $testExe
$R['bins'] = @(Get-ChildItem -LiteralPath $BIN -ErrorAction SilentlyContinue | ForEach-Object { '{0} {1}' -f $_.Name, $_.Length })

# the standard user
$pw = 'W44-' + [guid]::NewGuid().ToString('N').Substring(0, 12) + '!aA1'
& net.exe user w44std $pw /add /y | Out-Null
$sec = ConvertTo-SecureString $pw -AsPlainText -Force
$cred = New-Object System.Management.Automation.PSCredential('.\w44std', $sec)
$psexe = Join-Path $env:windir 'System32\WindowsPowerShell\v1.0\powershell.exe'
$cmdexe = Join-Path $env:windir 'System32\cmd.exe'

function Run-Std { param([string]$File, [string]$Args, [string]$Tag, [int]$MaxSec = 600)
    $so = Join-Path $PUB ($Tag + '.out.txt'); $se = Join-Path $PUB ($Tag + '.err.txt')
    $p = Start-Process -FilePath $File -ArgumentList $Args -Credential $cred -LoadUserProfile -WorkingDirectory $PUB -PassThru -RedirectStandardOutput $so -RedirectStandardError $se
    if (-not $p.WaitForExit($MaxSec * 1000)) { try { Stop-ProcessTree -ProcessId $p.Id } catch { } }
    $rc = $null; try { $rc = $p.ExitCode } catch { }
    foreach ($f in @($so, $se)) { if (Test-Path -LiteralPath $f) { Copy-Item -LiteralPath $f -Destination (Join-Path $OUT (Split-Path -Leaf $f)) -Force } }
    return $rc
}

# identity proof for the standard user (written by the user itself)
$idScript = Join-Path $PUB 'whoami-std.ps1'
[System.IO.File]::WriteAllText($idScript, "whoami /groups /fo csv /nh | Out-File -Encoding ascii C:\w44pub\whoami-std.txt; `$env:USERPROFILE | Out-File -Append -Encoding ascii C:\w44pub\whoami-std.txt; ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) | Out-File -Append -Encoding ascii C:\w44pub\whoami-std.txt")
$R['whoami_rc'] = Run-Std $psexe ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}"' -f $idScript) 'whoami-std' 120
if (Test-Path 'C:\w44pub\whoami-std.txt') { Copy-Item 'C:\w44pub\whoami-std.txt' (Join-Path $OUT 'w44-wh-whoami-std.txt') -Force; $R['whoami_std'] = @(Get-Content 'C:\w44pub\whoami-std.txt' | Where-Object { $_ -match 'Mandatory Label|Administrators|True|False|Users' } | Select-Object -First 8) }
$R['whoami_elevated'] = @(& whoami.exe /groups /fo csv /nh | Where-Object { $_ -match 'Mandatory Label|BUILTIN\\Administrators' })

# 1) the product test: elevated token, then standard user
$envs = 'set CYS_PACK_DIR={0}&& ' 
$o1 = Join-Path $OUT 'w44-wh-test-elevated-stdout.txt'; $e1 = Join-Path $OUT 'w44-wh-test-elevated-stderr.txt'
if ($R['test_exe_exists']) {
    $env:CYS_PACK_DIR = Join-Path $env:RUNNER_TEMP 'pk-elev'; New-Item -ItemType Directory -Force -Path $env:CYS_PACK_DIR | Out-Null
    $p = Start-Process -FilePath $testExe -ArgumentList 'wh_windows --nocapture --test-threads=1' -PassThru -Wait -RedirectStandardOutput $o1 -RedirectStandardError $e1 -WorkingDirectory $PUB
    $R['tests']['elevated_rc'] = $p.ExitCode
    $stdPack = Join-Path $PUB 'pk-std'; New-Item -ItemType Directory -Force -Path $stdPack | Out-Null
    $bat = Join-Path $PUB 'run-test-std.bat'
    [System.IO.File]::WriteAllText($bat, ("@echo off`r`nset CYS_PACK_DIR=" + $stdPack + "`r`n" + $testExe + " wh_windows --nocapture --test-threads=1`r`n"))
    $rc = Run-Std $cmdexe ('/d /c ' + $bat) 'w44-wh-test-standard' 600
    $R['tests']['standard_rc'] = $rc
    foreach ($k in @('elevated', 'standard')) {
        $lines = @()
        foreach ($f in @((Join-Path $OUT ('w44-wh-test-' + $k + '-stdout.txt')), (Join-Path $OUT ('w44-wh-test-' + $k + '-stderr.txt')), (Join-Path $OUT ('w44-wh-test-' + $k + '.out.txt')), (Join-Path $OUT ('w44-wh-test-' + $k + '.err.txt')))) {
            if (Test-Path -LiteralPath $f) { $lines += @(Get-Content -LiteralPath $f | Where-Object { $_ -match 'WH-WIN|^test |test result|panicked|assert' }) }
        }
        $R['tests'][$k + '_lines'] = @($lines | Select-Object -First 40)
    }
}
SaveR

# 2) a real daemon as the standard user (isolated pipe; the user's own LOCALAPPDATA state folder), one seat, 30 s of observation
$dscript = Join-Path $PUB 'daemon-probe.ps1'
$ds = @'
param([string]$Tag)
$ErrorActionPreference = 'Continue'
$o = 'C:\w44pub\daemon-' + $Tag + '.txt'
function L($m) { ((Get-Date).ToString('o') + ' ' + $m) | Out-File -Append -Encoding ascii $o }
$env:CYS_SOCKET = '\\.\pipe\cys-w44wh' + $Tag
$env:CYS_NO_OFFICE_BRIDGE = '1'
L ('user=' + [Environment]::UserName + ' profile=' + $env:USERPROFILE + ' admin=' + ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))
$d = Start-Process -FilePath 'C:\w44pub\bin\cysd.exe' -PassThru -WindowStyle Hidden -RedirectStandardError ('C:\w44pub\cysd-' + $Tag + '.err.txt') -RedirectStandardOutput ('C:\w44pub\cysd-' + $Tag + '.out.txt')
L ('daemon pid ' + $d.Id)
$ready = $false
for ($i = 0; $i -lt 45; $i++) { Start-Sleep -Seconds 2; & 'C:\w44pub\bin\cys.exe' ping *> $null; if ($LASTEXITCODE -eq 0) { $ready = $true; break } }
L ('ready=' + $ready)
$seat = & 'C:\w44pub\bin\cys.exe' new-surface --title w44seat --cmd cmd.exe 2>&1
L ('seat: ' + ($seat -join ' '))
Start-Sleep -Seconds 2
& 'C:\w44pub\bin\cys.exe' send --surface ($seat | Select-Object -First 1) 'ping -n 40 127.0.0.1 > nul' *> $null
& 'C:\w44pub\bin\cys.exe' send-key --surface ($seat | Select-Object -First 1) Return *> $null
for ($i = 0; $i -lt 6; $i++) {
    Start-Sleep -Seconds 5
    L ('status: ' + ((& 'C:\w44pub\bin\cys.exe' status 2>&1) -join ' | '))
    L ('list: ' + ((& 'C:\w44pub\bin\cys.exe' list 2>&1) -join ' | '))
}
$err = Get-Content ('C:\w44pub\cysd-' + $Tag + '.err.txt') -ErrorAction SilentlyContinue
L ('proc_count_high lines in daemon stderr: ' + @($err | Where-Object { $_ -match 'proc_count_high|process_count|proc count' }).Count)
foreach ($x in @($err | Where-Object { $_ -match 'proc_count|watchdog' } | Select-Object -First 10)) { L ('  ' + $x) }
Stop-Process -Id $d.Id -Force -ErrorAction SilentlyContinue
L 'done'
'@
[System.IO.File]::WriteAllText($dscript, $ds)
# elevated first (for contrast), then standard
$envMap = @{}
$old = $env:CYS_PACK_DIR; Remove-Item Env:\CYS_PACK_DIR -ErrorAction SilentlyContinue
$pe = Start-Process -FilePath $psexe -ArgumentList ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}" -Tag elev' -f $dscript) -PassThru -Wait -WorkingDirectory $PUB
$R['daemon']['elevated_rc'] = $pe.ExitCode
$R['daemon']['standard_rc'] = Run-Std $psexe ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}" -Tag std' -f $dscript) 'w44-wh-daemon-std-runner' 400
foreach ($t in @('elev', 'std')) {
    foreach ($f in @(('C:\w44pub\daemon-' + $t + '.txt'), ('C:\w44pub\cysd-' + $t + '.err.txt'))) { if (Test-Path -LiteralPath $f) { Copy-Item -LiteralPath $f -Destination (Join-Path $OUT ('w44-wh-' + (Split-Path -Leaf $f))) -Force } }
    $f = ('C:\w44pub\daemon-' + $t + '.txt')
    if (Test-Path -LiteralPath $f) { $R['daemon'][$t] = @(Get-Content -LiteralPath $f | ForEach-Object { if ($_.Length -gt 300) { $_.Substring(0, 300) } else { $_ } }) }
}
try { & net.exe user w44std /delete | Out-Null } catch { }
SaveR
Complete-DiagScript 'w44-wh'
