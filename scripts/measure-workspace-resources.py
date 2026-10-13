#!/usr/bin/env python3
"""Measure isolated service supervision and actual Rust Web payloads on Linux."""
import argparse
import datetime
import contextlib
import json
import hashlib
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tests'))
from workspace_web_harness import EventStream, Harness, sockets_allowed, until


def process(pid):
    path = Path('/proc') / str(pid)
    status = dict(line.split(':', 1) for line in (path / 'status').read_text().splitlines() if ':' in line)
    stat = (path / 'stat').read_text().rsplit(') ', 1)[1].split()
    return {'pid': pid, 'comm': status['Name'].strip(), 'rss_kib': int(status.get('VmRSS', '0 kB').split()[0]),
            'ticks': int(stat[11]) + int(stat[12])}


def aggregate(pids):
    samples = []
    initial = [process(pid) for pid in sorted(pids)]
    started = time.monotonic()
    for _ in range(3):
        time.sleep(.3)
        samples.append([process(pid) for pid in sorted(pids)])
    elapsed = time.monotonic() - started
    return {'processes': samples[-1], 'process_count': len(pids),
            'median_total_rss_kib': statistics.median(sum(p['rss_kib'] for p in sample) for sample in samples),
            'cpu_percent': 100 * (sum(p['ticks'] for p in samples[-1]) - sum(p['ticks'] for p in initial)) / os.sysconf('SC_CLK_TCK') / elapsed,
            'sampling_seconds': round(elapsed, 3)}


def services(binary, count):
    with tempfile.TemporaryDirectory(prefix='cranibox-resource-services-') as directory:
        root = Path(directory)
        env = {**os.environ, 'HOME': directory}
        units = root / '.config/systemd/user'
        units.mkdir(parents=True)
        state = root / '.local/run/user-systemd'
        scan = root / '.local/run/workspace-services/services'
        names = [f'baseline-{i}' for i in range(count)]

        def run(*args):
            return subprocess.run([str(binary), '--user', *args], env=env, capture_output=True, text=True, check=True, timeout=20)

        def pids(suffix):
            return {int((state / (name + suffix)).read_text()) for name in names}

        try:
            for name in names:
                (units / (name + '.service')).write_text('[Service]\nExecStart=/bin/sleep 120\nRestart=always\nRestartSec=.1\nTimeoutStopSec=.5\n')
                subprocess.run(['bash', str(ROOT / 'scripts/user-systemctl-legacy.sh'), '--user', 'start', name], env=env,
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True, timeout=10)
            legacy = {'supervision': aggregate(pids('.supervisor.pid')), 'business': aggregate(pids('.pid'))}
            report = json.loads(run('migrate', '--apply').stdout)
            assert report['completed'] and all(row['success'] for row in report['results'])
            scans = set()
            for entry in Path('/proc').iterdir():
                if not entry.name.isdigit():
                    continue
                try:
                    args = (entry / 'cmdline').read_bytes().split(b'\0')
                    if args and Path(os.fsdecode(args[0])).name == 's6-svscan' and os.fsencode(scan) in args:
                        scans.add(int(entry.name))
                except (OSError, ValueError):
                    pass
            assert len(scans) == 1, f'Expected one isolated s6-svscan, found {len(scans)}'
            native = {'supervision': aggregate(pids('.supervisor.pid') | scans), 'business': aggregate(pids('.pid'))}
            before = legacy['supervision']['median_total_rss_kib']
            after = native['supervision']['median_total_rss_kib']
            return {'scope': f'{count} identical /bin/sleep services in temporary HOME; supervision excludes business processes; 3 RSS samples per group',
                    'legacy': legacy, 'native_s6': native, 'supervision_rss_reduction_percent': round(100 * (1 - after / before), 2)}
        finally:
            for name in names:
                subprocess.run([str(binary), '--user', 'stop', name], env=env, capture_output=True, timeout=15)
            if (scan / '.s6-svscan/control').exists():
                for service in scan.iterdir():
                    if (service / 'supervise/control').exists():
                        subprocess.run(['s6-svc', '-x', str(service)], capture_output=True, timeout=5)
                subprocess.run(['s6-svscanctl', '-t', str(scan)], capture_output=True, timeout=5)
                until(lambda: subprocess.run(['s6-svscanctl', '-a', str(scan)], capture_output=True).returncode != 0)


