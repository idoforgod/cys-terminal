//! Owned-process native/core specimens; explicit fixture validators are not production authority.
use super::*;
use crate::state::{HideConsole, WriteReq};
use crate::writer_gate::{WriterSender, QueuedWrite};
use std::any::Any;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::mpsc;
use std::thread::JoinHandle;

const SUPERVISOR: &str = r###"# Isolated test fixture. Standard-library Python; no network or live state.
# Parent owns this process. Every descendant has a dedicated stdin pipe retained
# only by this supervisor, so supervisor exit closes it and makes fixtures exit.
import json
import os
import subprocess
import sys

agent_a, agent_b = sys.argv[1:3]
children = {}


def close_child(name):
    proc = children.pop(name, None)
    if proc is None:
        raise RuntimeError("no owned child " + name)
    if proc.stdin is not None:
        proc.stdin.close()
    try:
        status = proc.wait(timeout=2)
    except subprocess.TimeoutExpired:
        # Only our own unreaped Popen child; never a discovered arbitrary PID.
        proc.terminate()
        try:
            status = proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            proc.kill()
            status = proc.wait(timeout=2)
    return {"pid": proc.pid, "status": status}


try:
    print(json.dumps({"ready": os.getpid()}), flush=True)
    for line in sys.stdin:
        try:
            command = json.loads(line)
            op = command["op"]
            name = command.get("name")
            if op == "spawn":
                if name in children:
                    raise RuntimeError("owned slot already occupied")
                if name == "a":
                    argv = [agent_a, agent_b]
                elif name == "b":
                    argv = [agent_b]
                else:
                    raise RuntimeError("unknown fixture slot")
                proc = subprocess.Popen(
                    argv, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL, close_fds=True,
                    env={"PATH": "/usr/bin:/bin", "PYTHONDONTWRITEBYTECODE": "1"},
                )
                children[name] = proc
                result = {"pid": proc.pid}
            elif op == "close":
                result = close_child(name)
            elif op == "switch":
                proc = children["a"]
                proc.stdin.write(b"SWITCH\n")
                proc.stdin.flush()
                result = {"pid": proc.pid, "switch_requested": True}
            elif op == "alive":
                result = {"root": os.getpid(), "children": {n: p.poll() for n, p in children.items()}}
            else:
                raise RuntimeError("unknown operation")
            print(json.dumps({"ok": result}), flush=True)
        except Exception as exc:
            print(json.dumps({"error": type(exc).__name__ + ": " + str(exc)}), flush=True)
finally:
    for name in list(children):
        try:
            close_child(name)
        except Exception:
            # Parent drop still closes every inherited write end on process exit.
            pass
"###;

static NEXT:AtomicU64=AtomicU64::new(1);
mod ci_inputs { include!("c8_test_inputs.rs"); }
struct Scratch { path:PathBuf }
impl Scratch {
    fn new()->Self {
        Self { path:ci_inputs::private_root("C8_NATIVE_TEST_ROOT", "n")
            .expect("C8 native fixture private root provisioning failed") }
    }
}

