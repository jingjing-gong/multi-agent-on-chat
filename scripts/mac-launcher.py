#!/usr/bin/env python3
"""One mac-cli entry; immutable releases and explicit instance bindings."""
import argparse, contextlib, fcntl, hashlib, json, os, re, shutil, sqlite3, subprocess, sys, time, uuid
from pathlib import Path

ROOT = Path(os.environ.get('MAC_INSTALL_ROOT', str(Path.home()/'.local/share/mac'))).resolve()

def fail(message):
    raise RuntimeError(message)

def read(path):
    return json.loads(Path(path).read_text())

def atomic(path, value):
    path=Path(path);path.parent.mkdir(parents=True,exist_ok=True)
    temp=path.with_name(path.name+'.'+uuid.uuid4().hex+'.tmp')
    try:
        with temp.open('x') as f:
            os.chmod(temp,0o600);json.dump(value,f,ensure_ascii=False,indent=2);f.flush();os.fsync(f.fileno())
        os.replace(temp,path)
    finally:
        temp.unlink(missing_ok=True)

def valid(value):
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]{0,99}',value) or '..' in value:fail('Invalid environment/release identifier')
    return value

def selected():return valid(os.environ.get('MAC_ENV','prod'))
def config(name=None):
    name=name or selected();p=ROOT/'instances'/f'{valid(name)}.json'
    if not p.exists():fail(f'Environment {name} is not registered; use mac-cli env register')
    v=read(p)
    if v['name']!=name:fail('Instance name mismatch')
    return v

def release(identifier):
    directory=ROOT/'releases'/valid(identifier)
    m=read(directory/'manifest.json');binary=directory/'mac-cli'
    if m['release_id']!=identifier:fail('Release identity mismatch')
    if hashlib.sha256(binary.read_bytes()).hexdigest()!=m['binary_sha256']:fail('Release checksum mismatch')
    return binary,m

def resolved(v, override=True):
    identifier=os.environ.get('MAC_VERSION',v['release']) if override else v['release']
    binary,manifest=release(identifier)
    data=Path(v['data_dir']).resolve()
    if not data.is_dir():fail('Registered data directory missing')
    marker=read(data/'instance.json')
    if marker['instance_id']!=v['instance_id'] or marker['name']!=v['name']:fail('Data directory belongs to another instance')
    explicit=os.environ.get('MAC_DATA_DIR')
    if explicit and Path(explicit).resolve()!=data:fail('MAC_DATA_DIR conflicts with selected environment')
    env={**os.environ,'MAC_ENV':v['name'],'MAC_VERSION':identifier,'MAC_INSTANCE_ID':v['instance_id'],
         'MAC_INSTALL_ROOT':str(ROOT),'MAC_BOUND_DATA_DIR':str(data),'MAC_DATA_DIR':str(data),'MAC_CLI':str(binary)}
    if v.get('codex_home'):env['CODEX_HOME']=v['codex_home']
    env['PATH']=str(ROOT/'bin')+os.pathsep+env.get('PATH','')
    return binary,manifest,data,env

def db_daemon(data):
    db=data/'runtime.sqlite3'
    if not db.exists():return None
    c=sqlite3.connect(db.as_uri()+'?mode=ro',uri=True)
    try:
        version=c.execute('PRAGMA user_version').fetchone()[0]
        if version!=3:fail('Managed environments require database schema 3; migrate explicitly first')
        row=c.execute("SELECT data FROM records WHERE kind='control' AND id='daemon'").fetchone()
        return json.loads(row[0]) if row else None
    finally:c.close()

def live(data):
    # OS lock is authoritative; stale heartbeat/PID does not establish ownership.
    p=data/'service.lock'
    if not p.exists():return False
    with p.open('a') as f:
        try:fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)
        except BlockingIOError:return True
        finally:
            with contextlib.suppress(OSError):fcntl.flock(f,fcntl.LOCK_UN)
    return False

def run_cli(v,args,override=False):
    b,_,d,e=resolved(v,override)
    r=subprocess.run([str(b),'--data-dir',str(d),*args],env=e,text=True,capture_output=True,timeout=30)
    if r.returncode:fail(r.stderr.strip() or 'CLI failed')
    return json.loads(r.stdout)

