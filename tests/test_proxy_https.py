import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('proxy', Path(__file__).parents[1] / 'scripts/workspacectl-proxy.py')
proxy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proxy)


class HTTPSRoutes(unittest.TestCase):
    def test_tls_host_and_port(self):
        route = proxy.reverse_proxy_route(['api.example.com'], 'https://upstream.example.com:8443/', 'test')
        handler = route['handle'][0]
        self.assertEqual(handler['upstreams'], [{'dial': 'upstream.example.com:8443'}])
        self.assertEqual(handler['transport']['tls'], {'server_name': 'upstream.example.com'})
        self.assertEqual(handler['headers']['request']['set']['Host'], ['upstream.example.com:8443'])
        self.assertNotIn('rewrite', handler)

    def test_existing_local_routes(self):
        handler = proxy.reverse_proxy_route(['app.example.com'], '127.0.0.1:3000', 'test')['handle'][0]
        self.assertNotIn('transport', handler)
        self.assertNotIn('Host', handler['headers']['request']['set'])
        self.assertNotIn('X-Forwarded-For', handler['headers']['request']['set'])

    def test_rewrite_client_ip_headers(self):
        handler = proxy.reverse_proxy_route(
            ['app.example.com'],
            '127.0.0.1:3000',
            'test',
            rewrite_client_ip=True,
        )['handle'][0]
        headers = handler['headers']['request']['set']
        self.assertEqual(headers['X-Forwarded-For'], ['{http.request.remote.host}'])
        self.assertEqual(headers['X-Real-IP'], ['{http.request.remote.host}'])

    def test_origins_only(self):
        for value in ['https://u:p@example.com', 'https://example.com/path', 'https://example.com?token=x', 'ftp://example.com', 'https://example.com:99999']:
            with self.subTest(value=value), self.assertRaises(proxy.ProxyctlError):
                proxy.parse_target(value)
        self.assertEqual(proxy.parse_target('https://example.com/'), 'https://example.com')
        self.assertEqual(proxy.parse_target('http://example.com'), 'http://example.com')


if __name__ == '__main__':
    unittest.main()
