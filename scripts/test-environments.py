#!/usr/bin/env python3
"""Isolated launcher integration; never contacts channels or production data."""
import hashlib,importlib.util,json,os,sqlite3,subprocess,sys,tempfile,time
from pathlib import Path

repo=Path(__file__).resolve().parent.parent
binary=repo/'target/debug/mac-cli';launcher=repo/'scripts/mac-launcher.py'
with tempfile.TemporaryDirectory(prefix='mac-env-test-') as temp:
    root=Path(temp);install=root/'install';env={**os.environ,'MAC_INSTALL_ROOT':str(install),'MAC_ENV':'dev'}
    spec=importlib.util.spec_from_file_location('mac_launcher',launcher)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    database=root/'inspection.sqlite3'
    connection=sqlite3.connect(database)
    connection.executescript('PRAGMA journal_mode=WAL; CREATE TABLE probe(value TEXT); INSERT INTO probe VALUES ("preserved");')
    connection.close()
    connection=module.inspect_database(database)
    assert connection.execute('SELECT value FROM probe').fetchone()==('preserved',)
    try:
        connection.execute('INSERT INTO probe VALUES ("unexpected")')
        raise AssertionError('inspection accepted a database write')
    except sqlite3.OperationalError:
        pass
    finally:
        connection.close()
    missing=root/'missing.sqlite3'
    try:
        module.inspect_database(missing)
        raise AssertionError('inspection created a missing database')
    except sqlite3.OperationalError:
        assert not missing.exists()
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
    sentinel='isolated-daemon-must-not-inherit-task-credentials'
    result=call('service','start','--mock',extra={'LARKSUITE_CLI_TENANT_ACCESS_TOKEN':sentinel,'CODEX_WECHAT_CODEX_BIN':sentinel,'MAC_INTERNAL_TASK':sentinel});assert result['state']=='running'
    try:
        process_environment=subprocess.check_output(['ps','eww','-p',str(result['pid']),'-o','command='],text=True)
        assert sentinel not in process_environment
        assert ((root/'dev/service.lock').stat().st_mode & 0o077)==0
        status=call('status');assert status['daemon']['identity']['release']==release
        assert call('service','drain')['ready'] is True
        call('init',ok=False)
        assert call('service','resume')['draining'] is False
        call('service','start','--mock',ok=False)
        call('service','restart','--mock','--timeout','10')
        call('release','promote',release,'--timeout','10')
        assert call('status')['daemon']['mock'] is True
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
