"""Exercise browser APIs against the actual Rust Web host."""
import json
import os
import sys
import unittest

from workspace_web_harness import EventStream, Harness, WEB, sockets_allowed, until


@unittest.skipUnless(WEB.exists() and sys.platform.startswith('linux') and sockets_allowed(), 'Rust binaries, Linux and permitted loopback sockets required')
class WebOperationsTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness()
        self.addCleanup(self.h.close)

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



if __name__ == '__main__':
    unittest.main()
