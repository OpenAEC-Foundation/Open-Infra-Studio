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

    def test_exchange_commands(self):
        with patch.object(mcp.OCS, 'run', return_value={'ok': True}) as run:
            mcp.t_ifc_import({'path': 'C:/partner model.ifc', 'status': 'N'})
            self.assertEqual(run.call_args.args[0], 'CIV_IFC_IMPORT "C:/partner model.ifc" STATUS=N')
            mcp.t_ifc_ids({'ids': '/data/eisen.ids'})
            self.assertEqual(run.call_args.args[0], 'CIV_IFC_IDS IDS=/data/eisen.ids')
            mcp.t_ifc({'ids': '/data/eisen.ids'})
            self.assertIn('IDS=/data/eisen.ids', run.call_args.args[0])
            mcp.t_3d_export({'format': 'OBJ', 'selection': True})
            self.assertEqual(run.call_args.args[0], 'CIV_3D_EXPORT FORMAAT=OBJ SELECTIE')
            mcp.t_3d_import({'path': '/data/brug.stl', 'place': [105000, 425000, 1.5], 'scale': 0.001, 'rotation': 90})
            self.assertEqual(run.call_args.args[0], 'CIV_3D_IMPORT /data/brug.stl PLAATS=105000,425000,1.5 SCHAAL=0.001 ROTATIE=90')

    def test_layer_standard_show_and_set(self):
        with patch.object(mcp.OCS, 'run', return_value={'ok': True}) as run:
            mcp.t_layer_standard({})
            self.assertEqual(run.call_args.args[0], 'GIS_LAAGSTANDAARD')
            mcp.t_layer_standard({'id': 'ncs-us'})
            self.assertEqual(run.call_args.args[0], 'GIS_LAAGSTANDAARD ncs-us')
            with self.assertRaises(ValueError):
                mcp.t_layer_standard({'id': 'nlcs ID=x'})

    def test_service_country_filter_is_explicit(self):
        with patch.object(mcp.OCS, 'run', return_value={'ok':True}) as run:
            mcp.t_services({'country':'US;GLOBAL','query':'elevation'})
            self.assertIn('COUNTRY=US;GLOBAL',run.call_args.args[0])

    def test_water_commands(self):
        with patch.object(mcp.OCS, 'run', return_value={'ok': True}) as run:
            mcp.t_water_profile({'axis': '1F', 'q': 1.5, 'downstream_level': -0.4,
                                 'culverts': [{'station': 120, 'diameter': 0.8, 'length': 12, 'invert_in': 0.1, 'invert_out': 0.05},
                                              {'station': 300, 'width': 2, 'height': 1.5, 'length': 8, 'invert_in': 0, 'invert_out': 0}],
                                 'weirs': [{'station': 250, 'crest': 0.8, 'width': 2.5}]})
            cmd = run.call_args.args[0]
            self.assertTrue(cmd.startswith('CIV_WATERLOOP AS=1F Q=1.5 BENEDENSTROOMS=-0.4'), cmd)
            self.assertIn('DUIKER=120,0.8,12,0.1,0.05;300,2x1.5,8,0,0', cmd)
            self.assertIn('STUW=250,0.8,2.5', cmd)
            mcp.t_storage({'kind': 'wadi', 'bottom': 1.2, 'depth': 0.4, 'side_slope': 4, 'orifice_mm': 110, 'weir_level': 1.5, 'weir_width': 2, 'storm': 'T100', 'area_m2': 5000, 'tin': True})
            cmd = run.call_args.args[0]
            for part in ('CIV_BERGING', 'SOORT=wadi', 'TALUD=1:4', 'KNIJP=110', 'DREMPEL=1.5,2', 'BUI=T100', 'OPPERVLAK=5000', 'TIN=ja'):
                self.assertIn(part, cmd)
            mcp.t_pressure_network({'sources': [[10, 0, 30]], 'demand_total_ls': 2.5, 'hydrants': [[50, 0]], 'diameter_mm': 110})
            self.assertEqual(run.call_args.args[0], 'CIV_DRUKNET BRON=10,0,30 VRAAG=TOTAAL:2.5 DIAMETER=110 BRANDKRAAN=50,0')

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
