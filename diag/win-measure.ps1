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
# Required JSON writes are checked and registered; the vendored best-effort writer is overridden.
$script:EvidenceReceipts=[ordered]@{}
function Save-Json([string]$Name,$Object,[int]$Depth=30) {
    if([IO.Path]::GetFileName($Name) -ne $Name){throw 'invalid evidence filename'}
    $file=Join-Path $global:DiagOut $Name
    $text=ConvertTo-Json -InputObject $Object -Depth $Depth
    $encoding=New-Object Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($file,$text,$encoding)
    $read=[IO.File]::ReadAllText($file,$encoding)
    if($read -cne $text -or $read.Length -eq 0){throw ('evidence read-back failed: '+$Name)}
    $null=ConvertFrom-Json -InputObject $read -ErrorAction Stop
    $script:EvidenceReceipts[$Name]=[ordered]@{sha256=(Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant();bytes=(Get-Item -LiteralPath $file).Length;format='json'}
}
function Save-Run {
    $RUN.seed_cells=$cells
    Save-Json 'measurement.json' $RUN 60
    Save-Json 'cells.json' $cells 10
    $file=Join-Path $global:DiagOut 'evidence-receipts.json'
    [IO.File]::WriteAllText($file,(ConvertTo-Json -InputObject $script:EvidenceReceipts -Depth 10),(New-Object Text.UTF8Encoding($false)))
    $null=Get-Content -LiteralPath $file -Raw | ConvertFrom-Json -ErrorAction Stop
}
function Known($Value,[string]$Reason='') { return [ordered]@{known=$true;value=[bool]$Value;error=$null;reason=$Reason} }
function Unknown([string]$ErrorText) { return [ordered]@{known=$false;value=$null;error=$ErrorText} }
function State-Ready($State) { return ($State.versions.known -and $State.versions.value -and $State.daemon.known -and $State.daemon.value -and $State.pack.known -and $State.pack.value) }
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
    $o=[ordered]@{known=$false;value=$null;error=$null;version=$null;state=$null;manifest=$null;files=@()}
    try {
        foreach($name in @('.pack-version','.pack-state.json','.install-manifest.json')){
            if(-not(Test-Path -LiteralPath (Join-Path $script:Pack $name) -PathType Leaf -ErrorAction Stop)){$o.known=$true;$o.value=$false;$o.reason='missing metadata: '+$name;return $o}
        }
        $o.version=(Get-Content -LiteralPath (Join-Path $script:Pack '.pack-version') -Raw -ErrorAction Stop).Trim()
        $stateText=Get-Content -LiteralPath (Join-Path $script:Pack '.pack-state.json') -Raw -ErrorAction Stop
        $manifestText=Get-Content -LiteralPath (Join-Path $script:Pack '.install-manifest.json') -Raw -ErrorAction Stop
        try{$o.state=ConvertFrom-Json $stateText -ErrorAction Stop;$o.manifest=ConvertFrom-Json $manifestText -ErrorAction Stop}
        catch{$o.known=$true;$o.value=$false;$o.reason='corrupt JSON metadata';return $o}
        if($o.state -isnot [pscustomobject] -or $o.manifest -isnot [pscustomobject]){$o.known=$true;$o.value=$false;$o.reason='metadata JSON shape invalid';return $o}
        $bad=0;$count=0
        foreach($kv in $o.manifest.PSObject.Properties){
            $f=[IO.Path]::GetFullPath((Join-Path $script:Pack $kv.Name))
            if(-not $f.StartsWith(([IO.Path]::GetFullPath($script:Pack)+'\'),[StringComparison]::OrdinalIgnoreCase)){$bad++;continue}
            $exists=Test-Path -LiteralPath $f -PathType Leaf -ErrorAction Stop
            $h=$null;if($exists){$h=(Get-FileHash -LiteralPath $f -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()}
            $ok=$exists -and ($h -eq [string]$kv.Value);if(-not $ok){$bad++};$count++
            $o.files+=@([ordered]@{relative=$kv.Name;expected=$kv.Value;sha256=$h;ok=$ok})
        }
        $o.known=$true;$o.value=($o.version -eq $Version -and $count -gt 0 -and $bad -eq 0 -and $o.state.base_version -eq $Version)
    }catch{$o.known=$false;$o.value=$null;$o.error=$_.Exception.Message}
    return $o
}
function App-Version([string]$Tag) {
    $unique=$Tag+'-'+[Guid]::NewGuid().ToString('N')
    $cdpArgs='"'+(Join-Path $PSScriptRoot 'vendor\cdp-update.mjs')+'" --mode uipre --ready-wait-sec 30 --port 9333 --out "'+$global:DiagOut+'" --prefix '+$unique+' --max-wait-sec 30'
    $r=Invoke-Proc $CTX.driver_node $cdpArgs 65
    $f=Join-Path $global:DiagOut ($unique+'-cdp.json')
    if($r.startError -or $r.timedOut -or $r.rc -ne 0 -or -not(Test-Path $f)){return [ordered]@{known=$false;value=$null;error='CDP observer unavailable';process=$r}}
    $j=Get-Content $f -Raw | ConvertFrom-Json -ErrorAction Stop
    if(-not $j.attached -or $null -eq $j.app_version -or $null -eq $j.ui.ready.ok){return [ordered]@{known=$false;value=$null;error='CDP result incomplete';data=$j}}
    return [ordered]@{known=$true;data=$j;process=$r}
}
function Read-State([string]$Version,[string]$Tag) {
    $raw=[ordered]@{expected=$Version;images=$null;marker=$null;failure_file=$null;cli=$null;daemon_binary=$null;ping=$null;identify=$null;cim=$null;app=$null;pack=$null;error=$null}
    try {
        $raw.images=Image-Facts;$raw.pack=Pack-Facts $Version
        $markerPath=Join-Path $script:Install 'cys-installed-version.txt'
        if(Test-Path -LiteralPath $markerPath -PathType Leaf -ErrorAction Stop){$raw.marker=(Get-Content -LiteralPath $markerPath -Raw -ErrorAction Stop).Trim()}
        $raw.failure_file=Test-Path -LiteralPath (Join-Path $script:Install 'cys-install-failure.txt') -ErrorAction Stop
        $cli=Join-Path $script:Install 'cys.exe';$daemon=Join-Path $script:Install 'cysd.exe'
        $raw.cli=Invoke-Cli $cli '--version' 15;$raw.daemon_binary=Invoke-Proc $daemon '--version' 15
        $daemons=@(Get-CimInstance Win32_Process -Filter "Name='cysd.exe'" -ErrorAction Stop | Where-Object {$_.ExecutablePath -ieq $daemon} | Select-Object ProcessId,ExecutablePath,CreationDate)
        $apps=@(Get-CimInstance Win32_Process -Filter "Name='cys-app.exe'" -ErrorAction Stop | Where-Object {$_.ExecutablePath -ieq (Join-Path $script:Install 'cys-app.exe')} | Select-Object ProcessId,ExecutablePath,CreationDate)
        $raw.cim=[ordered]@{known=$true;daemons=$daemons;apps=$apps}
        if($daemons.Count -gt 0){
            $raw.ping=Invoke-Cli $cli 'ping' 15;$id=Invoke-Cli $cli 'identify' 15
            $raw.identify=$id
            if($id.rc -eq 0){$raw.identify.data=ConvertFrom-Json $id.out -ErrorAction Stop}
        }
        if($apps.Count -gt 0){$raw.app=App-Version $Tag}
    }catch{$raw.error=$_.Exception.Message}
    Save-Json ($Tag+'-raw-state.json') $raw 60
    $out=Join-Path $global:DiagOut ($Tag+'-state-reduced.json')
    & $CTX.driver_node (Join-Path $PSScriptRoot 'driver-observation.mjs') state (Join-Path $global:DiagOut ($Tag+'-raw-state.json')) $out
    if($LASTEXITCODE -ne 0){throw 'state reducer failed'}
    $o=Get-Content -LiteralPath $out -Raw | ConvertFrom-Json -ErrorAction Stop
    Save-Json ($Tag+'-state.json') $o 30
    # Keep raw evidence alongside the actual reducer output in RUN.
    return $o
}
function Seat-Check([string]$Tag) {
    $nonce='wm-'+[Guid]::NewGuid().ToString('N');$cli=Join-Path $script:Install 'cys.exe'
    $o=[ordered]@{known=$false;value=$null;error=$null;nonce=$nonce;create=$null;screen=$null;list=$null;exit_list=$null;exit_status=$null;audit=@()}
    try {
        $running=@(Get-CimInstance Win32_Process -Filter "Name='cysd.exe'" -ErrorAction Stop | Where-Object {$_.ExecutablePath -ieq (Join-Path $script:Install 'cysd.exe')})
        if($running.Count -eq 0){$o.known=$true;$o.value=$false;$o.reason='no daemon';return $o}
        $command='cmd.exe /d /q /c "echo '+$nonce+' & ping -n 6 127.0.0.1 >nul & exit /b 0"'
        $seatStart=(Get-Date).ToUniversalTime();$o.create=Invoke-Cli $cli ('new-surface --title win-measure --cmd '+(ConvertTo-CmdArg $command)) 20
        $sid=$o.create.out.Trim()
        if($o.create.rc -ne 0 -or $sid -notmatch '^surface:\d+$'){
            $o.cli_probe=Invoke-ProbeCreate $cli
            if($o.cli_probe.completed -eq $true){$o.known=$true;$o.value=$false;$o.reason='new-surface failed with measured CLI launchability'}
            else{$o.error='CLI observer unavailable'}
            return $o
        }
        $o.surface=$sid;Start-Sleep 1
        $o.screen=Invoke-Cli $cli ('read-screen --surface '+$sid+' --lines 100') 15;$o.list=Invoke-Cli $cli 'list' 15
        Start-Sleep 6;$o.exit_list=Invoke-Cli $cli 'list' 15
        if($o.list.rc -ne 0 -or $o.exit_list.rc -ne 0 -or $o.screen.rc -ne 0){$o.error='list/screen observer unavailable';return $o}
        $line=@($o.list.out -split "`n" | Where-Object {$_ -match ('^'+[regex]::Escape($sid)+'\s')})
        $end=@($o.exit_list.out -split "`n" | Where-Object {$_ -match ('^'+[regex]::Escape($sid)+'\s')})
        if($line.Count -ne 1 -or $line[0] -notmatch 'pid=([1-9]\d*)' -or $end.Count -ne 1 -or $end[0] -notmatch 'exited=true'){$o.known=$true;$o.value=$false;$o.reason='healthy list observer: missing child or child not exited';return $o}
        $seatPid=[int64]([regex]::Match($line[0],'pid=([1-9]\d*)').Groups[1].Value)
        $evs=@(Get-WinEvent -FilterHashtable @{LogName='Security';Id=4689;StartTime=$seatStart.ToLocalTime()} -ErrorAction Stop)
        foreach($ev in $evs){[xml]$xml=$ev.ToXml();$data=@{};foreach($d in $xml.Event.EventData.Data){$data[[string]$d.Name]=[string]$d.'#text'}
            if([Convert]::ToInt64($data.ProcessId,16) -eq $seatPid){$o.exit_status=[Convert]::ToInt64($data.Status,16);$o.audit+=@($ev.ToXml())}
        }
        if($null -eq $o.exit_status){$o.error='seat exit audit missing';return $o}
        $o.known=$true;$o.value=($o.screen.out.Contains($nonce) -and $o.exit_status -eq 0)
    }catch{$o.known=$false;$o.value=$null;$o.error=$_.Exception.Message}
    finally{Save-Json ($Tag+'-seat.json') $o 30}
    return $o
}
function Capture-RequiredChannels {
    $inventory=Get-Content -LiteralPath (Join-Path $PSScriptRoot 'log-inventory.json') -Raw | ConvertFrom-Json -ErrorAction Stop
    Save-Json 'log-inventory.json' $inventory 30
    $capture=[ordered]@{product_sha=$RUN.inputs.product_sha;reading=$cells.reading;interval=[ordered]@{start=([DateTimeOffset]::Parse($RUN.started).ToUnixTimeMilliseconds()/1000.0);end=([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()/1000.0)};channels=@()}
    foreach($binding in $inventory.channels){
        $row=[ordered]@{id=$binding.id;bound=$false;producer_pid=$null;sink=$binding.sink;collection_ok=$false;error='required producer/channel is not captured; filenames are not binding proof'}
        # Only directly observed redirected streams can be bound here. Other product-created
        # processes retain their real startup behavior; absent stdout/stderr stays UNMEASURED.
        if($binding.id -in @('installer-base:stdout','installer-base:stderr')){
            $proc=$RUN.base_install;$stream='out';$complete=$proc.stdout_complete
            if($binding.id -eq 'installer-base:stderr'){$stream='err';$complete=$proc.stderr_complete}
            if($binding.sink -eq ('capture:installer-base:'+ $stream) -and $binding.source_ref -and $inventory.product_sha -eq $RUN.inputs.product_sha -and $proc.pid -gt 0 -and $complete -eq $true){
                $name='installer-base-'+$stream+'.stream';$file=Join-Path $global:DiagOut $name
                $rawKey='stdout_bytes';if($stream -eq 'err'){$rawKey='stderr_bytes'}
                [IO.File]::WriteAllBytes($file,[Convert]::FromBase64String([string]$proc[$rawKey]))
                $row=[ordered]@{id=$binding.id;bound=$true;producer_pid=$proc.pid;sink=$binding.sink;producer_start=$proc.started_epoch;producer_end=$proc.ended_epoch;start=$proc.started_epoch;end=$proc.ended_epoch;collection_ok=$true;copy=$name;bytes=(Get-Item $file).Length;offset=0;end_offset=(Get-Item $file).Length;sha256=(Get-FileHash $file -Algorithm SHA256).Hash.ToLowerInvariant();error=$null}
            }
        }
        $capture.channels+=@($row)
    }
    Save-Json 'channel-capture.json' $capture 30
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
    $o.complete=$false;$o.errors+=@('supplemental file census only; required channel inventory decides completeness')
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
    foreach($name in @('input-lock.json','permit.json','build-metadata.json')){Save-Json $name (Get-Content -LiteralPath (Join-Path $global:DiagOut $name) -Raw | ConvertFrom-Json) 60}
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
    do{$baseline=Read-State '0.14.52' 'base';if(State-Ready $baseline){break};Start-Sleep 5}while((Get-Date) -lt $until)
    $cells.baseline=$null;if($baseline.versions.known -and $baseline.daemon.known -and $baseline.pack.known){$cells.baseline=[bool](State-Ready $baseline)}
    $RUN.base_images=Image-Facts;$RUN.baseline=$baseline;$RUN.base_logs=Capture-Logs 'base'
    if($cells.baseline -ne $true){$RUN.baseline_timeout=$true;throw 'BASELINE_STOP'}
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
    $RUN.launch=ConvertTo-CallRecord $call;Save-Json 'launch.json' $RUN.launch 10;$cells.launch_ok=[bool]$call.Ok;$cells.launch_known=($call.Completed -and -not $call.TimedOut)
    $cells.launch_blocked=($cells.launch_known -and -not $call.Ok -and $call.LastError -eq 4551)
    # Pinned tauri-plugin-updater2.10.1 updater.rs:855-865 ignores ShellExecuteW result then exits0.
    # A returned failure therefore ends the old app too; a still-pending call does not.
    if($cells.launch_known){Start-Sleep -Milliseconds 300;if(-not $baseApp.HasExited){Stop-Process -Id $baseApp.Id -Force}}
    # A pending ShellExecute is not settled; never dismiss a dialog and call it a measured block.
    if(-not $cells.launch_known){throw 'UNMEASURED: ShellExecute still pending'}
    $until=(Get-Date).AddSeconds(600)
    if($call.Ok){do{$audit=Installer-Exit $script:ApplyStart $dest;if($audit.complete){break};Start-Sleep 3}while((Get-Date) -lt $until);$cells.installer_exit=$audit.exit}
    else{$RUN.launch_refused=$true;Save-Json 'installer-process-evidence.json' ([ordered]@{not_launched=$true;launch=$RUN.launch;exit=$null}) 20}
    $RUN.images_after=Image-Facts;$cells.files_changed=$false
    foreach($n in @('cys-app.exe','cys.exe','cysd.exe')){if($RUN.images_after[$n].sha256 -ne $RUN.base_images[$n].sha256){$cells.files_changed=$true}}
    $until=(Get-Date).AddSeconds(300)
    do{$after=Read-State '0.14.53' 'after';if(State-Ready $after){break};if(-not $call.Ok){break};Start-Sleep 5}while((Get-Date) -lt $until)
    $RUN.after=$after
    $RUN.seat=Seat-Check 'after'
    Start-Sleep 60
    $RUN.final_state=Read-State '0.14.53' 'final'
    if($cells.reading -eq 'R2'){
        $cells.old_survives=(-not $baseApp.HasExited)
        if(-not $cells.files_changed){
            # Explicit re-launch of the unchanged old image is different from surviving PID.
            $restart=Invoke-ProbeCreate (Join-Path $script:Install 'cys-app.exe');$RUN.old_restart_probe=$restart
            if($restart.completed -ne $true){$RUN.old_restart_observer_error="probe incomplete"}
            if($restart.ok){
                $oldProcess=Start-Process -FilePath (Join-Path $script:Install 'cys-app.exe') -WorkingDirectory $script:Install -PassThru
                Start-Sleep 5;$oldState=Read-State '0.14.52' 'old-restart';$oldSeat=Seat-Check 'old-restart'
                $RUN.old_restart_state=$oldState;$RUN.old_restart_seat=$oldSeat
            }
        }
        $st=Get-SacRealState 'after-observation';$RUN.sac_after=$st
        $cells.sac_after=($st.registry.value -eq 1 -and $st.signals.citool_desktop_enforced -and $st.signals.defender_on -eq $true -and $st.signals.umci_flag)
        Save-Json 'r2-state.json' ([ordered]@{probe=$RUN.old_restart_probe;state=$RUN.old_restart_state;seat=$RUN.old_restart_seat;sac_before=$RUN.sac_before;sac_after=$RUN.sac_after;negative=$RUN.negative;positive=$RUN.positive;old_survives=$cells.old_survives}) 60
    }
    Capture-RequiredChannels
    $RUN.final_logs=Capture-Logs 'final'
    $cells.logs_complete=$false;$cells.tracebacks=$null # required channel validator, not supplemental census, sets these
    $RUN.events=Get-CIEvents $script:ApplyStart;Save-Json 'codeintegrity.json' $RUN.events 20
    $cells.evidence_complete=$false # finalizer verifies mandatory read-back/hash manifest
    Save-Run;Publish-Checkpoint 'after-observation'
}catch{if($_.Exception.Message -ne 'BASELINE_STOP'){$cells.harness_error=$_.Exception.Message};$RUN.error=$_.Exception.ToString();try{Save-Run}catch{Write-Error ('required evidence write failed: '+$_.Exception.Message) -ErrorAction Continue}}
finally {
    if($script:SacTouched){$RUN.restore=Restore-SacReal 'final';$cells.restored=$RUN.restore.restored;if(-not $cells.restored){$RUN.restore_fallback=Invoke-CmdRestoreFallback}}
    elseif($cells.reading -eq 'R2'){$cells.restored=$false}
    $RUN.finished=Get-IsoNow;Save-Run
    # Preserve processes and files until observations are saved. Disposable runner teardown cleans them.
    $env:GITHUB_TOKEN=$script:GhToken
}
& $CTX.driver_node (Join-Path $PSScriptRoot 'finish-measurement.mjs') $global:DiagOut
exit $LASTEXITCODE
