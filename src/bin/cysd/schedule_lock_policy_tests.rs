use super::*;
use std::collections::VecDeque;
use std::io::ErrorKind;
use std::sync::atomic::{AtomicU64, Ordering};

fn fixture(tag:&str)->PathBuf {
    static SEQ:AtomicU64=AtomicU64::new(0);
    let dir=std::env::temp_dir().join(format!("sched-policy-{tag}-{}-{}-{}",std::process::id(),now_epoch().to_bits(),SEQ.fetch_add(1,Ordering::Relaxed)));
    std::fs::create_dir(&dir).unwrap();dir.join("schedule.json")
}
struct Ops { steps:VecDeque<Option<ErrorKind>>, fallback:ErrorKind, time:Duration, oversleep:Option<Duration>, stale:bool, trace:Vec<String>, creates:usize, grants:usize }
impl Ops {
    fn new(steps:&[Option<ErrorKind>])->Self {Self{steps:steps.iter().copied().collect(),fallback:ErrorKind::PermissionDenied,time:Duration::ZERO,oversleep:None,stale:false,trace:vec![],creates:0,grants:0}}
}
impl ScheduleLockOps for Ops {
    type Clock=Duration;
    fn parent(&mut self,p:&std::path::Path){self.trace.push("parent".into());let _=std::fs::create_dir_all(p);}
    fn create(&mut self,p:&std::path::Path)->std::io::Result<()> {
        self.creates+=1;let x=self.steps.pop_front().unwrap_or(Some(self.fallback));self.trace.push(format!("create:{x:?}"));
        match x {Some(k)=>Err(std::io::Error::new(k,"call-owned injected mkdir result")),None=>{std::fs::create_dir(p)?;self.grants+=1;Ok(())}}
    }
    fn is_stale(&mut self,_:&std::path::Path,_:Duration)->bool {self.trace.push(format!("stale:{}",self.stale));self.stale}
    fn reclaim(&mut self,p:&std::path::Path){self.trace.push("reclaim".into());let _=std::fs::remove_dir_all(p);}
    fn now(&mut self)->Duration{self.trace.push(format!("clock:{}",self.time.as_millis()));self.time}
    fn sleep(&mut self,d:Duration){self.trace.push(format!("sleep:{}",d.as_millis()));self.time+=self.oversleep.unwrap_or(d);}
}
fn acquire(path:&std::path::Path,ops:&mut Ops,windows:bool,budget:u64)->Option<ScheduleFileLock>{ScheduleFileLock::acquire_with_ops(path,Duration::from_millis(budget),Duration::from_secs(60),windows,ops)}

