//! Real schedule entry-point specimens. Faults are call-owned; only fixture reads are denied.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

const ORIGINAL: &[u8] = br#"{ "owner_extension": {"keep": 17}, "jobs": [{"id":"kept","action":"push","via_queue":true,"custom":"retain"},{"id":"remove","action":"command"}] }"#;

#[test]
#[ignore = "owned subprocess fixture, exercised by each integrity cell"]
fn fixture_child() {
    let site = std::env::var("W54_SCHEDULE_SITE").expect("fixture site");
    let kind = std::env::var("W54_SCHEDULE_KIND").expect("fixture kind");
    let path = PathBuf::from(std::env::var_os("W54_SCHEDULE_PATH").expect("fixture path"));
    let faults = std::cell::Cell::new(0usize);
    let reads = std::cell::Cell::new(0usize);
    let read = |p: &std::path::Path| {
        assert!(ScheduleFileLock::lock_dir_for(p).is_dir(), "read must occur under the real writer lock");
        reads.set(reads.get()+1);
        if kind == "denied" {
            faults.set(faults.get()+1);
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "owned read fault"))
        } else { std::fs::read(p) }
    };
    let mut event = serde_json::Value::Null;
    match site.as_str() {
        "seed" => ensure_builtin_jobs_at_read(&path, &read),
        "hotreload" => { load_jobs_at_read(&path, LoadMode::HotReload, &read); }
        "remove" => remove_job_from_file_at_read(&path, "remove", &read),
        "requeue" => {
            let d=super::tests::test_daemon();
            let mut rx=d.bus.subscribe();
            let job:Job=serde_json::from_value(json!({"id":"wake","action":"push","at":1900000000i64})).unwrap();
            requeue_oneshot_after_frozen_at_read(&d,&path,&job,&read);
            if let Ok(e)=rx.try_recv() { event=serde_json::to_value(e).unwrap(); }
            crate::state::test_stop_daemon(&d);
        }
        _ => panic!("unknown fixture site"),
    }
    println!("W54_SCHEDULE_RESULT {}",json!({"faults":faults.get(),"reads":reads.get(),"event":event}));
}

