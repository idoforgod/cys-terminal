"""Runs INSIDE the product-created ConPTY shell. Never execute without external authority."""
import argparse
import datetime
import time
from pathlib import Path
from protocol import read_json, atomic_json
from controller import authorize, ReceiverController


def main():
    p=argparse.ArgumentParser();p.add_argument('spec');p.add_argument('authority');p.add_argument('authority_sha256');a=p.parse_args()
    spec=read_json(a.spec)
    grant=authorize(a.authority,a.authority_sha256,mode=spec['mode'],diagnostic_sha=spec['diagnostic_sha'],product_sha=spec['product_sha'])
    root=Path(spec['run_root']);start=root/'evidence'/'release-receiver'
    deadline=time.monotonic()+30
    # A controller release lets raw observation attach before the real receiver starts.
    while not start.is_file():
        if time.monotonic()>=deadline:raise RuntimeError('receiver release was not supplied')
        time.sleep(.01)
    controller=ReceiverController(grant,spec)
    try:
        receipt=controller.launch();atomic_json(root/'evidence'/'boundary-start.json',receipt)
        stop=root/'evidence'/'stop-receiver'
        end=datetime.datetime.fromisoformat(grant.latest_end_utc)
        while not stop.is_file() and datetime.datetime.now(datetime.timezone.utc)<end:
            controller.child.sample_lineage()
            code=controller.child.wait(100)
            if code is not None:
                atomic_json(root/'evidence'/'receiver-exit.json',{'code':code});break
    finally:
        atomic_json(root/'evidence'/'boundary-finish.json',controller.finish())

if __name__=='__main__':main()
