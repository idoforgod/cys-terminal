"""Private fixture RPC. No defaults/autostart/fallback to an installed daemon."""
import argparse
import json
import re
import socket
import subprocess
import sys
from pathlib import Path
from protocol import atomic_json, read_json
from boundary import Unavailable

METHODS={'system.identify','surface.create','surface.list','surface.set_meta','surface.send_text',
         'surface.send_key','surface.read_text','surface.resize','surface.close'}

def validate_endpoint(endpoint, root, run_id):
    if not re.fullmatch('[0-9a-f]{32}',run_id):raise Unavailable('private RPC identity')
    if sys.platform=='win32':
        if endpoint != r'\\.\pipe\cys-native-'+run_id:raise Unavailable('not this private named pipe')
    elif Path(endpoint) != Path(root)/('cys-native-'+run_id+'.sock'):
        raise Unavailable('not this private socket')

def exchange(endpoint, request):
    """Called only in an owned bounded worker; blocking Windows pipe I/O cannot hang the controller."""
    if request.get('method') not in METHODS or type(request.get('params')) is not dict:raise Unavailable('RPC method/params refused')
    packet=json.dumps(request,ensure_ascii=False).encode()+b'\n'
    if sys.platform=='win32':
        stream=open(endpoint,'r+b',buffering=0)
    else:
        connection=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);connection.settimeout(5);connection.connect(endpoint)
        stream=connection.makefile('rwb',buffering=0)
    try:
        stream.write(packet)
        line=stream.readline(1024*1024+1)
        if not line.endswith(b'\n') or len(line)>1024*1024:raise Unavailable('RPC truncated/oversized')
        reply=json.loads(line)
        if type(reply) is not dict or reply.get('id')!=request.get('id') or type(reply.get('ok')) is not bool:
            raise Unavailable('RPC identity/types')
        return reply
    finally:
        stream.close()
        if sys.platform!='win32':connection.close()

class FixtureRPC:
    def __init__(self, endpoint, root, run_id, daemon_pid):
        validate_endpoint(endpoint,root,run_id)
        self.endpoint=endpoint;self.root=Path(root);self.run_id=run_id;self.daemon_pid=daemon_pid;self.sequence=0
    def call(self,method,params,timeout=10):
        if method not in METHODS:raise Unavailable('RPC method not allowed')
        self.sequence+=1;request={'id':self.sequence,'method':method,'params':params}
        path=self.root/'evidence'/f'rpc-{self.sequence:06}.request.json';atomic_json(path,request)
        args=[sys.executable,'-B',str(Path(__file__).resolve()),'--worker',self.endpoint,str(path)]
        try:run=subprocess.run(args,capture_output=True,timeout=timeout)
        except subprocess.TimeoutExpired as e:raise Unavailable('owned RPC worker deadline') from e
        (self.root/'evidence'/f'rpc-{self.sequence:06}.stdout').write_bytes(run.stdout)
        (self.root/'evidence'/f'rpc-{self.sequence:06}.stderr').write_bytes(run.stderr)
        if run.returncode:raise Unavailable('owned RPC worker failed')
        reply=json.loads(run.stdout)
        if reply.get('ok') is not True:raise Unavailable('product refused RPC: '+str(reply.get('error')))
        result=reply.get('result')
        if method=='system.identify' and (type(result) is not dict or result.get('daemon_pid')!=self.daemon_pid):
            raise Unavailable('connected daemon is not the owned process')
        return result

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--worker',action='store_true');p.add_argument('endpoint');p.add_argument('request');a=p.parse_args()
    if not a.worker:raise SystemExit('private worker only')
    print(json.dumps(exchange(a.endpoint,read_json(a.request)),ensure_ascii=False))