#[test]
fn windows_transient_denial_then_real_ownership(){
    let p=fixture("transient");let mut o=Ops::new(&[Some(ErrorKind::PermissionDenied),Some(ErrorKind::PermissionDenied),None]);
    let lock=acquire(&p,&mut o,true,25).expect("TRANSIENT_DENIAL_DROPPED_WRITER");
    assert_eq!((o.creates,o.grants,o.time),(3,1,Duration::from_millis(20)));assert!(ScheduleFileLock::lock_dir_for(&p).is_dir());assert!(!o.trace.iter().any(|x|x=="reclaim"));drop(lock);
}
#[test]
fn contention_then_denial_retries_without_reclaim(){
    let p=fixture("contended");let mut o=Ops::new(&[Some(ErrorKind::AlreadyExists),Some(ErrorKind::PermissionDenied),None]);
    let _lock=acquire(&p,&mut o,true,30).expect("CONTENTION_TO_DENIAL_DROPPED_WRITER");assert_eq!(o.creates,3);assert!(!o.trace.iter().any(|x|x=="reclaim"));
}
#[test]
fn permanent_denial_consumes_one_deadline_without_grant(){
    let p=fixture("permanent");let mut o=Ops::new(&[]);assert!(acquire(&p,&mut o,true,25).is_none());
    assert_eq!((o.creates,o.grants,o.time),(3,0,Duration::from_millis(30)),"DENIAL_MUST_WAIT_BUT_NEVER_GRANT");assert!(!ScheduleFileLock::lock_dir_for(&p).exists());
}
#[test]
fn denied_wake_observes_expiry_before_another_attempt(){
    let p=fixture("wake");let mut o=Ops::new(&[Some(ErrorKind::PermissionDenied),None]);o.oversleep=Some(Duration::from_millis(40));
    assert!(acquire(&p,&mut o,true,25).is_none(),"DENIED_RETRY_AFTER_OBSERVED_EXPIRED_WAKE");assert_eq!((o.creates,o.time),(1,Duration::from_millis(40)),"denied path must sleep then fence");
}
#[test]
fn mixed_history_retains_legacy_already_exists_late_success(){
    let p=fixture("mixed");let mut o=Ops::new(&[Some(ErrorKind::PermissionDenied),Some(ErrorKind::AlreadyExists),Some(ErrorKind::AlreadyExists),None]);
    let _lock=acquire(&p,&mut o,true,25).expect("MIXED_HISTORY_LAST_ERROR_IS_NOT_DENIAL");assert_eq!(o.time,Duration::from_millis(30));
}
#[test]
fn other_errors_and_non_windows_keep_old_traces(){
    for win in [false,true] {
        for k in [ErrorKind::Interrupted,ErrorKind::NotFound,ErrorKind::InvalidInput,ErrorKind::Other] {
            let p=fixture("other");let mut o=Ops::new(&[Some(k)]);assert!(acquire(&p,&mut o,win,25).is_none());
            assert_eq!(o.trace,vec!["parent".to_string(),"clock:0".into(),format!("create:Some({k:?})")]);println!("UNCHANGED_TRACE {win} {k:?} {:?}",o.trace);
        }
    }
    let p=fixture("nonwin-pd");let mut o=Ops::new(&[Some(ErrorKind::PermissionDenied),None]);assert!(acquire(&p,&mut o,false,25).is_none());assert_eq!(o.creates,1);assert_eq!(o.time,Duration::ZERO);println!("UNCHANGED_TRACE nonwin-PD {:?}",o.trace);
}
#[test]
fn already_exists_zero_wait_and_success_paths_keep_old_traces(){
    for win in [false,true] {
        let p=fixture("live");std::fs::create_dir(ScheduleFileLock::lock_dir_for(&p)).unwrap();let mut o=Ops::new(&[Some(ErrorKind::AlreadyExists)]);o.fallback=ErrorKind::AlreadyExists;
        assert!(acquire(&p,&mut o,win,20).is_none());assert!(ScheduleFileLock::lock_dir_for(&p).exists());assert_eq!(o.time,Duration::from_millis(20));assert!(!o.trace.iter().any(|x|x=="reclaim"));println!("UNCHANGED_TRACE live-{win} {:?}",o.trace);
        let p=fixture("stale");let mut o=Ops::new(&[Some(ErrorKind::AlreadyExists),None]);o.stale=true;let _lock=acquire(&p,&mut o,win,20).unwrap();assert_eq!(o.grants,1);assert!(o.trace.iter().any(|x|x=="reclaim"));println!("UNCHANGED_TRACE stale-{win} {:?}",o.trace);
        let p=fixture("zero");let mut o=Ops::new(&[None]);let _lock=acquire(&p,&mut o,win,0).unwrap();assert_eq!(o.time,Duration::ZERO);println!("UNCHANGED_TRACE success-zero-{win} {:?}",o.trace);
        let p=fixture("zero-exists");let mut o=Ops::new(&[Some(ErrorKind::AlreadyExists)]);assert!(acquire(&p,&mut o,win,0).is_none());assert_eq!(o.time,Duration::ZERO);println!("UNCHANGED_TRACE exists-zero-{win} {:?}",o.trace);
    }
}
#[test]
fn windows_raw_five_mapping_and_real_directory_admission(){
    #[cfg(windows)] {
        assert_eq!(std::io::Error::from_raw_os_error(5).kind(),ErrorKind::PermissionDenied,"native Windows premise");
        let p=fixture("native-map");let mut o=Ops::new(&[Some(std::io::Error::from_raw_os_error(5).kind()),None]);let _guard=acquire(&p,&mut o,true,100).expect("native mapping retries");assert_eq!(o.grants,1);
    }
    #[cfg(not(windows))] { assert_ne!(std::env::consts::OS,"windows"); } // source-policy model is separate from native errno evidence
}

