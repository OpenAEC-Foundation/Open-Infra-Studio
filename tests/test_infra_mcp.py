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
