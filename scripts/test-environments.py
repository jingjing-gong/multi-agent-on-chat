#!/usr/bin/env python3
"""Isolated launcher integration; never contacts channels or production data."""
import hashlib,json,os,subprocess,sys,tempfile,time
from pathlib import Path

repo=Path(__file__).resolve().parent.parent
binary=repo/'target/debug/mac-cli';launcher=repo/'scripts/mac-launcher.py'
with tempfile.TemporaryDirectory(prefix='mac-env-test-') as temp:
    root=Path(temp);install=root/'install';env={**os.environ,'MAC_INSTALL_ROOT':str(install),'MAC_ENV':'dev'}
    for key in ['MAC_DATA_DIR','MAC_VERSION','MAC_INSTANCE_ID','MAC_BOUND_DATA_DIR','MAC_INTERNAL_TASK']:env.pop(key,None)
    def call(*args,ok=True,extra=None):
        r=subprocess.run([sys.executable,str(launcher),*args],env={**env,**(extra or {})},capture_output=True,text=True,timeout=35)
        assert (r.returncode==0)==ok,(args,r.stdout,r.stderr)
        return json.loads(r.stdout) if ok else r.stderr
    manifest=call('release','install',str(binary));release=manifest['release_id']
    call('env','register','dev','--release',release,'--data-dir',str(root/'dev'))
    call('env','register','test','--release',release,'--data-dir',str(root/'test'))
    call('init');call('init',extra={'MAC_ENV':'test'})
    call('bot','add','local','--name','devbot')
    call('bot','add','local','--name','testbot',extra={'MAC_ENV':'test'})
    agents=call('agent','list',extra={'MAC_ENV':'test'})
    other_workspace=agents[0]['workspace']
    call('agent','create','overlap','--bot','devbot','--template','default@1','--workspace',other_workspace,ok=False)

    call('status',ok=False,extra={'MAC_VERSION':'../../oops'})
    call('status',ok=False,extra={'MAC_DATA_DIR':str(root/'test')})
    call('release','install',str(binary),ok=False)
    before=hashlib.sha256((root/'test/runtime.sqlite3').read_bytes()).hexdigest()
    result=call('service','start','--mock');assert result['state']=='running'
    try:
        status=call('status');assert status['daemon']['identity']['release']==release
        assert call('service','drain')['ready'] is True
        call('init',ok=False)
        assert call('service','resume')['draining'] is False
        call('service','start','--mock',ok=False)
        call('service','restart','--mock','--timeout','10')
    finally:
        call('stop')
        for _ in range(100):
            daemon=call('env','show')
            if not daemon['running']:break
            time.sleep(.1)
        else:raise AssertionError('test service did not stop')
    assert hashlib.sha256((root/'test/runtime.sqlite3').read_bytes()).hexdigest()==before
    # Preflight must reject managed data even when bypassing launcher.
    direct=subprocess.run([str(binary),'--data-dir',str(root/'dev'),'status'],capture_output=True,env={k:v for k,v in env.items() if k!='MAC_ENV'})
    assert direct.returncode!=0
    print('PASS: environment selection, immutable install, isolation, mock start/restart/drain, managed-data guard')