const BEFORE:&[u8]=br#"{ "owner":17, "jobs":[{"id":"keep","action":"push","via_queue":true},{"id":"remove","action":"command"}] }"#;
fn caller(site:&str,p:&std::path::Path,read:&ScheduleReader<'_>,get:&ScheduleAcquirer<'_>)->serde_json::Value {
    match site {
        "seed"=>{ensure_builtin_jobs_at_access(p,read,get);json!(null)},
        "hotreload"=>{load_jobs_at_access(p,LoadMode::HotReload,read,get);json!(null)},
        "remove"=>{remove_job_from_file_at_access(p,"remove",read,get);json!(null)},
        "requeue"=>{let d=super::tests::test_daemon();let mut rx=d.bus.subscribe();let j:Job=serde_json::from_value(json!({"id":"wake","action":"push","at":1900000000i64})).unwrap();requeue_oneshot_after_frozen_at_access(&d,p,&j,read,get);let e=serde_json::to_value(rx.try_recv().expect("requeue result")).unwrap();crate::state::test_stop_daemon(&d);e},
        _=>panic!("site"),
    }
}
#[test]
fn four_real_callers_preserve_after_denial_then_locked_read_fault(){
    for site in ["seed","hotreload","remove","requeue"] {
        let p=fixture(site);std::fs::write(&p,BEFORE).unwrap();let ops=std::cell::RefCell::new(Ops::new(&[Some(ErrorKind::PermissionDenied),Some(ErrorKind::PermissionDenied),None]));
        let get=|p:&std::path::Path|acquire(p,&mut ops.borrow_mut(),true,100);
        let faults=std::cell::Cell::new(0);let read=|p:&std::path::Path|{assert!(ScheduleFileLock::lock_dir_for(p).is_dir());faults.set(faults.get()+1);Err(std::io::Error::new(ErrorKind::PermissionDenied,"read after lock"))};
        let event=caller(site,&p,&read,&get);assert_eq!(faults.get(),1,"{site}: fault must be reached under real lock after retry");assert_eq!(ops.borrow().grants,1);assert_eq!(std::fs::read(&p).unwrap(),BEFORE,"{site}: PRESERVE");
        if site=="requeue" {assert_eq!(event["payload"]["ok"],json!(false));assert_eq!(event["payload"]["reason"],json!("read_failed"));}
        // Prove later writes were actually permitted; no chmod/read-only filesystem vacuity.
        caller(site,&p,&|p|std::fs::read(p),&ScheduleFileLock::acquire);assert_ne!(std::fs::read(&p).unwrap(),BEFORE,"{site}: valid write positive control");
    }
}
#[test]
fn caller_none_dispositions_do_not_replay_or_write(){
    for site in ["seed","hotreload","remove","requeue"] {
        let p=fixture("none");std::fs::write(&p,BEFORE).unwrap();let count=std::cell::Cell::new(0);let reads=std::cell::Cell::new(0);
        let get=|_:&std::path::Path|{count.set(count.get()+1);None};let read=|p:&std::path::Path|{reads.set(reads.get()+1);std::fs::read(p)};
        let e=caller(site,&p,&read,&get);assert_eq!(count.get(),1);assert_eq!(reads.get(),usize::from(site=="hotreload"));assert_eq!(std::fs::read(&p).unwrap(),BEFORE);
        if site=="requeue" {assert_eq!(e["payload"]["ok"],json!(false));}
    }
}
#[test]
fn retry_alone_wipes_but_paired_real_requeue_preserves(){
    let p=fixture("composition");std::fs::write(&p,BEFORE).unwrap();let mut o=Ops::new(&[Some(ErrorKind::PermissionDenied),Some(ErrorKind::PermissionDenied),None]);
    let guard=acquire(&p,&mut o,true,100).expect("composition must reach actual lock ownership");
    let faults=std::cell::Cell::new(0);let denied=|p:&std::path::Path|->std::io::Result<String>{assert!(ScheduleFileLock::lock_dir_for(p).is_dir());faults.set(faults.get()+1);Err(std::io::Error::new(ErrorKind::PermissionDenied,"owned read fault"))};
    // Historical 0b25c511 requeue collapse (schedule.rs:1529-1542), retained ONLY as an adversarial test control.
    let mut root:serde_json::Value=denied(&p).ok().and_then(|c|serde_json::from_str(&c).ok()).unwrap_or_else(||json!({"jobs":[]}));
    root["jobs"].as_array_mut().unwrap().push(json!({"id":"wake","action":"push","at":1900000001i64}));
    let ok=write_schedule_atomic(&p,&root);assert!(ok);assert_eq!(faults.get(),1);assert_ne!(std::fs::read(&p).unwrap(),BEFORE);println!("RETRY_ALONE_WIPE: actual lock/read fault/write; ok=true; keep job lost");drop(guard);
    std::fs::write(&p,BEFORE).unwrap();let ops=std::cell::RefCell::new(Ops::new(&[Some(ErrorKind::PermissionDenied),Some(ErrorKind::PermissionDenied),None]));let get=|p:&std::path::Path|acquire(p,&mut ops.borrow_mut(),true,100);
    let hits=std::cell::Cell::new(0);let read=|p:&std::path::Path|{assert!(ScheduleFileLock::lock_dir_for(p).is_dir());hits.set(hits.get()+1);Err(std::io::Error::new(ErrorKind::PermissionDenied,"owned read fault"))};
    let e=caller("requeue",&p,&read,&get);assert_eq!(hits.get(),1);assert_eq!(std::fs::read(&p).unwrap(),BEFORE);assert_eq!(e["payload"]["ok"],json!(false));assert_eq!(e["payload"]["reason"],json!("read_failed"));println!("PAIRED_PRESERVE: real requeue entry; ok=false reason=read_failed; bytes identical");
}

