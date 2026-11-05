"""MCP requests use the same public command contract as the desktop UI."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('infra_mcp', Path(__file__).resolve().parents[1] / 'mcp/open_infra_mcp.py')
mcp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mcp)

class InfraMcpTest(unittest.TestCase):
    def test_catalog_and_connection_arguments(self):
        with patch.object(mcp.OCS, 'run', return_value={'ok': True}) as run:
            mcp.t_infra_catalog({'query': 'lichtmast', 'category': 'verlichting'})
            self.assertIn('GIS_INFRA_CATALOG', run.call_args.args[0])
            mcp.t_infra_place({'item':'afsluiter-pe75', 'connect':'main', 'port':'out', 'own':'in', 'confirm':True})
            cmd=run.call_args.args[0]
            self.assertIn('CONNECT=main',cmd)
            self.assertIn('PORT=out',cmd)
            self.assertIn('OWN=in',cmd)

    def test_nonfinite_parameter_rejected_before_mutation(self):
        with patch.object(mcp.OCS,'run') as run:
            with self.assertRaises(ValueError):
                mcp.t_infra_place({'item':'zitbank','parameters':{'width':float('nan')}})
            run.assert_not_called()

    def test_planning_commands_do_not_accept_credentials(self):
        with patch.object(mcp.OCS, 'run', return_value={'ok': True}) as run:
            mcp.t_planning_search({'x': 100000, 'y': 400000})
            self.assertIn('GIS_PLAN_SEARCH X=100000 Y=400000', run.call_args.args[0])
            with self.assertRaises(ValueError):
                mcp.t_planning_search({'x': 100000})
            mcp.t_planning_import({'plan': 'NL.IMRO.0000.synthetic-TEST'})
            self.assertIn('PLAN=NL.IMRO.0000.synthetic-TEST', run.call_args.args[0])
            mcp.t_planning_gml({'path': 'C:/synthetic plan.gml'})
            self.assertEqual(run.call_args.args[0], 'GIS_PLAN_GML "C:/synthetic plan.gml"')
        for tool in mcp.TOOLS:
            if tool['name'].startswith('planning_'):
                self.assertNotIn('key', tool['inputSchema']['properties'])

    def test_service_country_filter_is_explicit(self):
        with patch.object(mcp.OCS, 'run', return_value={'ok':True}) as run:
            mcp.t_services({'country':'US;GLOBAL','query':'elevation'})
            self.assertIn('COUNTRY=US;GLOBAL',run.call_args.args[0])

if __name__=='__main__':
    unittest.main()


class SuperCloudTest(unittest.TestCase):
    """Route B of the SuperCloud contract (OpenAEC-Blueprint docs/supercloud-koppeling.md, #75)."""

    def tearDown(self):
        mcp.PREFIX = ''

    def test_desktop_names_unchanged(self):
        names = [t['name'] for t in mcp.listed_tools()]
        self.assertIn('oi_status', names)
        self.assertNotIn('infra_stand', names)

    def test_one_prefix_unique_names_and_old_names_still_work(self):
        mcp.PREFIX = 'infra'
        names = [t['name'] for t in mcp.listed_tools()]
        self.assertTrue(all(n.startswith('infra_') for n in names), names)
        self.assertEqual(len(names), len(set(names)))
        for n in ('infra_stand', 'infra_open', 'infra_opslaan', 'infra_civ_tin', 'infra_oi_status', 'infra_catalog'):
            self.assertIn(n, names)
        self.assertIs(mcp.find_tool('infra_civ_tin'), mcp.find_tool('civ_tin'))
        with patch.object(mcp.OCS, 'request', return_value={'ok': True}) as req:
            r = mcp.call_tool('infra_open', {'path': '/werkmap/tekening.dwg'})
            self.assertFalse(r['isError'])
            self.assertEqual(req.call_args.args[:2], ('open', {'path': '/werkmap/tekening.dwg'}))

    def test_streamable_http(self):
        import json, threading, urllib.request
        server = mcp.http.server.ThreadingHTTPServer(('127.0.0.1', 0), mcp.HttpHandler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            url = f'http://127.0.0.1:{server.server_address[1]}/mcp'
            def post(body):
                req = urllib.request.Request(url, json.dumps(body).encode(), {'Content-Type': 'application/json', 'Accept': 'application/json, text/event-stream'})
                with urllib.request.urlopen(req) as r:
                    return r.status, (json.loads(r.read() or b'null'))
            status, init = post({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': '2025-06-18'}})
            self.assertEqual((status, init['result']['serverInfo']['name']), (200, 'open-infra'))
            self.assertEqual(post({'jsonrpc': '2.0', 'method': 'notifications/initialized'})[0], 202)
            status, listed = post({'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'})
            self.assertGreater(len(listed['result']['tools']), 30)
        finally:
            server.shutdown()


class ToolsFileTest(unittest.TestCase):
    def test_mcp_tools_json_is_current(self):
        import json
        mcp.PREFIX = 'infra'
        try:
            want = mcp.listed_tools()
        finally:
            mcp.PREFIX = ''
        have = json.loads((Path(__file__).resolve().parents[1] / 'mcp/mcp-tools.json').read_text(encoding='utf-8'))
        self.assertEqual(have, want, 'regenerate: python3 mcp/open_infra_mcp.py --prefix infra --tools-json > mcp/mcp-tools.json')