fn executable_python(path: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = path.canonicalize().ok()?;
    let metadata = std::fs::metadata(&path).ok()?;
    (metadata.is_file() && metadata.permissions().mode() & 0o111 != 0).then_some(path)
}
fn default_python() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).filter(|dir| dir.is_absolute())
            .map(|dir| dir.join("python3")).collect()).unwrap_or_default();
    candidates.extend(["/usr/bin/python3", "/usr/local/bin/python3", "/opt/homebrew/bin/python3"]
        .into_iter().map(PathBuf::from));
    candidates.into_iter().find_map(|path| executable_python(&path))
}
fn fixture_python(test: &'static str) -> Option<PathBuf> {
    // An explicit override is checked on EVERY resolution, before any cached default.
    // Invalid explicit configuration is a test failure; it never silently falls back.
    if let Some(value) = std::env::var_os("C8_NATIVE_FIXTURE_PYTHON") {
        let path = PathBuf::from(value);
        assert!(path.is_absolute(), "explicit C8_NATIVE_FIXTURE_PYTHON must be absolute");
        return Some(executable_python(&path).expect("explicit C8_NATIVE_FIXTURE_PYTHON is not an executable file"));
    }
    static DEFAULT: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    if let Some(path) = DEFAULT.get_or_init(default_python) { return Some(path.clone()); }
    // Stable libtest cannot dynamically ignore a test. Direct stderr Write bypasses
    // successful-test println/eprintln capture; the evidence runner MUST classify this
    // named marker as UNMEASURED even though libtest counts the returning test as `ok`.
    let record = serde_json::json!({"state":"UNMEASURED", "group":"C8_NATIVE_TRANSITIONS",
        "test":test, "reason":"python3_unavailable"});
    let line = format!("\nC8_NATIVE_UNMEASURED_V1 {record}\n");
    assert!(line.len() <= 512, "C8 UNMEASURED marker exceeded bounded output");
    let mut stderr = std::io::stderr().lock();
    stderr.write_all(line.as_bytes()).expect("C8 UNMEASURED marker could not be written");
    stderr.flush().expect("C8 UNMEASURED marker could not be flushed");
    None
}
impl Drop for Scratch { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.path); } }
struct OwnedRoot {
    child:Child, replies:mpsc::Receiver<serde_json::Value>,reader:Option<JoinHandle<()>>,
}
impl OwnedRoot {
    fn spawn(scratch:&Scratch, python:&Path, compiler_override:Option<&std::ffi::OsStr>)->Self {
        let a=scratch.path.join("np-agent-a");let b=scratch.path.join("np-agent-b");
        // Own minimal native programs: relocated Apple system binaries were observed exiting
        // with signal9 on this host. Do not alter signatures or live system executables.
        let fixture_source=scratch.path.join("agent.c");
        std::fs::write(&fixture_source,r#"#include <stdio.h>
#include <string.h>
#include <unistd.h>
int main(int argc, char **argv) {
    char line[64];
    while (fgets(line, sizeof line, stdin)) {
        if (argc == 2 && strcmp(line, "SWITCH\n") == 0) {
            execl(argv[1], argv[1], (char *)0);
            return 111;
        }
    }
    return ferror(stdin) ? 1 : 0;
}
"#).unwrap();
        let compiler=compiler_override.map(std::ffi::OsStr::to_owned)
            .unwrap_or_else(||std::env::var_os("C8_NATIVE_FIXTURE_CC").unwrap_or_else(||"/usr/bin/cc".into()));
        let built=Command::new(compiler).hide_console().arg("-std=c11").arg("-O0")
            .arg(&fixture_source).arg("-o").arg(&a).env_clear().env("PATH","/usr/bin:/bin")
            .env("PYTHONDONTWRITEBYTECODE","1")
            .output().expect("fixture C compiler must be available");
        assert!(built.status.success(),"owned native fixture compilation failed: {}",String::from_utf8_lossy(&built.stderr));
        std::fs::copy(&a,&b).unwrap();
        let script=scratch.path.join("supervisor.py");std::fs::write(&script,SUPERVISOR).unwrap();
        let mut child=Command::new(python)
            .hide_console()
            .arg("-u").arg(&script).arg(&a).arg(&b)
            .env_clear().env("PATH","/usr/bin:/bin").env("PYTHONDONTWRITEBYTECODE","1")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().unwrap();
        let stdout=child.stdout.take().unwrap();let (tx,replies)=mpsc::channel();
        let reader=std::thread::spawn(move||{
            for line in BufReader::new(stdout).lines() {
                let Ok(line)=line else { break; };
                let value=serde_json::from_str(&line).unwrap_or_else(|_|serde_json::json!({"error":"malformed fixture reply"}));
                if tx.send(value).is_err() { break; }
            }
        });
        // Install cleanup ownership BEFORE a startup assertion can unwind.
        let owned=Self { child,replies,reader:Some(reader) };
        let ready=owned.replies.recv_timeout(Duration::from_secs(5)).expect("fixture root startup timed out");
        assert_eq!(ready["ready"].as_u64(),Some(owned.child.id() as u64));
        owned
    }
    fn command(&mut self,op:&str,name:&str)->serde_json::Value {
        let input=serde_json::json!({"op":op,"name":name});
        let pipe=self.child.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *pipe,&input).unwrap();pipe.write_all(b"\n").unwrap();pipe.flush().unwrap();
        let reply=self.replies.recv_timeout(Duration::from_secs(5)).expect("fixture protocol timeout, not a mutant kill");
        assert!(reply.get("error").is_none(),"fixture error: {reply}");reply["ok"].clone()
    }
    fn start(&mut self,name:&str)->u32 { self.command("spawn",name)["pid"].as_u64().unwrap() as u32 }
    fn close(&mut self,name:&str) { self.command("close",name); }
    fn switch_a(&mut self) { self.command("switch","a"); }
}
impl Drop for OwnedRoot {
    fn drop(&mut self) {
        // EOF asks the supervisor to close/reap its owned descendants. A killed
        // supervisor also closes their sole stdin writers, causing EOF exit.
        drop(self.child.stdin.take());
        let deadline=Instant::now()+Duration::from_secs(5);
        loop {
            match self.child.try_wait() {
                Ok(Some(_))=>break,
                _ if Instant::now()>=deadline=>{
                    let _=self.child.kill(); // only this unreaped Child, never a discovered PID/group
                    let _=self.child.wait();break;
                }
                _=>std::thread::sleep(Duration::from_millis(5)),
            }
        }
        if let Some(reader)=self.reader.take() { let _=reader.join(); }
    }
}
fn base_epochs()->Epochs { Epochs { daemon_instance:10,surface_instance:20,launch:30,conversation:40,policy:50,binding:0 } }
fn fixture_policy(adapter:&str,bin:&str)->ReceiverPolicy {
    // Explicit synthetic configuration. Adapter labels select S4-2 behavior;
    // these are NOT observed/verified installations of the named real CLIs.
    ReceiverPolicy { policy_epoch:50,declaration:format!("fixture:{bin}"),config_digest:format!("fixture-digest:{bin}"),
        adapter:adapter.into(),recipe:receiver::Recipe { bin:bin.into(),declared_invocation:None },origin:RegistrationOrigin::HandObserved,
        class:ProofClass::FullRequired,weak_risk:None }
}
struct Rig {
    root:OwnedRoot, // close/reap before scratch directory destruction
    _scratch:Scratch,
    cell:Arc<ReceiverCell>,sender:WriterSender,receiver:mpsc::Receiver<QueuedWrite>,
}
impl Rig {
    fn new(windows_rule:bool, test:&'static str)->Option<Self> {
        let python=fixture_python(test)?;
        let scratch=Scratch::new();let root=OwnedRoot::spawn(&scratch,&python,None);
        let root_observation=process_observation::observe(root.child.id()).unwrap();
        let group=root_observation.identity.process_group;
        let (sender,receiver)=WriterSender::channel(2);
        let a=fixture_policy("claude","np-agent-a");let b=fixture_policy("gemini","np-agent-b");
        let cell=Arc::new(ReceiverCell { io:SeatIo { surface_id:7,root_pid:root.child.id(),
            foreground:Arc::new(move||Some(group)),writer:sender.gate() },
            executor:Arc::new(Executor::try_new(1).unwrap()),windows:windows_rule,
            core:Mutex::new(Core { epochs:base_epochs(),root:None,catalog:vec![a.clone(),b].into(),
                policy:PolicySlot::Fixed(Arc::new(a)),current:None,dead:HashSet::new(),pending:None,
                next_attempt:1,next_episode:1 }) });
        Some(Self { root,_scratch:scratch,cell,sender,receiver })
    }
    fn work(&self,label:&str)->Work {
        Work { owner:DiscoveryOwner::Registration(RegistrationObligation::new(
            format!("{label}-{}",NEXT.fetch_add(1,Ordering::Relaxed)),7)),
            obligation_id:format!("obligation-{label}"),target:Target::Surface,expected:self.cell.epochs() }
    }
    fn initial(&mut self)->Arc<PublishedNative> {
        let pid=self.root.start("a");let observation=process_observation::observe(pid).unwrap();
        assert_eq!(Path::new(std::str::from_utf8(&observation.identity.executable).unwrap()).file_name().unwrap(),"np-agent-a",
            "owned executable did not create the intended native fixture; no synthetic identity substitution");
        let work=self.work("initial");assert!(self.cell.begin_initial(work.clone()).unwrap());
        self.cell.open_window(&work,Instant::now()).unwrap();
        let active=activate_step(&self.cell,&work).unwrap();
        assert_eq!(active.token.agent_id().0,pid);active.token
    }
    fn prove_old_dead(&mut self,old:&Arc<PublishedNative>) {
        self.root.close("a");
        assert_eq!(old.binding.root.recheck(),Ok(()),"owned root must remain alive for the agent-only death control");
        assert_eq!(old.binding.agent.recheck(),Err(Failure::Dead),"fixture must establish actual agent death");
        let fact=self.cell.observe_agent_death().unwrap();self.cell.accept_death(fact).unwrap();
        assert!(old.sealed());
    }
}