fn cell(site:&str,kind:&str) {
    static SEQ:AtomicU64=AtomicU64::new(0);
    let dir=std::env::temp_dir().join(format!("w54-rmw-{}-{}-{}",std::process::id(),now_epoch().to_bits(),SEQ.fetch_add(1,Ordering::Relaxed)));
    std::fs::create_dir(&dir).expect("fresh owned directory");
    let path=dir.join("schedule.json");
    let bytes:Vec<u8>=match kind {
        "json"=>b"{broken json".to_vec(),
        "utf8"=>vec![b'{',0xff,b'}'],
        "root"=>b"  [1,2,3]  ".to_vec(),
        "null_jobs"=>br#"{ "owner_extension":17, "jobs":null }"#.to_vec(),
        "nonarray"=>br#"{ "owner_extension":17, "jobs":"bad" }"#.to_vec(),
        "entry"=>br#"{ "owner_extension":17, "jobs":[{"id":"legacy","action":"push","via_queue":true},{"id":7,"action":"push"}] }"#.to_vec(),
        "missing_jobs"=>br#"{ "owner_extension":{"keep":17} }"#.to_vec(),
        _=>ORIGINAL.to_vec(),
    };
    if kind!="absent" {std::fs::write(&path,&bytes).unwrap();}
    let mut child=std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact","schedule::integrity_tests::fixture_child","--ignored","--nocapture","--test-threads=1"])
        .env("W54_SCHEDULE_SITE",site).env("W54_SCHEDULE_KIND",kind).env("W54_SCHEDULE_PATH",&path)
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let deadline=std::time::Instant::now()+Duration::from_secs(60);
    loop { // wallclock: guard bounds only this owned fixture child; timeout is failure
        if child.try_wait().unwrap().is_some(){break;}
        if std::time::Instant::now()>=deadline {let _=child.kill();let _=child.wait();panic!("owned fixture hung: {site}/{kind}");}
        std::thread::sleep(Duration::from_millis(5));
    }
    let output=child.wait_with_output().unwrap();
    let log=format!("{}{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(),"normal RMW must not panic: {site}/{kind}: {log}");
    let result:serde_json::Value=serde_json::from_str(log.lines().find_map(|l|l.split_once("W54_SCHEDULE_RESULT ").map(|(_, value)| value)).expect("child completed result")).unwrap();
    assert!(result["reads"].as_u64().unwrap()>0,"entry must reach its read");
    if kind=="denied" {assert_eq!(result["faults"],json!(1),"fault must bite exactly once under lock");}
    let refused=matches!(kind,"denied"|"json"|"utf8"|"root"|"null_jobs"|"nonarray"|"entry");
    if refused {
        let reason=match kind {"denied"=>"read_failed","json"=>"invalid_json","utf8"=>"invalid_utf8",_=>"invalid_schema"};
        if site=="hotreload" && kind=="json" {
            let backups:Vec<_>=std::fs::read_dir(&dir).unwrap().map(|e|e.unwrap().path()).filter(|p|p!=&path).collect();
            assert!(backups.iter().any(|p|std::fs::read(p).ok().as_deref()==Some(bytes.as_slice())),"explicit recovery must preserve original bytes before reseed");
            assert!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap()["jobs"].is_array());
        } else {assert_eq!(std::fs::read(&path).unwrap(),bytes,"PRESERVE {site}/{kind}: no normal mutation on refused input");}
        assert!(log.contains(&format!("reason={reason}")),"named reason missing for {site}/{kind}: {log}");
        if site=="requeue" {assert!(log.contains("\"ok\":false"),"refusal must not report ok=true: {log}");}
    } else if kind=="absent" {
        if matches!(site,"hotreload"|"remove") {assert!(!path.exists(),"absence must remain absence");}
        else {assert!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap()["jobs"].is_array());}
    } else {
        let root:serde_json::Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(root["owner_extension"]["keep"],json!(17),"unknown root fields survive");
        if kind!="missing_jobs" {
            let kept=root["jobs"].as_array().unwrap().iter().find(|j|j["id"]=="kept").unwrap();
            assert_eq!(kept["custom"],json!("retain"),"unknown job fields survive");
            assert_eq!(kept["action"],json!("push_queued"),"valid legacy form retains canonicalization");
        }
        if site=="requeue" {assert!(root["jobs"].as_array().unwrap().iter().any(|j|j["id"]=="wake"));}
        if site=="remove" && kind!="missing_jobs" {assert!(!root["jobs"].as_array().unwrap().iter().any(|j|j["id"]=="remove"));}
    }
    assert!(!ScheduleFileLock::lock_dir_for(&path).exists(),"owned lock released");
    std::fs::remove_dir_all(&dir).expect("remove own unique fixture");
}
macro_rules! site_cells {($site:literal,$($name:ident=>$kind:literal),+)=>{$(#[test] fn $name(){cell($site,$kind);})+};}
site_cells!("seed", seed_denied=>"denied",seed_json=>"json",seed_utf8=>"utf8",seed_root=>"root",seed_null=>"null_jobs",seed_nonarray=>"nonarray",seed_entry=>"entry",seed_absent=>"absent",seed_valid=>"valid",seed_missing_jobs=>"missing_jobs");
site_cells!("hotreload", hotreload_denied=>"denied",hotreload_json_recovery=>"json",hotreload_utf8=>"utf8",hotreload_root=>"root",hotreload_null=>"null_jobs",hotreload_nonarray=>"nonarray",hotreload_entry=>"entry",hotreload_absent=>"absent",hotreload_valid=>"valid",hotreload_missing_jobs=>"missing_jobs");
site_cells!("remove", remove_denied=>"denied",remove_json=>"json",remove_utf8=>"utf8",remove_root=>"root",remove_null=>"null_jobs",remove_nonarray=>"nonarray",remove_entry=>"entry",remove_absent=>"absent",remove_valid=>"valid",remove_missing_jobs=>"missing_jobs");
site_cells!("requeue", requeue_denied=>"denied",requeue_json=>"json",requeue_utf8=>"utf8",requeue_root=>"root",requeue_null=>"null_jobs",requeue_nonarray=>"nonarray",requeue_entry=>"entry",requeue_absent=>"absent",requeue_valid=>"valid",requeue_missing_jobs=>"missing_jobs");
