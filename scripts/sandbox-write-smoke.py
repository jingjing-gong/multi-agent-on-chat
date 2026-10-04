"""Check native full-access policy permits writes outside the workspace and network connections."""
import json, os, pathlib, subprocess, tempfile

def rpc(p, i, method, params):
    p.stdin.write(json.dumps(dict(id=i, method=method, params=params)) + '\n')
    p.stdin.flush()
    for line in p.stdout:
        value = json.loads(line)
        if value.get('id') == i:
            if 'error' in value:
                raise RuntimeError(value['error'])
            return value['result']
    raise RuntimeError('app-server disconnected')

with tempfile.TemporaryDirectory() as root:
    root = pathlib.Path(root).resolve()
    home, work, data = [root / name for name in ('home', 'work', 'data')]
    for path in (home, work, data):
        path.mkdir()
    (home / 'config.toml').write_text('sandbox_mode = "read-only"\n')
    env = dict(os.environ, CODEX_HOME=str(home))
    p = subprocess.Popen([os.environ.get('CODEX_BINARY', 'codex'), 'app-server'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, env=env, cwd=work)
    try:
        rpc(p, 1, 'initialize', {'clientInfo':{'name':'sandbox-probe','version':'1'}, 'capabilities':{'experimentalApi':True}})
        response = rpc(p, 2, 'thread/start', {'cwd':str(work), 'approvalPolicy':'never', 'sandbox':'danger-full-access', 'config':{'sandbox_mode':'danger-full-access'}})
        policy = response['sandbox']
        assert policy['type'] == 'dangerFullAccess', policy
        script = 'import sqlite3; c=sqlite3.connect(' + repr(str(data/'runtime.sqlite3')) + '); c.execute("create table probe(value)"); c.execute("insert into probe values(1)"); c.commit()'
        result = rpc(p, 3, 'command/exec', {'cwd':str(work), 'command':['/usr/bin/python3','-c',script], 'sandboxPolicy':policy, 'timeoutMs':10000})
        assert result['exitCode'] == 0, result
        assert (data/'runtime.sqlite3').exists()
        import http.server, threading
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'network-ok')
            def log_message(self, *args):
                pass
        server = http.server.HTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            url = 'http://127.0.0.1:' + str(server.server_port)
            probe = 'import urllib.request; assert urllib.request.urlopen(' + repr(url) + ', timeout=5).read() == b"network-ok"'
            result = rpc(p, 4, 'command/exec', {'cwd':str(work), 'command':['/usr/bin/python3','-c',probe], 'sandboxPolicy':policy, 'timeoutMs':10000})
            assert result['exitCode'] == 0, result
        finally:
            server.shutdown()
            server.server_close()
            worker.join()
        print('PASS: danger-full-access permits writes outside workspace and TCP/HTTP')
    finally:
        p.kill()
        p.wait()
