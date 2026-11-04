use anyhow::{bail, ensure, Context, Result};
use clap::{Parser, Subcommand};
use iroh::{Endpoint, EndpointAddr, RelayMode, RelayUrl};
use iroh::endpoint::{presets, Connection, QuicTransportConfig, RecvStream, SendStream};
use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs::{File, OpenOptions}, io::Write, net::{IpAddr, Ipv4Addr, SocketAddr}, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
use tokio::{task::JoinSet, time::{sleep, timeout}};
use url::Url;

const ALPN: &[u8] = b"f57-s0-probe/1";
const CHUNK: usize = 64 * 1024;
const WINDOW: u64 = 4 * 1024 * 1024;
const ARM_CAPS: [Option<u64>; 4] = [None, Some(0), Some(256 * 1024), Some(4 * 1024 * 1024)];

#[derive(Parser)]
#[command(about="Throwaway F57 S0 transport measurement; no Jarvis RPCs or real stop actions")]
struct Cli { #[command(subcommand)] command: Cmd }
#[derive(Subcommand)]
enum Cmd {
    Local { #[arg(long)] output: PathBuf, #[arg(long, default_value_t=57)] seed: u64,
        #[arg(long, default_value_t=5)] trials: usize, #[arg(long)] hold_file: Option<PathBuf>,
        #[arg(long, default_value_t=0)] inject_ack_delay_ms: u64,
        #[arg(long)] skip_pause_store: bool },
    Relay { #[arg(long, default_value="127.0.0.1:3340")] bind: SocketAddr,
        #[arg(long)] allow_network: bool, #[arg(long, default_value_t=3600)] seconds: u64 },
    Listen { #[arg(long)] relay: String, #[arg(long, default_value="127.0.0.1:0")] bind: SocketAddr,
        #[arg(long)] relay_only: bool, #[arg(long)] allow_network: bool, #[arg(long)] proxy: Option<String>,
        #[arg(long)] address_out: PathBuf, #[arg(long)] result_out: PathBuf, #[arg(long, default_value_t=600)] seconds: u64 },
    Connect { #[arg(long)] address: PathBuf, #[arg(long)] result_out: PathBuf,
        #[arg(long, default_value="127.0.0.1:0")] bind: SocketAddr, #[arg(long)] relay_only: bool,
        #[arg(long)] allow_network: bool, #[arg(long)] proxy: Option<String>,
        #[arg(long, default_value_t=600)] seconds: u64, #[arg(long, default_value_t=0)] bulk_cap_bps: u64 },
}
fn epoch_ms() -> u128 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() }
fn emit(v: &Value) { println!("{v}"); let _ = std::io::stdout().flush(); }
fn new_file(p: &Path) -> Result<File> {
    if let Some(parent)=p.parent() { std::fs::create_dir_all(parent)?; }
    Ok(OpenOptions::new().write(true).create_new(true).open(p).with_context(||format!("refusing to overwrite {}",p.display()))?)
}
fn log_line(f: &mut File, v: &Value) -> Result<()> { writeln!(f,"{v}")?; f.flush()?; Ok(()) }
fn safe_bind(addr: SocketAddr, allow: bool) -> Result<()> { ensure!(allow || addr.ip().is_loopback(), "non-loopback bind requires --allow-network on an isolated machine"); Ok(()) }
fn checked_url(s: &str, allow: bool) -> Result<Url> {
    let u=Url::parse(s)?; ensure!(matches!(u.scheme(),"http"|"https"),"relay/proxy URL must be HTTP(S)");
    let host=u.host_str().context("URL has no host")?;
    ensure!(!["iroh.network","iroh.link","n0.computer"].iter().any(|suffix| host==*suffix || host.ends_with(&format!(".{suffix}"))),"public provider defaults are forbidden in this kit");
    ensure!(allow || host.parse::<IpAddr>().map(|ip|ip.is_loopback()).unwrap_or(false),"only numeric loopback URLs allowed without --allow-network");
    ensure!(u.username().is_empty() && u.password().is_none(),"credentials in URLs are not accepted"); Ok(u)
}
async fn endpoint(relay: RelayUrl, relay_only: bool, bind: SocketAddr, allow: bool, proxy: Option<&str>) -> Result<Endpoint> {
    safe_bind(bind,allow)?;
    let config=QuicTransportConfig::builder().send_window(WINDOW).build();
    let mut b=Endpoint::builder(presets::Minimal).clear_ip_transports().clear_address_lookup()
        .relay_mode(RelayMode::custom([relay])).alpns(vec![ALPN.to_vec()]).transport_config(config);
    if !relay_only { b=b.bind_addr(bind)?; }
    if let Some(p)=proxy { b=b.proxy_url(checked_url(p,allow)?); }
    let ep=b.bind().await?;
    timeout(Duration::from_secs(15),ep.online()).await.context("relay registration timeout")?;
    if relay_only { ensure!(ep.bound_sockets().is_empty(),"relay-only endpoint unexpectedly bound UDP"); }
    else { ensure!(allow || ep.bound_sockets().iter().all(|x|x.ip().is_loopback()),"unexpected external UDP bind"); }
    Ok(ep)
}
async fn relay_server(bind: SocketAddr, allow: bool) -> Result<Server> {
    safe_bind(bind,allow)?;
    let mut cfg=ServerConfig::default(); cfg.relay=Some(RelayConfig::new(bind)); cfg.quic=None; cfg.metrics_addr=None;
    Ok(Server::spawn(cfg).await?)
}
fn paths(c: &Connection) -> Value {
    json!(c.paths().iter().map(|p|json!({"relay":p.is_relay(),"ip":p.is_ip(),"selected":p.is_selected(),
        "remote":format!("{:?}",p.remote_addr()),"stats":format!("{:?}",p.stats())})).collect::<Vec<_>>())
}
fn relay_path(c: &Connection) -> bool {
    let p=c.paths(); !p.is_empty() && p.iter().all(|x|x.is_relay()&&!x.is_ip()) && p.iter().any(|x|x.is_selected())
}
async fn send_header(send: &mut SendStream, v: &Value) -> Result<()> {
    let data=serde_json::to_vec(v)?; ensure!(data.len()<=4096,"header too large");
    send.write_all(&(data.len() as u32).to_be_bytes()).await?; send.write_all(&data).await?; Ok(())
}
async fn read_header(recv: &mut RecvStream) -> Result<Value> {
    let mut len=[0;4]; recv.read_exact(&mut len).await?; let n=u32::from_be_bytes(len) as usize;
    ensure!(n<=4096,"header exceeds limit"); let mut data=vec![0;n]; recv.read_exact(&mut data).await?;
    Ok(serde_json::from_slice(&data)?)
}
#[derive(Default)]
struct Counts {
    received: AtomicU64,
    paused: AtomicBool,
    stops: AtomicU64,
    refused_after_stop: AtomicU64,
    ack_delay_ms: AtomicU64,
    skip_pause_store: AtomicBool,
}
async fn handle_bi(mut send: SendStream, mut recv: RecvStream, counts: Arc<Counts>) -> Result<()> {
    let req=read_header(&mut recv).await?;
    let answer=match req["kind"].as_str().unwrap_or("") {
        "ping" => json!({"kind":"pong","nonce":req["nonce"],"receiver_epoch_ms":epoch_ms(),"bulk_received":counts.received.load(Ordering::Relaxed)}),
        "stop" => {
            let delay=counts.ack_delay_ms.load(Ordering::Relaxed);
            if delay>0 { sleep(Duration::from_millis(delay)).await; }
            if !counts.skip_pause_store.load(Ordering::Relaxed) {
                counts.paused.store(true,Ordering::SeqCst);
            }
            let paused_now=counts.paused.load(Ordering::SeqCst);
            let seq=counts.stops.fetch_add(1,Ordering::SeqCst)+1;
            json!({"kind":"stop_ack","nonce":req["nonce"],"synthetic_admission_paused":paused_now,"stop_seq":seq,
                "receiver_transition_epoch_ms":epoch_ms(),"bulk_received_at_stop":counts.received.load(Ordering::SeqCst)})
        },
        "transfer" => {
            if counts.paused.load(Ordering::SeqCst) {
                let rseq=counts.refused_after_stop.fetch_add(1,Ordering::SeqCst)+1;
                json!({"kind":"transfer_refused","reason":"admission_paused","refused_seq":rseq,
                    "stop_seq":counts.stops.load(Ordering::SeqCst),"receiver_epoch_ms":epoch_ms()})
            } else {
                let expected=req["bytes"].as_u64().context("transfer byte count missing")?;
                ensure!(expected<=32*1024*1024,"transfer exceeds probe limit");
                let mut h=Sha256::new(); let mut n=0u64; let mut buf=vec![0;CHUNK];
                while let Some(got)=recv.read(&mut buf).await? { n+=got as u64; ensure!(n<=expected,"excess transfer bytes"); h.update(&buf[..got]); }
                ensure!(n==expected,"short transfer"); json!({"kind":"transfer_ack","bytes":n,"sha256":format!("{:x}",h.finalize())})
            }
        },
        _ => bail!("unknown synthetic request"),
    };
    let _=send.set_priority(100); send.write_all(&serde_json::to_vec(&answer)?).await?; send.finish()?; Ok(())
}
async fn handle_bulk(mut recv: RecvStream, counts: Arc<Counts>) -> Result<()> {
    ensure!(!counts.paused.load(Ordering::SeqCst),"new bulk stream refused after stop");
    let mut tag=[0u8;1]; recv.read_exact(&mut tag).await?; ensure!(tag[0]==b'B',"unknown bulk stream");
    let mut data=vec![0;CHUNK]; while let Some(n)=recv.read(&mut data).await? { counts.received.fetch_add(n as u64,Ordering::Relaxed); } Ok(())
}
async fn serve(c: Connection, counts: Arc<Counts>) {
    let mut jobs=JoinSet::new();
    loop { tokio::select! {
        incoming=c.accept_bi()=>match incoming {Ok((s,r))=>{jobs.spawn(handle_bi(s,r,counts.clone()));},Err(_)=>break},
        incoming=c.accept_uni()=>match incoming {Ok(r)=>{jobs.spawn(handle_bulk(r,counts.clone()));},Err(_)=>break},
        completed=jobs.join_next(), if !jobs.is_empty()=>{if let Some(Ok(Err(e)))=completed {eprintln!("probe stream error: {e:#}");}},
        _=c.closed()=>break,
    }}
    jobs.abort_all(); while jobs.join_next().await.is_some() {}
}
async fn request(c: &Connection, req: Value) -> Result<Value> {
    let (mut s,mut r)=c.open_bi().await?; let _=s.set_priority(100); send_header(&mut s,&req).await?; s.finish()?;
    let bytes=r.read_to_end(16384).await?; Ok(serde_json::from_slice(&bytes)?)
}
async fn transfer(c: &Connection) -> Result<Value> {
    let amount=1024*1024usize; let (mut s,mut r)=c.open_bi().await?;
    send_header(&mut s,&json!({"kind":"transfer","bytes":amount})).await?;
    let data=vec![0x5a;CHUNK]; let mut h=Sha256::new();
    for _ in 0..amount/CHUNK { s.write_all(&data).await?; h.update(&data); } s.finish()?;
    let ack:Value=serde_json::from_slice(&r.read_to_end(16384).await?)?;
    ensure!(ack["bytes"]==json!(amount)&&ack["sha256"]==json!(format!("{:x}",h.finalize())),"transfer digest/size mismatch"); Ok(ack)
}
#[derive(Default)]
struct BulkStats { sent: AtomicU64, calls: AtomicU64, waited_ns: AtomicU64, waits_over_1ms: AtomicU64 }
async fn bulk_send(c: Connection, cap: u64, done: Arc<AtomicBool>, stats: Arc<BulkStats>) -> Result<()> {
    let mut s=c.open_uni().await?; let _=s.set_priority(-10); s.write_all(b"B").await?;
    let data=vec![0x5a;CHUNK]; let start=tokio::time::Instant::now();
    while !done.load(Ordering::SeqCst) {
        let t=Instant::now(); s.write_all(&data).await?; let elapsed=t.elapsed();
        stats.waited_ns.fetch_add(elapsed.as_nanos().min(u64::MAX as u128) as u64,Ordering::Relaxed);
        stats.waits_over_1ms.fetch_add((elapsed>=Duration::from_millis(1)) as u64,Ordering::Relaxed);
        stats.calls.fetch_add(1,Ordering::Relaxed);
        let bytes=stats.sent.fetch_add(CHUNK as u64,Ordering::Relaxed)+CHUNK as u64;
        if cap>0 { tokio::time::sleep_until(start+Duration::from_secs_f64(bytes as f64/cap as f64)).await; }
        else { tokio::task::yield_now().await; }
    }
    s.finish()?; Ok(())
}
async fn local_trial(relay: &Server, url: &RelayUrl, arm: usize, trial: usize, warmup: bool, inject_ack_delay_ms: u64, skip_pause_store: bool) -> Result<Value> {
    let a=endpoint(url.clone(),true,(Ipv4Addr::LOCALHOST,0).into(),false,None).await?;
    let b=endpoint(url.clone(),true,(Ipv4Addr::LOCALHOST,0).into(),false,None).await?;
    let server_ep=b.clone(); let counts=Arc::new(Counts::default());
    counts.ack_delay_ms.store(inject_ack_delay_ms,Ordering::Relaxed);
    counts.skip_pause_store.store(skip_pause_store,Ordering::Relaxed);
    let cc=counts.clone();
    let server_task=tokio::spawn(async move { let incoming=server_ep.accept().await.context("accept ended")?;
        let conn=incoming.await?; serve(conn,cc).await; Ok::<(),anyhow::Error>(()) });
    let c=timeout(Duration::from_secs(15),a.connect(EndpointAddr::new(b.id()).with_relay_url(url.clone()),ALPN)).await??;
    let receiver=c.remote_id().to_string(); ensure!(receiver==b.id().to_string(),"peer identity mismatch");
    let transfer_result=timeout(Duration::from_secs(10),transfer(&c)).await??;
    let before_path=paths(&c); let route_before=relay_path(&c);
    let relay_before=json!({"sent":relay.metrics().server.bytes_sent.get(),"received":relay.metrics().server.bytes_recv.get()});
    let done=Arc::new(AtomicBool::new(false)); let stats=Arc::new(BulkStats::default());
    let bulk=ARM_CAPS[arm].map(|cap|tokio::spawn(bulk_send(c.clone(),cap,done.clone(),stats.clone())));
    sleep(Duration::from_millis(1750)).await;
    let recent_start=counts.received.load(Ordering::SeqCst);
    sleep(Duration::from_millis(250)).await;
    let received_before=counts.received.load(Ordering::SeqCst);
    let nonce=format!("{arm}-{trial}-{warmup}-{}",epoch_ms());
    // Start before stream opening, write scheduling, limiter waits or protocol work.
    let requested_epoch_ms=epoch_ms(); let started=Instant::now();
    let ack_result=timeout(Duration::from_secs(10),request(&c,json!({"kind":"stop","nonce":nonce}))).await;
    let latency_ms=started.elapsed().as_secs_f64()*1000.0;
    let (ack,error)=match ack_result {Ok(Ok(v))=>(Some(v),None),Ok(Err(e))=>(None,Some(format!("{e:#}"))),Err(e)=>(None,Some(e.to_string()))};
    let valid_ack=ack.as_ref().map(|v|v["kind"]=="stop_ack"&&v["nonce"]==nonce&&v["synthetic_admission_paused"]==true&&v["stop_seq"]==1).unwrap_or(false);
    // Witness post-stop admission refusal while existing bulk stream remains active.
    let post_stop_probe=timeout(Duration::from_secs(3),request(&c,json!({"kind":"transfer","bytes":0,"post_stop_witness":true}))).await;
    let (post_stop_resp,post_stop_error)=match post_stop_probe {Ok(Ok(v))=>(Some(v),None),Ok(Err(e))=>(None,Some(format!("{e:#}"))),Err(e)=>(None,Some(e.to_string()))};
    let post_stop_refused=post_stop_resp.as_ref().map(|v|v["kind"]=="transfer_refused"&&v["reason"]=="admission_paused"&&v["refused_seq"]==1).unwrap_or(false)
        && counts.refused_after_stop.load(Ordering::SeqCst)==1;
    let route_after=relay_path(&c); let after_path=paths(&c);
    let relay_after=json!({"sent":relay.metrics().server.bytes_sent.get(),"received":relay.metrics().server.bytes_recv.get()});
    let sent_at_ack=stats.sent.load(Ordering::SeqCst);
    let bytes_at_ack=counts.received.load(Ordering::SeqCst);
    done.store(true,Ordering::SeqCst);
    let mut cleanup_ok=true;
    if let Some(mut task)=bulk { match timeout(Duration::from_secs(3),&mut task).await {Ok(Ok(Ok(())))=>(),Ok(_)=>cleanup_ok=false,Err(_)=>{task.abort();let _=task.await;cleanup_ok=false;}} }
    c.close(0u32.into(),b"trial complete");
    if timeout(Duration::from_secs(3),a.close()).await.is_err(){cleanup_ok=false;}
    if timeout(Duration::from_secs(3),b.close()).await.is_err(){cleanup_ok=false;}
    let mut task=server_task;
    match timeout(Duration::from_secs(3),&mut task).await {Ok(Ok(Ok(())))=>(),Ok(_)=>cleanup_ok=false,Err(_)=>{task.abort();let _=task.await;cleanup_ok=false;}}
    let overlap=arm==0 || (received_before>recent_start && stats.calls.load(Ordering::Relaxed)>0);
    let relay_moved=relay_after["sent"].as_u64().unwrap_or(0)>relay_before["sent"].as_u64().unwrap_or(0);
    let setup_valid=route_before&&route_after&&overlap&&relay_moved;
    let capped_arm=ARM_CAPS[arm].unwrap_or(0)>0;
    let latency_under_5s=latency_ms<5000.0;
    let latency_bar_ok=!capped_arm || latency_under_5s;
    let trial_pass=valid_ack&&post_stop_refused&&latency_bar_ok;
    // Preserve measured FAIL even if cleanup_ok is false; never mask a failure as UNMEASURED.
    let verdict=if !setup_valid {"UNMEASURED"} else if !trial_pass {"FAIL"} else if !cleanup_ok {"UNMEASURED"} else {"PASS"};
    let valid=setup_valid&&cleanup_ok;
    Ok(json!({"schema":2,"measurement":"loopback-relay-stop-transport-proxy","arm":arm,"trial":trial,"warmup":warmup,
        "bulk_cap_bps":ARM_CAPS[arm],"capped_arm":capped_arm,"verdict":verdict,"setup_valid":setup_valid,"valid":valid,
        "ack_matches":valid_ack,"post_stop_admission_refused":post_stop_refused,"post_stop_witness":post_stop_resp,
        "post_stop_error":post_stop_error,"stop_latency_ms":latency_ms,"under_5s":latency_under_5s,
        "request_epoch_ms":requested_epoch_ms,"ack":ack,"error":error,"bulk_sent_at_ack":sent_at_ack,"bulk_received_before_request":received_before,
        "bulk_received_at_ack":bytes_at_ack,"recent_bulk_bytes":received_before-recent_start,"write_calls":stats.calls.load(Ordering::Relaxed),
        "write_wait_ns":stats.waited_ns.load(Ordering::Relaxed),"writes_over_1ms":stats.waits_over_1ms.load(Ordering::Relaxed),
        "relay_metrics_before":relay_before,"relay_metrics_after":relay_after,"paths_before":before_path,"paths_after":after_path,
        "endpoint_ip_sockets":a.bound_sockets(),"relay_url":url.to_string(),"transfer":transfer_result,"cleanup_ok":cleanup_ok,
        "injected_ack_delay_ms":inject_ack_delay_ms,"skipped_pause_store":skip_pause_store,
        "scope_limit":"synthetic receiver pause only; not a Jarvis all-lanes stop or WAN saturation proof"}))
}
async fn local(output: PathBuf, seed: u64, trials: usize, hold_file: Option<PathBuf>, inject_ack_delay_ms: u64, skip_pause_store: bool) -> Result<()> {
    ensure!((1..=20).contains(&trials),"trials outside safe bound"); let mut out=new_file(&output)?;
    let relay=relay_server((Ipv4Addr::LOCALHOST,0).into(),false).await?;
    let url:RelayUrl=format!("http://{}",relay.http_addr().context("relay has no HTTP listener")?).parse()?;
    let mut rng=seed; let mut order=vec![0,1,2,3];
    for i in (1..order.len()).rev(){rng=rng.wrapping_mul(6364136223846793005).wrapping_add(1);order.swap(i,(rng as usize)%(i+1));}
    log_line(&mut out,&json!({"type":"run_start","epoch_ms":epoch_ms(),"seed":seed,"order":order,"trials_per_arm":trials,
        "iroh":"1.4.1","relay":"1.4.1","relay_listener":relay.http_addr(),"relay_quic_listener":relay.quic_addr(),
        "public_discovery":false,"ip_transports":false,"chunk_bytes":CHUNK,"send_window_bytes":WINDOW,
        "inject_ack_delay_ms":inject_ack_delay_ms,"skip_pause_store":skip_pause_store}))?;
    let mut counts=[0usize;4]; let mut invalid=[0usize;4]; let mut fail=false; let mut hold=false;
    'rounds: for round in 0..=trials {
        for pos in 0..order.len() {
            let arm=order[(pos+round)%order.len()];
            if hold_file.as_ref().map(|p|p.exists()).unwrap_or(false) {hold=true;break;}
            let result=local_trial(&relay,&url,arm,round,round==0,inject_ack_delay_ms,skip_pause_store).await;
            let row=match result {Ok(v)=>v,Err(e)=>json!({"arm":arm,"trial":round,"warmup":round==0,"verdict":"UNMEASURED","error":format!("{e:#}")})};
            if round>0 {if row["valid"]==true {counts[arm]+=1;} else {invalid[arm]+=1;} if row["verdict"]=="FAIL"{fail=true;}}
            emit(&row);log_line(&mut out,&row)?;
            if row["cleanup_ok"]!=true {break 'rounds;}
        }
        if hold {break;}
    }
    // Missing samples remain missing. Failure takes precedence over incomplete; no latency failure is replaced or discarded.
    let complete=counts.iter().all(|n|*n==trials);
    let summary=json!({"type":"summary","epoch_ms":epoch_ms(),"complete":complete,"valid_samples":counts,"invalid_samples":invalid,
        "held_for_priority":hold,"verdict":if fail {"FAIL"}else if !complete {"UNMEASURED"}else{"PASS"}});
    log_line(&mut out,&summary)?;emit(&summary);
    timeout(Duration::from_secs(5),relay.shutdown()).await??;
    ensure!(complete&&!fail,"measurement incomplete or failed; raw evidence retained"); Ok(())
}
async fn listen(relay_s: String, bind: SocketAddr, relay_only: bool, allow: bool, proxy: Option<String>, address_out: PathBuf, result_out: PathBuf, seconds: u64) -> Result<()> {
    let relay:RelayUrl=checked_url(&relay_s,allow)?.as_str().parse()?;
    let ep=endpoint(relay,relay_only,bind,allow,proxy.as_deref()).await?;
    let addr=ep.addr(); let mut af=new_file(&address_out)?; serde_json::to_writer_pretty(&mut af,&addr)?; af.flush()?;
    let mut out=new_file(&result_out)?;let start=epoch_ms(); let end=start+seconds as u128*1000;
    log_line(&mut out,&json!({"type":"listen_ready","epoch_ms":start,"address":addr,"bound_sockets":ep.bound_sockets(),"relay_only":relay_only}))?;
    emit(&json!({"type":"listen_ready","address_file":address_out,"id":ep.id().to_string()}));
    let mut jobs=JoinSet::new();
    while epoch_ms()<end {tokio::select! {
        incoming=ep.accept()=>{if let Some(incoming)=incoming {match incoming.await {Ok(c)=>{log_line(&mut out,&json!({"type":"accepted","epoch_ms":epoch_ms(),"peer":c.remote_id().to_string(),"paths":paths(&c)}))?;jobs.spawn(serve(c,Arc::new(Counts::default())));},Err(e)=>log_line(&mut out,&json!({"type":"accept_error","error":e.to_string()}))?}}},
        _=sleep(Duration::from_secs(1))=>(),
        _=tokio::signal::ctrl_c()=>break,
    }}
    ep.close().await; jobs.abort_all(); while jobs.join_next().await.is_some(){}; Ok(())
}
async fn connect(address: PathBuf, result_out: PathBuf, bind: SocketAddr, relay_only: bool, allow: bool, proxy: Option<String>, seconds: u64, bulk_cap_bps: u64) -> Result<()> {
    let addr:EndpointAddr=serde_json::from_slice(&std::fs::read(address)?)?;
    for r in addr.relay_urls(){checked_url(r.as_str(),allow)?;}
    for ip in addr.ip_addrs(){safe_bind(*ip,allow)?;}
    let relay=addr.relay_urls().next().context("address must name an operator-owned relay")?.clone();
    let ep=endpoint(relay,relay_only,bind,allow,proxy.as_deref()).await?;
    let mut out=new_file(&result_out)?; let end=epoch_ms()+seconds as u128*1000;
    let mut conn:Option<Connection>=None; let mut outage:Option<u128>=None;let mut last_success:Option<u128>=None;let mut sequence=0u64;
    let mut bulk:Option<tokio::task::JoinHandle<Result<()>>>=None; let mut bulk_done=Arc::new(AtomicBool::new(false));
    log_line(&mut out,&json!({"type":"connect_start","epoch_ms":epoch_ms(),"id":ep.id().to_string(),"relay_only":relay_only,"bound_sockets":ep.bound_sockets()}))?;
    while epoch_ms()<end {
        sequence+=1; let began=Instant::now();let nonce=format!("ping-{sequence}");
        let attempt:Result<Value>=async {
            if conn.is_none(){let c=timeout(Duration::from_secs(10),ep.connect(addr.clone(),ALPN)).await??;
                bulk_done=Arc::new(AtomicBool::new(false));bulk=Some(tokio::spawn(bulk_send(c.clone(),bulk_cap_bps,bulk_done.clone(),Arc::new(BulkStats::default()))));conn=Some(c);}
            let c=conn.as_ref().unwrap(); let ack=timeout(Duration::from_secs(5),request(c,json!({"kind":"ping","nonce":nonce}))).await??;
            ensure!(ack["kind"]=="pong"&&ack["nonce"]==nonce,"ping ACK mismatch");Ok(ack)
        }.await;
        let now=epoch_ms();
        match attempt {
            Ok(ack)=>{let row=json!({"type":"heartbeat","epoch_ms":now,"sequence":sequence,"roundtrip_ms":began.elapsed().as_secs_f64()*1000.0,
                "observed_gap_ms":last_success.map(|x|now.saturating_sub(x)),"outage_from_first_failure_ms":outage.map(|x|now.saturating_sub(x)),
                "paths":paths(conn.as_ref().unwrap()),"ack":ack});log_line(&mut out,&row)?;emit(&row);last_success=Some(now);outage=None;},
            Err(e)=>{outage.get_or_insert(now);log_line(&mut out,&json!({"type":"unreachable","epoch_ms":now,"error":format!("{e:#}")}))?;
                if let Some(c)=conn.take(){c.close(1u32.into(),b"retry");}bulk_done.store(true,Ordering::SeqCst);if let Some(t)=bulk.take(){t.abort();let _=t.await;}},
        }
        tokio::select!{_=sleep(Duration::from_secs(1))=>(),_=tokio::signal::ctrl_c()=>break}
    }
    bulk_done.store(true,Ordering::SeqCst);if let Some(t)=bulk.take(){t.abort();let _=t.await;}
    if let Some(c)=conn{c.close(0u32.into(),b"watch complete");}ep.close().await;Ok(())
}
#[tokio::main(flavor="multi_thread",worker_threads=2)]
async fn main() {
    let result=match Cli::parse().command {
        Cmd::Local{output,seed,trials,hold_file,inject_ack_delay_ms,skip_pause_store}=>local(output,seed,trials,hold_file,inject_ack_delay_ms,skip_pause_store).await,
        Cmd::Relay{bind,allow_network,seconds}=>async {let s=relay_server(bind,allow_network).await?;
            emit(&json!({"type":"relay_ready","epoch_ms":epoch_ms(),"http_listener":s.http_addr(),"quic_listener":s.quic_addr(),"tls":false}));
            tokio::select!{_=sleep(Duration::from_secs(seconds))=>(),_=tokio::signal::ctrl_c()=>()};s.shutdown().await?;Ok(())}.await,
        Cmd::Listen{relay,bind,relay_only,allow_network,proxy,address_out,result_out,seconds}=>listen(relay,bind,relay_only,allow_network,proxy,address_out,result_out,seconds).await,
        Cmd::Connect{address,result_out,bind,relay_only,allow_network,proxy,seconds,bulk_cap_bps}=>connect(address,result_out,bind,relay_only,allow_network,proxy,seconds,bulk_cap_bps).await,
    };
    if let Err(e)=result {emit(&json!({"type":"probe_error","epoch_ms":epoch_ms(),"error":format!("{e:#}")}));std::process::exit(1);}
}