struct SafeFixtureInput;
impl PublicationValidator for SafeFixtureInput {
    type Guard<'a>=();
    fn acquire_and_validate<'a>(&'a self,c:&CommitContext<'_>)->Result<((),InputStamp),String> {
        // Deliberately not real input/role evidence; these specimens test native/core transitions.
        Ok(((),InputStamp { epochs:c.work.expected,surface_id:7,role:None,input_generation:0,
            output_generation:0,raw_human_bytes:0,pending_bytes:0,last_human_input:None }))
    }
}
struct FixtureStore;
impl DurableActivationValidator for FixtureStore {
    fn validate_transition_and_retokenization(&self,_:&TransitionRecord)->Result<String,String> {
        Ok("fixture-only:no-production-durability-claim".into())
    }
}
struct Authorizer { allow:bool,calls:AtomicUsize }
impl Authorizer { fn new(allow:bool)->Self { Self { allow,calls:AtomicUsize::new(0) } } }
impl RegistrationAuthorizer for Authorizer {
    fn validate_registration(&self,c:&ChangeContext)->Result<String,String> {
        self.calls.fetch_add(1,Ordering::SeqCst);
        assert_eq!(c.surface_id,7);assert_eq!(c.old_adapter,"claude");assert_eq!(c.new_adapter,"gemini");
        if self.allow { Ok("fixture-authenticated-transaction".into()) } else { Err("fixture authorizer refused".into()) }
    }
}

type WorkerAnswer<T>=Result<Result<T,CoreError>,Box<dyn Any+Send>>;
fn wait_ready(cell:&ReceiverCell) {
    let ceiling=Instant::now()+Duration::from_secs(5);
    loop {
        let after={ let core=cell.core.lock().unwrap();
            core.pending.as_ref().and_then(|p|p.gate.budget.last_start)
                .map(|t|t+Duration::from_millis(receiver::MIN_START_SPACING_MS)) };
        if !cell.executor.slot_busy(cell.io.surface_id) && after.is_none_or(|t|Instant::now()>=t) { return; }
        assert!(Instant::now()<ceiling,"fixture slot/spacing wait exceeded ceiling; not a behavioral mutant kill");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn worker_step<T:Send+'static>(cell:&Arc<ReceiverCell>,work:&Work,
    action:impl FnOnce(&Arc<ReceiverCell>,Result<NativeCandidate,CoreError>)->Result<T,CoreError>+Send+'static)
    ->Result<T,CoreError>
{
    wait_ready(cell);
    let (tx,rx)=mpsc::channel::<WorkerAnswer<T>>();
    let action=Arc::new(Mutex::new(Some(action)));
    let deadline=Instant::now()+Duration::from_secs(2);
    loop {
        let c=cell.clone();let out=tx.clone();let f=action.clone();
        let result=cell.schedule(work.clone(),move|candidate|{
            let observed=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
                f.lock().unwrap().take().expect("one completion owns the action")(&c,candidate)
            }));
            // Every assertion/unwind reaches the test thread. Executor swallowing
            // a worker panic cannot be credited as an intended mutant failure.
            let _=out.send(observed);
        });
        match result {
            Ok(())=>break,
            Err(CoreError::Busy)|Err(CoreError::Start(StartRefusal::AdmissionBusy)) if Instant::now()<deadline=>
                std::thread::sleep(Duration::from_millis(2)),
            Err(e)=>return Err(e),
        }
    }
    match rx.recv_timeout(Duration::from_secs(10)).expect("native callback produced no conclusive result; timeout is NOT a killed mutant") {
        Ok(result)=>result,Err(panic)=>std::panic::resume_unwind(panic),
    }
}
fn activate_step(cell:&Arc<ReceiverCell>,work:&Work)->Result<ActivatedPublication,CoreError> {
    worker_step(cell,work,|c,candidate|{
        let stage=match c.prepare_publication(candidate?,&SafeFixtureInput)? {
            PreparedOutcome::Prepared(stage)=>stage,
            PreparedOutcome::WindowsFirstSample=>return Err(CoreError::PendingConflict),
        };
        let capability=stage.durable_capability(&FixtureStore)?;
        c.activate(stage,capability,&SafeFixtureInput)
    })
}
fn change_work(rig:&Rig,auth:&Authorizer)->Work {
    let work=rig.work("change");
    let grant=rig.cell.authorize_change("gemini",RegistrationOrigin::ClaimRole,auth).unwrap();
    rig.cell.begin_change(work.clone(),grant).unwrap();rig.cell.open_window(&work,Instant::now()).unwrap();work
}

