"""Managed mode (OPENAEC_BEHEERD=1): every file route of the MCP server stays
inside the work folder (OPENAEC_MAP), also through symlinks, hard links, the
old tool names and free command lines.

The tests drive the real automation path (`Ocs.request`) against a stand-in
window that records what it is asked to open, save, capture or run."""
import importlib.util
import os
import re
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('infra_mcp_werkmap', Path(__file__).resolve().parents[1] / 'mcp/open_infra_mcp.py')
mcp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mcp)


class Window:
    """An Open CAD Studio window: records every request and answers like one."""

    def __init__(self, results):
        self.sent = []
        self.results = results

    def __call__(self, desc, request, timeout=120):
        self.sent.append(request)
        op = request.get('op')
        if op == 'state':
            return {'document_id': 1}
        if op == 'run':
            m = re.search(r'\bID=(\w+)', request['cmd'])
            if m:
                (self.results / (m.group(1) + '.json')).write_text('{"ok": true}')
        if op == 'capture':
            Path(request['path']).write_bytes(b'png')
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
        os.symlink(self.outside, self.work / 'naar-buiten')
        os.symlink(self.outside / 'private.dwg', self.work / 'link.dwg')
        os.symlink(self.outside / 'nieuw.dwg', self.work / 'dood.dwg')
        results = base / 'api'
        results.mkdir()
        self.window = Window(results)
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

    def test_symlinks_out_of_the_work_folder(self):
        for name, args in [
            ('infra_open', {'path': 'naar-buiten/private.dwg'}),
            ('infra_open', {'path': 'link.dwg'}),
            ('infra_opslaan', {'path': 'naar-buiten/nieuw.dwg'}),
            ('infra_opslaan', {'path': 'link.dwg'}),
            ('infra_opslaan', {'path': 'dood.dwg'}),
        ]:
            with self.subTest(name=name, args=args):
                self.refused(name, args)

    def test_hard_link_to_a_file_outside(self):
        os.link(self.outside / 'private.dwg', self.work / 'hard.dwg')
        self.refused('infra_open', {'path': 'hard.dwg'})
        self.refused('infra_opslaan', {'path': 'hard.dwg'})

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
        tools = [('planning_gml', {}), ('ifc_export', {}), ('civ_points_import', {}), ('civ_points_export', {}),
                 ('civ_landxml', {}), ('civ_klic3d', {}), ('civ_citygml', {}), ('civ_landxml_export', {}), ('oi_capture', {})]
        for name, extra in tools:
            with self.subTest(name=name):
                self.refused(name, dict(extra, path='../../outside/bestand.xml'))
                self.refused(name, dict(extra, path=str(self.outside / 'bestand.xml')))
                self.refused(name, dict(extra, path='naar-buiten/bestand.xml'))
                sent = self.allowed(name, dict(extra, path='in de map.xml'))
                body = sent[0].get('cmd') or sent[0].get('path')
                self.assertIn(os.path.join(self.real, 'in de map.xml'), body)
        self.assertEqual(os.listdir(self.outside), ['private.dwg'])

    def test_capture_without_path_uses_its_own_folder(self):
        sent = self.allowed('oi_capture', {})
        self.assertTrue(os.path.basename(sent[0]['path']).startswith('capture-'))

    def test_command_lines(self):
        for cmd in ['CIV_PUNTEN_IMPORT ../../outside/private.csv',
                    f'CIV_PUNTEN_IMPORT "{self.outside}/met spatie.csv"',
                    'CIV_IFC BESTAND=../../outside/x.ifc',
                    f'CIV_PUNTEN_EXPORT PAD={self.outside}/x.csv',
                    'GIS_KLIC_MAP MAP=naar-buiten',
                    'GIS_EXPORT ../../outside/x.gpkg',
                    'SAVEAS ../../outside/x.dwg',
                    'OPEN ../../outside/private.dwg',
                    'ZOOM EXTENTS\nOPEN ../../outside/private.dwg',
                    'CIV_INFO\nSAVEAS x.dwg',
                    '-VIEW TOP OPEN']:
            with self.subTest(cmd=cmd):
                self.refused('infra_oi_command', {'command': cmd})
        sent = self.allowed('infra_oi_command', {'command': 'CIV_PUNTEN_IMPORT punten.csv FORMAAT=PNEZD'})
        self.assertTrue(sent[0]['cmd'].startswith(f'CIV_PUNTEN_IMPORT {self.real}/punten.csv FORMAAT=PNEZD ID='), sent[0]['cmd'])
        sent = self.allowed('infra_oi_command', {'command': 'GIS_SERVICES ZOEK="PE 110/90" HIER'})
        self.assertIn('ZOEK="PE 110/90" HIER', sent[0]['cmd'])
        sent = self.allowed('infra_oi_command', {'command': 'ZOOM EXTENTS'})
        self.assertEqual(sent[0]['cmd'], 'ZOOM EXTENTS')

    def test_values_cannot_smuggle_a_file_argument(self):
        self.refused('infra_civ_tin', {'layer': 'x" BESTAND="../../outside/p.xyz'})
        self.refused('infra_civ_tin', {'layer': '"a b" BESTAND=../../outside/p.xyz'})

    def test_view_still_works(self):
        sent = self.allowed('infra_oi_view', {'preset': 'swiso', 'style': 'gouraudshaded'})
        self.assertEqual([s['cmd'] for s in sent], ['-VIEW SWISO', 'VSCURRENT GOURAUDSHADED', 'ZOOM EXTENTS'])

    def test_without_a_work_folder_no_file_is_reachable(self):
        for value in (None, 'relatief/map', str(self.work / 'bestaat-niet')):
            with self.subTest(OPENAEC_MAP=value):
                env = {k: v for k, v in os.environ.items() if k != 'OPENAEC_MAP'}
                if value:
                    env['OPENAEC_MAP'] = value
                with patch.dict(os.environ, env, clear=True):
                    self.refused('infra_open', {'path': str(self.work / 'tekening.dwg')})

    def test_desktop_paths_unchanged(self):
        with patch.dict(os.environ, {'OPENAEC_BEHEERD': ''}):
            sent = self.allowed('oi_open', {'path': '../elders/tekening.dwg'})
            self.assertEqual(sent[0]['path'], '../elders/tekening.dwg')
            sent = self.allowed('oi_command', {'command': 'SAVEAS C:/elders/x.dwg'})
            self.assertEqual(sent[0]['cmd'], 'SAVEAS C:/elders/x.dwg')


if __name__ == '__main__':
    unittest.main()