@contextlib.contextmanager
def operation(name):
    p=ROOT/'locks'/f'operation-{valid(name)}.lock';p.parent.mkdir(parents=True,exist_ok=True)
    with p.open('a') as f:
        try:fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)
        except BlockingIOError:fail('Another instance operation is running')
        yield

def start(v,mock=False):
    binary,m,data,env=resolved(v,False)
    if v['name']=='prod' and m.get('dirty'):fail('Production requires a clean release')
    if live(data):fail('Instance already running')
    # Resume a previously drained instance only when no server owns it.
    run_cli(v,['service','resume'])
    log=data/'logs'/f'service-{m["release_id"]}-{int(time.time())}.log';log.parent.mkdir(exist_ok=True)
    args=[str(binary),'--data-dir',str(data),'serve','--workers',str(v.get('workers',4))]
    if mock:args.append('--mock')
    with log.open('ab') as stream:
        p=subprocess.Popen(args,env=env,stdin=subprocess.DEVNULL,stdout=stream,stderr=stream,start_new_session=True)
    atomic(data/'background-process.json',{'pid':p.pid,'log':str(log),'started_at':int(time.time()),'release':m['release_id']})
    for _ in range(100):
        if p.poll() is not None:fail(f'Service exited; inspect {log}')
        daemon=db_daemon(data)
        if live(data) and daemon and daemon.get('pid')==p.pid and daemon.get('identity',{}).get('release')==m['release_id']:
            return {'pid':p.pid,'release':m['release_id'],'log':str(log),'state':'running'}
        time.sleep(.1)
    fail(f'Start timed out; inspect {log}; do not start a duplicate')

def stop_drained(v,timeout):
    _,_,data,_=resolved(v,False)
    if not live(data):return
    run_cli(v,['service','drain'])
    deadline=time.monotonic()+timeout
    while True:
        readiness=run_cli(v,['service','readiness'])
        if readiness['ready']:break
        if time.monotonic()>=deadline:fail('Drain timed out; old service remains running and draining: '+json.dumps(readiness,ensure_ascii=False))
        time.sleep(.5)
    run_cli(v,['stop'])
    while live(data):
        if time.monotonic()>=deadline:fail('Stop timed out; did not start a duplicate')
        time.sleep(.1)

def backup(v):
    _,_,data,_=resolved(v,False)
    dst=data/'backups'/f'deploy-{time.time_ns()}';dst.mkdir(parents=True,mode=0o700)
    if (data/'runtime.sqlite3').exists():
        with sqlite3.connect(str(data/'runtime.sqlite3')) as src, sqlite3.connect(str(dst/'runtime.sqlite3')) as out:src.backup(out)
        os.chmod(dst/'runtime.sqlite3',0o600)
    atomic(dst/'instance.json',v)
    atomic(dst/'scope.json',{'scope':'SQLite + instance configuration','external_media_and_credentials':'remain in original data directory','restore_external_sends':'must reconcile before enabling bots'})
    return str(dst)

def promote(v,identifier,timeout):
    _,m=release(identifier)
    if v['name']=='prod' and m.get('dirty'):fail('Cannot promote dirty build to production')
    old=dict(v);new={**v,'release':identifier};b,_,d,e=resolved(new,False)
    # version performs no database writes. Store preflight runs at actual command boundary.
    daemon=db_daemon(d)
    if daemon and live(d) and daemon.get('identity',{}).get('release')!=old['release']:fail('Registered release and running daemon differ')
    journal=ROOT/'deployments'/f'{v["name"]}.json'
    if journal.exists() and read(journal).get('state') not in ['complete','failed_before_stop']:fail('Incomplete deployment exists; inspect journal and recover explicitly')
    state={'state':'preflight','old':old,'new':new,'mock':bool(daemon and daemon.get('mock')),'created_at':int(time.time())};atomic(journal,state)
    try:stop_drained(old,timeout)
    except Exception:
        state['state']='failed_before_stop';atomic(journal,state);raise
    state['state']='stopped';atomic(journal,state)
    state['backup']=backup(old);state['state']='backed_up';atomic(journal,state)
    # Database schema 3 is compatible with current builds; no implicit migrations.
    atomic(ROOT/'instances'/f'{v["name"]}.json',new);state['state']='selected';atomic(journal,state)
    result=start(new,mock=state['mock']);state['state']='complete';state['result']=result;atomic(journal,state)
    return state

