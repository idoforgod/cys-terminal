// Diagnostic-only helper, compiled into a private run directory, never shipped.
using System;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Web.Script.Serialization;
class NativeReceiverHelper {
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
                File.WriteAllText(Path.Combine(output,id+".json"),Json.Serialize(new {input=input,bytes=bytes.Length,pid=System.Diagnostics.Process.GetCurrentProcess().Id,utc=DateTime.UtcNow.ToString("o")}),new UTF8Encoding(false));
                return 0;
            }
            if(mode=="connect") {
                if(args.Length!=5)throw new Exception("connect root output literal-ip port");
                string output=Owned(root,args[2]); IPAddress address=IPAddress.Parse(args[3]);int port=int.Parse(args[4]);
                if(port<1||port>65535)throw new Exception("invalid port");
                bool connected=false;string error=null;
                using(Socket s=new Socket(address.AddressFamily,SocketType.Stream,ProtocolType.Tcp)) {
                    IAsyncResult call=s.BeginConnect(new IPEndPoint(address,port),null,null);
                    if(!call.AsyncWaitHandle.WaitOne(2000))error="bounded_connect_timeout";
                    else {try{s.EndConnect(call);connected=s.Connected;}catch(SocketException e){error=e.SocketErrorCode+":"+e.NativeErrorCode;}}
                }
                File.WriteAllText(output,Json.Serialize(new {connected=connected,error=error,address=address.ToString(),port=port,pid=System.Diagnostics.Process.GetCurrentProcess().Id,utc=DateTime.UtcNow.ToString("o")}),new UTF8Encoding(false));
                return 0; // outcome is recorded, not interpreted as isolation success
            }
            throw new Exception("unknown mode");
        } catch(Exception e) { Console.Error.WriteLine(e.ToString());return 2; }
    }
}