#[test]
fn np_native_agent_death_with_root_alive_allows_distinct_same_policy_adoption() {
    let Some(mut rig)=Rig::new(false, "native_publication::native_transition_tests::np_native_agent_death_with_root_alive_allows_distinct_same_policy_adoption") else { return; };let old=rig.initial();let before=rig.cell.epochs();
    rig.prove_old_dead(&old);
    let stale_fact=rig.cell.observe_agent_death().unwrap(); // second REAL native Dead observation
    let new_pid=rig.root.start("a");let work=rig.work("adopt");
    rig.cell.begin_adoption(work.clone()).unwrap();rig.cell.open_window(&work,Instant::now()).unwrap();
    let adopted=activate_step(&rig.cell,&work).unwrap();
    assert_ne!(adopted.token.agent_id(),old.agent_id());assert_eq!(adopted.token.agent_id().0,new_pid);
    assert_eq!(adopted.token.epochs().policy,before.policy);
    assert_eq!(adopted.token.epochs().binding,before.binding+1);
    assert_eq!(adopted.token.epochs().conversation,before.conversation+1);
    assert!(old.sealed());assert!(old.check_native(&*rig.cell.io.foreground).is_err());
    assert!(matches!(rig.cell.accept_death(stale_fact),Err(DeathError::Superseded)),"stale death fact sealed the replacement");
    assert!(!adopted.token.sealed());
    assert_eq!(old.binding.root.recheck(),Ok(()));
}

#[test]
fn np_native_same_pid_exec_is_changed_not_agent_death() {
    let Some(mut rig)=Rig::new(false, "native_publication::native_transition_tests::np_native_same_pid_exec_is_changed_not_agent_death") else { return; };let old=rig.initial();let before=old.agent_id();
    assert!(matches!(rig.cell.observe_agent_death(),Err(DeathError::Alive)));
    assert!(matches!(rig.cell.begin_adoption(rig.work("live-old-agent")),Err(CoreError::NotDead)),
        "a live old receiver must not enter the adoption transition");
    rig.root.switch_a();
    let deadline=Instant::now()+Duration::from_secs(2);
    loop {
        match old.binding.agent.recheck() {
            Err(Failure::Changed)=>break,
            Ok(())|Err(Failure::Unknown { .. }) if Instant::now()<deadline=>std::thread::sleep(Duration::from_millis(2)),
            other=>panic!("fixture did not establish same-PID exec Changed: {other:?}"),
        }
    }
    let new_observation=process_observation::observe(before.0).unwrap();
    assert_eq!(native_id(&new_observation),before,"fixture accidentally changed PID/birth instead of exec");
    assert_eq!(old.binding.root.recheck(),Ok(()));
    assert!(matches!(rig.cell.observe_agent_death(),Err(DeathError::Native(Failure::Changed))));
    assert!(!old.sealed(),"Changed was falsely accepted as native agent death");
    assert!(rig.cell.core.lock().unwrap().dead.is_empty());
    assert_eq!(rig.cell.fixed_policy().unwrap().class,ProofClass::FullRequired);
}

