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
elif args[0]=="im":
 assert args[1]=="+messages-send"
 assert args[args.index("--user-id")+1]=="ou_owner"
 assert args[args.index("--as")+1]=="bot"
 assert args[args.index("--msg-type")+1]=="post"
 assert "接入成功" in json.loads(args[args.index("--content")+1])["zh_cn"]["content"][0][0]["text"]
 pathlib.Path(os.environ["CHECK_SENT"]).write_text("yes")
 print('{"code":0}')
else:raise AssertionError(args)
''')
fake.chmod(0o700)
env=os.environ.copy();env['PATH']=str(root)+':'+env['PATH'];env['CHECK_SENT']=str(root/'sent')
exe=str(pathlib.Path('target/debug/mac-cli').resolve())
p=subprocess.run([exe,'--data-dir',str(root/'data'),'bot','add','lark','--name','test','--app-id','cli_test','--app-secret','test-only-secret','--owner','ou_owner'],capture_output=True,text=True,env=env)
assert p.returncode==0,p.stderr
assert 'test-only-secret' not in p.stdout+p.stderr
# config init forwards subprocess stdout, so last JSON object starts at bot record.
assert '"status": "sent"' in p.stdout,p.stdout
assert (root/'sent').read_text()=='yes'
print('PASS: argument input, child stdin only, verified identity, owner-directed bot welcome; no real network/send')

# Add without owner, then bind and explicitly send. No daemon or Agent required.
env['CHECK_SENT']=str(root/'second-sent')
def run(*args):
 return subprocess.run([exe,'--data-dir',str(root/'pending-data'),*args],capture_output=True,text=True,env=env)
p=run('bot','add','lark','--name','pending','--app-id','cli_test','--app-secret','test-only-secret')
assert p.returncode==0 and 'pending_owner' in p.stdout,p.stderr
assert not (root/'second-sent').exists()
p=run('bot','notify','pending');assert p.returncode!=0
assert not (root/'second-sent').exists()
p=run('bot','owner','pending','ou_owner');assert p.returncode==0,p.stderr
p=run('bot','notify','pending');assert p.returncode==0,p.stderr
assert json.loads(p.stdout)['status']=='sent'
assert (root/'second-sent').read_text()=='yes'
print('PASS: pending_owner -> owner -> notify; no daemon, no real message')
# Exercise /bot add through the same application service, using fake Lark auth.
p=run('bot','add','local','--name','controller');assert p.returncode==0,p.stderr
p=run('chat','controller-main',"/bot add lark --name via-chat --app-id cli_second --app-secret 'test-only-secret'")
# Fake auth returns cli_test, so mismatched identity must never register the Bot.
assert p.returncode==0,p.stderr
assert '执行失败' in p.stdout
p=run('bot','show','via-chat');assert p.returncode!=0
p=run('chat','controller-main','/bot disable pending');assert p.returncode==0,p.stderr
p=run('bot','show','pending');assert json.loads(p.stdout)['enabled'] is False
p=run('chat','controller-main','/bot enable pending');assert p.returncode==0,p.stderr
p=run('bot','show','pending');assert json.loads(p.stdout)['enabled'] is True
print('PASS: chat Bot onboarding enforces verified App ID and cross-Bot enable/disable works')
