import subprocess,json,tempfile,time,pathlib
root=pathlib.Path(tempfile.mkdtemp(prefix='mac-cli-acceptance-'))
project=pathlib.Path(__file__).resolve().parents[1]
exe=str(project/'target/debug/mac-cli')
def cli(*args):
 p=subprocess.run([exe,'--data-dir',str(root),*args],capture_output=True,text=True,check=True)
 return json.loads(p.stdout)
cli('init'); cli('template','register',str(project/'examples/researcher.yaml'))
for a in ['a','b']:
 (root/a).mkdir();cli('bot','add','local','--name',a)
 cli('agent','create',a,'--bot',a,'--template','researcher@1','--workspace',str(root/a))
child=cli('chat','a','/ask b explain the design -r')
parent=cli('task',child['parent_id']);assert parent['state']=='waiting_dependency'
first=cli('submit','a','first','--scope','steer-test')
second=cli('submit','a','correction','--scope','steer-test');assert first['id']==second['id']
log=open(root/'service.log','w')
service=subprocess.Popen([exe,'--data-dir',str(root),'serve','--mock','--workers','2'],stdout=log,stderr=log)
try:
 for _ in range(100):
  tasks=cli('tasks')
  if all(t['state']=='succeeded' for t in tasks):break
  time.sleep(.1)
 else:raise AssertionError(tasks)
 assert 'Agent response' in cli('task',parent['id'])['result']
 assert 'correction' in cli('task',first['id'])['result']
 assert cli('status')['daemon']['workers']==2
 cli('stop');service.wait(timeout=5)
 assert service.returncode==0
 assert cli('status')['daemon']['state']=='stopped'
 print(json.dumps({'passed':True,'data_dir':str(root),'tasks':len(tasks),'checks':['query return continuation','steer','CLI stop','daemon status']}))
finally:
 if service.poll() is None:service.terminate();service.wait()
