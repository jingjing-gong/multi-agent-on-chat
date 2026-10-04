# Verify context filtering with a local fake provider; no real model calls.
# CODEX_BINARY may select a specific installed Codex version.
import json,os,pathlib,tempfile,subprocess,threading,queue,select,http.server
captured=queue.Queue()
class Handler(http.server.BaseHTTPRequestHandler):
 def do_POST(self):
  body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
  captured.put(body);self.send_response(400);self.end_headers();self.wfile.write(b'{"error":{"message":"fixture stop","type":"invalid_request_error"}}')
 def log_message(self,*args):pass
server=http.server.HTTPServer(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
def rpc(p,i,method,params):
 p.stdin.write(json.dumps({'id':i,'method':method,'params':params})+'\n');p.stdin.flush()
 while True:
  if not select.select([p.stdout],[],[],20)[0]:raise RuntimeError('timeout '+method)
  v=json.loads(p.stdout.readline())
  if v.get('id')==i:
   if 'error'in v:raise RuntimeError(v['error'])
   return v['result']
with tempfile.TemporaryDirectory() as root:
 root=pathlib.Path(root);home=root/'home';home.mkdir();work=root/'work';work.mkdir()
 for name in ['include-fixture','exclude-fixture']:
  d=work/'.agents/skills'/name;d.mkdir(parents=True);(d/'SKILL.md').write_text(f'---\nname: {name}\ndescription: {name}-unique-description\n---\nfixture')
 (home/'config.toml').write_text(f'model = "gpt-5.4"\nmodel_provider = "fixture"\n[model_providers.fixture]\nname = "fixture"\nbase_url = "http://127.0.0.1:{server.server_port}/v1"\nwire_api = "responses"\nrequires_openai_auth = false\n')
 env=os.environ.copy();env['CODEX_HOME']=str(home)
 def start(overrides=None):
  args=[os.environ.get('CODEX_BINARY', 'codex'),'app-server']
  if overrides:args+=['-c','skills.config='+overrides]
  p=subprocess.Popen(args,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,env=env,cwd=work)
  try:
   rpc(p,1,'initialize',{'clientInfo':{'name':'probe','version':'1'},'capabilities':{'experimentalApi':True}});return p
  except BaseException:
   p.kill();p.wait();raise
 p=start()
 try:
  allskills=rpc(p,2,'skills/list',{'cwds':[str(work)],'forceReload':True})['data'][0]['skills']
 finally:p.kill();p.wait()
 overrides='['+','.join('{path='+json.dumps(s['path'])+',enabled='+str(s['name']=='include-fixture').lower()+'}'for s in allskills)+']'
 for label,config in [('inherit',None),('allowlist',overrides)]:
  p=start(config)
  try:
   thread=rpc(p,3,'thread/start',{'cwd':str(work),'approvalPolicy':'never'})['thread']['id']
   rpc(p,4,'turn/start',{'threadId':thread,'input':[{'type':'text','text':'Reply OK','text_elements':[]}]})
   body=captured.get(timeout=20);text=json.dumps(body,ensure_ascii=False)
   result={'mode':label,'include': 'include-fixture-unique-description' in text,'exclude':'exclude-fixture-unique-description' in text,'request_characters':len(text)}
   print(json.dumps(result),flush=True)
   assert result['include'];assert result['exclude']==(label=='inherit')
  finally:p.kill();p.wait()
server.shutdown()
