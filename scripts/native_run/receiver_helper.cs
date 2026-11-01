// Diagnostic-only helper, compiled into a private run directory, never shipped.
using System;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Runtime.InteropServices;
using System.Diagnostics;
using System.Threading;
using System.Collections.Generic;
using System.Security.Cryptography;
using System.Web.Script.Serialization;
class NativeReceiverHelper {
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr GetStdHandle(int kind);
    [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetConsoleMode(IntPtr handle,out uint mode);
    [StructLayout(LayoutKind.Explicit,Size=20)] struct InputRecord {
        [FieldOffset(0)] public ushort type;
        [FieldOffset(4)] public int keyDown;
        [FieldOffset(8)] public ushort repeat;
        [FieldOffset(10)] public ushort virtualKey;
        [FieldOffset(12)] public ushort scan;
        [FieldOffset(14)] public ushort character;
        [FieldOffset(16)] public uint control;
    }
    [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetNumberOfConsoleInputEvents(IntPtr handle,out uint count);
    [DllImport("kernel32.dll",EntryPoint="ReadConsoleInputW",SetLastError=true)] static extern bool ReadConsoleInput(IntPtr handle,[Out] InputRecord[] records,uint length,out uint read);
    static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
    static string Owned(string root, string input) {
        root=Path.GetFullPath(root).TrimEnd(Path.DirectorySeparatorChar)+Path.DirectorySeparatorChar;
        string full=Path.GetFullPath(input);
        if(!full.StartsWith(root,StringComparison.OrdinalIgnoreCase))throw new Exception("path outside owned child root");
        for(string p=full; p!=null && p.Length>=root.Length-1; p=Path.GetDirectoryName(p))
            if((File.Exists(p)||Directory.Exists(p)) && (File.GetAttributes(p)&FileAttributes.ReparsePoint)!=0)
                throw new Exception("reparse path refused");
        return full;
    }
    static void Publish(string path,string text,Encoding encoding) {
        string temp=path+"."+Guid.NewGuid().ToString("N")+".tmp";
        File.WriteAllText(temp,text,encoding);
        File.Move(temp,path); // publication is the completion receipt, never existence mid-write
    }
    static string Hash(byte[] bytes) {
        using(SHA256 digest=SHA256.Create())return BitConverter.ToString(digest.ComputeHash(bytes)).Replace("-","").ToLowerInvariant();
    }
    static int Main(string[] args) {
        try {
            if(args.Length<2)throw new Exception("mode/root required");
            string mode=args[0],root=Path.GetFullPath(args[1]);
            if(mode=="editor") {
                if(args.Length!=4)throw new Exception("editor root output-dir input");
                string input=Owned(root,args[3]),output=Owned(root,args[2]);
                Directory.CreateDirectory(output);
                byte[] bytes=File.ReadAllBytes(input);
                string id=Guid.NewGuid().ToString("N");
                // Observation only: never rewrite the application's editor file.
                File.WriteAllBytes(Path.Combine(output,id+".bin"),bytes);
                Publish(Path.Combine(output,id+".json"),Json.Serialize(new {input=input,bytes=bytes.Length,sha256=Hash(bytes),pid=System.Diagnostics.Process.GetCurrentProcess().Id,utc=DateTime.UtcNow.ToString("o")}),new UTF8Encoding(false));
                Stopwatch editorGuard=Stopwatch.StartNew();
                while(!File.Exists(Path.Combine(output,id+".ack"))) {
                    if(editorGuard.ElapsedMilliseconds>=15000)throw new Exception("editor completion acknowledgement absent");
                    Thread.Sleep(10);
                }
                return 0;
            }
            if(mode=="origin") {
                if(args.Length!=5)throw new Exception("origin root sideband emit|silent go-file");
                string sideband=Owned(root,args[2]),go=Owned(root,args[4]);
                bool emit=args[3]=="emit";
                if(!emit && args[3]!="silent")throw new Exception("origin mode");
                uint inputMode,outputMode;
                bool gotInput=GetConsoleMode(GetStdHandle(-10),out inputMode),gotOutput=GetConsoleMode(GetStdHandle(-11),out outputMode);
                Publish(sideband+".ready.json",Json.Serialize(new {pid=Process.GetCurrentProcess().Id,mode=args[3],input_mode=gotInput?(uint?)inputMode:null,output_mode=gotOutput?(uint?)outputMode:null}),new UTF8Encoding(false));
                Stopwatch guard=Stopwatch.StartNew();
                while(!File.Exists(go)) { if(guard.ElapsedMilliseconds>=20000)throw new Exception("origin handshake absent");Thread.Sleep(10); }
                long before=Stopwatch.GetTimestamp();int wrote=0;
                if(emit) { byte[] query=new byte[]{27,91,54,110};Stream output=Console.OpenStandardOutput();output.Write(query,0,query.Length);output.Flush();wrote=query.Length; }
                long after=Stopwatch.GetTimestamp();
                Publish(sideband+".emission.json",Json.Serialize(new {pid=Process.GetCurrentProcess().Id,mode=args[3],emitted_bytes=wrote,before_ticks=before,after_ticks=after,tick_frequency=Stopwatch.Frequency,utc=DateTime.UtcNow.ToString("o")}),new UTF8Encoding(false));
                // Only this no-Claude child reads its own console input. No mode
                // changes and no replies are ever fabricated or echoed to output.
                List<object> inputs=new List<object>();string inputError=null;
                while(!File.Exists(go+".stop")) {
                    if(guard.ElapsedMilliseconds>=40000)break;
                    uint count;
                    if(!GetNumberOfConsoleInputEvents(GetStdHandle(-10),out count)) {inputError="count:"+Marshal.GetLastWin32Error();break;}
                    if(count>0) {
                        InputRecord[] buffer=new InputRecord[Math.Min(count,32)];uint read;
                        if(!ReadConsoleInput(GetStdHandle(-10),buffer,(uint)buffer.Length,out read)) {inputError="read:"+Marshal.GetLastWin32Error();break;}
                        for(int i=0;i<read;i++)inputs.Add(new {ticks=Stopwatch.GetTimestamp(),event_type=buffer[i].type,key_down=buffer[i].keyDown,repeat=buffer[i].repeat,virtual_key=buffer[i].virtualKey,scan=buffer[i].scan,character_u16=buffer[i].character,control=buffer[i].control});
                        if(inputs.Count>4096) {inputError="input_inventory_bound";break;}
                    }
                    Thread.Sleep(10);
                }
                Publish(sideband+".input.json",Json.Serialize(new {pid=Process.GetCurrentProcess().Id,events=inputs,error=inputError,tick_frequency=Stopwatch.Frequency}),new UTF8Encoding(false));
                return 0;
            }
            if(mode=="connect") {
                if(args.Length!=5)throw new Exception("connect root output literal-ip port");
                string output=Owned(root,args[2]); IPAddress address=IPAddress.Parse(args[3]);int port=int.Parse(args[4]);
                if(port<1||port>65535)throw new Exception("invalid port");
                bool connected=false;string error=null;
                using(Socket s=new Socket(address.AddressFamily,SocketType.Stream,ProtocolType.Tcp)) {
                    try {
                        IAsyncResult call=s.BeginConnect(new IPEndPoint(address,port),null,null);
                        using(WaitHandle wait=call.AsyncWaitHandle) {
                            if(!wait.WaitOne(2000))error="bounded_connect_timeout";
                            else {s.EndConnect(call);connected=s.Connected;}
                        }
                    } catch(SocketException e) {error=e.SocketErrorCode+":"+e.NativeErrorCode;}
                }
                Publish(output,Json.Serialize(new {connected=connected,error=error,address=address.ToString(),port=port,pid=System.Diagnostics.Process.GetCurrentProcess().Id,utc=DateTime.UtcNow.ToString("o")}),new UTF8Encoding(false));
                return 0; // outcome is recorded, not interpreted as isolation success
            }
            throw new Exception("unknown mode");
        } catch(Exception e) { Console.Error.WriteLine(e.ToString());return 2; }
    }
}
