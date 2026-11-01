"""Collect only registered owned roots; never scan host TEMP or user profiles."""
from pathlib import Path
import re
import shutil
from protocol import atomic_json,read_json,sha
from boundary import Unavailable

def register(root,config):
    output=Path(config['artifact_root']);registry=output/'owned-roots.json'
    roots=read_json(registry) if registry.exists() else []
    value=str(Path(root).resolve())
    if value in roots:raise Unavailable('owned root registered twice')
    atomic_json(registry,[*roots,value])

def collect(output):
    output=Path(output);registry=output/'owned-roots.json';rows=[];errors=[]
    if not registry.exists():
        result={'roots':[],'errors':['no owned roots registered'],'release_verdict':'NONE'}
        atomic_json(output/'archive.json',result);return result
    for name in read_json(registry):
        root=Path(name)
        try:
            if not re.fullmatch('cys-native-[0-9a-f]{32}',root.name) or root.is_symlink() or root.resolve()!=root:raise Unavailable('unowned registry root')
            marker=read_json(root/'owned-run.json')
            if marker.get('run_id')!=root.name.removeprefix('cys-native-') or marker.get('kind')!='native_fixture':raise Unavailable('root marker mismatch')
            sources=[root/'owned-run.json']
            for sub in ('evidence','trace'):
                directory=root/sub
                if directory.exists():sources.extend(p for p in directory.rglob('*') if p.is_file())
            origin=list((root/'child').glob('origin.*.json'))
            sources.extend(origin)
            journal=root/'child/origin.native.jsonl'
            # The helper writes this before publishing any origin JSON. Include
            # the exact owned path even if absent, so copy failure is an error.
            if origin or journal.exists():sources.append(journal)
            for source in sources:
                rel=source.relative_to(root)
                for node in (source,*source.parents):
                    if node==root.parent:break
                    if node.is_symlink() or (hasattr(node,'is_junction') and node.is_junction()):raise Unavailable('reparse in collected artifact')
                if source.name=='origin.input.json' and source.parent==root/'child':
                    error=read_json(source).get('error')
                    marker='operands/result in '
                    if isinstance(error,str) and marker in error and error.split(marker,1)[1]!=str(journal):
                        raise Unavailable('origin input references an unexpected operand journal: '+repr({'expected':str(journal),'observed':error.split(marker,1)[1]}))
                target=output/'fixtures'/root.name/rel;target.parent.mkdir(parents=True,exist_ok=True)
                shutil.copyfile(source,target)
                rows.append({'original':str(source),'path':str(target.relative_to(output)),'sha256':sha(target.read_bytes()),'bytes':target.stat().st_size})
        except (OSError,ValueError,Unavailable) as error:errors.append({'root':name,'error':str(error)})
    result={'artifacts':rows,'errors':errors,'release_verdict':'NONE'};atomic_json(output/'archive.json',result);return result
