"""The actual collector, not the design simulation, owns these rejection contracts."""
import importlib.util
import pathlib
import sys
import unittest
ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import protocol

class ProtocolTests(unittest.TestCase):
    def test_fixed_population_and_cost(self):
        rows = protocol.plan()
        self.assertEqual(len({r['id'] for r in rows}), 740)
        self.assertEqual(protocol.cost()['runner_minutes'], 1190)
        self.assertEqual(len(protocol.matrix()), 14)
    def test_collector_exists_independently(self):
        spec = importlib.util.find_spec('collector')
        self.assertIsNotNone(spec, 'COLLECTOR_NOT_IMPLEMENTED: simulator cannot stand in for it')

if __name__ == '__main__': unittest.main()
