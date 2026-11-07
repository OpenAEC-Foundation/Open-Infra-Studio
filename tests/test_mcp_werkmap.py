"""Managed mode (OPENAEC_BEHEERD=1): every file route of the MCP server stays
inside the work folder (OPENAEC_MAP), also through symlinks, hard links, the
old tool names, files the plugins name themselves and free command lines.

The tests drive the real automation path (`Ocs.request`) against a stand-in
window that records what it is asked to open, save, capture or run."""
import importlib.util
import os
import re
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('infra_mcp_werkmap', Path(__file__).resolve().parents[1] / 'mcp/open_infra_mcp.py')
mcp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mcp)


class Window:
    """An Open CAD Studio window: records every request and answers like one."""

    def __init__(self, results, drawing):
        self.sent = []
        self.results = results
        self.drawing = drawing

    def __call__(self, desc, request, timeout=120):
        self.sent.append(request)
        op = request.get('op')
        if op == 'state':
            return {'document_id': 1, 'documents': [{'id': 1, 'path': self.drawing, 'name': 'tekening'}]}
        if op == 'run':
            m = re.search(r'\bID=(\w+)', request['cmd'])
            if m:
                (self.results / (m.group(1) + '.json')).write_text('{"ok": true}')
        if op == 'capture':
            Path(request['path']).write_bytes(b'png')
        if op == 'save':
            self.drawing = request['path']
        return {'ok': True}

    def actions(self):
        return [r for r in self.sent if r.get('op') != 'state']


class WorkFolderTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        base = Path(tmp.name)
        # Two levels deep, so Sol's `../../outside/…` reaches the folder beside it.
        self.work = base / 'container' / 'openaec'
        self.work.mkdir(parents=True)
        self.real = os.path.realpath(self.work)
        self.outside = base / 'outside'
        self.outside.mkdir()
        (self.outside / 'private.dwg').write_bytes(b'secret')
        (self.work / 'tekening.dwg').write_bytes(b'dwg')
        results = base / 'api'
        results.mkdir()
        self.window = Window(results, os.path.join(self.real, 'tekening.dwg'))
        for p in (patch.dict(os.environ, {'OPENAEC_BEHEERD': '1', 'OPENAEC_MAP': str(self.work)}),
                  patch.object(mcp, '_exchange', self.window),
                  patch.object(mcp, 'RESULTS', str(results)),
                  patch.object(mcp, 'PREFIX', 'infra'),
                  patch.object(mcp.OCS, 'session', {'kind': 'automation', 'session_id': 't', 'port': 1, 'token': 't', 'pid': None})):
            p.start()
            self.addCleanup(p.stop)

    def refused(self, name, args):
        r = mcp.call_tool(name, args)
        self.assertTrue(r['isError'], f'{name} {args}: {r}')
        self.assertEqual(self.window.actions(), [], f'{name} {args} reached the window')
        self.window.sent.clear()
        return r['content'][0]['text']

    def allowed(self, name, args):
        r = mcp.call_tool(name, args)
        self.assertFalse(r['isError'], f'{name} {args}: {r}')
        sent = self.window.actions()
        self.window.sent.clear()
        return sent

    def test_managed_writes_are_not_replayed_on_another_window(self):
        first = dict(mcp.OCS.session)
        second = dict(first, session_id='other', port=2)
        attempts = [('run', {'cmd': 'QSAVE', 'document_id': 1}),
                    ('save', {'path': str(self.work / 'nieuw.dwg')}),
                    ('open', {'path': str(self.work / 'tekening.dwg')})]
        for op, body in attempts:
            with self.subTest(op=op):
                sent = []
                mcp.OCS.session = first
                def exchange(desc, request, timeout=120):
                    if request['op'] == 'state':
                        path = self.window.drawing if desc['session_id'] == first['session_id'] else str(self.outside / 'private.dwg')
                        return {'document_id': 1, 'documents': [{'id': 1, 'path': path}]}
                    sent.append((desc['session_id'], request))
                    if desc['session_id'] == first['session_id']:
                        raise OSError('answer lost after request')
                    return {'ok': True}
                with patch.object(mcp, '_exchange', exchange), patch.object(mcp, 'sessions', return_value=[second]):
                    with self.assertRaises(mcp.OcsError):
                        mcp.OCS.request(op, body)
                self.assertEqual([sid for sid, _ in sent], [first['session_id']])
                self.assertIsNone(mcp.OCS.session)
        mcp.OCS.session = first

    def test_state_can_reconnect_after_a_window_goes_away(self):
        first = dict(mcp.OCS.session)
        second = dict(first, session_id='other', port=2)
        sent = []
        def exchange(desc, request, timeout=120):
            sent.append(desc['session_id'])
            if desc['session_id'] == first['session_id']:
                raise OSError('window closed')
            return {'document_id': 1}
        with patch.object(mcp, '_exchange', exchange), patch.object(mcp, 'sessions', return_value=[second]):
            self.assertEqual(mcp.OCS.request('state'), {'document_id': 1})
        self.assertEqual(sent, [first['session_id'], second['session_id']])
        self.assertEqual(mcp.OCS.session, second)

    def test_session_selection_cannot_move_a_guarded_request(self):
        first = dict(mcp.OCS.session)
        second = dict(first, session_id='other', port=2)
        checking = threading.Event()
        release = threading.Event()
        selected = threading.Event()
        choosing = threading.Event()
        replies = []
        sent = []
        states = 0
        def exchange(desc, request, timeout=120):
            nonlocal states
            sent.append((desc['session_id'], request.copy()))
            if request['op'] == 'state':
                states += 1
                if states == 2:
                    checking.set()
                    if not release.wait(3):
                        raise AssertionError('test did not release guard')
                path = self.window.drawing if desc['session_id'] == first['session_id'] else str(self.outside / 'private.dwg')
                return {'document_id': 1, 'documents': [{'id': 1, 'path': path}]}
            return {'ok': True}
        def save():
            replies.append(mcp.call_tool('infra_opslaan', {}))
        def choose():
            choosing.set()
            replies.append(mcp.call_tool('oi_sessions', {'select': 'other'}))
            selected.set()
        with patch.object(mcp, '_exchange', exchange), patch.object(mcp, 'sessions', return_value=[second]):
            saving = threading.Thread(target=save)
            selecting = threading.Thread(target=choose)
            saving.start()
            try:
                self.assertTrue(checking.wait(3), 'save did not reach guard')
                selecting.start()
                self.assertTrue(choosing.wait(3))
                # Before the fix the real pick finishes here and changes the
                # guarded request's session. With the lock it waits for save.
                selected.wait(0.1)
            finally:
                release.set()
                saving.join(3)
                if selecting.ident is not None:
                    selecting.join(3)
            self.assertFalse(saving.is_alive())
            self.assertFalse(selecting.is_alive())
        self.assertEqual(len(replies), 2)
        self.assertTrue(all(not r['isError'] for r in replies), replies)
        self.assertEqual([sid for sid, req in sent if req['op'] == 'run'], [first['session_id']])
        self.assertTrue(selected.is_set(), 'selection must still finish')

    def test_a_tab_switch_does_not_guard_the_wrong_document(self):
        states = 0
        sent = []
        def exchange(desc, request, timeout=120):
            nonlocal states
            if request['op'] == 'state':
                states += 1
                # host() read document 2, then the user activated document 1.
                return {'document_id': 2 if states == 1 else 1,
                        'documents': [{'id': 1, 'path': self.window.drawing},
                                      {'id': 2, 'path': str(self.outside / 'private.dwg')}]}
            sent.append(request)
            return {'ok': True}
        with patch.object(mcp, '_exchange', exchange):
            result = mcp.call_tool('infra_opslaan', {})
        self.assertTrue(result['isError'], result)
        self.assertEqual(sent, [], 'QSAVE must not reach the outside document')

    def test_an_outside_file_alias_cannot_choose_an_outside_output_folder(self):
        alias = self.outside / 'alias.dwg'
        os.symlink(self.work / 'tekening.dwg', alias)
        self.window.drawing = str(alias)
        self.refused('infra_civ_points_export', {})
        self.assertFalse((self.outside / 'alias_punten.csv').exists())

    def test_a_directory_alias_with_outputs_inside_the_work_folder_is_allowed(self):
        alias = self.outside / 'werkmap-link'
        os.symlink(self.work, alias)
        self.window.drawing = str(alias / 'tekening.dwg')
        sent = self.allowed('infra_civ_points_export', {})
        self.assertEqual(sent[0]['cmd'].split()[0], 'CIV_PUNTEN_EXPORT')

    def test_sol_cases_open_and_save_outside(self):
        cases = [
            ('infra_open', {'path': '../../outside/private.dwg'}),
            ('infra_open', {'path': str(self.outside / 'private.dwg')}),
            ('infra_opslaan', {'path': '../../outside/overwrite.dwg'}),
            ('infra_opslaan', {'path': str(self.outside / 'overwrite.dwg')}),
            ('infra_oi_save', {'path': '../../outside/overwrite.dwg'}),
            ('infra_oi_open', {'path': '../../outside/private.dwg'}),
            ('infra_oi_open', {'template': str(self.outside / 'private.dwg')}),
            ('oi_save', {'path': '../../outside/overwrite.dwg'}),
            ('oi_open', {'path': '../../outside/private.dwg'}),
        ]
        for name, args in cases:
            with self.subTest(name=name, args=args):
                self.refused(name, args)
        self.assertFalse((self.outside / 'overwrite.dwg').exists())

    def test_symlinks_in_the_work_folder(self):
        links = [('naar-buiten', self.outside), ('link.dwg', self.outside / 'private.dwg'), ('dood.dwg', self.outside / 'nieuw.dwg'),
                 ('tekening_punten.csv', self.outside / 'private.csv')]
        for name, target in links:
            os.symlink(target, self.work / name)
            with self.subTest(link=name):
                self.refused('infra_open', {'path': name})
                self.refused('infra_opslaan', {'path': 'nieuw.dwg'})
                self.refused('infra_opslaan', {})
                # The plugin names <drawing>_punten.csv itself.
                self.refused('infra_civ_points_export', {})
                self.refused('infra_quantities', {})
            os.remove(self.work / name)
        os.makedirs(self.work / 'levering')
        os.symlink(self.outside, self.work / 'levering' / 'diep')
        self.refused('infra_civ_klic3d', {'path': 'levering'})
        self.assertFalse((self.outside / 'private.csv').exists())

    def test_hard_link_to_a_file_outside(self):
        os.link(self.outside / 'private.dwg', self.work / 'hard.dwg')
        self.refused('infra_open', {'path': 'hard.dwg'})
        self.refused('infra_opslaan', {'path': 'hard.dwg'})
        self.refused('infra_nlcs_check', {})

    def test_backslashes_and_control_characters(self):
        self.refused('infra_open', {'path': '..\\..\\outside\\private.dwg'})
        self.refused('infra_opslaan', {'path': 'a.dwg\nb.dwg'})
        self.refused('infra_opslaan', {'path': 'a".dwg'})

    def test_paths_inside_the_work_folder_keep_working(self):
        sent = self.allowed('infra_open', {'path': 'tekening.dwg'})
        self.assertEqual((sent[0]['op'], sent[0]['path']), ('open', os.path.join(self.real, 'tekening.dwg')))
        sent = self.allowed('infra_open', {'path': str(self.work / 'tekening.dwg')})
        self.assertEqual(sent[0]['path'], os.path.join(self.real, 'tekening.dwg'))
        sent = self.allowed('oi_open', {'path': 'plannen/../tekening.dwg'})
        self.assertEqual(sent[0]['path'], os.path.join(self.real, 'tekening.dwg'))
        sent = self.allowed('infra_opslaan', {'path': 'plannen/nieuw.dwg'})
        self.assertEqual((sent[0]['op'], sent[0]['path']), ('save', os.path.join(self.real, 'plannen', 'nieuw.dwg')))
        sent = self.allowed('infra_opslaan', {})
        self.assertEqual((sent[0]['op'], sent[0]['cmd']), ('run', 'QSAVE'))
        sent = self.allowed('infra_oi_open', {})
        self.assertEqual(sent[0]['op'], 'new')

    def test_every_file_tool_is_confined(self):
        (self.work / 'in de map.xml').write_text('x')
        tools = ['planning_gml', 'ifc_export', 'civ_points_import', 'civ_points_export', 'civ_landxml', 'civ_klic3d',
                 'civ_citygml', 'civ_landxml_export', 'oi_capture']
        for name in tools:
            with self.subTest(name=name):
                self.refused(name, {'path': '../../outside/bestand.xml'})
                self.refused(name, {'path': str(self.outside / 'bestand.xml')})
                sent = self.allowed(name, {'path': 'in de map.xml'})
                body = sent[0].get('cmd') or sent[0].get('path')
                self.assertIn(os.path.join(self.real, 'in de map.xml'), body)
        sent = self.allowed('civ_points_import', {'path': 'PRIVATE'})
        self.assertIn(f'CIV_PUNTEN_IMPORT {self.real}/PRIVATE ', sent[0]['cmd'])
        self.refused('civ_klic3d', {})
        # A missing path with a comma made the plugin take the delivery imported last (review round 2).
        self.refused('civ_klic3d', {'path': 'bestaat,niet.zip'})
        self.refused('civ_klic3d', {'path': 'bestaat-niet.zip'})
        self.assertEqual(os.listdir(self.outside), ['private.dwg'])

    def test_capture_without_path_uses_its_own_folder(self):
        sent = self.allowed('oi_capture', {})
        self.assertTrue(os.path.basename(sent[0]['path']).startswith('capture-'))

    def test_commands_only_on_a_drawing_in_the_work_folder(self):
        for drawing in (None, str(self.outside / 'private.dwg')):
            with self.subTest(drawing=drawing):
                self.window.drawing = drawing
                self.refused('infra_civ_points_export', {})
                self.refused('infra_quantities', {})
                self.refused('infra_opslaan', {})
                # A view needs no file.
                self.allowed('infra_oi_view', {'preset': 'top'})
        # Saving it in the work folder makes the drawing usable again.
        self.allowed('infra_opslaan', {'path': 'nieuw.dwg'})
        sent = self.allowed('infra_civ_points_export', {})
        self.assertEqual(sent[0]['cmd'].split()[0], 'CIV_PUNTEN_EXPORT')
        self.assertNotIn('PAD=', sent[0]['cmd'])

    def test_no_free_command_lines(self):
        for cmd in ['CIV_INFO', f'"CIV_PUNTEN_IMPORT {self.outside}/private.csv"', 'CIV_"IFC BESTAND=../../outside/x.ifc"',
                    'CIV_PUNTEN_IMPORT punten.csv CODETABEL=../../outside/codes.csv', 'GIS_GEODATA output.dwg',
                    'GIS_KLIC_MAP', 'SAVEAS ../../outside/x.dwg', 'ZOOM EXTENTS']:
            with self.subTest(cmd=cmd):
                self.refused('infra_oi_command', {'command': cmd})
                self.refused('oi_command', {'command': cmd})

    def test_the_request_check_itself(self):
        for cmd in [f'"CIV_PUNTEN_IMPORT {self.outside}/private.csv"', 'CIV_"IFC BESTAND=x"', 'SAVEAS x.dwg', 'OPEN x.dwg',
                    'ZOOM EXTENTS\nOPEN x.dwg', 'CIV_INFO\nSAVEAS x.dwg', '-VIEW TOP OPEN', 'QSAVE x.dwg']:
            with self.subTest(cmd=cmd):
                with self.assertRaises(mcp.OcsError):
                    mcp.OCS.request('run', {'cmd': cmd})
                self.assertEqual(self.window.actions(), [])

    def test_values_cannot_add_an_argument(self):
        self.refused('infra_civ_tin', {'layer': 'x" BESTAND="../../outside/p.xyz'})
        self.refused('infra_civ_tin', {'layer': '"a b" BESTAND=../../outside/p.xyz'})
        self.refused('infra_civ_points_import', {'path': 'punten.csv', 'format': 'AUTO"" CODETABEL=../../outside/c.csv'})
        self.refused('infra_oi_site', {'x': 1, 'y': 2, 'name': 'a" PAD="x'})
        self.refused('infra_gis_server', {'id': 'x"'})
        sent = self.allowed('infra_civ_points_import', {'path': 'punten.csv', 'format': 'AUTO CODETABEL=../../outside/c.csv'})
        self.assertIn('FORMAAT="AUTO CODETABEL=../../outside/c.csv"', sent[0]['cmd'])

    def test_other_whitespace_cannot_split_an_argument(self):
        # The plugins split on every Unicode space; only a plain space is quoted (review round 2).
        for space in ('\u00a0', '\u2003', '\u202f', '\u3000', '\t'):
            with self.subTest(space=repr(space)):
                self.refused('infra_civ_points_export', {'format': f'PNEZD{space}PAD={self.outside}/overwrite.csv'})
                self.refused('infra_civ_points_import', {'path': 'punten.csv', 'format': f'AUTO{space}CODETABEL={self.outside}/codes.csv'})
                self.refused('infra_oi_site', {'x': 1, 'y': 2, 'name': f'a{space}b'})
                self.refused('infra_open', {'path': f'a{space}b.dwg'})
        sent = self.allowed('infra_oi_site', {'x': 1, 'y': 2, 'name': 'Den Haag'})
        self.assertTrue(sent[0]['cmd'].startswith('GIS_SITE RD 1 2 300 250 Den Haag'), sent[0]['cmd'])

    def test_assemblies_only_by_name(self):
        (self.outside / 'private.json').write_text('{}')
        for name, args in [('infra_civ_corridor_range', {'assembly': str(self.outside / 'private.json')}),
                           ('infra_civ_corridor_range', {'assembly': 'eigen.JSON'}),
                           ('infra_civ_junction', {'main': '1A', 'side': '2B', 'main_assembly': '../../outside/private.json'}),
                           ('infra_civ_junction', {'main': '1A', 'side': '2B', 'side_assembly': 'map/eigen'}),
                           ('infra_civ_roundabout', {'axes': ['1A', '2B', '3C'], 'assembly': str(self.outside / 'private.json')}),
                           ('infra_civ_assemblies', {'name': str(self.outside / 'private.json')})]:
            with self.subTest(name=name, args=args):
                self.refused(name, args)
        sent = self.allowed('infra_civ_corridor_range', {'assembly': 'stadsstraat'})
        self.assertIn('PROFIELTYPE=stadsstraat', sent[0]['cmd'])

    def test_lists_and_objects_are_not_text(self):
        # A list became its Python text, quotes and all (review round 3).
        inject = f'" FORMAAT=PNEZD PAD={self.outside}/overwrite.csv "'
        for value in ([inject], {'a': inject}):
            with self.subTest(value=value):
                self.refused('infra_civ_points_export', {'format': value})
                self.refused('infra_oi_site', {'x': 1, 'y': 2, 'name': value})
                self.refused('infra_gis_server', {'id': value})
                self.refused('infra_open', {'path': value})
                self.refused('infra_civ_corridor_range', {'assembly': value})

    def test_no_argument_of_any_tool_reaches_outside(self):
        """Every tool, every argument: a list, an object or text carrying a file
        argument outside the work folder never reaches the window."""
        def sample(schema):
            if 'enum' in schema:
                return schema['enum'][0]
            kind = schema.get('type')
            if kind == 'array':
                items = schema.get('items', {})
                if isinstance(items, list):
                    return [sample(i) for i in items]
                return [sample(items) for _ in range(max(schema.get('minItems', 2), 1))]
            return {'number': 1, 'boolean': False, 'object': {}}.get(kind, 'x')

        marker = f'{self.outside}/overwrite.csv'
        attempts = [[f'" PAD={marker} "'], {'k': f'" PAD={marker} "'}, f'x" PAD="{marker}', f'x\u00a0PAD={marker}']
        for t in mcp.TOOLS:
            props = t['inputSchema']['properties']
            for prop in props:
                # A plain path outside only where the argument is a file; elsewhere it is text (a search).
                for value in attempts + ([marker] if prop in ('path', 'template') else []):
                    args = {k: sample(props[k]) for k in t['inputSchema']['required']}
                    args[prop] = value
                    try:
                        mcp.call_tool(t['name'], args)
                    except Exception:  # a crash on a wrong type is not a file route
                        pass
                    self.assertNotIn(str(self.outside), repr(self.window.actions()), f"{t['name']} {prop}={value!r}")
                    self.window.sent.clear()
        self.assertEqual(os.listdir(self.outside), ['private.dwg'])

    def test_points_stay_points(self):
        sent = self.allowed('infra_place', {'item': 'kabel', 'path': [[0, 0], [10, 0]]})
        self.assertIn('PATH=0,0;10,0', sent[0]['cmd'])
        sent = self.allowed('infra_civ_alignment', {'pis': [[0, 0], [100, 0], [100, 100]], 'radii': [25]})
        self.assertIn('PI=0,0;100,0;100,100', sent[0]['cmd'])

    def test_view_still_works(self):
        sent = self.allowed('infra_oi_view', {'preset': 'swiso', 'style': 'gouraudshaded'})
        self.assertEqual([s['cmd'] for s in sent], ['-VIEW SWISO', 'VSCURRENT GOURAUDSHADED', 'ZOOM EXTENTS'])
        self.refused('infra_oi_view', {'preset': 'top open', 'zoom_extents': False})

    def test_without_a_work_folder_no_file_is_reachable(self):
        for value in (None, 'relatief/map', str(self.work / 'bestaat-niet')):
            with self.subTest(OPENAEC_MAP=value):
                env = {k: v for k, v in os.environ.items() if k != 'OPENAEC_MAP'}
                if value:
                    env['OPENAEC_MAP'] = value
                with patch.dict(os.environ, env, clear=True):
                    self.refused('infra_open', {'path': str(self.work / 'tekening.dwg')})
                    self.refused('infra_quantities', {})

    def test_desktop_unchanged(self):
        with patch.dict(os.environ, {'OPENAEC_BEHEERD': ''}):
            os.symlink(self.outside, self.work / 'naar-buiten')
            sent = self.allowed('oi_open', {'path': '../elders/tekening.dwg'})
            self.assertEqual(sent[0]['path'], '../elders/tekening.dwg')
            sent = self.allowed('oi_command', {'command': 'SAVEAS C:/elders/x.dwg'})
            self.assertEqual(sent[0]['cmd'], 'SAVEAS C:/elders/x.dwg')
            self.window.drawing = None
            sent = self.allowed('civ_points_export', {})
            self.assertEqual(sent[0]['cmd'].split()[0], 'CIV_PUNTEN_EXPORT')


if __name__ == '__main__':
    unittest.main()
