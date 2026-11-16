"""The MCP server's tool classes and its propose and apply modes (#92).

No Open CAD Studio: the server's requests are caught and answered as the
plugins would, so these tests show what the server sends for an assistant.
"""
import importlib.util
import json
import os
import re
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('assistant_mcp', Path(__file__).resolve().parents[1] / 'mcp/open_infra_mcp.py')
mcp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mcp)


class ToolClassTest(unittest.TestCase):
    def test_every_tool_has_a_class(self):
        # A new tool needs its class in mcp.CLASS: read, edit, export or save.
        self.assertEqual(mcp.unclassified(), [], 'add the new tool to CLASS in mcp/open_infra_mcp.py')
        self.assertEqual(set(mcp.CLASS) - {t['name'] for t in mcp.TOOLS}, set(), 'CLASS names a tool that is gone')

    def test_a_tool_without_class_is_caught(self):
        @mcp.tool('zz_unclassified_test_tool', 'test only')
        def _t(a):
            return {}
        try:
            self.assertEqual(mcp.unclassified(), ['zz_unclassified_test_tool'])
            listed = next(t for t in mcp.listed_tools() if t['name'] == 'zz_unclassified_test_tool')
            self.assertNotIn('annotations', listed)
        finally:
            mcp.TOOLS.pop()

    def test_annotations_follow_the_class(self):
        listed = {t['name']: t for t in mcp.listed_tools()}
        for name, t in listed.items():
            kind = t['_meta']['org.openaec/class']
            self.assertEqual(kind, mcp.CLASS[name])
            a = t['annotations']
            self.assertEqual(set(a), {'readOnlyHint', 'destructiveHint', 'idempotentHint', 'openWorldHint'})
            self.assertEqual(a['readOnlyHint'], kind == 'read', name)
            if kind == 'read':
                self.assertFalse(a['destructiveHint'], name)
        self.assertEqual(listed['oi_status']['_meta']['org.openaec/class'], 'read')
        self.assertEqual(listed['civ_tin']['_meta']['org.openaec/class'], 'edit')
        self.assertTrue(listed['civ_tin']['annotations']['openWorldHint'])
        self.assertEqual(listed['ifc_export']['_meta']['org.openaec/class'], 'export')
        self.assertEqual(listed['oi_save']['_meta']['org.openaec/class'], 'save')
        self.assertTrue(listed['civ_feature_line_edit']['annotations']['destructiveHint'])
        self.assertTrue(listed['proposal_reject']['annotations']['destructiveHint'])


class FakePlugins:
    """Answers the server's run requests as the plugins do, and keeps the
    request files the server wrote beside them."""

    def __init__(self, results, data=None, ok=True):
        self.results, self.data, self.ok = results, data or {}, ok
        self.commands, self.requests, self.host = [], [], []

    def __call__(self, op, body=None, timeout=120):
        if op == 'state':
            return {'document_id': 1}
        cmd = body['cmd']
        m = re.search(r'\bID=(\S+)', cmd)
        if not m:
            self.host.append(cmd)
            return {'ok': True}
        rid = m.group(1)
        self.commands.append(cmd)
        req = Path(self.results) / f'{rid}.request.json'
        self.requests.append(json.loads(req.read_text()) if req.exists() else None)
        if req.exists():
            req.unlink()
        data = self.data(cmd) if callable(self.data) else self.data
        (Path(self.results) / f'{rid}.json').write_text(json.dumps({'ok': self.ok, 'message': 'done', 'data': data}))
        return {'ok': True}


class ModeTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.results = os.path.join(self.dir.name, 'api')
        self.settings = os.path.join(self.dir.name, 'settings.json')
        self.patches = [patch.object(mcp, 'RESULTS', self.results), patch.object(mcp, 'SETTINGS', self.settings),
                        patch.dict(os.environ, {}, clear=False)]
        for p in self.patches:
            p.start()
        os.environ.pop('OI_ASSISTANT', None)
        os.environ.pop('OI_MCP_REVIEW', None)
        mcp.handle({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize',
                    'params': {'protocolVersion': '2025-06-18', 'clientInfo': {'name': 'test-client', 'version': '1.2'}}})

    def tearDown(self):
        for p in reversed(self.patches):
            p.stop()
        self.dir.cleanup()
        mcp.CLIENT = 'unknown client'

    def call(self, fake, name, args):
        with patch.object(mcp.OCS, 'request', side_effect=fake):
            return mcp.call_tool(name, args)

    def test_default_mode_is_propose(self):
        self.assertEqual(mcp.assistant_mode(), 'propose')
        Path(self.settings).write_text(json.dumps({'language': 'en', 'assistant': 'apply'}))
        self.assertEqual(mcp.assistant_mode(), 'apply')
        os.environ['OI_ASSISTANT'] = 'propose'
        self.assertEqual(mcp.assistant_mode(), 'propose', 'the environment of the server comes first')
        Path(self.settings).write_text('not json')
        os.environ.pop('OI_ASSISTANT')
        self.assertEqual(mcp.assistant_mode(), 'propose')

    def test_an_edit_in_propose_mode_says_who_asked_and_reports_the_proposal(self):
        proposal = {'proposal': {'id': 'P1', 'state': 'pending', 'added': 12, 'modified': 0, 'removed': 0},
                    'activity': {'state': 'pending', 'proposal': 'P1'}}
        fake = FakePlugins(self.results, proposal)
        r = self.call(fake, 'civ_tin', {'source': 'AHN', 'spacing': 5})
        self.assertFalse(r['isError'])
        self.assertTrue(fake.commands[0].startswith('CIV_TIN BRON=AHN AFSTAND=5'))
        req = fake.requests[0]
        self.assertEqual(req, {'client': 'test-client 1.2', 'tool': 'civ_tin', 'class': 'edit', 'mode': 'propose',
                               'arguments': {'source': 'AHN', 'spacing': 5}, 'review': False})
        text = r['content'][0]['text']
        self.assertIn("Proposal P1 is waiting for the user's review", text)
        self.assertEqual(r['structuredContent']['proposal']['id'], 'P1')

    def test_an_edit_in_apply_mode_is_marked_apply(self):
        Path(self.settings).write_text(json.dumps({'assistant': 'apply'}))
        fake = FakePlugins(self.results, {'activity': {'state': 'applied'}})
        r = self.call(fake, 'civ_alignment', {'pis': [[0, 0], [100, 0], [150, 50]], 'radii': [50], 'name': 'As 1'})
        self.assertFalse(r['isError'])
        self.assertEqual(fake.requests[0]['mode'], 'apply')
        self.assertNotIn('Proposal', r['content'][0]['text'])

    def test_exports_and_saves_are_logged_reads_are_not(self):
        fake = FakePlugins(self.results)
        self.call(fake, 'ifc_export', {})
        self.assertEqual(fake.requests[0]['class'], 'export')
        self.call(fake, 'oi_status', {})
        self.assertIsNone(fake.requests[1], 'a read tool sends no request file')
        self.call(fake, 'proposals_list', {})
        self.assertIsNone(fake.requests[2])
        # A save first notes itself in the drawing's log, then saves.
        self.call(fake, 'oi_save', {})
        self.assertTrue(fake.commands[3].startswith('CIV_ACTIVITEIT_NOTEER'))
        self.assertEqual((fake.requests[3]['class'], fake.requests[3]['note']), ('save', 'Saved the drawing'))
        self.assertEqual(fake.host, ['QSAVE'])
        self.assertFalse(list(Path(self.results).glob('*.request.json')), 'no request file left behind')

    def test_review_tools_are_refused_without_the_operators_consent(self):
        fake = FakePlugins(self.results)
        for name in ('proposal_accept', 'proposal_reject'):
            r = self.call(fake, name, {'id': 'P1'})
            self.assertTrue(r['isError'])
            self.assertIn('does not accept or reject its own proposals', r['content'][0]['text'])
        r = self.call(fake, 'oi_command', {'command': 'CIV_VOORSTEL_ACCEPTEREN VOORSTEL=P1'})
        self.assertTrue(r['isError'])
        self.assertEqual(fake.commands, [], 'nothing reached the plugins')

    def test_review_tools_with_consent_are_marked_as_review(self):
        os.environ['OI_MCP_REVIEW'] = '1'
        fake = FakePlugins(self.results)
        r = self.call(fake, 'proposal_reject', {'id': 'P2'})
        self.assertFalse(r['isError'])
        self.assertEqual(fake.commands[0].split(' ID=')[0], 'CIV_VOORSTEL_AFWIJZEN VOORSTEL=P2')
        self.assertTrue(fake.requests[0]['review'])
        self.assertEqual(fake.requests[0]['client'], 'test-client 1.2')
        self.call(fake, 'proposal_accept', {'id': 'P3'})
        self.assertEqual(fake.commands[1].split(' ID=')[0], 'CIV_VOORSTEL_ACCEPTEREN VOORSTEL=P3')

    def test_oi_command_in_propose_mode_runs_open_infra_and_view_commands_only(self):
        fake = FakePlugins(self.results)
        r = self.call(fake, 'oi_command', {'command': 'ERASE ALL'})
        self.assertTrue(r['isError'])
        self.assertIn('cannot be kept as a proposal', r['content'][0]['text'])
        self.assertEqual(fake.host, [])
        self.assertFalse(self.call(fake, 'oi_command', {'command': 'ZOOM EXTENTS'})['isError'])
        self.assertFalse(self.call(fake, 'oi_command', {'command': '-VIEW SWISO'})['isError'])
        self.assertEqual(fake.host, ['ZOOM EXTENTS', '-VIEW SWISO'])
        r = self.call(fake, 'oi_command', {'command': 'CIV_AS PI=0,0;10,0 NAAM=x'})
        self.assertFalse(r['isError'])
        self.assertEqual(fake.requests[-1]['tool'], 'oi_command')
        # In apply mode a host edit runs.
        os.environ['OI_ASSISTANT'] = 'apply'
        self.assertFalse(self.call(fake, 'oi_command', {'command': 'ERASE ALL'})['isError'])
        self.assertEqual(fake.host[-1], 'ERASE ALL')

    def test_client_name_comes_from_initialize(self):
        mcp.handle({'jsonrpc': '2.0', 'id': 2, 'method': 'initialize', 'params': {'clientInfo': {'name': 'x' * 200}}})
        self.assertEqual(len(mcp.CLIENT), 80)
        mcp.handle({'jsonrpc': '2.0', 'id': 3, 'method': 'initialize', 'params': {}})
        self.assertEqual(mcp.CLIENT, 'unknown client')


if __name__ == '__main__':
    unittest.main()