#[test]
fn np_different_builtin_requires_authorizer_and_strict_whole_catalog() {
    let Some(mut rig)=Rig::new(false, "native_publication::native_transition_tests::np_different_builtin_requires_authorizer_and_strict_whole_catalog") else { return; };let old=rig.initial();rig.prove_old_dead(&old);
    rig.root.start("a");rig.root.start("b"); // deliberately competing actual native declarations
    let denied=Authorizer::new(false);
    assert!(matches!(rig.cell.authorize_change("gemini",RegistrationOrigin::LaunchAgent,&denied),Err(CoreError::UnsupportedTransition)));
    assert_eq!(denied.calls.load(Ordering::SeqCst),0);
    assert!(matches!(rig.cell.authorize_change("gemini",RegistrationOrigin::ClaimRole,&denied),Err(CoreError::Authority(_))));
    assert_eq!(denied.calls.load(Ordering::SeqCst),1);
    let allowed=Authorizer::new(true);let work=change_work(&rig,&allowed);
    assert_eq!(allowed.calls.load(Ordering::SeqCst),1,"successful authorization adapter was bypassed");
    let count=rig.cell.executor.counters().started.load(Ordering::Acquire);
    let ambiguous=worker_step(&rig.cell,&work,|_,result|{
        assert!(matches!(result,Err(CoreError::Native(Unknown::Ambiguous(_)))),"a second adapter was ignored by S4-2 strict selection");
        Ok(())
    });ambiguous.unwrap();
    assert!(Arc::ptr_eq(&rig.cell.current().unwrap(),&old));assert_eq!(rig.cell.fixed_policy().unwrap().adapter,"claude");
    rig.root.close("a"); // retain B only; old current token is still the already-dead old A
    let active=activate_step(&rig.cell,&work).unwrap();
    assert_eq!(active.record.adapter,"gemini");assert_eq!(active.record.origin,RegistrationOrigin::ClaimRole);
    assert_eq!(active.token.epochs().policy,old.epochs().policy+1);assert!(old.sealed());
    assert_eq!(rig.cell.executor.counters().started.load(Ordering::Acquire)-count,2,
        "classification plus binding must share one actual start per attempted sample");
}

struct HoldingWriterStore { sender:WriterSender }
impl DurableActivationValidator for HoldingWriterStore {
    fn validate_transition_and_retokenization(&self,_:&TransitionRecord)->Result<String,String> {
        // A REAL ordinary admission lease, held in the fixture channel. No PTY writer runs.
        assert!(self.sender.try_send(WriteReq::Data(b"fixture-owned-operation".to_vec())).is_ok());
        Ok("fixture-store-with-concurrent-ordinary-operation".into())
    }
}
#[test]
fn np_actual_writer_operation_blocks_activation_after_prepare() {
    let Some(mut rig)=Rig::new(false, "native_publication::native_transition_tests::np_actual_writer_operation_blocks_activation_after_prepare") else { return; };let old=rig.initial();rig.prove_old_dead(&old);rig.root.start("a");
    let work=rig.work("idle");rig.cell.begin_adoption(work.clone()).unwrap();rig.cell.open_window(&work,Instant::now()).unwrap();
    let sender=rig.sender.clone();
    worker_step(&rig.cell,&work,move|c,result|{
        let PreparedOutcome::Prepared(stage)=c.prepare_publication(result?,&SafeFixtureInput)? else { panic!("adoption is not Windows different-policy staging"); };
        let capability=stage.durable_capability(&HoldingWriterStore { sender })?;
        assert_eq!(c.io.writer.pending(),1,"the fixture did not actually hold an admitted operation");
        assert!(matches!(c.activate(stage,capability,&SafeFixtureInput),Err(CoreError::WriterBusy)),
            "native selection overwrote an occupied writer after staging");
        Ok(())
    }).unwrap();
    assert!(Arc::ptr_eq(&rig.cell.current().unwrap(),&old));
    let (_,lease)=rig.receiver.recv_timeout(Duration::from_secs(2)).unwrap().into_parts();
    assert_eq!(rig.sender.gate().pending(),1);drop(lease);assert_eq!(rig.sender.gate().pending(),0);
    assert!(activate_step(&rig.cell,&work).is_ok(),"released operation must not leave adoption permanently blocked");
}

#[test]
fn np_windows_rule_requires_two_distinct_actual_native_starts() {
    // Unix native observations + Windows STATE RULE, not native Windows evidence.
    let Some(mut rig)=Rig::new(true, "native_publication::native_transition_tests::np_windows_rule_requires_two_distinct_actual_native_starts") else { return; };let old=rig.initial();rig.prove_old_dead(&old);rig.root.start("b");
    let work=change_work(&rig,&Authorizer::new(true));let count=rig.cell.executor.counters().started.load(Ordering::Acquire);
    worker_step(&rig.cell,&work,|c,result|{
        assert!(matches!(c.prepare_publication(result?,&SafeFixtureInput)?,PreparedOutcome::WindowsFirstSample),
            "one Windows replacement sample published a binding");Ok(())
    }).unwrap();
    assert!(Arc::ptr_eq(&rig.cell.current().unwrap(),&old));assert_eq!(rig.cell.epochs(),old.epochs());
    let new=activate_step(&rig.cell,&work).unwrap();assert_eq!(new.record.adapter,"gemini");
    assert_eq!(rig.cell.executor.counters().started.load(Ordering::Acquire)-count,2);
    assert_eq!(rig.cell.core.lock().unwrap().pending.as_ref().unwrap().gate.budget.starts,2);
    assert!(old.sealed());
}

#[test]
fn np_windows_real_candidate_change_restarts_sample_confirmation() {
    let Some(mut rig)=Rig::new(true, "native_publication::native_transition_tests::np_windows_real_candidate_change_restarts_sample_confirmation") else { return; };let old=rig.initial();rig.prove_old_dead(&old);let first_pid=rig.root.start("b");
    let work=change_work(&rig,&Authorizer::new(true));
    worker_step(&rig.cell,&work,|c,result|{
        assert!(matches!(c.prepare_publication(result?,&SafeFixtureInput)?,PreparedOutcome::WindowsFirstSample));Ok(())
    }).unwrap();
    let first_identity={ let core=rig.cell.core.lock().unwrap();native_id(&core.pending.as_ref().unwrap().first_windows.as_ref().unwrap().summary.agent) };
    assert_eq!(first_identity.0,first_pid);rig.root.close("b");let second_pid=rig.root.start("b");
    worker_step(&rig.cell,&work,|c,result|{
        assert!(matches!(c.prepare_publication(result?,&SafeFixtureInput)?,PreparedOutcome::WindowsFirstSample),
            "a different actual receiver borrowed the previous receiver's confirmation");Ok(())
    }).unwrap();
    assert!(Arc::ptr_eq(&rig.cell.current().unwrap(),&old));
    let active=activate_step(&rig.cell,&work).unwrap();
    assert_eq!(active.token.agent_id().0,second_pid);assert_ne!(active.token.agent_id(),first_identity);
    assert_eq!(rig.cell.core.lock().unwrap().pending.as_ref().unwrap().gate.budget.starts,3);
}

