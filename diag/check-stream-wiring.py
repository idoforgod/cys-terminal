"""Source integration assertions only; not execution of native PowerShell."""
from pathlib import Path
import sys
s=Path(sys.argv[1] if len(sys.argv)>1 else 'diag/win-measure.ps1').read_text()
seat=s[s.index('function Seat-Check'):s.index('function Capture-RequiredChannels')]
assert '$creation=Observe-ProcessOutput $o.create' in seat, 'Seat-Check must test complete create stdout before reading out'
assert 'if(-not $creation.known){$o.error=$creation.error;return $o}' in seat, 'unknown create output must return UNKNOWN'
assert seat.index('$creation=Observe-ProcessOutput')<seat.index('$sid=$o.create.out.Trim()'), 'create completeness precedes parsing'
assert "foreach($name in @('screen','list','exit_list'))" in seat
assert 'if(-not $stream.known){$o.error=$name' in seat, 'unknown screen/list output must return UNKNOWN'
assert seat.index('$stream=Observe-ProcessOutput')<seat.index('$line=@'), 'screen/list completeness precedes missing-child decision'
assert 'Invoke-ProbeCreate' not in seat, 'image-admission probe must not replace missing output'
print('PASS source wiring: actual Seat-Check gates new-surface, screen, list and exit_list through shared process reducer')
