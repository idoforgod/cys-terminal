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
assert "$o.exit_status -eq 0" in s and "$null -ne $o.exit_status" in s
assert "return $x" in s[s.index('function Must-Permit'):s.index('function Invoke-Cli')]
print('PASS: authorization order, immutable artifact ID, plugin exit semantics, observer no-autostart, SAC restore, seat status (source checks only)')