#[test]
#[ignore = "owned native-filesystem child, invoked by real_contention_preserves_all_ids"]
fn contention_child() {
    use std::sync::atomic::{AtomicBool,AtomicUsize};
    let path=fixture("native-contention");std::fs::write(&path,br#"{"jobs":[]}"#).unwrap();
    let stop=Arc::new(AtomicBool::new(false));let reads=Arc::new(AtomicUsize::new(0));let active=Arc::new(AtomicUsize::new(0));
    let (ready_tx,ready_rx)=std::sync::mpsc::channel();
    let reader={let path=path.clone();let stop=stop.clone();let reads=reads.clone();std::thread::spawn(move||{
        let mut first=true;
        while !stop.load(Ordering::Acquire){
            let root:serde_json::Value=serde_json::from_slice(&std::fs::read(&path).expect("reader snapshot")).expect("atomic parseable snapshot");assert!(root["jobs"].is_array());reads.fetch_add(1,Ordering::Relaxed);
            if first {ready_tx.send(()).unwrap();first=false;}
            std::thread::yield_now();
        }
    })};
    ready_rx.recv_timeout(Duration::from_secs(30)).expect("reader acknowledged initial valid document");
    let barrier=Arc::new(std::sync::Barrier::new(3));
    struct Critical<'a>(&'a AtomicUsize);
    impl Drop for Critical<'_>{fn drop(&mut self){self.0.fetch_sub(1,Ordering::SeqCst);}}
    struct Record {native:NativeScheduleLockOps,errors:Vec<(String,Option<i32>)>}
    impl ScheduleLockOps for Record {
        type Clock=std::time::Instant;
        fn parent(&mut self,p:&std::path::Path){self.native.parent(p)}
        fn create(&mut self,p:&std::path::Path)->std::io::Result<()>{let r=self.native.create(p);if let Err(e)=&r{self.errors.push((format!("{:?}",e.kind()),e.raw_os_error()));}r}
        fn is_stale(&mut self,p:&std::path::Path,d:Duration)->bool{self.native.is_stale(p,d)}
        fn reclaim(&mut self,p:&std::path::Path){self.native.reclaim(p)}
        fn now(&mut self)->Self::Clock{self.native.now()}
        fn sleep(&mut self,d:Duration){self.native.sleep(d)}
    }
    let writers:Vec<_>=(0..3).map(|writer|{let path=path.clone();let active=active.clone();let barrier=barrier.clone();std::thread::spawn(move||{
        barrier.wait();let mut observations=Vec::new();
        for i in 0..25 {
            let mut ops=Record{native:NativeScheduleLockOps,errors:vec![]};
            let lock=ScheduleFileLock::acquire_with_ops(&path,Duration::from_millis(SCHEDULE_LOCK_WAIT_MS),Duration::from_secs(SCHEDULE_LOCK_STALE_SECS),cfg!(windows),&mut ops).expect("no update after failed acquisition");
            let was=active.fetch_add(1,Ordering::SeqCst);let cs=Critical(&active);assert_eq!(was,0,"at most one protected RMW");
            let mut root:serde_json::Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();root["jobs"].as_array_mut().unwrap().push(json!({"id":format!("w{writer}-{i}"),"action":"command"}));assert!(write_schedule_atomic(&path,&root));
            drop(cs);drop(lock); // decrement while still owning the directory token
            observations.extend(ops.errors);
        }
        observations
    })}).collect();
    let mut events=Vec::new();for w in writers{events.extend(w.join().expect("writer completed"));}
    stop.store(true,Ordering::Release);reader.join().expect("reader completed");assert!(reads.load(Ordering::Relaxed)>0);assert_eq!(active.load(Ordering::Relaxed),0);
    let root:serde_json::Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();let ids:std::collections::BTreeSet<_>=root["jobs"].as_array().unwrap().iter().map(|j|j["id"].as_str().unwrap().to_owned()).collect();let want:std::collections::BTreeSet<_>=(0..3).flat_map(|w|(0..25).map(move|i|format!("w{w}-{i}"))).collect();assert_eq!(ids,want);assert_eq!(root["jobs"].as_array().unwrap().len(),75);
    assert!(!ScheduleFileLock::lock_dir_for(&path).exists());println!("NATIVE_CONTENTION {}",json!({"ids":75,"reader_snapshots":reads.load(Ordering::Relaxed),"errors":events,"permission_denied_observed":events.iter().any(|(k,_)|k=="PermissionDenied")}));std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
#[test]
fn real_contention_preserves_all_ids() {
    let mut c=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","schedule::lock_policy_tests::contention_child","--ignored","--nocapture","--test-threads=1"]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let end=std::time::Instant::now()+Duration::from_secs(60);
    loop { // wallclock: guard bounds the owned contention child and potentially stuck joins
        if c.try_wait().unwrap().is_some(){break;}
        if std::time::Instant::now()>=end{let _=c.kill();let _=c.wait();panic!("owned contention child exceeded hang guard");}
        std::thread::sleep(Duration::from_millis(5));
    }
    let o=c.wait_with_output().unwrap();let text=format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr));assert!(o.status.success(),"{text}");assert!(text.contains("NATIVE_CONTENTION "),"missing child completion");println!("{text}");
}
