import argparse
import hashlib, http.server, json, os, re, subprocess, tarfile, threading, time
from pathlib import Path

parser = argparse.ArgumentParser(description='Capture official Claude Code requests against a loopback mock. Uses fake credentials only.')
parser.add_argument('artifacts', type=Path, help='Directory containing the official Linux x64 CLI npm tarball')
parser.add_argument('--oauth', action='store_true', help='Use a fake OAuth Bearer token instead of a fake API key')
args = parser.parse_args()
root = args.artifacts.resolve()
root.mkdir(parents=True, exist_ok=True)
archive = next(root.glob('anthropic-ai-claude-code-linux-x64-*.tgz'))
with tarfile.open(archive) as package:
    package.extractall(root, filter='data')
meta = json.loads((root/'package/package.json').read_text())
observations = []
class Mock(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get('content-length', 0))))
        system = body.get('system', [])
        texts = [v.get('text','') for v in system if isinstance(v,dict)] if isinstance(system,list) else [str(system)]
        billing = [v for v in texts if 'billing-header' in v or 'cch=' in v]
        observations.append({'path':self.path, 'model':body.get('model'), 'billing':billing,
            'auth_scheme': self.headers.get('authorization','').split(' ',1)[0], 'api_key_header': bool(self.headers.get('x-api-key')),
            'cch_header_names':[k for k in self.headers if 'cch' in k.lower()],
            'cch_json_keys': re.findall(r'\"([^\"]*cch[^\"]*)\"\s*:',json.dumps(body)),
            'cch':re.findall(r'cch=([^;\s]+)', '\n'.join(texts)),
            'body_sha256':hashlib.sha256(json.dumps(body,sort_keys=True).encode()).hexdigest(),
            'stream':body.get('stream',False)})
        message = {'id':'msg_local_probe','type':'message','role':'assistant',
            'model':body.get('model','mock'), 'content':[{'type':'text','text':'OK'}],
            'stop_reason':'end_turn','stop_sequence':None,'usage':{'input_tokens':1,'output_tokens':1}}
        if self.path.startswith('/v1/messages/count_tokens'):
            result = json.dumps({'input_tokens':1}).encode(); content_type='application/json'
        elif body.get('stream'):
            events = [('message_start',{'type':'message_start','message':{**message,'content':[],'stop_reason':None}}),
                ('content_block_start',{'type':'content_block_start','index':0,'content_block':{'type':'text','text':''}}),
                ('content_block_delta',{'type':'content_block_delta','index':0,'delta':{'type':'text_delta','text':'OK'}}),
                ('content_block_stop',{'type':'content_block_stop','index':0}),
                ('message_delta',{'type':'message_delta','delta':{'stop_reason':'end_turn','stop_sequence':None},'usage':{'output_tokens':1}}),
                ('message_stop',{'type':'message_stop'})]
            result = ''.join('event: '+name+'\ndata: '+json.dumps(value)+'\n\n' for name,value in events).encode(); content_type='text/event-stream'
        else: result=json.dumps(message).encode(); content_type='application/json'
        self.send_response(200); self.send_header('content-type',content_type); self.send_header('content-length',str(len(result))); self.end_headers(); self.wfile.write(result)
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Mock)
threading.Thread(target=server.serve_forever,daemon=True).start()
env={k:v for k,v in os.environ.items() if not k.startswith(('ANTHROPIC_','CLAUDE_'))}
env.update({('CLAUDE_CODE_OAUTH_TOKEN' if args.oauth else 'ANTHROPIC_API_KEY'):'test-only-not-a-real-token','ANTHROPIC_BASE_URL':f'http://127.0.0.1:{server.server_port}',
    'CLAUDE_CONFIG_DIR':str(root/('isolated-oauth-config' if args.oauth else 'isolated-config')),'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC':'1','DISABLE_TELEMETRY':'1','DISABLE_ERROR_REPORTING':'1'})
work=root/'empty-project'; work.mkdir(exist_ok=True)
results=[]
for prompt in ['Reply OK only.','Reply OK only.','Reply HELLO only.']:
    start=len(observations)
    binary=root/'package/claude'; binary.chmod(0o700)
    command=[str(binary),'--print','--no-session-persistence','--tools','','--setting-sources','','--model','claude-sonnet-4-5','--output-format','json',prompt]
    try:
        run=subprocess.run(command,cwd=work,env=env,stdin=subprocess.DEVNULL,capture_output=True,text=True,timeout=45)
        results.append({'prompt':prompt,'exit_code':run.returncode,'requests':observations[start:],
            'stdout_is_json':run.stdout.lstrip().startswith('{'),'stderr_present':bool(run.stderr)})
    except subprocess.TimeoutExpired:
        results.append({'prompt':prompt,'timeout':True,'requests':observations[start:]})
server.shutdown()
report={'at':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'scope':'official CLI to isolated loopback mock; no real upstream or account',
    'package':meta['name'],'version':meta['version'],'archive_sha256':hashlib.sha256(archive.read_bytes()).hexdigest(),'runs':results}
path=root/('observations-oauth.json' if args.oauth else 'observations.json'); path.write_text(json.dumps(report,indent=2)); path.chmod(0o600)
print(json.dumps(report,indent=2))
