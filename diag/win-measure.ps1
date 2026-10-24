# Adapted from a240cfbf diag/app-upgrade.ps1 (emulate), app-e2e.ps1 and sac-lib.ps1.
# Only GitHub's disposable Windows runner may execute this file. No product code patched.
param([Parameter(Mandatory=$true)][string]$Permit)
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:OS -ne 'Windows_NT' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') { throw 'Runner-only measurement; local execution refused' }
Get-ChildItem Env:CYS_* | Remove-Item -ErrorAction SilentlyContinue
Remove-Item Env:\CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue
. (Join-Path $PSScriptRoot 'vendor\lib.ps1')
. (Join-Path $PSScriptRoot 'vendor\sac-lib.ps1')
$ErrorActionPreference = 'Stop'
Start-DiagScript 'win-measure'
$K_PFX='winmeasure'; $K_JOB='winmeasure'; $K_CITOOL=Join-Path $env:windir 'System32\CiTool.exe'
$K_REG=Join-Path $env:windir 'System32\reg.exe'; $K_KEY='HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy'; $K_VAL='VerifiedAndReputablePolicyState'
$script:SacTouched=$false; $script:GhToken=[string]$env:GITHUB_TOKEN
Remove-Item Env:\GITHUB_TOKEN -ErrorAction SilentlyContinue
$CTX=@{subjects=@();driver_node=(Get-Command node.exe).Source}
$RUN=[ordered]@{started=(Get-IsoNow);steps=[ordered]@{};states=[ordered]@{};checkpoints=[ordered]@{};notes=(New-Object 'System.Collections.Generic.List[string]')}
$cells=[ordered]@{reading=$null;input=$false;baseline=$null;installer_exit=$null;versions=$null;daemon=$null;pack=$null;seat=$null;tracebacks=$null;logs_complete=$false;evidence_complete=$false;harness_error=$null;sac_before=$null;sac_after=$null;neg_blocked=$null;pos_allowed=$null;launch_known=$null;launch_blocked=$null;old_restart=$null;old_survives=$null;files_changed=$null;state_complete=$false;restored=$null}
$script:Install=Join-Path $env:LOCALAPPDATA 'cys'; $script:Pack=Join-Path $env:USERPROFILE '.cys\pack'
$script:ApplyStart=$null; $script:AuditStarted=$false
function Save-Run { Save-Json 'measurement.json' $RUN 30; Save-Json 'cells.json' $cells 10 }
function Must-Permit {
    $x=Get-Content -LiteralPath $Permit -Raw | ConvertFrom-Json
    if ($x.authorized -ne $true -or $x.plan_sha256 -ne 'ddf2c693c7e93598593a3f06c7c2e77aa8dd8b538efe5baf64fec9bdc8c3ee44' -or @('R1','R2') -notcontains $x.reading) { throw 'UNMEASURED_INPUT: invalid permit' }
    foreach($p in @(@($x.base_path,$x.base_sha256),@($x.candidate_path,$x.candidate_sha256))) {
        if (-not (Test-Path -LiteralPath $p[0] -PathType Leaf)) { throw 'UNMEASURED_INPUT: installer missing before install' }
        if ((Get-FileHash -LiteralPath $p[0] -Algorithm SHA256).Hash.ToLowerInvariant() -cne $p[1]) { throw 'UNMEASURED_INPUT: installer hash mismatch before install' }
    }
    return $x
}
# Observer CLI must never repair a missing daemon. Seal only this child, not the app/installer.
function Invoke-Cli([string]$File,[string]$Arguments,[int]$TimeoutSec) {
    $prior=$env:CYS_NO_AUTOSTART
    try{$env:CYS_NO_AUTOSTART='1';return (Invoke-Proc $File $Arguments $TimeoutSec)}
    finally{if($null -eq $prior){Remove-Item Env:\CYS_NO_AUTOSTART -ErrorAction SilentlyContinue}else{$env:CYS_NO_AUTOSTART=$prior}}
}
function Version3([string]$File) {
    $v=(Get-Item -LiteralPath $File).VersionInfo
    return ('{0}.{1}.{2}' -f $v.FileMajorPart,$v.FileMinorPart,$v.FileBuildPart)
}
function Image-Facts {
    $o=[ordered]@{}
    foreach($n in @('cys-app.exe','cys.exe','cysd.exe')) {
        $f=Join-Path $script:Install $n
        if(Test-Path -LiteralPath $f -PathType Leaf){$o[$n]=[ordered]@{path=$f;version=(Version3 $f);sha256=(Get-FileHash $f -Algorithm SHA256).Hash;signature=(Get-AuthenticodeSignature $f).Status.ToString()}}else{$o[$n]=$null}
    }
    return $o
}
function Pack-Facts([string]$Version) {
    $o=[ordered]@{ok=$false;version=$null;state=$null;manifest=$null;files=@();error=$null}
    try {
        $o.version=(Get-Content -LiteralPath (Join-Path $script:Pack '.pack-version') -Raw).Trim()
        $o.state=Get-Content -LiteralPath (Join-Path $script:Pack '.pack-state.json') -Raw | ConvertFrom-Json
        $manifestPath=Join-Path $script:Pack '.install-manifest.json'
        $o.manifest=Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        $bad=0; $count=0
        foreach($kv in $o.manifest.PSObject.Properties) {
            $f=[IO.Path]::GetFullPath((Join-Path $script:Pack $kv.Name))
            if(-not $f.StartsWith(([IO.Path]::GetFullPath($script:Pack)+'\'),[StringComparison]::OrdinalIgnoreCase)){throw 'manifest outside pack'}
            $exists=Test-Path -LiteralPath $f -PathType Leaf
            $h=$null;if($exists){$h=(Get-FileHash -LiteralPath $f -Algorithm SHA256).Hash.ToLowerInvariant()}
            $ok=$exists -and ($h -eq [string]$kv.Value); if(-not $ok){$bad++};$count++
            $o.files+=@([ordered]@{relative=$kv.Name;expected=$kv.Value;sha256=$h;ok=$ok})
        }
        $o.ok=($o.version -eq $Version -and $count -gt 0 -and $bad -eq 0 -and $o.state.base_version -eq $Version)
    }catch{$o.error=$_.Exception.Message}
    return $o
}
function App-Version([string]$Tag) {
    $cdpArgs='"'+(Join-Path $PSScriptRoot 'vendor\cdp-update.mjs')+'" --mode uipre --ready-wait-sec 30 --port 9333 --out "'+$global:DiagOut+'" --prefix '+$Tag+' --max-wait-sec 30'
    $r=Invoke-Proc $CTX.driver_node $cdpArgs 65
    $f=Join-Path $global:DiagOut ($Tag+'-cdp.json')
    if(Test-Path $f){return (Get-Content $f -Raw | ConvertFrom-Json)}
    return $null
}
function Read-State([string]$Version,[string]$Tag) {
    $o=[ordered]@{images=(Image-Facts);marker=$null;cli=$null;daemon_binary=$null;ping=$null;identify=$null;daemon_process=$null;app=$null;pack=(Pack-Facts $Version);versions=$false;daemon=$false;error=$null}
    try {
        $o.marker=(Get-Content -LiteralPath (Join-Path $script:Install 'cys-installed-version.txt') -Raw).Trim()
        $cli=Join-Path $script:Install 'cys.exe';$daemon=Join-Path $script:Install 'cysd.exe'
        $o.cli=Invoke-Cli $cli '--version' 15; $o.daemon_binary=Invoke-Proc $daemon '--version' 15
        $running=@(Get-CimInstance Win32_Process -Filter "Name='cysd.exe'" | Where-Object {$_.ExecutablePath -ieq $daemon})
        $o.daemon_processes_before_rpc=@($running | Select-Object ProcessId,ParentProcessId,ExecutablePath,CreationDate)
        # cys request may autostart: no RPC/seat creation unless a product daemon is already present.
        $id=$null
        if($running.Count -gt 0){$o.ping=Invoke-Cli $cli 'ping' 15;$id=Invoke-Cli $cli 'identify' 15;$o.identify=$id}
        if($id.rc -eq 0) {
            $j=$id.out | ConvertFrom-Json
            $dp=Get-CimInstance Win32_Process -Filter ('ProcessId='+[int]$j.daemon_pid)
            $o.daemon_process=[ordered]@{pid=$j.daemon_pid;path=$dp.ExecutablePath;created=$dp.CreationDate;version=$j.version}
            $o.daemon=($o.ping.rc -eq 0 -and $j.version -eq $Version -and $dp.ExecutablePath -ieq $daemon)
        }
        $o.app=App-Version $Tag
        $imageOK=$true;foreach($n in @('cys-app.exe','cys.exe','cysd.exe')){if($null -eq $o.images[$n] -or $o.images[$n].version -ne $Version){$imageOK=$false}}
        $o.versions=($imageOK -and $o.marker -eq $Version -and $o.cli.rc -eq 0 -and $o.cli.out.Trim() -eq ('cys '+$Version) -and $o.daemon_binary.rc -eq 0 -and $o.daemon_binary.out.Trim() -eq ('cysd '+$Version) -and $o.app.attached -eq $true -and $o.app.ui.ready.ok -eq $true -and $o.app.app_version -eq $Version -and -not (Test-Path (Join-Path $script:Install 'cys-install-failure.txt')))
    }catch{$o.error=$_.Exception.Message}
    Save-Json ($Tag+'-state.json') $o 30
    return $o
}
function Seat-Check([string]$Tag) {
    $nonce='wm-'+[Guid]::NewGuid().ToString('N');$cli=Join-Path $script:Install 'cys.exe'
    # Normal product PTY, no agent credentials. Keep it alive while checking pid/output, then prove exit0.
    $command='cmd.exe /d /q /c "echo '+$nonce+' & ping -n 6 127.0.0.1 >nul & exit /b 0"'
    $running=@(Get-CimInstance Win32_Process -Filter "Name='cysd.exe'" | Where-Object {$_.ExecutablePath -ieq (Join-Path $script:Install 'cysd.exe')})
    if($running.Count -eq 0){return [ordered]@{ok=$false;measured=$true;error='no pre-existing daemon; CLI autostart forbidden'}}
    $seatStart=(Get-Date).ToUniversalTime()
    $r=Invoke-Cli $cli ('new-surface --title win-measure --cmd '+(ConvertTo-CmdArg $command)) 20
    $o=[ordered]@{ok=$false;measured=$false;cli_probe=$null;exit_status=$null;nonce=$nonce;create=$r;surface=$null;screen=$null;list=$null;exit_list=$null;error=$null}
    try {
        $sid=$r.out.Trim();if($r.rc -ne 0 -or $sid -notmatch '^surface:\d+$'){
            $o.cli_probe=Invoke-ProbeCreate $cli
            if($o.cli_probe.completed -eq $true -and $o.cli_probe.ok -eq $false){$o.measured=$true;Save-Json ($Tag+'-seat.json') $o 12;return $o}
            throw 'surface not created or observer could not classify it'
        };$o.surface=$sid
        Start-Sleep -Seconds 1
        $o.screen=Invoke-Cli $cli ('read-screen --surface '+$sid+' --lines 100') 15
        $o.list=Invoke-Cli $cli 'list' 15
        Start-Sleep -Seconds 6
        $o.exit_list=Invoke-Cli $cli 'list' 15
        $line=@($o.list.out -split "`n" | Where-Object {$_ -match ('^'+[regex]::Escape($sid)+'\s')})
        $end=@($o.exit_list.out -split "`n" | Where-Object {$_ -match ('^'+[regex]::Escape($sid)+'\s')})
        # A visible nonce and exited=true alone do not prove exit0. Require the process's audit status.
        if($line.Count -eq 1 -and $line[0] -match 'pid=([1-9]\d*)'){
            $seatPid=[int64]$Matches[1]
            $evs=@(Get-WinEvent -FilterHashtable @{LogName='Security';Id=4689;StartTime=$seatStart.ToLocalTime()} -ErrorAction Stop)
            foreach($ev in $evs){[xml]$xml=$ev.ToXml();$data=@{};foreach($d in $xml.Event.EventData.Data){$data[[string]$d.Name]=[string]$d.'#text'}
                if([Convert]::ToInt64($data.ProcessId,16) -eq $seatPid){$o.exit_status=[Convert]::ToInt64($data.Status,16)}
            }
        }
        $o.measured=($null -ne $o.exit_status)
        $o.ok=($o.screen.rc -eq 0 -and $o.screen.out.Contains($nonce) -and $line.Count -eq 1 -and $line[0] -match 'pid=[1-9]\d*' -and $line[0] -match 'exited=false' -and $end.Count -eq 1 -and $end[0] -match 'exited=true' -and $null -ne $o.exit_status -and $o.exit_status -eq 0)
    }catch{$o.error=$_.Exception.Message}
    Save-Json ($Tag+'-seat.json') $o 12;return $o
}
function Capture-Logs([string]$Tag) {
    $out=Join-Path $global:DiagOut ('logs-'+$Tag);New-Item -ItemType Directory -Force -Path $out | Out-Null
    $roots=@((Join-Path $env:LOCALAPPDATA 'cys'),(Join-Path $env:USERPROFILE '.cys'))
    $o=[ordered]@{complete=$true;count=0;tracebacks=0;roots=$roots;files=@();errors=@()}
    foreach($root in $roots) {
        try {
            if(-not(Test-Path $root -PathType Container)){throw ('missing root '+$root)}
            $files=@(Get-ChildItem -LiteralPath $root -Recurse -File -ErrorAction Stop | Where-Object {$_.Name -like '*.log*' -or $_.Name -like '*log*.jsonl'})
            if($files.Count -eq 0){throw ('no log files '+$root)}
            foreach($f in $files){
                $stream=New-Object IO.FileStream($f.FullName,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
                try{$memory=New-Object IO.MemoryStream;$stream.CopyTo($memory);$bytes=$memory.ToArray();$memory.Dispose()}finally{$stream.Dispose()}
                $dst=Join-Path $out ('{0:D5}.log' -f $o.count);[IO.File]::WriteAllBytes($dst,$bytes)
                $offset=0
                if($Tag -eq 'final' -and $RUN.base_logs){
                    $prior=@($RUN.base_logs.files | Where-Object {$_.path -eq $f.FullName})
                    if($prior.Count -eq 1 -and $prior[0].bytes -le $bytes.Length -and $prior[0].bytes -gt 0){
                        $prefix=[byte[]]$bytes[0..([int]$prior[0].bytes-1)]
                        $hasher=[Security.Cryptography.SHA256]::Create()
                        try{$prefixHash=([BitConverter]::ToString($hasher.ComputeHash($prefix))).Replace('-','')}finally{$hasher.Dispose()}
                        if($prefixHash -eq $prior[0].sha256){$offset=[int]$prior[0].bytes}
                    }
                }
                $text=[Text.Encoding]::UTF8.GetString($bytes,$offset,$bytes.Length-$offset);$hits=[regex]::Matches($text,'Traceback').Count
                $o.files+=@([ordered]@{path=$f.FullName;copy=$dst;bytes=$bytes.Length;scan_offset=$offset;sha256=(Get-FileHash $dst -Algorithm SHA256).Hash;tracebacks=$hits;captured=(Get-IsoNow)})
                $o.count++;$o.tracebacks+=$hits
            }
        }catch{$o.complete=$false;$o.errors+=@($_.Exception.Message)}
    }
    $daemonLogs=@($o.files | Where-Object {$_.path -match '(cysd|daemon|phoenix-restore).*\.log'})
    $packLogs=@($o.files | Where-Object {$_.path.StartsWith((Join-Path $env:USERPROFILE '.cys'),[StringComparison]::OrdinalIgnoreCase)})
    if($daemonLogs.Count -eq 0 -or $packLogs.Count -eq 0){$o.complete=$false;$o.errors+=@('daemon/pack log coverage missing; no zero-count inference')}
    if([regex]::Matches('positive Traceback sentinel','Traceback').Count -ne 1){throw 'Traceback counter failed'}
    Save-Json ($Tag+'-logs.json') $o 12;return $o
}
# Security events supply native process exit status; launch success is not process success.
function Installer-Exit([datetime]$Since,[string]$Path) {
    $o=[ordered]@{exit=$null;complete=$false;events=@();roots=@();error=$null}
    try {
        $events=@(Get-WinEvent -FilterHashtable @{LogName='Security';Id=4688,4689;StartTime=$Since.ToLocalTime()} -ErrorAction Stop | Sort-Object TimeCreated)
        $starts=@{};$ends=@{}
        foreach($ev in $events){
            [xml]$xml=$ev.ToXml();$data=@{};foreach($d in $xml.Event.EventData.Data){$data[[string]$d.Name]=[string]$d.'#text'}
            $o.events+=@([ordered]@{id=$ev.Id;time=$ev.TimeCreated.ToUniversalTime().ToString('o');data=$data;xml=$ev.ToXml()})
            if($ev.Id -eq 4688 -and $data.NewProcessName -ieq $Path){$starts[$data.NewProcessId]=$true;$o.roots+=@($data.NewProcessId)}
            if($ev.Id -eq 4689 -and $data.ProcessName -ieq $Path){$ends[$data.ProcessId]=$data.Status}
        }
        if($starts.Count -eq 1){$id=@($starts.Keys)[0];if($ends.ContainsKey($id)){$o.exit=[Convert]::ToInt64($ends[$id],16);$o.complete=$true}}
    }catch{$o.error=$_.Exception.Message}
    Save-Json 'installer-process-evidence.json' $o 16;return $o
}
try {
    $inputLock=Must-Permit; $cells.reading=$inputLock.reading;$cells.input=$true;$K_JOB='winmeasure-'+$inputLock.reading.ToLowerInvariant()
    $RUN.inputs=$inputLock;$RUN.harness_sha=$env:GITHUB_SHA;$RUN.run_id=$env:GITHUB_RUN_ID;$RUN.attempt=$env:GITHUB_RUN_ATTEMPT
    $RUN.environment=[ordered]@{os=(Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber);arch=$env:PROCESSOR_ARCHITECTURE;image=$env:ImageVersion}
    $initial=Get-SacRealState 'initial';$RUN.initial_sac=$initial
    if($initial.enforced){throw 'UNMEASURED: baseline must be established before SAC is enabled'}
    Initialize-DiagNative;if(-not $global:DiagNativeReady){throw 'native observer unavailable'}
    # Warm observers and compile fresh unsigned control before SAC can restrict language mode.
    $null=Get-MpComputerStatus; $null=Get-CimInstance Win32_Process; $null=Get-AuthenticodeSignature $CTX.driver_node
    $neg=Join-Path $global:DiagWork ('negative-'+[Guid]::NewGuid().ToString('N')+'.exe')
    Add-Type -TypeDefinition 'public class WMNegative { public static void Main(){System.Console.WriteLine("winmeasure-negative");} }' -OutputAssembly $neg -OutputType ConsoleApplication
    $RUN.neg_before=Invoke-Proc $neg '' 15
    if($RUN.neg_before.rc -ne 0){throw 'negative control did not run before SAC'}
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9333 --remote-allow-origins=*'
    Remove-Item Env:\CYS_UPDATE_MANIFEST_URL -ErrorAction SilentlyContinue
    $null=Must-Permit
    $RUN.base_install=Invoke-Proc $inputLock.base_path '/S' 600
    if($RUN.base_install.rc -ne 0){throw 'base installer failed'}
    $baseApp=Start-Process -FilePath (Join-Path $script:Install 'cys-app.exe') -WorkingDirectory $script:Install -PassThru
    $RUN.base_app_pid=$baseApp.Id
    $until=(Get-Date).AddSeconds(300)
    do{$baseline=Read-State '0.14.52' 'base';if($baseline.versions -and $baseline.daemon -and $baseline.pack.ok){break};Start-Sleep 5}while((Get-Date) -lt $until)
    $cells.baseline=($baseline.versions -and $baseline.daemon -and $baseline.pack.ok)
    $RUN.baseline=$baseline;$RUN.base_logs=Capture-Logs 'base'
    if(-not $cells.baseline){throw 'baseline not ready; do not apply candidate'}
    # Product-root-only audit; do not kill or reset the old daemon.
    foreach($guid in @('{0CCE922B-69AE-11D9-BED3-505054503030}','{0CCE922C-69AE-11D9-BED3-505054503030}')){
        $a=Invoke-Proc 'auditpol.exe' ('/set /subcategory:'+ $guid +' /success:enable') 30
        if($a.rc -ne 0){throw 'installer process audit unavailable'}
    }
    $script:AuditStarted=$true
    Save-Run;Publish-Checkpoint 'before-apply'
    if($cells.reading -eq 'R2'){
        Save-Json 'sac-touched.json' ([ordered]@{time=(Get-IsoNow);initial_value=$initial.registry.value}) 5
        $script:SacTouched=$true;$RUN.sac_set=Set-SacValue 1 'on';Start-Sleep 10
        $st=Get-SacRealState 'before-apply';$RUN.sac_before=$st
        $cells.sac_before=($st.registry.value -eq 1 -and $st.signals.citool_desktop_enforced -and $st.signals.defender_on -eq $true -and $st.signals.umci_flag)
        $RUN.negative=Invoke-ProbeCreate $neg;$RUN.positive=Invoke-ProbeCreate $CTX.driver_node
        $cells.neg_blocked=($RUN.negative.ok -eq $false -and $RUN.negative.last_error -eq 4551)
        $cells.pos_allowed=($RUN.positive.ok -eq $true)
        if(-not($cells.sac_before -and $cells.neg_blocked -and $cells.pos_allowed)){throw 'UNMEASURED_SAC: enforcement/control prerequisite'}
    }
    $null=Must-Permit
    $temp=Join-Path $env:TEMP ('cys-0.14.53-updater-'+[Guid]::NewGuid().ToString('N'));New-Item -ItemType Directory $temp | Out-Null
    $dest=Join-Path $temp 'cys-0.14.53-installer.exe';Copy-Item -LiteralPath $inputLock.candidate_path -Destination $dest
    if((Get-FileHash $dest -Algorithm SHA256).Hash.ToLowerInvariant() -ne $inputLock.candidate_sha256){throw 'candidate copy hash mismatch'}
    $RUN.candidate_copy=$dest;$RUN.candidate_authenticode=(Get-AuthenticodeSignature $dest).Status.ToString()
    $script:ApplyStart=(Get-Date).ToUniversalTime();$oldCwd=[Environment]::CurrentDirectory
    try{[Environment]::CurrentDirectory=$script:Install;$call=[DiagNative]::ShellExecStart($dest,'/P /R /UPDATE /ARGS',20000)}finally{[Environment]::CurrentDirectory=$oldCwd}
    $RUN.launch=ConvertTo-CallRecord $call;$cells.launch_known=($call.Completed -and -not $call.TimedOut)
    $cells.launch_blocked=($cells.launch_known -and -not $call.Ok -and $call.LastError -eq 4551)
    # Pinned tauri-plugin-updater2.10.1 updater.rs:855-865 ignores ShellExecuteW result then exits0.
    # A returned failure therefore ends the old app too; a still-pending call does not.
    if($cells.launch_known){Start-Sleep -Milliseconds 300;if(-not $baseApp.HasExited){Stop-Process -Id $baseApp.Id -Force}}
    # A pending ShellExecute is not settled; never dismiss a dialog and call it a measured block.
    if(-not $cells.launch_known){throw 'UNMEASURED: ShellExecute still pending'}
    $until=(Get-Date).AddSeconds(600)
    if($call.Ok){do{$audit=Installer-Exit $script:ApplyStart $dest;if($audit.complete){break};Start-Sleep 3}while((Get-Date) -lt $until);$cells.installer_exit=$audit.exit}
    else{$RUN.launch_refused=$true}
    $RUN.images_after=Image-Facts;$cells.files_changed=$false
    foreach($n in @('cys-app.exe','cys.exe','cysd.exe')){if($RUN.images_after[$n].sha256 -ne $baseline.images[$n].sha256){$cells.files_changed=$true}}
    $until=(Get-Date).AddSeconds(300)
    do{$after=Read-State '0.14.53' 'after';if($after.versions -and $after.daemon -and $after.pack.ok){break};if(-not $call.Ok){break};Start-Sleep 5}while((Get-Date) -lt $until)
    $RUN.after=$after;$cells.versions=$after.versions;$cells.daemon=$after.daemon;$cells.pack=$after.pack.ok;if($after.pack.error){$cells.pack=$null}
    $seat=Seat-Check 'after';$RUN.seat=$seat;$cells.seat=$seat.ok;if(-not $seat.measured){$cells.seat=$null}
    Start-Sleep 60
    $RUN.final_state=Read-State '0.14.53' 'final'
    $cells.versions=($cells.versions -and $RUN.final_state.versions);$cells.daemon=($cells.daemon -and $RUN.final_state.daemon);if($null -ne $cells.pack -and -not $RUN.final_state.pack.error){$cells.pack=($cells.pack -and $RUN.final_state.pack.ok)}else{$cells.pack=$null}
    if($cells.reading -eq 'R2'){
        $cells.old_survives=(-not $baseApp.HasExited)
        $cells.old_restart=$false
        if(-not $cells.files_changed){
            # Explicit re-launch of the unchanged old image is different from surviving PID.
            $restart=Invoke-ProbeCreate (Join-Path $script:Install 'cys-app.exe');$RUN.old_restart_probe=$restart
            if($restart.completed -ne $true){$cells.old_restart=$null}
            if($restart.ok){
                $oldProcess=Start-Process -FilePath (Join-Path $script:Install 'cys-app.exe') -WorkingDirectory $script:Install -PassThru
                Start-Sleep 5;$oldState=Read-State '0.14.52' 'old-restart';$oldSeat=Seat-Check 'old-restart'
                $RUN.old_restart_state=$oldState;$cells.old_restart=($oldState.versions -and $oldState.daemon -and $oldState.pack.ok -and $oldSeat.ok)
            }
        }
        $st=Get-SacRealState 'after-observation';$RUN.sac_after=$st
        $cells.sac_after=($st.registry.value -eq 1 -and $st.signals.citool_desktop_enforced -and $st.signals.defender_on -eq $true -and $st.signals.umci_flag)
        $cells.state_complete=($null -ne $cells.old_restart -and $null -ne $cells.files_changed -and $null -ne $RUN.after)
    }
    $RUN.final_logs=Capture-Logs 'final';$cells.logs_complete=($RUN.base_logs.complete -and $RUN.final_logs.complete)
    $cells.tracebacks=($RUN.base_logs.tracebacks + $RUN.final_logs.tracebacks)
    $RUN.events=Get-CIEvents $script:ApplyStart;Save-Json 'codeintegrity.json' $RUN.events 20
    $cells.evidence_complete=$true
    Save-Run;Publish-Checkpoint 'after-observation'
}catch{$cells.harness_error=$_.Exception.Message;$RUN.error=$_.Exception.ToString();Save-Run}
finally {
    if($script:SacTouched){$RUN.restore=Restore-SacReal 'final';$cells.restored=$RUN.restore.restored;if(-not $cells.restored){$RUN.restore_fallback=Invoke-CmdRestoreFallback}}
    elseif($cells.reading -eq 'R2'){$cells.restored=$false}
    $RUN.finished=Get-IsoNow;Save-Run
    # Preserve processes and files until observations are saved. Disposable runner teardown cleans them.
    $env:GITHUB_TOKEN=$script:GhToken
}
& $CTX.driver_node (Join-Path $PSScriptRoot 'measure-contract.mjs') verdict (Join-Path $global:DiagOut 'cells.json') (Join-Path $global:DiagOut 'verdict.json')
exit $LASTEXITCODE