#[test]
fn np_windows_duplicate_sample_identity_does_not_count_as_confirmation() {
    let Some(mut rig)=Rig::new(true, "native_publication::native_transition_tests::np_windows_duplicate_sample_identity_does_not_count_as_confirmation") else { return; };let old=rig.initial();rig.prove_old_dead(&old);rig.root.start("b");
    let work=change_work(&rig,&Authorizer::new(true));
    worker_step(&rig.cell,&work,|c,r|{
        assert!(matches!(c.prepare_publication(r?,&SafeFixtureInput)?,PreparedOutcome::WindowsFirstSample));Ok(())
    }).unwrap();
    worker_step(&rig.cell,&work,|c,r|{
        let candidate=r?;
        // Negative corruption of correspondence metadata only. Both native
        // candidates came from real started jobs; no native success is fabricated.
        let original={ let mut state=c.core.lock().unwrap();let pending=state.pending.as_mut().unwrap();
            let first=pending.first_windows.as_mut().unwrap();let saved=first.ticket.clone();
            first.ticket.attempt_id=candidate.lease.ticket.attempt_id;saved };
        let result=c.prepare_publication(candidate,&SafeFixtureInput);
        { let mut state=c.core.lock().unwrap();state.pending.as_mut().unwrap().first_windows.as_mut().unwrap().ticket=original; }
        assert!(matches!(result,Err(CoreError::Superseded)),"the same actual-start ID was credited twice");
        Ok(())
    }).unwrap();
    assert!(Arc::ptr_eq(&rig.cell.current().unwrap(),&old));
    assert!(activate_step(&rig.cell,&work).is_ok());
}

#[test]
fn np_windows_first_sample_from_other_old_generation_is_discarded() {
    let Some(mut rig)=Rig::new(true, "native_publication::native_transition_tests::np_windows_first_sample_from_other_old_generation_is_discarded") else { return; };let old=rig.initial();rig.prove_old_dead(&old);rig.root.start("b");
    let work=change_work(&rig,&Authorizer::new(true));
    worker_step(&rig.cell,&work,|c,r|{
        assert!(matches!(c.prepare_publication(r?,&SafeFixtureInput)?,PreparedOutcome::WindowsFirstSample));Ok(())
    }).unwrap();
    { let mut state=rig.cell.core.lock().unwrap();
        state.pending.as_mut().unwrap().first_windows.as_mut().unwrap().base.binding+=1; }
    worker_step(&rig.cell,&work,|c,r|{
        assert!(matches!(c.prepare_publication(r?,&SafeFixtureInput)?,PreparedOutcome::WindowsFirstSample),
            "a sample from another old binding generation authorized this transition");Ok(())
    }).unwrap();
    assert!(Arc::ptr_eq(&rig.cell.current().unwrap(),&old));
    assert!(activate_step(&rig.cell,&work).is_ok());
}

#[test]
fn np_cancelled_real_candidate_never_publishes_or_replaces_pending_owner() {
    let Some(mut rig)=Rig::new(false, "native_publication::native_transition_tests::np_cancelled_real_candidate_never_publishes_or_replaces_pending_owner") else { return; };rig.root.start("a");let work=rig.work("cancel");
    rig.cell.begin_initial(work.clone()).unwrap();rig.cell.open_window(&work,Instant::now()).unwrap();
    let cancel=work.clone();
    worker_step(&rig.cell,&work,move|c,r|{
        let candidate=r?;c.cancel(&cancel)?;
        let distinct=Work { owner:DiscoveryOwner::Registration(RegistrationObligation::new("replacement".into(),7)),
            obligation_id:"replacement".into(),target:Target::Surface,expected:c.epochs() };
        assert!(matches!(c.begin_initial(distinct),Err(CoreError::PendingConflict)),
            "a different owner replaced the still-owned cancelled native attempt");
        assert!(matches!(c.prepare_publication(candidate,&SafeFixtureInput),Err(CoreError::Cancelled)),
            "a cancelled native result published");Ok(())
    }).unwrap();
    assert!(rig.cell.current().is_none());
    wait_ready(&rig.cell);rig.cell.retire_finished_window(&work).unwrap();
    let next=rig.work("after-cancel");rig.cell.begin_initial(next.clone()).unwrap();rig.cell.open_window(&next,Instant::now()).unwrap();
    assert!(activate_step(&rig.cell,&next).is_ok());
}

