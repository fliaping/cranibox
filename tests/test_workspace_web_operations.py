"""Exercise browser APIs against actual Rust services and a connected node."""
import json
import os
from pathlib import Path
import sys
import time
import unittest

from workspace_web_harness import EventStream, Harness, RUNTIME, WEB, sockets_allowed, until


@unittest.skipUnless(WEB.exists() and RUNTIME.exists() and sys.platform.startswith('linux') and sockets_allowed(), 'Rust binaries, Linux and permitted loopback sockets required')
class WebOperationsTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness()
        self.addCleanup(self.h.close)

    def test_node_registration_rotation_and_secret_boundaries(self):
        h = self.h
        self.assertEqual(h.post('/api/nodes/enroll', {'id':'../bad'})[0], 400)
        status, secret = h.post('/api/nodes/enroll', {'id':'new-node'})
        self.assertEqual(status, 200)
        self.assertEqual(h.post('/api/nodes/enroll', {'id':'new-node'})[0], 400)
        rotated = h.post('/api/nodes/enroll', {'id':'new-node','rotate':True})[1]
        self.assertNotEqual(rotated['token'], secret['token'])
        snapshot = h.post('/api/poll', {})[1]
        self.assertNotIn(secret['token'], json.dumps(snapshot))
        self.assertNotIn(rotated['token'], json.dumps(snapshot))
        self.assertEqual(h.credentials.stat().st_mode & 0o777, 0o600)
        self.assertEqual(h.post('/api/nodes/revoke', {'id':'new-node'})[0], 200)
        self.assertNotIn('new-node', json.loads(h.credentials.read_text())['nodes'])
        self.assertEqual(h.post('/api/nodes/enroll', {'id':'unauthenticated'})[0], 200)
        self.assertEqual(h.http('POST','/api/nodes/jobs',{},headers={'Cookie':h.cookie})[0], 401)

    def test_remote_execution_incremental_output_cancel_and_gateway_restart(self):
        h = self.h
        job_id = h.remote_job("print('远程输出')")
        until(lambda: h.post('/api/nodes/job', {'id':job_id})[1]['outcome'] != 'running')
        value = h.post('/api/nodes/job', {'id':job_id,'stdoutCursor':0,'stderrCursor':0})[1]
        self.assertEqual(value['stdoutDelta'], '远程输出\n')
        self.assertEqual(value['program'], sys.executable)
        cursor = value['stdoutCursor']
        self.assertEqual(h.post('/api/nodes/job', {'id':job_id,'stdoutCursor':cursor})[1]['stdoutDelta'], '')
        self.assertIn(job_id, [v['id'] for v in h.post('/api/nodes/jobs',{})[1]['jobs']])
        long = h.remote_job("import time; print('waiting',flush=True); time.sleep(60)", timeout=120)
        h.post('/api/nodes/cancel', {'id':long})
        until(lambda: h.post('/api/nodes/job',{'id':long})[1]['outcome'] != 'running')
        self.assertEqual(h.post('/api/nodes/job',{'id':long})[1]['outcome'], 'cancelled')
        h.stop(h.server)
        h.start_gateway()
        self.assertEqual(h.post('/api/nodes/job',{'id':job_id})[1]['stdout'], '远程输出\n')
        self.assertEqual(h.post('/api/nodes/job',{'id':job_id})[1]['outcome'], 'exited')

    def test_four_streams_use_incremental_utf8_logs_and_bound_connections(self):
        h = self.h
        streams = [EventStream(h) for _ in range(4)]
        for stream in streams:
            self.assertEqual(stream.status, 200)
            self.assertIn('snapshot', stream.next())
        self.assertEqual(EventStream(h).status, 429)
        self.assertEqual(h.post('/api/action',{'type':'showLogs','id':'long-job'})[0], 202)
        until(lambda: '中文输出' in ''.join(v.get('outputDelta','') for v in streams[0].values))
        until(lambda: '中文输出' in h.post('/api/poll',{})[1]['output'])
        self.assertNotIn('\ufffd', h.post('/api/poll',{})[1]['output'])
        records = h.post('/api/history/list',{})[1]['records']
        self.assertEqual(len(records), 1)
        record_id = records[0]['id']
        value = h.post('/api/history/get',{'id':record_id,'outputCursor':0})[1]
        self.assertTrue(value['outputReset'])
        self.assertTrue(value['truncated'])
        self.assertGreater(value['outputCursor'], 128 * 1024)
        repeated = h.post('/api/history/get',{'id':record_id,'outputCursor':value['outputCursor']})[1]
        self.assertNotIn('Z', repeated['outputDelta'])
        self.assertTrue(all('snapshot' not in v for v in streams[0].values[1:]))
        h.post('/api/cancel',{})
        until(lambda: not h.post('/api/poll',{})[1]['busy'])
        self.assertEqual(h.post('/api/history/get',{'id':record_id})[1]['outcome'], 'cancelled')
        for stream in streams:
            stream.close()
        until(lambda: EventStream(h).status == 200)

    def test_web_crash_preserves_history_and_does_not_replay(self):
        h = self.h
        h.post('/api/action',{'type':'showLogs','id':'long-job'})
        until(lambda: (h.root / 'child.pid').exists())
        record = h.post('/api/history/list',{})[1]['records'][0]
        until(lambda: '中文输出' in json.loads((h.root / '.local/state/cranibox/web-history' / (record['id']+'.json')).read_text())['output'])
        h.stop(h.web, force=True)
        h.start_web()
        h.login()
        value = h.post('/api/history/get',{'id':record['id']})[1]
        self.assertEqual(value['outcome'], 'interrupted')
        self.assertIn('中文输出', value['output'])
        calls = [json.loads(line) for line in (h.root / 'calls').read_text().splitlines()]
        self.assertEqual(sum(args[:2] == ['logs','long-job'] for args in calls), 1)
        self.assertEqual((h.root / '.local/state/cranibox/web-history' / (record['id']+'.json')).stat().st_mode & 0o777, 0o600)

    def test_gateway_crash_marks_job_interrupted_without_replay(self):
        h = self.h
        marker = h.root / 'executions'
        job_id = h.remote_job(f"from pathlib import Path; import time; p=Path({str(marker)!r}); p.write_text(p.read_text()+'x' if p.exists() else 'x'); print('started',flush=True); time.sleep(60)", timeout=120)
        until(marker.exists)
        h.stop(h.server, force=True)
        h.start_gateway()
        value = h.post('/api/nodes/job',{'id':job_id})[1]
        self.assertEqual(value['outcome'], 'interrupted')
        until(lambda: h.gateway('/api/v1/nodes')['nodes'][0]['online'])
        self.assertEqual(marker.read_text(), 'x')

    def test_gateway_proxy_keeps_event_stream_open_past_old_timeout(self):
        h = self.h
        stream = EventStream(h, h.gateway_port)
        self.assertEqual(stream.status, 200)
        self.assertIn('snapshot', stream.next())
        # The old total request timeout closed streaming responses after 40 seconds.
        time.sleep(41)
        h.post('/api/action',{'type':'serviceAction','id':'demo','kind':'systemd','action':'start'})
        until(lambda: any('completed service start demo' in v.get('outputDelta','') for v in stream.values))


if __name__ == '__main__':
    unittest.main()