def register(args):
    name=valid(args.name)
    with operation('registry'):
        p=ROOT/'instances'/f'{name}.json'
        if p.exists():fail('Environment already registered')
        b,m=release(args.release)
        if name=='prod' and m.get('dirty'):fail('Production requires clean release')
        data=Path(args.data_dir).expanduser().resolve()
        for other in (ROOT/'instances').glob('*.json'):
            q=Path(read(other)['data_dir']).resolve()
            if data==q or data in q.parents or q in data.parents:fail('Instance data directories overlap')
        if live(data):fail('Stop existing service before registering its data directory')
        if data.exists() and any(data.iterdir()) and not args.adopt_existing:fail('Nonempty directory requires --adopt-existing')
        if (data/'instance.json').exists():fail('Directory already has an instance marker')
        v={'name':name,'instance_id':str(uuid.uuid4()),'release':args.release,'data_dir':str(data),'workers':args.workers,'codex_home':str(Path(args.codex_home).expanduser().resolve()) if args.codex_home else None}
        if args.adopt_existing and (data/'runtime.sqlite3').exists():
            with sqlite3.connect((data/'runtime.sqlite3').as_uri()+'?mode=ro',uri=True) as c:
                if c.execute('PRAGMA user_version').fetchone()[0]!=3:fail('Managed adoption requires database schema 3')
        data.mkdir(parents=True,exist_ok=True)
        atomic(data/'instance.json',{'name':name,'instance_id':v['instance_id'],'format_version':1})
        atomic(p,v)
        # Existing DB has legacy schema 0; retain it (no migration necessary).
        return v