#[test]
fn np_expired_real_candidate_keeps_original_obligation_for_fresh_episode() {
    let Some(mut rig)=Rig::new(false, "native_publication::native_transition_tests::np_expired_real_candidate_keeps_original_obligation_for_fresh_episode") else { return; };rig.root.start("a");let work=rig.work("expiry");
    rig.cell.begin_initial(work.clone()).unwrap();rig.cell.open_window(&work,Instant::now()).unwrap();
    let original=work.clone();
    worker_step(&rig.cell,&work,move|c,r|{
        let candidate=r?;
        // Controlled clock boundary after actual native completion; this is not a
        // native timing/cost measurement. No candidate/native evidence is replaced.
        { c.core.lock().unwrap().pending.as_mut().unwrap().gate.budget.deadline=Some(Instant::now()-Duration::from_millis(1)); }
        assert!(matches!(c.prepare_publication(candidate,&SafeFixtureInput),Err(CoreError::Expired)),
            "late real native completion published after its absolute deadline");
        assert!(c.expire_if_due(&original)?);Ok(())
    }).unwrap();
    assert!(rig.cell.current().is_none());wait_ready(&rig.cell);
    let before={ rig.cell.core.lock().unwrap().pending.as_ref().unwrap().episode };
    rig.cell.begin_recovery_episode(&work,Instant::now()).unwrap();
    { let state=rig.cell.core.lock().unwrap();let pending=state.pending.as_ref().unwrap();
      assert!(pending.episode>before);assert!(pending.work.same(&work)); }
    assert!(activate_step(&rig.cell,&work).is_ok());
}

#[test]
fn np_real_candidate_compares_all_epochs_and_rejects_supersession() {
    let Some(mut rig)=Rig::new(false, "native_publication::native_transition_tests::np_real_candidate_compares_all_epochs_and_rejects_supersession") else { return; };rig.root.start("a");let work=rig.work("epochs");
    rig.cell.begin_initial(work.clone()).unwrap();rig.cell.open_window(&work,Instant::now()).unwrap();
    worker_step(&rig.cell,&work,|c,r|{
        let candidate=r?;
        for axis in 0..6 {
            let result={ let mut state=c.core.lock().unwrap();let saved=state.epochs;
                match axis { 0=>state.epochs.daemon_instance+=1,1=>state.epochs.surface_instance+=1,
                    2=>state.epochs.launch+=1,3=>state.epochs.conversation+=1,
                    4=>state.epochs.policy+=1,5=>state.epochs.binding+=1,_=>unreachable!() }
                let result=validate_candidate(&state,&candidate,c.io.root_pid,Instant::now());state.epochs=saved;result };
            assert!(matches!(result,Err(CoreError::Superseded)),"stale native candidate ignored epoch axis {axis}");
        }
        c.core.lock().unwrap().epochs.conversation+=1;
        assert!(matches!(c.prepare_publication(candidate,&SafeFixtureInput),Err(CoreError::Superseded)),
            "full publication path accepted a superseded conversation");
        Ok(())
    }).unwrap();assert!(rig.cell.current().is_none());
}

struct PrefixThenError { accepted:Vec<u8>,calls:usize }
impl Write for PrefixThenError {
    fn write(&mut self,bytes:&[u8])->io::Result<usize> {
        self.calls+=1;
        if self.calls==1 { let n=bytes.len().min(2);self.accepted.extend_from_slice(&bytes[..n]);Ok(n) }
        else { Err(io::Error::new(io::ErrorKind::BrokenPipe,"owned in-memory partial fixture")) }
    }
    fn flush(&mut self)->io::Result<()> { Ok(()) }
}
fn actual_partial_request()->Arc<Request> {
    use crate::agent_write::{self,AgentJob,Plan,PassThrough,WriterIo,Terminal};
    let request=Request::new(format!("partial-{}",NEXT.fetch_add(1,Ordering::Relaxed)),7,"native-fixture","digest".into());
    let job=AgentJob { attempt:None,request:request.clone(),plan:Plan::raw("abcdef").with_submit(0,0),proof:Arc::new(PassThrough),
        journal:None,resume:None,settle:None,guard:None,note_body:false,body_owner:None };
    let mut writer=PrefixThenError { accepted:vec![],calls:0 };let mut anchor=None;let mut first=||{};let mut body=|_:&str|{};
    let mut io=WriterIo { sleep:&|_|{},last_program_write:&mut anchor,on_first_byte:&mut first,on_body_written:&mut body };
    let result=agent_write::execute(&mut writer,&job,&mut io);
    assert!(matches!(result.terminal,Terminal::PartialUnknown { .. }),"fixture did not produce an actual partial writer result");
    assert_eq!(writer.accepted,b"ab");
    let terminal=agent_write::finish(job,&result);
    assert!(matches!(terminal,Terminal::PartialUnknown { .. }));
    assert!(request.is_settled(),"the old in-memory writer operation must be conclusively terminal");
    assert!(!request.native_progress().proven_zero());request
}
#[test]
fn np_actual_partial_progress_cannot_be_retokenized_to_replacement() {
    let Some(mut rig)=Rig::new(false, "native_publication::native_transition_tests::np_actual_partial_progress_cannot_be_retokenized_to_replacement") else { return; };let old=rig.initial();let request=actual_partial_request();
    rig.prove_old_dead(&old);rig.root.start("a");
    let work=Work { owner:DiscoveryOwner::ExistingWrite(request.clone()),obligation_id:"old-partial".into(),
        target:Target::Surface,expected:rig.cell.epochs() };
    rig.cell.begin_adoption(work.clone()).unwrap();rig.cell.open_window(&work,Instant::now()).unwrap();
    worker_step(&rig.cell,&work,|c,r|{
        assert!(matches!(c.prepare_publication(r?,&SafeFixtureInput),Err(CoreError::PossibleWrite)),
            "accepted old prefix obtained replacement publication authority");Ok(())
    }).unwrap();assert!(Arc::ptr_eq(&rig.cell.current().unwrap(),&old));
    assert_eq!(request.native_progress().accepted_total,2);assert!(old.sealed());
}

