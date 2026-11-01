"""Tiny owned Git object graph for source-binding/census controls (no hooks/push)."""
import os,subprocess,pathlib,tempfile,json
class Repo:
    def __init__(self):
        self.temp=tempfile.TemporaryDirectory();self.root=pathlib.Path(self.temp.name)
        self.git('init','-q')
    def git(self,*args,input=None):
        env={**os.environ,'GIT_AUTHOR_NAME':'Fixture','GIT_AUTHOR_EMAIL':'fixture@example.invalid','GIT_COMMITTER_NAME':'Fixture','GIT_COMMITTER_EMAIL':'fixture@example.invalid'}
        return subprocess.check_output(['git','-C',str(self.root),*args],input=input,env=env,stderr=subprocess.PIPE).decode().strip()
    def commit(self,files,parent=None):
        tree={}
        for name,value in files.items():
            node=tree;parts=name.split('/')
            for part in parts[:-1]:node=node.setdefault(part,{})
            node[parts[-1]]=value
        def write(node):
            rows=[]
            for name,value in sorted(node.items()):
                if isinstance(value,dict):mode,kind,oid='040000','tree',write(value)
                else:mode,kind,oid='100644','blob',self.git('hash-object','-w','--stdin',input=value)
                rows.append(mode+' '+kind+' '+oid+'\t'+name+'\n')
            return self.git('mktree',input=''.join(rows).encode())
        return self.git('commit-tree',write(tree),*(['-p',parent] if parent else []),input=b'owned fixture\n')
    def close(self):self.temp.cleanup()
def base_files():
    return {'.github/workflows/'+name:json.dumps({'on':{'push':{'branches':['feat/**','fix/**']}}}).encode() for name in ('ci-branch.yml','windows-build.yml','windows-health.yml')}
