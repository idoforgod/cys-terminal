import pathlib,sys,unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from supervisor import PhaseClock,Unavailable,CAPS
class PhaseTests(unittest.TestCase):
    def test_heartbeat_cannot_extend_cap(self):
        c=PhaseClock(10)
        self.assertEqual(c.observe({'phase':'setup','monotonic_ns':11_000_000_000},12),610)
        self.assertEqual(c.observe({'phase':'setup','monotonic_ns':500_000_000_000},501),610)
        with self.assertRaises(Unavailable):c.observe({'phase':'network_boundary','monotonic_ns':612_000_000_000},613)
    def test_no_reentry_or_future_clock(self):
        c=PhaseClock(0);c.observe({'phase':'setup','monotonic_ns':1},1)
        c.observe({'phase':'network_boundary','monotonic_ns':2},2)
        with self.assertRaises(Unavailable):c.observe({'phase':'setup','monotonic_ns':3},3)
        with self.assertRaises(Unavailable):PhaseClock(0).observe({'phase':'setup','monotonic_ns':2_000_000_000},1)
    def test_allocations_still_thirty_minutes(self):self.assertEqual(sum(CAPS.values()),1800)

if __name__=='__main__':unittest.main()