struct RejectOldSubmit {
    id:String,plan:crate::agent_write::Plan,old_epochs:Epochs,calls:Arc<AtomicUsize>,
}
impl DurableActivationValidator for RejectOldSubmit {
    fn validate_transition_and_retokenization(&self,r:&TransitionRecord)->Result<String,String> {
        self.calls.fetch_add(1,Ordering::SeqCst);
        assert_eq!(r.request_id.as_deref(),Some(self.id.as_str()));assert_eq!(r.old_epochs,self.old_epochs);
        assert!(self.plan.body().is_none() && self.plan.owns_submit());
        // Explicit fixture store veto based on retained plan/association input.
        // The CORE does not itself possess that plan; this is NOT proof that the
        // absent production store currently enforces the old-Return rule.
        Err("old standalone Return cannot acquire a replacement body association".into())
    }
}
#[test]
fn np_core_preserves_old_submit_identity_for_store_veto_without_exposing_token() {
    let Some(mut rig)=Rig::new(false, "native_publication::native_transition_tests::np_core_preserves_old_submit_identity_for_store_veto_without_exposing_token") else { return; };let old=rig.initial();rig.prove_old_dead(&old);rig.root.start("a");
    let request=Request::new(format!("old-return-{}",NEXT.fetch_add(1,Ordering::Relaxed)),7,"native-fixture","digest".into());
    let work=Work { owner:DiscoveryOwner::ExistingWrite(request.clone()),obligation_id:"old-submit".into(),
        target:Target::Surface,expected:rig.cell.epochs() };
    rig.cell.begin_adoption(work.clone()).unwrap();rig.cell.open_window(&work,Instant::now()).unwrap();
    let calls=Arc::new(AtomicUsize::new(0));let counted=calls.clone();let old_epochs=old.epochs();
    worker_step(&rig.cell,&work,move|c,r|{
        let PreparedOutcome::Prepared(stage)=c.prepare_publication(r?,&SafeFixtureInput)? else { panic!("same-policy adoption should stage a private token"); };
        let veto=RejectOldSubmit { id:request.id.clone(),plan:crate::agent_write::Plan::submit_only(0),old_epochs,calls:counted };
        assert!(matches!(stage.durable_capability(&veto),Err(CoreError::Authority(_))));
        assert!(stage.token.check_native(&*c.io.foreground).is_err(),"old Return could reach a staged executable token without store authority");
        Ok(())
    }).unwrap();assert_eq!(calls.load(Ordering::SeqCst),1);assert!(Arc::ptr_eq(&rig.cell.current().unwrap(),&old));
}


#[test]
fn np_negative_unknown_result_cannot_mint_a_death_fact() {
    let Some(mut rig)=Rig::new(false, "native_publication::native_transition_tests::np_negative_unknown_result_cannot_mint_a_death_fact") else { return; };let old=rig.initial();
    assert_eq!(old.binding.agent.recheck(),Ok(()),"owned native child must actually be alive");
    let failure=Failure::Unknown { stage:"negative-only-fixture",os_error:Some(libc::EPERM) };
    let result=finish_agent_death_probe(old.clone(),rig.cell.epochs(),Err(failure.clone()));
    assert!(matches!(result,Err(DeathError::Native(ref got)) if got==&failure),
        "an injected Unknown failure was promoted into a native-death fact");
    assert!(!old.sealed());assert!(rig.cell.core.lock().unwrap().dead.is_empty());
    assert_eq!(rig.cell.fixed_policy().unwrap().class,ProofClass::FullRequired);
}

#[test]
fn np_compiler_child_preserves_bytecode_seal_after_env_clear() {
    use std::os::unix::fs::PermissionsExt;
    let Some(python)=fixture_python("native_publication::native_transition_tests::np_compiler_child_preserves_bytecode_seal_after_env_clear") else { return; };
    let scratch=Scratch::new();let wrapper=scratch.path.join("seal-compiler");let observed=scratch.path.join("compiler-environment");
    let compiler=std::env::var_os("C8_NATIVE_FIXTURE_CC").unwrap_or_else(||"/usr/bin/cc".into());
    let quote=|value:&std::ffi::OsStr|format!("'{}'",value.to_str().expect("fixture compiler/path must be UTF-8").replace('\'',"'\\''"));
    // Observe the environment at OwnedRoot's ACTUAL compiler call site, then
    // repair the seal before the real compiler starts, so the removal mutant
    // remains a safe observation. No global environment or installed file changes.
    let script=format!("#!/bin/sh\nprintf '%s\\n' \"${{PYTHONDONTWRITEBYTECODE-unset}}\" > {}\nexport PYTHONDONTWRITEBYTECODE=1\nexec {} \"$@\"\n",
        quote(observed.as_os_str()),quote(&compiler));
    std::fs::write(&wrapper,script).unwrap();
    std::fs::set_permissions(&wrapper,std::fs::Permissions::from_mode(0o700)).unwrap();
    let root=OwnedRoot::spawn(&scratch,&python,Some(wrapper.as_os_str()));
    assert_eq!(std::fs::read(&observed).unwrap(),b"1\n",
        "actual fixture compiler call site lost PYTHONDONTWRITEBYTECODE after env_clear");
    drop(root); // ordinary owned-supervisor cleanup, before Scratch destruction
}
