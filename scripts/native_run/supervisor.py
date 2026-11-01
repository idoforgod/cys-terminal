"""Native owned-job supervisor. Phase timeouts cannot be reset by heartbeats.

Only the explicit job entry point invokes this. Pure deadline tests launch no
process and never claim a Windows lifetime measurement.
"""
import datetime
import os
from pathlib import Path
import sys
import time
from boundary import OwnedProcess, Unavailable, cleanup_valid
from protocol import atomic_json, read_json

CAPS={'setup':600,'network_boundary':180,'auth_free_startup':240,'origin_calibration':60,
      'genuine_open_query':180,'no_submit_calibration':240,'collection':300}

class PhaseClock:
    def __init__(self,start):self.start=start;self.phase=None;self.deadline=start+600;self.seen=set()
    def observe(self,record,now):
        if type(record) is not dict or record.get('phase') not in CAPS:raise Unavailable('unknown phase')
        phase=record['phase'];stamp=record.get('monotonic_ns')
        if type(stamp) is not int or stamp<0:raise Unavailable('phase timestamp type')
        began=stamp/1e9
        if began<self.start or began>now:raise Unavailable('phase timestamp outside supervisor interval')
        if phase!=self.phase:
            if self.phase is None and phase!='setup':raise Unavailable('missing setup phase')
            if phase in self.seen:raise Unavailable('phase repetition refused')
            if self.phase is not None and began>self.deadline:raise Unavailable('prior phase exceeded cap')
            self.seen.add(phase);self.phase=phase;self.deadline=(self.start+600 if phase=='setup' else began+CAPS[phase])
        # Same-phase publications cannot extend a deadline.
        return min(self.deadline,self.start+1500)

class PhasePublisher:
    def __init__(self,path):self.path=Path(path);self.seen=set();self.history=[]
    def enter(self,name):
        if name not in CAPS or name in self.seen:raise Unavailable('invalid/repeated phase')
        self.seen.add(name)
        self.history.append({'phase':name,'monotonic_ns':time.monotonic_ns()})
        atomic_json(self.path,self.history)

def run_owned_worker(argv,evidence,latest_end,job_start):
    if sys.platform!='win32':raise Unavailable('native supervisor requires Windows')
    root=Path(evidence).resolve()
    elapsed=(datetime.datetime.now(datetime.timezone.utc)-datetime.datetime.fromisoformat(job_start)).total_seconds()
    if not 0<=elapsed<600:raise Unavailable('setup allocation exhausted before worker')
    clock=PhaseClock(time.monotonic()-elapsed);cursor=0;child=None;code=None;failure=None
    end=datetime.datetime.fromisoformat(latest_end)
    # No credential variables are passed to the controller or its build children.
    names=('SystemRoot','WINDIR','PATH','PATHEXT','COMSPEC','TEMP','TMP','USERPROFILE','HOME',
           'LOCALAPPDATA','APPDATA','CARGO_HOME','RUSTUP_HOME','PROCESSOR_ARCHITECTURE',
           'NUMBER_OF_PROCESSORS','ProgramFiles','ProgramFiles(x86)','ProgramData')
    env={k:os.environ[k] for k in names if k in os.environ}
    env.update(PYTHONUTF8='1',PYTHONDONTWRITEBYTECODE='1',PYTHONNOUSERSITE='1')
    try:
        child=OwnedProcess(sys.executable,['-B',*argv],env,Path.cwd(),process_limit=128)
        while True:
            now=time.monotonic();phase=root/'phase.json'
            if phase.exists():
                history=read_json(phase)
                if type(history) is not list or len(history)<cursor:raise Unavailable('phase history truncated')
                for row in history[cursor:]:clock.observe(row,now)
                cursor=len(history)
            deadline=min(clock.deadline,clock.start+1500)
            if now>=deadline or datetime.datetime.now(datetime.timezone.utc)>=end:
                raise Unavailable('owned worker deadline: '+str(clock.phase))
            code=child.wait(50)
            if code is not None:break
    except (OSError,ValueError,Unavailable) as error:failure=str(error)
    finally:
        if child:child.close()
        receipt={'exit_code':code,'failure':failure,'last_phase':clock.phase,
                 'cleanup':None if child is None else child.cleanup,'release_verdict':'NONE'}
        atomic_json(root/'supervisor.json',receipt)
    if failure or code!=0 or not cleanup_valid(receipt['cleanup']):raise Unavailable('owned worker incomplete; see supervisor.json')
    return receipt
