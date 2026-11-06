"""Security of the MCP server's streamable HTTP transport.

The MCP specification (2025-06-18, basic/transports, "Security Warning"):
servers MUST validate the Origin header, SHOULD bind only to localhost when
running locally, and SHOULD authenticate every connection. These tests
pin that down: a token is required off loopback, Host and Origin are checked
on every request (DNS rebinding), the token is compared in constant time, and
only application/json is accepted for JSON-RPC.
"""
import http.client
import importlib.util
import io
import json
import os
import threading
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

spec = importlib.util.spec_from_file_location('infra_mcp_http', Path(__file__).resolve().parents[1] / 'mcp/open_infra_mcp.py')
mcp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mcp)

TOKEN = 'k' * 40
CLEAN = {'OI_MCP_TOKEN': '', 'OI_MCP_HOSTS': '', 'OI_MCP_ORIGINS': ''}
INIT = {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': '2025-06-18'}}
CALL = {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call', 'params': {'name': 'oi_status', 'arguments': {}}}


class Server:
    """The handler on 127.0.0.1 with a free port, as `--http <port>` starts it."""

    def __enter__(self):
        self.server = mcp.http.server.ThreadingHTTPServer(('127.0.0.1', 0), mcp.HttpHandler)
        self.port = self.server.server_address[1]
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        return self

    def __exit__(self, *exc):
        self.server.shutdown()
        self.server.server_close()

    def send(self, body=INIT, method='POST', path='/mcp', host=None, **headers):
        c = http.client.HTTPConnection('127.0.0.1', self.port, timeout=10)
        try:
            data = body if isinstance(body, bytes) else json.dumps(body).encode()
            c.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
            c.putheader('Host', host if host is not None else f'127.0.0.1:{self.port}')
            hdrs = {'Content-Type': 'application/json', 'Accept': 'application/json, text/event-stream'}
            hdrs.update({k.replace('_', '-'): v for k, v in headers.items()})
            for k, v in hdrs.items():
                if v is not None:
                    c.putheader(k, v)
            if method == 'POST':
                c.putheader('Content-Length', str(len(data)))
            c.endheaders(data if method == 'POST' else None)
            r = c.getresponse()
            raw = r.read()
            return r.status, (json.loads(raw) if raw else None)
        finally:
            c.close()


class BindingTest(unittest.TestCase):
    """Starting: a token is required unless the server listens on loopback only."""

    def start(self, where, **env):
        err = io.StringIO()
        with patch.dict(os.environ, dict(CLEAN, **env)), patch.object(mcp.http.server, 'ThreadingHTTPServer') as cls, \
                patch.object(mcp.sys, 'stderr', err):
            try:
                mcp.serve_http(where)
                refused = None
            except SystemExit as e:
                refused = e
        return refused, cls, err.getvalue()

    def test_all_interfaces_without_token_is_refused(self):
        for where in ('0.0.0.0:8765', '[::]:8765', '192.0.2.10:8765', 'example.invalid:8765'):
            with self.subTest(where=where):
                refused, cls, err = self.start(where)
                self.assertIsNotNone(refused, f'{where} started without a token')
                self.assertNotEqual(refused.code, 0)
                cls.assert_not_called()
                self.assertIn('OI_MCP_TOKEN', err)

    def test_empty_or_short_token_is_refused_off_loopback(self):
        for token in ('', '   ', 'short-token'):
            with self.subTest(token=token):
                refused, cls, err = self.start('0.0.0.0:8765', OI_MCP_TOKEN=token)
                self.assertIsNotNone(refused)
                cls.assert_not_called()
                self.assertIn('OI_MCP_TOKEN', err)

    def test_all_interfaces_with_token_starts(self):
        refused, cls, _ = self.start('0.0.0.0:8765', OI_MCP_TOKEN=TOKEN)
        self.assertIsNone(refused)
        self.assertEqual(cls.call_args.args[0], ('0.0.0.0', 8765))
        cls.return_value.serve_forever.assert_called_once()

    def test_loopback_without_token_starts(self):
        # localhost is bound as 127.0.0.1, not looked up again by bind().
        for where in ('8765', '127.0.0.1:8765', 'localhost:8765'):
            with self.subTest(where=where):
                refused, cls, _ = self.start(where)
                self.assertIsNone(refused)
                self.assertEqual(cls.call_args.args[0], ('127.0.0.1', 8765))

    def test_host_name_needs_a_token_whatever_dns_says(self):
        # A name that resolves to 127.0.0.1 now may resolve to 0.0.0.0 when
        # bind() looks it up again: the exemption may not depend on DNS.
        loop = [(mcp.socket.AF_INET, mcp.socket.SOCK_STREAM, 6, '', ('127.0.0.1', 0))]
        with patch.object(mcp.socket, 'getaddrinfo', return_value=loop) as dns:
            self.assertFalse(mcp.is_loopback('rebind.invalid'))
            refused, cls, err = self.start('rebind.invalid:8765')
            dns.assert_not_called()
        self.assertIsNotNone(refused)
        cls.assert_not_called()
        self.assertIn('OI_MCP_TOKEN', err)

    def test_ipv6_loopback_counts_as_loopback(self):
        self.assertTrue(mcp.is_loopback('::1'))
        self.assertTrue(mcp.is_loopback('127.0.0.1'))
        self.assertTrue(mcp.is_loopback(''))
        self.assertTrue(mcp.is_loopback('localhost'))
        self.assertFalse(mcp.is_loopback('::'))
        self.assertFalse(mcp.is_loopback('0.0.0.0'))
        self.assertFalse(mcp.is_loopback('192.0.2.10'))


class RequestTest(unittest.TestCase):
    """Every request: Host and Origin, the token and the Content-Type."""

    def setUp(self):
        env = patch.dict(os.environ, CLEAN)
        env.start()
        self.addCleanup(env.stop)
        tool = patch.object(mcp, 'call_tool', return_value={'content': [{'type': 'text', 'text': 'ok'}], 'isError': False})
        self.tool = tool.start()
        self.addCleanup(tool.stop)

    def test_local_route_keeps_working(self):
        with Server() as s:
            status, init = s.send(INIT)
            self.assertEqual((status, init['result']['serverInfo']['name']), (200, 'open-infra'))
            self.assertEqual(s.send({'jsonrpc': '2.0', 'method': 'notifications/initialized'})[0], 202)
            for host in (f'localhost:{s.port}', 'localhost', f'[::1]:{s.port}', f'127.0.0.1:{s.port}'):
                with self.subTest(host=host):
                    self.assertEqual(s.send(INIT, host=host)[0], 200)
            self.assertEqual(s.send(CALL, Content_Type='application/json; charset=utf-8')[0], 200)
            self.tool.assert_called_once()

    def test_sol_probe_is_refused_before_the_tool(self):
        # The probe from the review: no token, a foreign Origin and Host and text/plain.
        with Server() as s:
            status, _ = s.send(CALL, host='attacker.example:8765', Origin='http://attacker.example', Content_Type='text/plain')
            self.assertEqual(status, 403)
        self.tool.assert_not_called()

    def test_foreign_origin_is_refused(self):
        with Server() as s:
            for origin in ('http://attacker.example', 'null', f'http://localhost:{s.port + 1}'):
                with self.subTest(origin=origin):
                    self.assertEqual(s.send(CALL, Origin=origin)[0], 403)
            self.assertEqual(s.send(None, method='GET', Origin='http://attacker.example')[0], 403)
        self.tool.assert_not_called()

    def test_foreign_host_is_refused(self):
        # DNS rebinding: attacker.example now resolves to 127.0.0.1.
        with Server() as s:
            for host in (f'attacker.example:{s.port}', 'attacker.example', 'localhost.attacker.example', '',
                         f'[::1].attacker.example:{s.port}', '[127.0.0.1].attacker.example', '[::1]x', '[::1', f'localhost:{s.port}x'):
                with self.subTest(host=host):
                    self.assertEqual(s.send(CALL, host=host)[0], 403)
            self.assertEqual(s.send(None, method='GET', host='attacker.example')[0], 403)
        self.tool.assert_not_called()

    def test_listed_host_and_origin_are_allowed(self):
        with patch.dict(os.environ, {'OI_MCP_HOSTS': 'open-infra, mcp.example:443', 'OI_MCP_ORIGINS': 'https://cloud.example'}):
            with Server() as s:
                self.assertEqual(s.send(CALL, host='open-infra:8765', Origin='https://cloud.example')[0], 200)
                self.assertEqual(s.send(CALL, host='MCP.example', Origin='https://cloud.example/')[0], 200)
                self.assertEqual(s.send(CALL, host='other.example', Origin='https://cloud.example')[0], 403)
                self.assertEqual(s.send(CALL, host='open-infra', Origin='https://cloud.example.attacker')[0], 403)
        self.assertEqual(self.tool.call_count, 2)

    def test_wrong_content_type_is_refused(self):
        with Server() as s:
            for ctype in ('text/plain', 'application/x-www-form-urlencoded', 'multipart/form-data; boundary=x', 'application/jsonx', None):
                with self.subTest(ctype=ctype):
                    self.assertEqual(s.send(CALL, Content_Type=ctype)[0], 415)
        self.tool.assert_not_called()

    def test_token_when_set(self):
        with patch.dict(os.environ, {'OI_MCP_TOKEN': TOKEN}):
            with Server() as s:
                self.assertEqual(s.send(CALL)[0], 401)
                self.assertEqual(s.send(CALL, Authorization='Bearer wrong')[0], 401)
                self.assertEqual(s.send(CALL, Authorization=f'Basic {TOKEN}')[0], 401)
                self.assertEqual(s.send(CALL, Authorization='Bearer ' + TOKEN[:-1])[0], 401)
                self.tool.assert_not_called()
                with patch.object(mcp.hmac, 'compare_digest', wraps=mcp.hmac.compare_digest) as cmp:
                    self.assertEqual(s.send(CALL, Authorization=f'Bearer {TOKEN}')[0], 200)
                    cmp.assert_called()
                self.assertEqual(s.send(CALL, Authorization=f'bearer {TOKEN}')[0], 200)
        self.assertEqual(self.tool.call_count, 2)


if __name__ == '__main__':
    unittest.main()