def web(unified=False):
    if not sockets_allowed():
        return {'status': 'skipped', 'reason': 'Loopback sockets are not permitted'}
    harness = Harness(unified=unified)
    try:
        harness.post('/api/poll', {'refresh': True})
        streams = [EventStream(harness) for _ in range(4)]
        for stream in streams:
            assert stream.status == 200
            stream.next()
        before = process(harness.web.pid)
        started = time.monotonic()
        harness.post('/api/action', {'type': 'showLogs', 'id': 'long-job'})
        until(lambda: '中文输出' in harness.post('/api/poll', {})[1]['output'])
        poll_bytes = 0
        for _ in range(20):
            time.sleep(.2)
            for _ in range(4):
                harness.post('/api/poll', {})
                poll_bytes += harness.last_size
        after = process(harness.web.pid)
        elapsed = time.monotonic() - started
        metrics = harness.post('/api/metrics', {})[1]
        return {'status': 'measured', 'scope': 'Actual isolated Rust Web + gateway + Linux node; 4 SSE consumers and 80 full polls during the same 4-second log operation; SSE bytes include initial sync; HTTP headers excluded; CPU includes both methods',
                'unified_server': unified, 'server_process_count': 1 if unified else 2,
                'web_process_rss_kib': after['rss_kib'], 'gateway_process_rss_kib': process(harness.server.pid)['rss_kib'],
                'node_process_rss_kib': process(harness.node.pid)['rss_kib'], 'full_poll_bytes': poll_bytes,
                'sse_bytes': sum(stream.bytes for stream in streams), 'seconds': round(elapsed, 3),
                'web_cpu_percent': round(100 * (after['ticks'] - before['ticks']) / os.sysconf('SC_CLK_TCK') / elapsed, 2),
                'metrics': metrics}
    finally:
        harness.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--services', type=int, default=6)
    args = parser.parse_args()
    if platform.system() != 'Linux' or not shutil.which('s6-svscan'):
        parser.error('Linux and s6 are required')
    if not 1 <= args.services <= 32:
        parser.error('services must be between 1 and 32')
    core = Path(os.environ['CRANIBOX_BIN']).resolve() if os.environ.get('CRANIBOX_BIN') else None
    temporary = contextlib.ExitStack()
    if core:
        alias = Path(temporary.enter_context(tempfile.TemporaryDirectory(prefix='cranibox-measure-alias-'))) / 'workspace-services'
        alias.symlink_to(core)
        binary = alias
    else:
        binary = Path(os.environ.get('WORKSPACE_SERVICES_BIN', ROOT / 'target/debug/workspace-services'))
    args.report.parent.mkdir(parents=True, exist_ok=True)
    report = {'measured_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'kernel': platform.release(), 'architecture': platform.machine(),
              'commit': subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip(),
              'source': 'working tree including current changes; debug binaries unless environment overrides them',
              'service_supervision': services(binary, args.services)}
    with tempfile.TemporaryDirectory(prefix='cranibox-resource-payload-') as directory:
        payload = Path(directory) / 'payload.json'
        subprocess.run(['cargo', 'test', '--offline', '--locked', '-p', 'agent-workspace-web', 'measure_stream_payloads', '--', '--ignored'],
                       cwd=ROOT, env={**os.environ, 'CRANIBOX_STREAM_REPORT': str(payload)}, check=True, timeout=120)
        report['in_process_payloads'] = json.loads(payload.read_text())
    report['web_end_to_end'] = web(unified=core is not None)
    binaries = [('server', core), ('node', Path(os.environ.get('CRANIBOX_NODE_BIN', ROOT / 'target/debug/cranibox-node')))] if core else [('services', binary), ('web', Path(os.environ.get('WORKSPACE_WEB_BIN', ROOT / 'target/debug/workspace-web'))), ('runtime', Path(os.environ.get('WORKSPACE_RUNTIME_BIN', ROOT / 'target/debug/workspace-runtime')))]
    report['binaries'] = {name: {'path': str(path), 'bytes': path.stat().st_size, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()} for name, path in binaries}
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    temporary.close()
    print(args.report)


if __name__ == '__main__':
    main()
