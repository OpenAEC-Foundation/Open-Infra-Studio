"""MCP requests use the same public command contract as the desktop UI."""
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
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

    def test_geotechnics_command_lines(self):
        with patch.object(mcp.OCS, 'run', return_value={'ok': True}) as run:
            mcp.t_slope_stability({'line': '1A', 'layers': [[0, 'klei_schoon_slap'], [-3, 'zand_schoon_vast']], 'base': -20,
                                   'overrides': {'veen_slap': {'c': 2, 'phi': 17}}, 'load': [13, 2, 6], 'phreatic': -1.2})
            cmd = run.call_args.args[0]
            for part in ('CIV_SLOPE_STABILITY', 'LIJN=1A', 'FREATISCH=-1.2', 'BELASTING=13:2:6', 'LAGEN=0:klei_schoon_slap;-3:zand_schoon_vast', 'BASIS=-20', 'GROND=veen_slap:c=2,phi=17'):
                self.assertIn(part, cmd)
            mcp.t_settlement({'area': [[1, 2], [3, 4], [5, 6]], 'height': 1.5, 'years': 30, 'method': 'terzaghi'})
            cmd = run.call_args.args[0]
            for part in ('CIV_SETTLEMENT', 'GEBIED=1,2;3,4;5,6', 'HOOGTE=1.5', 'TIJD=30', 'METHODE=terzaghi'):
                self.assertIn(part, cmd)
            mcp.t_soil_profile({'line': '2B', 'radius': 40})
            self.assertEqual(run.call_args.args[0], 'CIV_SOIL_PROFILE LIJN=2B STRAAL=40')

if __name__=='__main__':
    unittest.main()


@unittest.skipIf(sys.platform in ('win32', 'darwin'), 'the flatpak is Linux only')
class FlatpakTest(unittest.TestCase):
    """#157: a flatpak window describes itself in its own <config>, with the
    pid inside the sandbox, and writes its results there."""

    def test_a_flatpak_window_is_found_and_answers_in_its_own_folder(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        native, flatpak = Path(tmp.name, 'config'), Path(tmp.name, 'flatpak')
        automation = flatpak / 'OpenCADStudio' / 'automation'
        automation.mkdir(parents=True)
        results = flatpak / 'OpenCADStudio' / 'open-infra' / 'api'
        (automation / 'w.json').write_text(json.dumps({'session_id': 'w', 'pid': 2, 'port': 5, 'token': 't'}))
        state = {'version': '2026.47', 'document_id': 1, 'documents': [{'id': 1, 'path': '/p/OIS leerproject.dwg'}]}

        def exchange(desc, request, timeout=120):
            if request['op'] == 'run':
                rid = request['cmd'].rsplit('ID=', 1)[1]
                results.mkdir(parents=True, exist_ok=True)
                (results / f'{rid}.json').write_text(json.dumps({'ok': True}))
                return {'ok': True}
            return state

        for p in (patch.object(mcp, 'AUTOMATION', str(native / 'OpenCADStudio' / 'automation')),
                  patch.object(mcp, 'RESULTS', str(native / 'OpenCADStudio' / 'open-infra' / 'api')),
                  patch.object(mcp, 'FLATPAK_CONFIG', str(flatpak)),
                  patch.object(mcp, '_exchange', exchange),
                  patch.object(mcp, '_alive', return_value=False),
                  patch.object(mcp.OCS, 'session', None)):
            p.start()
            self.addCleanup(p.stop)

        [window] = mcp.sessions()
        self.assertEqual((window['session_id'], window['pid'], window['results']), ('w', None, str(results)))
        self.assertTrue(mcp.t_status({})['ok'])


class SessionChoiceTest(unittest.TestCase):
    """Sol, round 1 of #164: with a native and a flatpak window, OI_SESSION
    that matches neither is an error, and oi_sessions names the window the
    next command goes to."""

    def setUp(self):
        native = {'kind': 'automation', 'session_id': 'n', 'port': 1, 'token': 't', 'pid': 41, 'document': '/p/Brug.dwg'}
        flatpak = {'kind': 'automation', 'session_id': 'f', 'port': 2, 'token': 't', 'pid': None, 'document': '/p/Kruising.dwg', 'results': '/f/api'}
        self.live = [flatpak, native]
        for p in (patch.object(mcp, 'sessions', return_value=self.live), patch.object(mcp.OCS, 'session', None)):
            p.start()
            self.addCleanup(p.stop)

    def test_an_unmatched_oi_session_is_refused(self):
        with patch.dict(os.environ, {'OI_SESSION': 'Weg.dwg'}):
            with self.assertRaisesRegex(mcp.OcsError, 'Weg.dwg'):
                mcp.OCS.pick()
            self.assertIsNone(mcp.t_sessions({})['selected'])

    def test_oi_sessions_names_the_window_a_command_uses(self):
        with patch.dict(os.environ, {'OI_SESSION': '41'}):
            self.assertEqual(mcp.t_sessions({})['selected'], 'n')
            self.assertEqual(mcp.OCS.pick()['session_id'], 'n')
        mcp.OCS.session = None
        self.assertEqual(mcp.t_sessions({})['selected'], 'f')


class TabSwitchTest(unittest.TestCase):
    """Sol, round 2 of #164: a chosen window stays chosen when its current
    drawing changes, in the list and for the next save."""

    def test_the_chosen_window_survives_a_tab_switch(self):
        docs = {'old': '/p/Weg.dwg', 'new': '/p/Brug.dwg'}
        sent = []

        def windows():
            return [{'kind': 'automation', 'session_id': sid, 'port': port, 'token': 't', 'pid': None, 'document_id': 1, 'document': docs[sid]}
                    for sid, port in (('new', 2), ('old', 1))]

        def exchange(desc, request, timeout=120):
            if request['op'] == 'state':
                return {'document_id': 1, 'documents': [{'id': 1, 'path': docs[desc['session_id']]}]}
            sent.append((desc['session_id'], request))
            return {'ok': True}

        for p in (patch.object(mcp, 'sessions', side_effect=windows), patch.object(mcp, '_exchange', exchange),
                  patch.object(mcp.OCS, 'session', None), patch.dict(os.environ, {}, clear=False)):
            p.start()
            self.addCleanup(p.stop)
        os.environ.pop('OI_SESSION', None)
        self.assertFalse(mcp.call_tool('oi_sessions', {'select': 'Weg.dwg'})['isError'])
        docs['old'] = '/p/Kruising.dwg'
        listed = mcp.call_tool('oi_sessions', {})
        self.assertFalse(listed['isError'])
        self.assertIn('"selected": "old"', listed['content'][0]['text'])
        self.assertFalse(mcp.call_tool('oi_save', {})['isError'])
        self.assertEqual([sid for sid, _ in sent], ['old'])


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
