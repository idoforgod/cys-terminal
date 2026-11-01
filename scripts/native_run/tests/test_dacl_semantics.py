import pathlib, re, sys, unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from boundary import verify_dacl, protected_sddl, Unavailable, decode_allow_ace
import struct

# Independent fixture decoder: ONLY plain allow ACE specimens, never production.
ALIASES={'SY':'S-1-5-18','BA':'S-1-5-32-544','WD':'S-1-1-0'}
RIGHTS={'FA':0x1f01ff,'FR':0x120089,'FX':0x1200a0}
FLAGS={'OI':1,'CI':2,'NP':4,'IO':8,'ID':16}
def specimen(text):
    header=text[2:].split('(')[0]
    rows=[]
    for ace in re.findall(r'\(([^()]*)\)',text):
        kind,flags,mask,obj,inherit,sid=ace.split(';')
        bits=0
        for i in range(0,len(flags),2):bits|=FLAGS[flags[i:i+2]]
        rights=int(mask,16) if mask.startswith('0x') else RIGHTS[mask]
        rows.append([0 if kind=='A' else 1,bits,rights,ALIASES.get(sid,sid)])
    return {'present':True,'protected':'P' in header,'aces':rows}

def check(a,b):return verify_dacl(a,b,specimen(a),specimen(b))

class DaclSemantics(unittest.TestCase):
    BASE='D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;S-1-5-21-1-2-3-1001)'
    def test_representation_equivalence_both_directions(self):
        variants=[
            'D:PAI(A;CIOI;0x1f01ff;;;S-1-5-18)(A;OICI;FA;;;S-1-5-21-1-2-3-1001)',
            'D:PARAI(A;OICI;FA;;;S-1-5-21-1-2-3-1001)(A;OICI;FA;;;SY)',
        ]
        for b in variants:
            for a,c in [(self.BASE,b),(b,self.BASE)]:
                with self.subTest(a=a,b=c):self.assertTrue(check(a,c)['verified'])
    def test_permission_changes_refuse_both_directions(self):
        variants=[self.BASE+'(A;;FR;;;WD)',self.BASE.replace('FA;;;SY','FR;;;SY'),
                  self.BASE.replace('OICI;FA;;;SY','OI;FA;;;SY'),self.BASE.replace('D:P','D:'),
                  self.BASE.replace('1001','1002'),self.BASE.replace('(A;','(D;',1)]
        for b in variants:
            for a,c in [(self.BASE,b),(b,self.BASE)]:
                with self.subTest(a=a,b=c):
                    with self.assertRaises(Unavailable):check(a,c)
    def test_mismatch_contains_both_operands(self):
        b=self.BASE+'(A;;FR;;;WD)'
        with self.assertRaises(Unavailable) as caught:check(self.BASE,b)
        self.assertIn(self.BASE,str(caught.exception));self.assertIn(b,str(caught.exception))
    def test_read_execute_is_documented_numeric_mask(self):
        for role in ('root','bin'):
            text=protected_sddl('S-1-5-21-1-2-3-1001','S-1-15-2-456',role,True)
            self.assertIn(';0x1200a9;;;S-1-15-2-456)',text)

class BinaryAceTests(unittest.TestCase):
    def test_numeric_sid_and_all_mask_bits(self):
        raw=struct.pack('<BBHI',0,3,28,0x1f01ff)+bytes([1,3])+bytes.fromhex('000000000005')+struct.pack('<III',21,42,1001)
        self.assertEqual(decode_allow_ace(raw),[0,3,0x1f01ff,'S-1-5-21-42-1001'])
        for offset,value in [(0,1),(2,27),(8,2),(9,15)]:
            bad=bytearray(raw);bad[offset]=value
            with self.subTest(offset=offset):
                with self.assertRaises(Unavailable):decode_allow_ace(bytes(bad))
    def test_unsupported_equal_policies_refused(self):
        valid=specimen(DaclSemantics.BASE)
        for bad in ({**valid,'present':False},{**valid,'protected':False},{**valid,'aces':[[1,0,1,'S-1-5-18']]},
                    {**valid,'aces':[[0,64,1,'S-1-5-18']]},None):
            with self.assertRaises(Unavailable):verify_dacl('same','same',bad,bad)
    def test_duplicate_or_missing_ace_is_not_erased(self):
        base=DaclSemantics.BASE
        for other in (base+'(A;OICI;FA;;;SY)','D:P(A;OICI;FA;;;SY)'):
            with self.assertRaises(Unavailable):check(base,other)

if __name__=='__main__':unittest.main()
