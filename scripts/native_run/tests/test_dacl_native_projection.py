"""ctypes-memory stand-in, NOT evidence of Windows API execution."""
import ctypes as C
import pathlib,struct,sys,tempfile,types,unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from boundary import WindowsAPI,Unavailable,P,U32,ACL_SIZE_INFORMATION
import test_dacl_semantics as fixtures
specimen=fixtures.specimen

def acl_bytes(text):
    aces=[]
    for kind,flags,mask,sid in specimen(text)['aces']:
        parts=[int(x) for x in sid.split('-')[1:]]
        body=bytes([parts[0],len(parts)-2])+parts[1].to_bytes(6,'big')+struct.pack('<'+'I'*(len(parts)-2),*parts[2:])
        aces.append(struct.pack('<BBHI',kind,flags,8+len(body),mask)+body)
    return struct.pack('<BBHHH',2,0,8+sum(map(len,aces)),len(aces),0)+b''.join(aces)

def make_api(path,expected,observed):
    api=WindowsAPI.__new__(WindowsAPI)
    api.audit_path=path;api.audit_error=None;api.last_call=None;api.call_index=0
    texts={1:expected,2:observed};buffers={key:C.create_string_buffer(acl_bytes(text)) for key,text in texts.items()}
    api._fixture_buffers=buffers
    def out(ptr,ctype,value):C.cast(ptr,C.POINTER(ctype))[0]=value
    def control(desc,ctrl,rev):
        out(ctrl,C.c_uint16,0x1000 if specimen(texts[desc.value])['protected'] else 0);out(rev,U32,1);return 1
    def dacl(desc,present,acl,defaulted):
        out(present,C.c_int,1);out(acl,P,C.addressof(buffers[desc.value]));out(defaulted,C.c_int,0);return 1
    def information(acl,info,size,kind):
        header=C.string_at(acl,8);_,_,length,count,_=struct.unpack('<BBHHH',header)
        C.cast(info,C.POINTER(ACL_SIZE_INFORMATION))[0]=ACL_SIZE_INFORMATION(count,length,0);return 1
    def ace(acl,index,result):
        offset=8
        for i in range(index):offset+=C.c_uint16.from_address(acl.value+offset+2).value
        out(result,P,acl.value+offset);return 1
    def construct(text,revision,desc,length):out(desc,P,1);return 1
    def query(*args):out(args[-1],P,2);return 0
    api.a=types.SimpleNamespace(GetSecurityDescriptorControl=control,GetSecurityDescriptorDacl=dacl,
        IsValidAcl=lambda acl:1,GetAclInformation=information,GetAce=ace,
        ConvertStringSecurityDescriptorToSecurityDescriptorW=construct,
        SetNamedSecurityInfoW=lambda *args:0,GetNamedSecurityInfoW=query)
    api.k=types.SimpleNamespace(LocalFree=lambda ptr:None)
    api.sddl=lambda desc:texts[desc.value]
    return api

class NativeProjection(unittest.TestCase):
    def test_protect_uses_binary_ace_projection(self):
        expected=fixtures.DaclSemantics.BASE
        observed='D:PAI(A;OICI;0x1f01ff;;;S-1-5-21-1-2-3-1001)(A;CIOI;FA;;;S-1-5-18)'
        with tempfile.TemporaryDirectory() as d:
            api=make_api(pathlib.Path(d)/'calls',expected,observed)
            receipt=api.protect('owned',expected)
            self.assertTrue(receipt['verified']);self.assertEqual(receipt['expected_policy']['aces'],specimen(expected)['aces'])
            self.assertEqual(receipt['observed_policy']['aces'],specimen(observed)['aces'])
    def test_same_rendered_text_cannot_hide_binary_extra_grant(self):
        expected=fixtures.DaclSemantics.BASE;observed=expected+'(A;;FR;;;WD)'
        with tempfile.TemporaryDirectory() as d:
            api=make_api(pathlib.Path(d)/'calls',expected,observed)
            api.sddl=lambda desc:expected
            with self.assertRaisesRegex(Unavailable,'permission tuples differ'):api.protect('owned',expected)
    def test_native_descriptor_malformed_bounds_refused_with_text(self):
        expected=fixtures.DaclSemantics.BASE
        with tempfile.TemporaryDirectory() as d:
            api=make_api(pathlib.Path(d)/'calls',expected,expected)
            # Claim a first ACE longer than the whole allocated ACL.
            C.c_uint16.from_buffer(api._fixture_buffers[2],10).value=65535
            with self.assertRaises(Unavailable) as caught:api.protect('owned',expected)
            self.assertIn(expected,str(caught.exception));self.assertIn('ACE size outside DACL',str(caught.exception))

if __name__=='__main__':unittest.main()
