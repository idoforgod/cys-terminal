"""Source-bound integration checks; these do not claim native PowerShell execution."""
from pathlib import Path
import yaml
s=Path('diag/win-measure.ps1').read_text()
w=yaml.load(Path('.github/workflows/win-measure.yml').read_text(),Loader=yaml.BaseLoader)
steps=w['jobs']['measure']['steps']
names=[x.get('name','') for x in steps]
assert names.index('Download base and authorize exact installer bytes') < names.index('Observe requested update path (result is JSON, not this step color)')
assert any(x.get('with',{}).get('artifact-ids')=='${{ needs.inputs.outputs.artifact_id }}' for x in steps)
assert all('continue-on-error' not in x for x in steps)
assert s.index('$inputLock=Must-Permit')<s.index('$RUN.base_install=')
assert s.index('$null=Must-Permit',s.index('$RUN.base_install='))<s.index('$dest=Join-Path')
assert "if($cells.launch_known){Start-Sleep -Milliseconds 300;if(-not $baseApp.HasExited){Stop-Process -Id $baseApp.Id -Force}}" in s
assert "if($call.Ok){Start-Sleep -Milliseconds 300" not in s
assert "try{$env:CYS_NO_AUTOSTART='1';return (Invoke-Proc $File $Arguments $TimeoutSec)}" in s
assert 'Invoke-Proc $cli ' not in s
assert 'Invoke-Cli $cli ' in s
assert "if($script:SacTouched){$RUN.restore=Restore-SacReal 'final'" in s
assert "Save-Json 'sac-touched.json'" in s
assert "$o.exit_status -eq 0" in s and "if($null -eq $o.exit_status)" in s
assert "return $x" in s[s.index('function Must-Permit'):s.index('function Invoke-Cli')]
print('PASS: authorization order, immutable artifact ID, plugin exit semantics, observer no-autostart, SAC restore, seat status (source checks only)')

assert "driver-observation.mjs" in s and "finish-measurement.mjs" in s
assert "$RUN.old_restart_seat=$oldSeat" in s
assert "$cells.evidence_complete=$true" not in s
assert "function Save-Json" in s and "evidence read-back failed" in s
assert "Capture-RequiredChannels" in s
assert "filename" in s or "filenames" in s
assert "versions=$false;daemon=$false" not in s

# R1-GAPS: source-bound reachability checks, not native PowerShell execution.
assert 'observer_schema=2' in s
start=s.index('try {\n    $inputLock=Must-Permit')
body=s[start:]
assert 'Capture-RequiredChannels;$script:InventoryReady=$true' in body, 'early capture must be persisted before base installer'
assert body.index('Capture-RequiredChannels;$script:InventoryReady=$true') < body.index('$RUN.base_install=Invoke-Proc')
assert body.index('$RUN.base_pack_absent_before_install=') < body.index('$RUN.base_install=Invoke-Proc')
pack=s[s.index('function Pack-Facts'):s.index('function App-Version')]
for guard in ["$Tag -eq 'base'", "$Version -eq '0.14.52'", '$RUN.base_pack_absent_before_install -eq $true', '$RUN.base_install.rc -eq 0']:
    assert guard in pack, 'absence route needs '+guard
assert "driver-observation.mjs') pack" in pack
assert '$raw.pack=Pack-Facts $Version $Tag' in s
assert "if($script:InventoryReady){try{Capture-RequiredChannels}" in body[body.index('finally {'):]
assert "Save-Json 'log-inventory.json'" in s and "Save-Json 'channel-capture.json'" in s
assert "status=$status;bound=$false" in s
assert "$script:Phases.baseline='stopped';throw 'BASELINE_STOP'" in s
assert 'pack-repair-channel' not in pack
command=next(x['run'] for x in w['jobs']['inputs']['steps'] if x.get('name')=='Contract negative controls (no installers launched)')
assert 'diag/pack-observation.test.mjs' in command and 'diag/cdp-discovery.test.mjs' in command
print('PASS GAPS source wiring: early/final capture, fresh-base route guard, shared pack observer and CI fixture routing; native PS UNMEASURED')
