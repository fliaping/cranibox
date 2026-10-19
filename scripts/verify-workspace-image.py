#!/usr/bin/env python3
"""Check a local product image before publishing; requires a Linux Docker engine.

Uses disposable private volumes, no published ports and no host Docker socket.
Only the sanitized JSON report may be uploaded; logs and credentials stay private.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import tempfile
import time
import uuid

LABEL = 'io.cranibox.image-check'

# Probe inside the container, without exposing a host port. Credentials arrive
# through stdin or the container environment and are never returned in the report.
PROBE = r'''
import base64,hashlib,http.client,json,os,pathlib,re,ssl,stat,subprocess,sys,time
request=json.load(sys.stdin); root=pathlib.Path('/config'); out={}
def get(port,path,method='GET',body=None,headers=None,tls=False):
    options={'timeout':10}
    if tls: options['context']=ssl._create_unverified_context()
    connection=(http.client.HTTPSConnection if tls else http.client.HTTPConnection)('127.0.0.1',port,**options)
    try:
        connection.request(method,path,body=body,headers=headers or {})
        response=connection.getresponse()
        return response.status,response.read(),{key.lower():value for key,value in response.getheaders()}
    finally: connection.close()
for port in (9080,9090):
    status,body,_=get(port,'/healthz')
    out['health_'+str(port)]={'status':status,'value':json.loads(body)}
status,body,headers=get(9080,'/'); html=body.decode()
out['web_status']=status
bootstrap=json.loads(re.search(r'window.workspaceWeb=(.*?);</script>',html).group(1))
out['static_assets']={path:get(9080,path)[0] for path in ('/web.js','/web.css')}
status,body,_=get(9080,'/api/peers','POST',json.dumps({'operation':'list'}),
    {'Cookie':headers['set-cookie'].split(';')[0],'X-Workspace-CSRF':bootstrap['csrf'],'Content-Type':'application/json'})
out['peers_status']=status; out['api_node_id']=json.loads(body).get('node_id')
peers=root/'.config/cranibox-node/peers.json'
out['node_id']=json.loads(peers.read_text())['node_id']
out['peers_mode']=stat.S_IMODE(peers.stat().st_mode)
credentials=root/'.config/agent-workspace/runtime.json'
out['credentials_mode']=stat.S_IMODE(credentials.stat().st_mode)
out['credentials_sha256']=hashlib.sha256(credentials.read_bytes()).hexdigest()
out['versions']={}
for name,path in [('cranibox',root/'bin/cranibox'),('workspacectl',root/'bin/workspacectl'),
                  ('herdr',root/'bin/herdr'),('cranibox-node',pathlib.Path('/usr/local/lib/agent-workspace/cranibox-node'))]:
    result=subprocess.run([str(path),'--version'],capture_output=True,text=True,timeout=10)
    out['versions'][name]={'exit_code':result.returncode,'version':result.stdout.strip()}
out['image_binary_matches']=hashlib.sha256((root/'bin/cranibox').read_bytes()).digest()==hashlib.sha256(pathlib.Path('/usr/local/lib/agent-workspace/cranibox').read_bytes()).digest()
out['aliases']={name:(root/'bin'/name).resolve()==(root/'bin/cranibox').resolve()
                for name in ('workspacectl','workspace-services','workspace-runtime')}
out['service']={}
for operation in ('is-enabled','is-active'):
    result=subprocess.run(['/config/bin/workspace-services','--user',operation,'agent-workspace-web.service'],capture_output=True,text=True,timeout=15)
    out['service'][operation]={'exit_code':result.returncode,'status':result.stdout.strip()}
out['desktop_anonymous_status']=get(3001,'/',tls=True)[0]
auth=base64.b64encode((os.environ['CUSTOM_USER']+':'+os.environ['PASSWORD']).encode()).decode()
out['desktop_authenticated_status']=get(3001,'/',headers={'Authorization':'Basic '+auth},tls=True)[0]
out['applications_root_status']=get(3002,'/',tls=True)[0]
sentinel=root/'Workspace/.cranibox-image-check'
out['sentinel_sha256']=hashlib.sha256(sentinel.read_bytes()).hexdigest() if sentinel.exists() else None
if request:
    target=request['node_id']; base='/api/v1/mesh'
    headers={'X-Cranibox-Target':target,'Authorization':'Bearer '+request['token'],'Content-Type':'application/json'}
    status,body,_=get(9020,base+'/nodes',headers=headers)
    out['enrollment']={'status':status,'node_ids':[node['id'] for node in json.loads(body).get('nodes',[])]}
    headers['Authorization']='Bearer invalid-image-check-token'
    out['invalid_token_status']=get(9020,base+'/nodes',headers=headers)[0]
    headers['Authorization']='Bearer '+request['token']
    status,body,_=get(9020,base+'/nodes/'+target+'/exec','POST',json.dumps({'program':'/bin/sh','args':['-c','printf cranibox-image-exec-ok'],'timeout_secs':10}),headers)
    value=json.loads(body); end=time.monotonic()+12
    while status==200 and value.get('outcome')=='running' and time.monotonic()<end:
        time.sleep(.1)
        status,body,_=get(9020,base+'/jobs/'+value['id'],headers=headers)
        value=json.loads(body)
    out['execution']={'status':status,'exit_code':value.get('exit_code'),'stdout':value.get('stdout'),'outcome':value.get('outcome')}
print(json.dumps(out))
'''

SEED = '''set -eu
directory=/config/.config/cranibox-node
install -d -m 700 -o 1000 -g 1000 "$directory"
test ! -e "$directory/peers.json"
umask 077
cat > "$directory/peers.json"
chown 1000:1000 "$directory/peers.json"
'''


class ImageCheck:
    def __init__(self, args):
        self.args = args
        self.docker_prefix = ['sudo', '-n', 'docker'] if args.sudo else ['docker']
        self.identity = uuid.uuid4().hex
        self.resources = []
        self.report = {'started_at': self.now(), 'image': args.image,
                       'expected_revision': args.revision, 'expected_platform': args.platform,
                       'checks': [], 'passed': False}
        self.platform_args = ['--platform', args.platform] if args.platform else []

    @staticmethod
    def now():
        return datetime.datetime.now(datetime.timezone.utc).isoformat()

    def save(self):
        self.args.report.parent.mkdir(parents=True, exist_ok=True)
        with open(self.args.report, 'w', opener=lambda p, f: os.open(p, f, 0o600)) as stream:
            os.fchmod(stream.fileno(), 0o600)
            json.dump(self.report, stream, indent=2)
            stream.write('\n')
        self.args.report.chmod(0o600)

    def check(self, name, passed, **details):
        value = {'name': name, 'passed': bool(passed), **details}
        self.report['checks'].append(value)
        self.save()
        print(json.dumps(value), flush=True)
        if not passed:
            raise RuntimeError('Check failed: ' + name)

    def docker(self, *args, data=None, timeout=45):
        result = subprocess.run([*self.docker_prefix, *args], input=data,
                                capture_output=True, text=True, timeout=timeout)
        if result.returncode:
            # Docker output or a container traceback can contain credentials.
            raise RuntimeError('Docker action failed: ' + args[0])
        return result.stdout

    def inspect(self, kind, name):
        return json.loads(self.docker(kind, 'inspect', name))[0]

    @staticmethod
    def platform_manifest(index, platform):
        system, architecture = platform.split('/')
        if not isinstance(index, dict) or not isinstance(index.get('manifests'), list):
            raise ValueError('Candidate must be a multi-platform image index')
        matches = []
        for manifest in index['manifests']:
            if not isinstance(manifest, dict) or not isinstance(manifest.get('platform'), dict):
                continue
            target = manifest['platform']
            if (target.get('os') == system and target.get('architecture') == architecture
                    and target.get('variant', '') in (('', 'v8') if architecture == 'arm64' else ('',))):
                matches.append(manifest)
        if (len(matches) != 1 or not isinstance(matches[0].get('digest'), str)
                or not re.fullmatch(r'sha256:[0-9a-f]{64}', matches[0]['digest'])):
            raise ValueError('Candidate must contain exactly one matching platform manifest')
        return matches[0]['digest']

    def image_reference(self):
        if not self.args.pull:
            return self.args.image
        index = json.loads(self.docker('buildx', 'imagetools', 'inspect', '--raw', self.args.image))
        digest = self.platform_manifest(index, self.args.platform)
        image = self.args.image.split('@')[0] + '@' + digest
        self.report['platform_image'] = image
        self.save()
        self.docker('pull', '--platform', self.args.platform, image, timeout=900)
        return image

    def wait(self, name, credentials, phase):
        end = time.monotonic() + self.args.timeout
        self.report['phase'] = phase
        self.save()
        while time.monotonic() < end:
            container = self.inspect('container', name)
            if not container['State']['Running']:
                raise RuntimeError(phase + ': container stopped')
            if container['State'].get('Health', {}).get('Status') == 'healthy':
                try:
                    output = self.docker('exec', '-i', '-u', '1000:1000', name,
                                         'python3', '-c', PROBE,
                                         data=json.dumps(credentials), timeout=90)
                    probe = json.loads(output)
                    failed = [key for key, passed in self.conditions(probe, credentials).items() if not passed]
                    self.report['unready_checks'] = failed
                    if not failed:
                        self.report.setdefault('probes', {})[phase] = probe
                        for key in self.conditions(probe, credentials):
                            self.check(phase+'-'+key, True)
                        return probe
                except (RuntimeError, ValueError, subprocess.TimeoutExpired):
                    self.report['unready_checks'] = ['probe-unavailable']
                self.save()
            time.sleep(3)
        raise RuntimeError(phase + ': readiness timed out')

    @staticmethod
    def conditions(probe, credentials):
        result = {
            'rust-listeners': all(probe['health_'+str(p)] ==
                {'status': 200, 'value': {'ok': True, 'protocol_version': 1}} for p in (9080, 9090)),
            'web-and-node': probe['web_status'] == 200 and probe['peers_status'] == 200
                and probe['api_node_id'] == probe['node_id']
                and set(probe['static_assets']) == {'/web.js', '/web.css'}
                and all(status == 200 for status in probe['static_assets'].values()),
            'built-in-programs': all(value['exit_code'] == 0 for value in probe['versions'].values())
                and set(probe['versions']) == {'cranibox', 'workspacectl', 'herdr', 'cranibox-node'}
                and set(probe['aliases']) == {'workspacectl', 'workspace-services', 'workspace-runtime'}
                and all(probe['aliases'].values()) and probe['image_binary_matches'],
            'default-service': set(probe['service']) == {'is-enabled', 'is-active'}
                and all(value['exit_code'] == 0 for value in probe['service'].values()),
            'private-credentials': probe['credentials_mode'] == 0o600 and probe['peers_mode'] == 0o600,
            'desktop-auth': probe['desktop_anonymous_status'] == 401
                and probe['desktop_authenticated_status'] == 200,
            'applications-origin': probe['applications_root_status'] == 404,
        }
        if credentials:
            result.update({
                'seeded-identity': probe['node_id'] == credentials['node_id'],
                'enrollment': probe['enrollment'] == {'status': 200, 'node_ids': [credentials['node_id']]},
                'peer-auth': probe['invalid_token_status'] == 401,
                'peer-execution': probe['execution']['status'] == 200
                    and probe['execution']['exit_code'] == 0
                    and probe['execution']['stdout'] == 'cranibox-image-exec-ok',
            })
        return result

    def scenario(self, mode, image_id, directory):
        name = 'cranibox-image-check-' + self.identity[:12] + '-' + mode
        volume = name + '-config'
        self.resources.append(('volume', volume))
        self.docker('volume', 'create', '--label', LABEL+'='+self.identity, volume)
        credentials = {}
        if mode == 'space':
            credentials = {'node_id': 'node-'+uuid.uuid4().hex, 'token': secrets.token_hex(32)}
            seed = {'version': 1, 'node_id': credentials['node_id'],
                    'listener': {'address': '0.0.0.0:9020', 'allow_insecure': True},
                    'desktop': None, 'desktop_epoch': 0, 'routes': {},
                    'grants': {'node-'+uuid.uuid4().hex: {'token': credentials['token'],
                        'permissions': {'exec': True, 'terminals': True, 'projects': True, 'desktop': False}}}}
            seed_name = name+'-seed'
            self.resources.append(('container', seed_name))
            self.docker('run', *self.platform_args, '--name', seed_name, '--label', LABEL+'='+self.identity,
                        '--pull=never', '-i', '--network', 'none',
                        '--cpus', '1', '--memory', '128m', '--memory-swap', '128m', '--pids-limit', '32',
                        '--mount', 'type=volume,source='+volume+',target=/config',
                        '--entrypoint', '/bin/sh', image_id, '-c', SEED, data=json.dumps(seed))
            self.docker('container', 'rm', seed_name)
            self.resources.remove(('container', seed_name))
        env = directory / (mode+'.env')
        env.write_text('PUID=1000\nPGID=1000\nTZ=Etc/UTC\nCUSTOM_USER=image-check\nPASSWORD='+
                       secrets.token_hex(32)+'\nSTART_DOCKER=false\nUSE_CHINA_MIRROR=false\n')
        env.chmod(0o600)
        self.resources.append(('container', name))
        self.docker('run', *self.platform_args, '-d', '--pull=never', '--name', name, '--label', LABEL+'='+self.identity,
                    '--restart', 'no', '--cpus', '2', '--memory', '4g', '--memory-swap', '4g',
                    '--shm-size', '512m', '--pids-limit', '1024', '--env-file', str(env),
                    '--mount', 'type=volume,source='+volume+',target=/config', image_id, timeout=90)
        container = self.inspect('container', name)
        self.check(mode+'-isolated-container', not container['HostConfig']['Privileged']
                   and not container['HostConfig'].get('PortBindings')
                   and len(container['Mounts']) == 1
                   and container['Mounts'][0]['Name'] == volume
                   and container['Mounts'][0]['Destination'] == '/config'
                   and container['Image'] == image_id)
        before = self.wait(name, credentials, mode+'-first-boot')
        self.docker('exec', '-u', '1000:1000', name, 'python3', '-c',
                    "from pathlib import Path; import secrets; Path('/config/Workspace/.cranibox-image-check').write_text(secrets.token_hex(32))")
        marker = self.docker('exec', '-u', '1000:1000', name, 'python3', '-c',
                    "from pathlib import Path; import hashlib; print(hashlib.sha256(Path('/config/Workspace/.cranibox-image-check').read_bytes()).hexdigest())").strip()
        self.docker('restart', '--time', '25', name, timeout=90)
        after = self.wait(name, credentials, mode+'-restart')
        self.check(mode+'-persistent-data-and-identity', before['node_id'] == after['node_id']
                   and before['credentials_sha256'] == after['credentials_sha256']
                   and after['sentinel_sha256'] == marker)
        self.docker('stop', '--time', '25', name, timeout=90)

    def cleanup(self):
        cleaned = []
        for kind, name in reversed(self.resources):
            try:
                value = self.inspect(kind, name)
                labels = (value['Config'].get('Labels') if kind == 'container' else value.get('Labels')) or {}
                if labels.get(LABEL) != self.identity:
                    raise RuntimeError('resource ownership changed')
                self.docker(kind, 'rm', *(['--force'] if kind == 'container' else []), name, timeout=90)
                cleaned.append({'kind': kind, 'name': name, 'removed': True})
            except (RuntimeError, OSError, ValueError, subprocess.TimeoutExpired):
                cleaned.append({'kind': kind, 'name': name, 'removed': False})
        self.report['cleanup'] = cleaned
        return all(resource['removed'] for resource in cleaned)

    def run(self):
        try:
            image = self.inspect('image', self.image_reference())
            labels = image['Config'].get('Labels') or {}
            self.report['image_id'] = image['Id']
            self.report['platform'] = image['Os']+'/'+image['Architecture']
            self.report['revision'] = labels.get('org.opencontainers.image.revision')
            self.check('space-protocol', labels.get('io.cranibox.space-protocol') == '1')
            if self.args.revision:
                self.check('source-revision', self.report['revision'] == self.args.revision)
            self.check('linux-image', image['Os'] == 'linux')
            if self.args.platform:
                self.check('requested-platform', self.report['platform'] == self.args.platform)
            with tempfile.TemporaryDirectory(prefix='cranibox-image-check-') as temporary:
                for mode in ('fresh', 'space'):
                    self.scenario(mode, image['Id'], Path(temporary))
            self.report['passed'] = True
        except (RuntimeError, OSError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
            self.report['error_type'] = type(error).__name__
            # Never print subprocess exceptions, commands, stderr or private input.
            print(json.dumps({'passed': False, 'error_type': type(error).__name__}), flush=True)
        finally:
            cleaned = self.cleanup()
            self.report['passed'] = self.report['passed'] and cleaned
            self.report['finished_at'] = self.now()
            self.save()
        return 0 if self.report['passed'] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True, help='Image already loaded in this Docker engine')
    parser.add_argument('--revision', help='Required full org.opencontainers.image.revision label')
    parser.add_argument('--platform', choices=('linux/amd64', 'linux/arm64'),
                        help='Require this image platform and use it for every test container')
    parser.add_argument('--pull', action='store_true',
                        help='Explicitly select and pull a platform manifest from an immutable registry index')
    parser.add_argument('--report', required=True, type=Path, help='Sanitized acceptance JSON')
    parser.add_argument('--timeout', type=int, default=600, help='Readiness deadline per startup, seconds')
    parser.add_argument('--sudo', action='store_true', help='Invoke Docker through sudo -n')
    args = parser.parse_args()
    if args.revision and not re.fullmatch(r'[0-9a-f]{40,64}', args.revision):
        parser.error('--revision must be a full lowercase Git revision')
    if args.pull and (not args.platform or not re.fullmatch(r'[^@\s]+@sha256:[0-9a-f]{64}', args.image)):
        parser.error('--pull requires --platform and --image REPOSITORY@sha256:DIGEST')
    if not 30 <= args.timeout <= 1800:
        parser.error('--timeout must be between 30 and 1800 seconds')
    os.umask(0o077)
    return ImageCheck(args).run()


if __name__ == '__main__':
    raise SystemExit(main())