def main(argv):
    if not argv or argv==['--help'] or argv==['-h']:
        return {'usage':'mac-cli [env|release|service|existing CLI command]', 'selection':'MAC_ENV=prod|dev, optional MAC_VERSION=release-id','environment_commands':'env list/show/check/register','release_commands':'release list/install/verify/promote/rollback/recover','service_commands':'service start/restart/drain/resume/readiness','help':'mac-cli env --help; mac-cli release --help; other command help uses selected release'}
    if argv[:1]==['env']:
        parser=argparse.ArgumentParser(prog='mac-cli env');subs=parser.add_subparsers(dest='action',required=True)
        for x in ['list','show','check']:subs.add_parser(x)
        p=subs.add_parser('register');p.add_argument('name');p.add_argument('--release',required=True);p.add_argument('--data-dir',required=True);p.add_argument('--codex-home');p.add_argument('--workers',type=int,choices=range(1,21),default=4);p.add_argument('--adopt-existing',action='store_true')
        a=parser.parse_args(argv[1:])
        if a.action=='register':return register(a)
        if a.action=='list':return [read(p) for p in sorted((ROOT/'instances').glob('*.json'))]
        v=config();b,m,d,e=resolved(v);return {**v,'selected_release':m['release_id'],'binary':str(b),'running':live(d),'daemon':db_daemon(d)}
    if argv[:1]==['release']:
        parser=argparse.ArgumentParser(prog='mac-cli release');subs=parser.add_subparsers(dest='action',required=True)
        subs.add_parser('list')
        p=subs.add_parser('recover');p.add_argument('--use',choices=['old','new'],required=True)
        p=subs.add_parser('install');p.add_argument('binary');p.add_argument('--id')
        p=subs.add_parser('verify');p.add_argument('id')
        for x in ['promote','rollback']:
            p=subs.add_parser(x);p.add_argument('id');p.add_argument('--timeout',type=int,default=60)
        a=parser.parse_args(argv[1:])
        if a.action=='list':return [read(p) for p in sorted((ROOT/'releases').glob('*/manifest.json'))]
        if a.action=='install':
            binary=Path(a.binary).resolve();info=json.loads(subprocess.check_output([str(binary),'version','--json'],text=True))
            if info.get('control_protocol')!=1 or info.get('database_schema_max')!=3:fail('Unsupported release protocol/schema')
            identifier=valid(a.id or info['release'])
            if identifier!=info['release']:fail('Release ID must match embedded build identity')
            dst=ROOT/'releases'/identifier
            if dst.exists():fail('Release already exists; never overwrite')
            temp=dst.with_name('.install-'+uuid.uuid4().hex);temp.mkdir(parents=True)
            shutil.copy2(binary,temp/'mac-cli');os.chmod(temp/'mac-cli',0o555)
            manifest={**info,'release_id':identifier,'binary_sha256':hashlib.sha256((temp/'mac-cli').read_bytes()).hexdigest(),'installed_at':int(time.time()),'schema_read_range':[3,3],'schema_write_range':[3,3]}
            atomic(temp/'manifest.json',manifest);os.rename(temp,dst);return manifest
        if a.action=='verify':return release(a.id)[1]
        if a.action=='recover':
            v=config()
            with operation(v['name']):
                path=ROOT/'deployments'/f'{v["name"]}.json';state=read(path)
                if live(Path(v['data_dir'])):fail('Stop the running instance before recovering a deployment')
                chosen=state[a.use];release(chosen['release']);db_daemon(Path(chosen['data_dir']))
                atomic(ROOT/'instances'/f'{v["name"]}.json',chosen)
                result=start(chosen,mock=state.get('mock',False));state['state']='complete';state['recovered_using']=a.use;state['result']=result;atomic(path,state)
                return state
        v=config()
        with operation(v['name']):return promote(v,a.id,a.timeout)
    if argv[:2] in [['service','start'],['service','restart']]:
        p=argparse.ArgumentParser();p.add_argument('service');p.add_argument('action');p.add_argument('--mock',action='store_true');p.add_argument('--timeout',type=int,default=60);a=p.parse_args(argv)
        v=config()
        if os.environ.get('MAC_VERSION',v['release'])!=v['release']:fail('Select release using release promote before service start/restart')
        with operation(v['name']):
            if a.action=='restart':stop_drained(v,a.timeout)
            return start(v,a.mock)
    v=config();b,m,d,e=resolved(v)
    for i,a in enumerate(argv):
        if a=='--data-dir':
            if i+1>=len(argv) or Path(argv[i+1]).expanduser().resolve()!=d:fail('--data-dir conflicts with selected environment')
        elif a.startswith('--data-dir=') and Path(a.split('=',1)[1]).expanduser().resolve()!=d:fail('--data-dir conflicts with selected environment')
    diagnostic=argv[:1] in [['status'],['version'],['--version'],['--help'],['-h']]
    if v['name']=='prod' and m['release_id']!=v['release'] and not diagnostic:fail('Production write requires registered release')
    daemon=db_daemon(d)
    if (d/'runtime.sqlite3').exists() and not diagnostic and os.environ.get('MAC_INTERNAL_TASK')!='1' and argv[:1]!=['stop'] and argv[:1]!=['service']:
        with sqlite3.connect((d/'runtime.sqlite3').as_uri()+'?mode=ro',uri=True) as c:
            row=c.execute("SELECT data FROM records WHERE kind='control' AND id='draining'").fetchone()
            if row and json.loads(row[0]):fail('Instance is draining; resume before submitting new commands')
    if live(d) and daemon and daemon.get('identity',{}).get('release')!=m['release_id']:
        if argv[:1]==['status']:return {'caller':m,'daemon':daemon,'data_dir':str(d),'version_mismatch':True}
        if not diagnostic:fail('CLI and running daemon release differ; writes rejected')
    os.execve(str(b),[str(b),'--data-dir',str(d),*argv],e)

if __name__=='__main__':
    try:
        value=main(sys.argv[1:])
        if value is not None:print(json.dumps(value,ensure_ascii=False,indent=2))
    except (RuntimeError,OSError,ValueError,KeyError,sqlite3.Error,subprocess.SubprocessError) as error:
        print(f'error: {error}',file=sys.stderr);sys.exit(1)
