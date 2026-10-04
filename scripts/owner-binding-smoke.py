import tempfile,pathlib,subprocess,os,json
root=pathlib.Path(tempfile.mkdtemp(prefix='mac-onboarding-test-'))
fake=root/'lark-cli'
fake.write_text('''#!/usr/bin/env python3
import sys,json,os,pathlib
args=sys.argv[1:]
assert "test-only-secret" not in args
if args[:2]==["config","init"]:
 assert sys.stdin.read().strip()=="test-only-secret"
 print('{}')
elif args[:2]==["auth","status"]:
 print(json.dumps({"identity":"bot","verified":True,"appId":"cli_test","brand":"feishu","identities":{"bot":{"status":"ready","available":True,"verified":True,"openId":"ou_bot"}}}))
elif args[0]=="whoami":print(json.dumps({"identity":"bot","appId":"cli_test","brand":"feishu"}))
elif args[0]=="event":
 import time
 for typ,sender,id in [("group","ou_wrong","group"),("p2p","ou_owner","first"),("p2p","ou_owner","first")]:
  print(json.dumps({"type":"im.message.receive_v1","app_id":"cli_test","sender_type":"user","sender_id":sender,"chat_type":typ,"chat_id":"oc_chat","message_id":id,"message_type":"text","content":"/sh must-not-run"}),flush=True)
 for mid,text in [("command","/bot list"),("raw-command","/bot list -r"),("help","/help")]:
  print(json.dumps({"type":"im.message.receive_v1","app_id":"cli_test","sender_type":"user","sender_id":"ou_owner","chat_type":"p2p","chat_id":"oc_chat","message_id":mid,"message_type":"text","content":text}),flush=True)
 time.sleep(30)
elif args[0]=="im":
 assert args[1]=="+messages-reply"
 assert args[args.index("--message-id")+1] in ["first","command","raw-command","help"]
 assert args[args.index("--as")+1]=="bot"
 if args[args.index("--message-id")+1]=="help":
  rows=json.loads(args[args.index("--content")+1])["zh_cn"]["content"]
  assert rows[0][0]["text"]=="/help"
  assert all(span["tag"]=="text" for row in rows for span in row)
  assert any("/profile [list | 名称 | default]"==span["text"] for row in rows for span in row)
  kind="help"
 elif "--text" in args:
  assert json.loads(args[args.index("--text")+1])["bots"][0]["id"]=="test"
  kind="raw"
 else:
  assert args[args.index("--msg-type")+1]=="post"
  body=json.loads(args[args.index("--content")+1])["zh_cn"]["content"][0][0]
  rows=json.loads(args[args.index("--content")+1])["zh_cn"]["content"]
  text=chr(10).join("".join(span["text"] for span in row) for row in rows)
  assert "owner 设置成功" in text or ("Bot：test" in text and "| ---" not in text)
  kind="owner" if "owner 设置成功" in text else "markdown"
 with pathlib.Path(os.environ["CHECK_SENT"]).open("a") as f:f.write(kind+chr(10))
 print('{"code":0}')
else:raise AssertionError(args)
''')
fake.chmod(0o700)
env=os.environ.copy();env['PATH']=str(root)+':'+env['PATH'];env['CHECK_SENT']=str(root/'sent')
exe=str(pathlib.Path('target/debug/mac-cli').resolve())

import time
def cli(*args):
 return subprocess.run([exe,'--data-dir',str(root/'data'),*args],capture_output=True,text=True,env=env,check=True)
cli('bot','add','lark','--name','test','--app-id','cli_test','--app-secret','test-only-secret')
log=(root/'service.log').open('w')
service=subprocess.Popen([exe,'--data-dir',str(root/'data'),'serve','--workers','1'],stdout=log,stderr=log,env=env)
try:
 for _ in range(100):
  if (root/'sent').exists() and len((root/'sent').read_text().splitlines())==4:break
  time.sleep(.1)
 else:raise AssertionError((root/'service.log').read_text())
 bot=json.loads(cli('bot','show','test').stdout)
 assert bot['owner']=='ou_owner' and bot['enabled']
 assert json.loads(cli('tasks').stdout)==[]
 assert len(json.loads(cli('deliveries').stdout))==4
 assert set((root/'sent').read_text().splitlines())=={'owner','markdown','raw','help'}
 cli('stop');service.wait(timeout=5)
 assert service.returncode==0
 print('PASS: pending bot listener, private-only owner binding, deduplicated acknowledgement; no Agent/task or real send')
finally:
 if service.poll() is None:service.terminate();service.wait()
 log.close()
