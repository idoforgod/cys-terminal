//! cys (CYSJavis Terminal) — shared protocol types, socket path resolution, and key mapping.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub mod action_catalog;
pub mod factory_reset;
/// 앱 번들 완본 검증 + 원자 교체 계약(ATOMIC-1) — 2026-08-01 "손상되었기 때문에 열 수 없습니다" 사고의
/// 재발 차단. SEAL-1(아래 `ENV_PY_NO_BYTECODE`)이 **번들이 스스로 봉인을 깨는 것**을 막는다면,
/// 이쪽은 **교체가 반쪽으로 끝나는 것**을 막는다(같은 사고의 다른 절반).
pub mod app_bundle;
pub mod directive_compose;
pub mod edit_kinds;
/// 첫기동 관문 코퍼스 — **코드 임베드 정본**(U-12 · K-1). `agents.json` 값 수정은 사용자 소유
/// 파일 계층에 막혀 기존 설치 기계에 도달하지 못하므로, 관문 데이터의 진실원천은 코드다.
pub mod first_run_gates;
/// 주입·제출 관문 가드(U-14) + 폴더신뢰 Return 정책(U-15) — "지금 이 pane 에 키를 보내도 되는가".
/// ready 를 잘못 선언했을 때 실제로 좌석을 죽이는 것은 **제출 Return** 이므로, 판정(U-13)과 별개로
/// 전송 직전에 한 번 더 화면을 본다. 생애 창은 첫 각성 ack 이전으로 상한이 걸려 있다.
pub mod inject_guard;
pub mod license;
pub mod pack;
/// 프로필 인증 전제 판정기(U-17) — "이 프로필로 좌석을 만들면 로그인 관문 앞에 서는가".
/// 시드(U-19)는 로그인 화면을 **지우므로** 판정이 시드보다 먼저 있어야 한다. 판정은 순수함수
/// 하나(`profile_gate::classify`)가 소유하고, `auth_class` 8값 중 `unknown` 은 **통과가 아니다**.
pub mod profile_gate;
/// ready 술어 단일화(U-13) — `ready = 입력활성 증거 ∧ 관문 문면 부재`. 판정은 순수함수 하나가
/// 소유하고(`readiness::judge`), 부트 폴링과 `adapter_ready` 두 소비처가 같은 술어를 경유한다.
/// 종전엔 네 자리가 각자 ready 를 선언해 "마커 축만 고치면 아무것도 안 바뀌는" 상태였다.
pub mod readiness;
pub mod packsig;
pub mod overrides;
pub mod todo_decl;
pub mod todo_scan;
pub mod wire;
#[cfg(target_os = "macos")]
pub mod launchd;

pub const ENV_SOCKET: &str = "CYS_SOCKET";
pub const ENV_SURFACE_ID: &str = "CYS_SURFACE_ID";
pub const ENV_SURFACE_REF: &str = "CYS_SURFACE_REF";
pub const ENV_ROLE: &str = "CYS_ROLE";

/// ★SEAL-1 (2026-08-01 실사고 근본원인 · "손상되었기 때문에 열 수 없습니다"):
/// 동봉 Python 이 **자기 번들 안에** 바이트코드를 쓰면 코드서명 봉인이 스스로 깨진다.
///
/// 재현 사슬: 번들 python(`Contents/Resources/runtime/python/bin/python3`)이 stdlib 을 import
/// → CPython 이 `.../lib/python3.12/__pycache__/*.pyc` 를 **번들 안에** 새로 쓴다 →
/// `codesign --verify` 가 `a sealed resource is missing or invalid / file added:
/// .../__pycache__/_compression.cpython-312.pyc` 로 실패 → 브라우저로 받은 사본은 quarantine 이
/// 붙어 있어 **첫 실행 시 Gatekeeper 전체 재검증**에 걸려 실행이 차단된다(공증·staple 은 정상).
///
/// **왜 PYTHONDONTWRITEBYTECODE 이고 PYTHONPYCACHEPREFIX(번들 밖 캐시)가 아닌가** — 근거 3:
/// ① **활용할 동봉 .pyc 가 애초에 없다**: 동봉본은 python-build-standalone `install_only`
///    tar 를 그대로 전개한 것이라(`scripts/prep-mac-runtime.sh`) 선컴파일 `.pyc` 가 없다.
///    실측(2026-08-01 설치본): `.pyc` 46개 · `__pycache__` 디렉터리 **5개뿐**이고 그 집합이
///    정확히 기동 시 import 되는 것들(최상위·re·encodings·json·collections)이며 mtime 이
///    전부 그날 실행 시각이다 — 전 stdlib 선컴파일이라면 200개 넘는 디렉터리여야 한다.
///    즉 "기존 동봉 .pyc 활용"은 성립하지 않아 PYCACHEPREFIX 의 재사용 이점이 사라진다
///    (첫 실행에서 어차피 전부 새로 컴파일한다).
/// ② **기동 대가는 지불 가능한 크기**(동봉 python 실측 · 2026-08-01 · 격리 사본):
///    캐시 적중(PYCACHEPREFIX warm) 대비 매번 재컴파일(DONTWRITEBYTECODE)의 차이는
///    훅 hot path(`import sys,json` · 20회 평균) **13.6ms → 37.5ms (+23.9ms)**,
///    무거운 팩 스크립트급 import 16종(10회 평균) **23.8ms → 88.8ms (+65.0ms)** 였다.
///    자식 대부분이 수백 ms~수십 s 짜리 잡(스케줄·phoenix·게이트)이라 감내 가능하고,
///    **이 손해를 되찾는 올바른 수단은 PYCACHEPREFIX 가 아니라 ※의 빌드타임 선컴파일이다**
///    (그쪽은 캐시 이득과 봉인 안전을 동시에 준다 — 트레이드오프가 아니다).
/// ③ **실패모드가 안전하다**: PYCACHEPREFIX 는 값이 **빈 문자열이면 미설정과 동일**하게
///    취급돼 in-tree 쓰기(=봉인 파손)로 조용히 되돌아간다. 즉 "env 가 어긋나면 사고가
///    그대로 재발"한다. 게다가 캐시 디렉터리라는 새 쓰기 상태·경로 계산을 하나 더 떠안는다
///    (HOME 붕괴 환경까지 따라온다 — hooks/_lib.sh 가 backfill 하는 바로 그 문제).
///    DONTWRITEBYTECODE 는 상태 0 · 경로 0 이고 "안 쓴다"는 한 방향으로만 실패한다
///    (최악 = 매번 재컴파일, 봉인은 절대 안 깨짐). 봉인 파손의 대가가 **앱 실행 불가**라
///    비대칭이 압도적이다.
///
/// ※ 이 상수가 못 덮는 갭과 그 해소(SEAL-2 · 빌드 층위 · **구현 완료**): 이 env 는 **우리가
///   스폰하는** python 만 덮는다. 사용자·에이전트가 번들 python 을 절대경로로 **직접** 부르면
///   env 를 못 타고 `.pyc` 가 번들 안에 쓰인다. 그래서 서명 **전**에
///   `compileall --invalidation-mode unchecked-hash` 로 동봉 런타임 전체(stdlib·site-packages·
///   node-gyp/gyp)를 미리 컴파일해 `.pyc` 를 **서명 대상**으로 넣는다
///   (`scripts/precompile-bundled-python.sh` — prep-mac-runtime.sh·build-macos-signed.sh 가 호출).
///   그러면 ⓐ 어떤 호출 경로에서도 새로 쓸 것이 없고 ⓑ ②의 기동 손해도 사라진다.
///   ★두 층은 대체가 아니라 심층방어다 — 이 env 는 사용자가 팩을 밖으로 꺼내 쓰는 등
///   봉인 밖 경로까지 덮고, SEAL-2 는 env 를 못 타는 직접 호출을 덮는다.
///   기계 확인은 `scripts/verify-gatekeeper-user-path.sh` ⑥-A(SEAL-2)·⑥-B(이 env).
pub const ENV_PY_NO_BYTECODE: &str = "PYTHONDONTWRITEBYTECODE";
/// `ENV_PY_NO_BYTECODE` 의 켜짐 값. CPython 은 **비어 있지 않으면** 참으로 읽는다 —
/// 빈 문자열은 "끔"이므로 반드시 이 상수를 쓴다(빈 값 주입 = 봉인 파손 복귀).
pub const PY_NO_BYTECODE_ON: &str = "1";

/// ★UTF-8 모드 강제(감사 blocker #4 · W-B2): 한국어 Windows 는 콘솔·ANSI 코드페이지가 cp949 라
/// 파이썬 stdio 인코딩·`open()` 기본 인코딩이 cp949 로 잡히고, 부트 체인(`javis_orchestra.py
/// check` 등)이 '✓' 같은 비-cp949 문자를 찍는 순간 UnicodeEncodeError 로 즉사한다(UTF-8 팩
/// 파일을 ANSI 코드페이지로 읽다 UnicodeDecodeError 로 죽는 RC-6 도 같은 뿌리). PYTHONUTF8=1 은
/// PEP 540 UTF-8 모드를 강제해 stdio·open() 두 경로를 함께 봉인한다(mac 실측 재현:
/// `LC_ALL=ko_KR.eucKR python3 -c 'print("✓")'` 즉사 → PYTHONUTF8=1 로 치유 · unix 는 이미
/// UTF-8 로케일이라 무해 no-op). 팩 쪽도 R3(D-IMPL-3)로 orchestra·bootstrap 이 자기 stdio 를
/// `reconfigure(encoding="utf-8")` 보강했지만 그 방패는 **자기 stdio 한정**이다 — 팩 채널
/// 스큐(구팩+신앱)·비보강 스크립트(훅·기타 javis_*)·`open()` 기본 인코딩까지 덮는 이 env 층과는
/// 대체가 아니라 심층방어 관계다.
///
/// pane 스폰(state.rs · RC-6)은 같은 쌍을 literal("PYTHONUTF8","1")로 이미 주입한다 — 이 상수의
/// `spawn_env_pairs` 편입으로 pane 경로엔 **중복 주입**이 생기지만, 두 빌더(portable-pty
/// `CommandBuilder`·std `Command`) 모두 env 를 맵으로 들어 **나중 주입이 이기고**(vendor
/// cmdbuilder.rs `envs: BTreeMap` insert 실측), 양쪽 값이 동일한 "1" 이라 어느 쪽이 이겨도
/// 결과가 같다(무해). literal→상수 단일화는 state.rs 접촉이 필요해 후속 티켓 소관이다
/// (W-B2 는 state.rs 무접촉 제약).
pub const ENV_PY_UTF8: &str = "PYTHONUTF8";
/// `ENV_PY_UTF8` 의 켜짐 값. ★`PY_NO_BYTECODE_ON`("비어 있지 않으면 참")과 값 규약이 다르다 —
/// CPython 은 PYTHONUTF8 을 "1"/"0" 만 유효로 읽고 그 외 비어 있지 않은 값은
/// "Fatal Python error: preconfig_init_utf8_mode: invalid PYTHONUTF8 environment variable value"
/// 로 **파이썬 기동 자체가 죽는다**(실측). 빈 문자열은 미설정 취급(끔). 반드시 이 상수만 쓴다.
pub const PY_UTF8_ON: &str = "1";

/// 동봉 Python 을 **직접** 스폰하는 모든 지점의 단일 팩토리(SEAL-1 · 중복 구현 금지).
/// `std::process::Command::new(python)` 을 이걸로 바꾸기만 하면 `.pyc` 번들 오염이 봉쇄된다.
///
/// 셸을 거쳐 python 이 도는 경로(pane·스케줄 잡·훅)는 이 팩토리를 못 타므로
/// `spawn_env_pairs`(PATH·HOME 과 같은 자리)가 같은 쌍을 얹어 **상속**으로 덮는다 —
/// 두 층이 같은 상수를 소비하므로 규약이 산재하지 않는다.
/// tokio 자식처럼 `std::process::Command` 가 아닌 빌더는
/// `.env(cys::ENV_PY_NO_BYTECODE, cys::PY_NO_BYTECODE_ON)` 한 줄로 같은 상수를 소비한다.
pub fn python_command<S: AsRef<std::ffi::OsStr>>(program: S) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    cmd.env(ENV_PY_NO_BYTECODE, PY_NO_BYTECODE_ON);
    cmd
}

/// ★SEAL-1 층3 — **프로세스 자기 env 에 한 번 심어 모든 자손이 상속하게 한다**(드리프트 봉인).
///
/// 층1(`python_command`)·층2(`spawn_env_pairs`)는 **우리가 프로그램을 아는** 스폰만 덮는다.
/// 그런데 세 프로세스 패밀리에는 프로그램을 **모르는** 스폰이 남는다 — 대표 둘(실측):
///   · cysd `channels.rs::spawn_bridge` — 사용자 설정 `bridge_cmd` 를 `sh -c`/`cmd /C` 로 실행
///   · cysd `accounts.rs` cmd 어댑터 — 사용자 설정 `cmd` 를 **≥60초 주기로 반복** 실행
/// 여기에 python 이 들어오면 층1·층2 어느 쪽도 닿지 않는다. 특히 두 번째는 반복 스폰이라
/// "한 번만 새면 봉인이 깨진다"는 이 사고의 성질과 최악으로 맞물린다.
///
/// **왜 호출부마다 `.env(...)` 를 더하지 않는가**: 그건 스폰 지점 수에 비례하는 사본이고,
/// 새 스폰이 생기는 순간 또 빠진다(= 이 사고의 원래 기제). 프로세스 env 는 스폰 지점 수와
/// 무관하게 한 번에 닫힌다 — 층1·층2 를 대체하는 게 아니라 **그 아래를 받치는 바닥**이다
/// (층1·층2 는 명시적이라 회귀 핀을 걸 수 있고, 이 층은 빠짐없음을 보장한다).
///
/// ★계약: **스레드가 생기기 전**(각 바이너리 `main` 첫 줄)에만 부른다. `set_var` 는 프로세스
/// 전역 상태라 다른 스레드가 env 를 읽는 중이면 경합한다. 세 진입점이 이 계약을 지킨다 —
/// `cys`(sync main 선두) · `cysd`(sync `main` 이 tokio 런타임을 만들기 **전**) · `cys-app`
/// (tauri Builder 전). 그래서 cysd 의 `#[tokio::main]` 은 `async_main` 으로 내려가 있다.
///
/// 무조건 강제(setdefault 아님): 이 키의 실패 방향은 하나뿐이고(최악 = 매번 재컴파일),
/// 반대 방향의 대가는 **앱 실행 불가**다. 비대칭이 압도적이라 운영자 오프스위치를 두지 않는다.
pub fn seal_python_bytecode_in_process() {
    std::env::set_var(ENV_PY_NO_BYTECODE, PY_NO_BYTECODE_ON);
}

// ════════════════════════════════════════════════════════════════════════════
// ★U-7 · 자식 스폰 분리 규약 단일 정의처 (detached spawn convention)
// ════════════════════════════════════════════════════════════════════════════

/// 데몬 autostart 옵트아웃 env 키. **데몬이 낳은 자식 CLI** 는 반드시 이걸 켜야 한다 —
/// 켜지 않으면 그 CLI 가 소켓 연결 실패(데몬 종료 중·소켓 교체 중)를 만났을 때
/// `spawn_detached_daemon` 으로 **라이벌 데몬을 낳는다**(재귀 기동 · 폭주 ①).
pub const ENV_NO_AUTOSTART: &str = "CYS_NO_AUTOSTART";
/// `ENV_NO_AUTOSTART` 의 켜짐 값. 소비측(`cys.rs` autostart 게이트)이 `== "1"` 로 읽으므로
/// 이 상수만 쓴다("true"·"yes" 는 통하지 않는다 — 실측 계약).
pub const NO_AUTOSTART_ON: &str = "1";

/// Windows `CREATE_NEW_PROCESS_GROUP` — 자식을 **새 콘솔 프로세스 그룹**의 루트로 만든다.
/// 부모 콘솔의 Ctrl-C/Ctrl-Break 가 자식에게 전파되지 않는다.
///
/// ★효력 범위(과장 금지 · 2026-08-24 정정): 이 flag 가 막는 것은 **콘솔 컨트롤 이벤트 전파**
/// 하나다. Windows 의 **트리 종료**는 job object(부모가 쥔 job 이 끝나면 자식도 끝난다)와
/// `taskkill /T`(스냅샷의 부모-자식 링크를 따라 내려간다)로 일어나며 **둘 다 프로세스 그룹과
/// 무관**하다 — 즉 "훅이 트리를 죽여도 살아남는다"를 이 flag 로 얻지는 못한다.
/// unix `setsid` 의 **부분** 대응물이라고 읽어야 정확하다(setsid 는 세션·그룹을 실제로 갈라
/// 시그널 전파를 끊는다). 이 비대칭을 지운 채 두 플랫폼을 같은 문장으로 적으면, Windows 에서
/// 보호받고 있다고 **잘못 믿는** 상태가 된다.
#[cfg(windows)]
const WIN_CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
/// Windows `CREATE_NO_WINDOW` — 콘솔 자식에게 새 콘솔 창을 할당하지 않는다.
/// 콘솔 없는 프로세스(cysd·cys-app)가 콘솔 자식을 낳을 때 빈 검은 창이 뜨는 실사고(2026-07-10) 차단.
#[cfg(windows)]
const WIN_CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// **자식 생사 등급** — "이 자식은 부모가 죽을 때 같이 죽어야 하는가"를 스폰 지점마다 명시한다.
///
/// 이 분류가 필요한 이유(실측 · PROBE_RESULTS.md V-c): Claude Code 훅은 잘릴 때
/// **프로세스 그룹/트리 단위로 자식까지 죽인다**. 그리고 `setsid(1)` 은 이 맥에도,
/// Windows 동봉 PortableGit 에도 **없다**(PROBE_RESULTS_WINDOWS.md WIN-6). 즉 분리는
/// 외부 유틸리티가 아니라 **스폰 시점의 flag/pre_exec 로만** 얻을 수 있고,
/// 잘못 분리하면 정반대의 사고(고아 잔존 = 자원 누적)가 난다. 그래서 등급을 강제한다.
///
/// | 등급 | 부모 사망 시 자식 | 회수 책임 |
/// |---|---|---|
/// | `Survivor`     | **살아남아야 한다** | 자기 자신(데몬 flock·수명주기) |
/// | `GroupScoped`  | 시그널로는 안 죽는다 | **원장(ledger)의 명시적 그룹 kill** |
/// | `ConsoleScoped`| unix=시그널 면역 / **Windows=콘솔 Ctrl-C 로 함께 죽는다** | 부모의 `kill_group`(정상 종료 경로) + Windows 콘솔 전파(비정상 종료 경로) |
/// | `Attached`     | **같이 죽어야 한다** | 부모(`wait`/`kill_on_drop`/트리 kill) |
///
/// ★U-7 이동의 정직한 손익(2026-08-24 정정 — 종전 주석의 "값·행동 무변경, 정의처만 이동"은
/// 이 지점에서 **거짓**이었다):
/// · `spawn_detached_daemon`(cys.rs · `Survivor`)의 Windows arm 은 종전 `CREATE_NO_WINDOW`
///   **단독**이었고, 지금은 `CREATE_NEW_PROCESS_GROUP` 이 **새로 걸린다**. 무해한 방향이지만
///   무변경은 아니다.
/// · `state.rs::HideConsole::hide_console`(`Attached` 별칭)은 값·행동 모두 무변경이다
///   (`CREATE_NO_WINDOW` 단독 · unix 무동작).
/// · 그리고 위 `WIN_CREATE_NEW_PROCESS_GROUP` 주석대로, Windows 에서 그 flag 는 **트리 종료
///   (job object · `taskkill /T`)로부터 보호해 주지 않는다.** 등급표의 "부모 사망 시 자식"
///   열은 **unix 기준**으로 읽고, Windows 는 콘솔 Ctrl-C 축만 이 flag 가 다룬다고 읽어라.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildLifetime {
    /// 부모(CLI)가 죽어도 살아남아야 하는 자식. **유일 사례 = `cys` → `cysd` 기동.**
    /// 새 세션(unix)·새 프로세스 그룹(Windows) + 콘솔 없음 + **부모 파이프 무점유**.
    Survivor,
    /// 시그널 그룹에서는 떼되 **원장이 pgid 로 회수**하는 장수 자식(채널 브리지).
    /// 부모 콘솔이 없다는 전제라 Windows 콘솔 창도 숨긴다.
    GroupScoped,
    /// 사용자 콘솔을 **그대로 물려받는** 전경 자식(`cys run -- <명령>`).
    /// unix 는 `setsid` + 부모의 SIGINT/SIGTERM/SIGHUP 핸들러가 `killpg` 로 회수한다.
    /// ★Windows 는 **일부러 분리하지 않는다**: 대응 콘솔 컨트롤 핸들러가 없어서,
    ///   분리하면 CLI 가 Ctrl-C 로 죽는 순간 `kill_group`(= `child.wait()` 이후에만 도달)이
    ///   영영 실행되지 않아 자식이 **영구 고아**로 남는다. 회수 수단을 얻기 전의 분리는
    ///   개선이 아니라 자원 누적이다. `CREATE_NO_WINDOW` 도 금지 — 사용자가 실행을 요청한
    ///   명령의 출력을 가리게 된다.
    ConsoleScoped,
    /// 부모가 `wait`/`kill_on_drop` 으로 붙들고 있는 유계 자식(auto-restore·브리지 재기동·
    /// launch-agent 호출·스케줄 명령). **분리 금지** — 트리 kill 로 함께 죽는 것이 정상 동작이다.
    /// Windows 콘솔 창만 숨긴다. `cysd/state.rs::HideConsole::hide_console` 은 **이 등급의
    /// 별칭**이다(같은 flag · 같은 행동 — 정의처가 둘이던 것을 하나로 접었다).
    Attached,
}

impl ChildLifetime {
    /// 이 등급의 Windows `creation_flags` **완성값** — flag word 의 **유일한 정의처**다.
    ///
    /// `creation_flags` 는 누적이 아니라 **덮어쓰기**라, 한 빌더 체인에서 두 번 얹으면 뒤엣것이
    /// 앞엣것을 통째로 지운다(channels.rs 원주석이 지적한 함정). 등급 하나가 flag word 전체를
    /// 결정하므로 **"등급끼리 섞여 반쪽 flag 가 되는" 조합은 표현할 수 없다.**
    ///
    /// ★남은 위험은 정직하게 적는다: `hide_console()`(= `Attached` 별칭)을 다른 등급 뒤에
    /// 이어 붙이는 **병용**은 타입으로는 여전히 가능하고, 그 순간 앞 등급의 flag 가 조용히
    /// 사라진다(mac/Linux 무증상 · CI 전부 초록). 그 조합은 타입이 아니라 소스 핀
    /// (`spawn_policy_tests::lifetime_grade_and_hide_console_are_never_mixed`)이 막는다 —
    /// "구조적으로 불가능"이 아니라 "기계가 막는다"가 사실이다.
    #[cfg(windows)]
    const fn win_creation_flags(self) -> u32 {
        match self {
            ChildLifetime::Survivor => WIN_CREATE_NEW_PROCESS_GROUP | WIN_CREATE_NO_WINDOW,
            ChildLifetime::GroupScoped => WIN_CREATE_NEW_PROCESS_GROUP | WIN_CREATE_NO_WINDOW,
            ChildLifetime::ConsoleScoped => 0,
            ChildLifetime::Attached => WIN_CREATE_NO_WINDOW,
        }
    }

    /// unix 에서 새 세션(`setsid`)으로 떼어낼 등급인가.
    const fn unix_setsid(self) -> bool {
        matches!(
            self,
            ChildLifetime::Survivor | ChildLifetime::GroupScoped | ChildLifetime::ConsoleScoped
        )
    }

    /// 부모의 stdin/stdout/stderr 를 **물려받지 않게** 할 등급인가.
    /// 분리 자식이 부모 파이프를 쥐고 있으면, 부모가 먼저 끝나도 그 파이프를 읽는 쪽
    /// (`$(cys …)`·CI 러너)이 자식이 죽을 때까지 EOF 를 못 받아 **부모 종료가 지연**된다.
    /// `Survivor` 만 해당 — 나머지는 로그/캡처가 유일한 진단 채널이라 호출부가 정한다
    /// (침묵 고장 클래스를 새로 만들지 않는다).
    const fn detach_stdio(self) -> bool {
        matches!(self, ChildLifetime::Survivor)
    }
}

/// 자식 스폰 규약을 **한 곳에서** 얹는 확장 트레이트. `std::process::Command` 와
/// `tokio::process::Command` 양쪽에 같은 의미로 구현된다(빌더 종류가 규약을 갈라놓지 않는다).
///
/// ★규약: 프로덕션의 모든 자식 스폰은 `pre_exec`/`creation_flags` 를 **직접 부르지 않고**
/// 이 트레이트를 경유한다. 소스 핀 테스트가 우회 스폰을 적색으로 잡는다.
pub trait SpawnPolicy {
    /// 생사 등급에 맞는 세션/그룹 분리 + Windows 콘솔 정책 + (해당 등급이면) 파이프 무점유.
    fn spawn_policy(&mut self, class: ChildLifetime) -> &mut Self;
    /// 이 자식(및 그 자손)이 라이벌 데몬을 autostart 하지 못하게 봉인한다.
    fn no_autostart(&mut self) -> &mut Self;
}

macro_rules! impl_spawn_policy {
    ($t:ty) => {
        impl SpawnPolicy for $t {
            fn spawn_policy(&mut self, class: ChildLifetime) -> &mut Self {
                if class.detach_stdio() {
                    self.stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null());
                }
                #[cfg(unix)]
                {
                    if class.unix_setsid() {
                        // SAFETY: 클로저는 fork 후 exec 전에 돈다 — async-signal-safe 호출만 한다
                        // (`setsid` 는 async-signal-safe). 할당·락·I/O 금지 계약을 지킨다.
                        #[allow(unused_unsafe)]
                        unsafe {
                            self.pre_exec(|| {
                                libc::setsid();
                                Ok(())
                            });
                        }
                    }
                }
                #[cfg(windows)]
                {
                    let f = class.win_creation_flags();
                    if f != 0 {
                        self.creation_flags(f);
                    }
                }
                self
            }

            fn no_autostart(&mut self) -> &mut Self {
                self.env(ENV_NO_AUTOSTART, NO_AUTOSTART_ON)
            }
        }
    };
}

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;
#[cfg(windows)]
use std::os::windows::process::CommandExt as _;

impl_spawn_policy!(std::process::Command);
impl_spawn_policy!(tokio::process::Command);

/// 이행기 호환: CYS_* 우선 → 구 JAVIS_* → 구 AITERM_* 순 폴백.
pub fn env_compat(primary: &str) -> Option<String> {
    let javis = primary.replacen("CYS_", "JAVIS_", 1);
    let aiterm = primary.replacen("CYS_", "AITERM_", 1);
    [primary, javis.as_str(), aiterm.as_str()]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

/// Wire protocol: one JSON object per line (NDJSON), request/response with id echo.
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

pub fn ok_response(id: &Value, result: Value) -> Value {
    serde_json::json!({"id": id, "ok": true, "result": result})
}

pub fn err_response(id: &Value, code: &str, message: &str) -> Value {
    serde_json::json!({"id": id, "ok": false, "error": {"code": code, "message": message}})
}

/// Default socket path: ~/.local/state/cys/cys.sock (unix),
/// \\.\pipe\cys (windows). Overridable via CYS_SOCKET (legacy JAVIS_/AITERM_ honored).
pub fn socket_path() -> PathBuf {
    if let Some(p) = env_compat(ENV_SOCKET) {
        return PathBuf::from(p);
    }
    #[cfg(windows)]
    {
        PathBuf::from(r"\\.\pipe\cys")
    }
    #[cfg(not(windows))]
    {
        let base = dirs::state_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let dir = if base.ends_with(".local/state") || base.to_string_lossy().contains("state") {
            base.join("cys")
        } else {
            base.join(".local/state/cys")
        };
        dir.join("cys.sock")
    }
}

/// Windows named pipe busy-retry 정책 — CLI(cys)·GUI(cys-app) 클라이언트 공용 **단일 진실**
/// (이원 정의는 정책 변경 시 샷건 서저리). ERROR_PIPE_BUSY(os error 231, "모든 파이프
/// 인스턴스가 사용 중")는 데몬 다운이 아니라 listening 인스턴스 순간 소진(정상 혼잡)이다 —
/// 서버(cysd 리스너 풀)는 accept 직후 인스턴스를 재생성하므로 잠깐 기다리면 열린다.
/// Microsoft 파이프 클라이언트 계약상 busy 는 대기·재시도가 필수(WaitNamedPipe 관례)이며,
/// 재시도 없는 1회 open 은 멀티 노드 동시 RPC 에서 상시 실패한다(2026-07-10 Windows 실사고).
/// 그 외 오류(파이프 부재 ERROR_FILE_NOT_FOUND = 데몬 다운 등)는 **즉시 반환**이 계약이다
/// (autostart 판단은 호출부 몫 — 다운을 대기로 오처리하면 autostart 가 데드라인만큼 늦는다).
///
/// 재시도 리듬: 고정 간격 폴링은 fan-out(앱 기동 daemon_status+pane별 attach+event forwarder
/// 동시 연결)에서 전 클라이언트가 같은 위상으로 충돌한다 — busy 시 ① `wait_named_pipe`
/// (WaitNamedPipeW 커널 대기, `PIPE_BUSY_WAIT_SLICE` 슬라이스)로 인스턴스 가용을 기다리고
/// ② 타임아웃이면 `next_busy_delay`(decorrelated jitter, `PIPE_BUSY_RETRY_INTERVAL` 하한 ·
/// `PIPE_BUSY_BACKOFF_CAP` 상한)로 재개 시점을 분산한다. 총 데드라인(5s)은 유지 — 상향은
/// wedge 감지를 늦춘다. 전 OS에서 컴파일되는 pub 상수·순수 함수라 비-Windows 테스트가
/// 정책 불변을 박제할 수 있다.
pub const PIPE_BUSY_ERROR: i32 = 231;
pub const PIPE_BUSY_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);
pub const PIPE_BUSY_RETRY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);
/// busy 1회당 WaitNamedPipeW 커널 대기 슬라이스 — 데드라인(5s)보다 충분히 짧아 슬라이스
/// 사이마다 open 재판정(비-busy 오류 즉시 반환 계약)이 돈다.
pub const PIPE_BUSY_WAIT_SLICE: std::time::Duration = std::time::Duration::from_millis(250);
/// jitter 백오프 상한 — 상한 없는 지수 증가는 데드라인 내 재시도 횟수를 고갈시킨다.
pub const PIPE_BUSY_BACKOFF_CAP: std::time::Duration = std::time::Duration::from_millis(200);

/// busy 백오프의 다음 대기 시간(순수 · decorrelated jitter): `[RETRY_INTERVAL, min(prev*3, CAP)]`
/// 구간을 rand01(∈[0,1])로 샘플한다. 이전 대기와 탈상관돼 fan-out 동시 재시도의 위상 충돌을
/// 깬다. 어떤 입력(0·NaN·범위 밖 rand01 포함)에도 결과는 [RETRY_INTERVAL, BACKOFF_CAP] 안 —
/// 0 반환은 busy spin, 과대 반환은 데드라인 낭비라 양쪽 다 클램프가 계약이다.
pub fn next_busy_delay(prev: std::time::Duration, rand01: f64) -> std::time::Duration {
    let r = if rand01.is_finite() {
        rand01.clamp(0.0, 1.0)
    } else {
        0.0 // 오염 난수는 하한(즉시 재시도 리듬)으로 — 대기 자체는 항상 성립.
    };
    let hi = prev
        .saturating_mul(3)
        .clamp(PIPE_BUSY_RETRY_INTERVAL, PIPE_BUSY_BACKOFF_CAP);
    let span = hi - PIPE_BUSY_RETRY_INTERVAL; // hi ≥ 하한이 클램프로 보장됨.
    PIPE_BUSY_RETRY_INTERVAL + std::time::Duration::from_secs_f64(span.as_secs_f64() * r)
}

/// jitter 전용 저비용 난수(∈[0,1)) — 암호학적 품질 불요(토큰 생성 금지, 위상 분산 전용).
/// 시계 나노초 + pid 해시라 프로세스·호출 간 위상이 갈린다.
pub fn rand01_cheap() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0) as u64;
    let mixed = nanos
        .wrapping_mul(6364136223846793005)
        .wrapping_add(std::process::id() as u64);
    (mixed % 1_000_000) as f64 / 1_000_000.0
}

/// WaitNamedPipeW 래퍼 — busy 파이프의 인스턴스 가용을 커널에서 기다린다(폴링 아님).
/// true = 인스턴스 가용(즉시 open 재시도 가치), false = 타임아웃·오류(파이프 소멸 포함).
/// 최종 판정자는 언제나 다음 open 이다 — 여기 false 를 다운 판정으로 쓰지 않는다.
#[cfg(windows)]
pub fn wait_named_pipe(path: &Path, timeout: std::time::Duration) -> bool {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // 0 은 NMPWAIT_USE_DEFAULT_WAIT(서버 기본값) 의미라 최소 1ms 로 못박는다.
    let ms = timeout.as_millis().clamp(1, u32::MAX as u128) as u32;
    unsafe { windows_sys::Win32::System::Pipes::WaitNamedPipeW(wide.as_ptr(), ms) != 0 }
}

/// 타이핑 가드 거부의 **에러 코드·메시지 단일 소스**(T-0147-6).
///
/// 생산자 = `cysd` handlers(`surface.send_text`·`surface.send_key`), 소비자 = `cys` 클라이언트의
/// 주입 경로. 클라이언트는 와이어에서 `error.message` 를 받으므로(`rpc_roundtrip`) 이 문구가 곧 판정
/// 근거다 — **양쪽이 리터럴을 따로 들고 있으면** 문구를 다듬는 순간 소비자의 `--queued` 폴백이
/// 조용히 죽는다(RC1 문자열 계약 드리프트). 그래서 상수로 못박고 양쪽이 이것만 쓴다.
pub const ERR_TYPING_GUARD: &str = "typing_guard";
pub const MSG_TYPING_GUARD: &str = "human is typing in this pane; retry later or use --queued";

/// `cys boot` 가 **무스폰 skip**(다른 boot 가 락 보유)을 낼 때의 종료코드 — EX_TEMPFAIL(75).
///
/// ★(T-0147-7 W4 · G11·하드 제약 6-⑧) bare exit 계약: **0 = Fatal 없음(Degrade-only 포함) ·
/// 1 = Fatal 실패(mandatory 역할의 failed·missing) · 75 = busy(무스폰)**.
/// 구계약에서 busy 는 0(성공)이었고, 그래서 소비부가 '팀을 세웠다'로 읽어 1회성 CEO 티켓을
/// **무스폰 상태로 소각**했다(G11). 반대로 1(Fatal)로 접으면 정상적인 훅↔GUI 중첩 부트마다
/// '팀 기동 실패' 위경보가 난다(P3-B16 부류) — 그래서 **별도 값**이다.
/// 75 는 sysexits.h EX_TEMPFAIL 로, clap 사용오류(2)·EX_USAGE(64)·부트 게이트 exit 공간(3~11)과
/// 겹치지 않는다.
///
/// **왜 lib 상수인가**: 생산자(`cys.rs::run_boot`)와 소비자(GUI `cys-app`)가 다른 크레이트이고,
/// 파이썬 소비자(`javis_bootstrap.CYS_BOOT_EXIT_BUSY`)까지 셋이다. 값을 세 곳에 적으면 그것이
/// RC1(다중 구현) 의 새 인스턴스다 — 검체 `H-EXIT-2` 가 3자 파리티를 기계 대조한다.
pub const EXIT_BOOT_BUSY: i32 = 75;

// ─────────────────────────────────────────────────────────────────────────────
// ★(U-10) 좌석 **제4 등급** `gate_pending` 축 — 스키마 자리 + 롤백 킬스위치(단일 지점)
// ─────────────────────────────────────────────────────────────────────────────
//
// **무엇인가**: "프로세스는 살아 있으나 첫기동 관문(테마·로그인방식·OAuth·폴더신뢰·면책·
// 새기능안내)에 갇혀 입력을 받을 수 없는 좌석". 종전 3등급(AwakeConfirmed / AlivePresumed /
// Absent + 판정불가 Unknown)에는 이 사실을 담을 자리가 없어서, 관문에 갇힌 좌석이
// `agent_alive == true` 하나로 `AlivePresumed` 가 되고 `run_boot` 이 그것을
// **"이미 가동 중 — 건너뜀"(already_alive)** 으로 접었다. 그 상태로 U-11(readiness 실패 시
// close 대신 보류)을 착지시키면 **관문에 갇힌 팀 전체가 "정상 가동 중" 으로 집계**된다.
//
// **이 단위(U-10)의 범위 — additive 만**: 자리(스키마 키·등급 변형·미러 상수)를 만들고,
// 그 등급을 '충족' 에서 제외해야 하는 소비처를 **전수 동시 정합**시킨다.
// **생산자는 이 단위에 없다** — 값은 항상 `null`(미도입) 이고, 생산은 U-11/U-13 이 한다.
// 그래서 구 데몬 ↔ 신 CLI, 신 데몬 ↔ 구 CLI 어느 혼재에서도 거동이 오늘과 같다.
//
// **`null` 규약(래치 '부재≠부정' 과 동형)**: 키 부재·`null` 은 "관문에 갇히지 **않았다**" 가
// 아니라 **'이 축에 대해 말할 것이 없음'**(구 데몬 = 축 미도입)이다. 소비자는 그 경우
// 이 항을 **통째로 생략**하고 종전 판정으로 흘러야 한다. 반대로 읽으면(= null 을 부정으로)
// 새 사망 경로가 열린다.
//
// **형(型) 규약**: 값은 `null` 또는 **object**(`{"gate": …, "since": …}`) 둘 중 하나다.
// object 가 아닌 non-null(스큐·손상)은 **무신호로 접는다**(fail-open → 종전 동작). 여기서
// 'gated' 로 접으면 판정불가가 미충족을 만들어 부트 재시도 라이브락(A1 클래스)이 된다 —
// 이 축의 fail-open 방향은 "오늘보다 나빠지지 않는다" 로 고정한다.

/// `gate_pending` 축 **롤백 킬스위치**의 env 이름(단일 지점 · 기본 = 축 노출).
///
/// `CYS_GATE_PENDING=0` 이면 데몬은 이 키를 항상 `null` 로 직렬화하고 CLI·팩은 이 축을
/// 읽지 않는다 → 전 소비자가 **종전 3등급 판정으로 즉시 복귀**한다. 생산자가 붙는
/// U-11/U-13 이후에도 이 스위치 하나로 축 전체를 무장해제할 수 있어야 한다(그것이 이
/// 스위치의 존재 이유다 — 관문 문면 오탐 1건이 팀 전체를 미충족으로 만드는 사고의 탈출구).
/// python 미러: `javis_boot_node.gate_pending_axis_enabled()`(같은 이름·같은 극성 ·
/// **같은 3스위치 접기** — 마스터 `CYS_BOOT_GATES=0` · 강등 `CYS_GATE_PENDING_CLOSE=1` ·
/// 축 `CYS_GATE_PENDING=0` 어느 하나로도 꺼진다. 두 언어의 동형성은 헬스 검체가 기계 대조).
pub const ENV_GATE_PENDING: &str = "CYS_GATE_PENDING";

/// `gate_pending` 축의 **wire 키 이름 정본** — `surface.list` · `org.status` · `topology.json`
/// · python 미러가 **같은 키·같은 의미**로 쓴다(동형성 핀이 기계 대조).
pub const GATE_PENDING_KEY: &str = "gate_pending";

/// 축이 **실제로 노출되는가** — 데몬 직렬화 지점(`state.rs::gate_pending_wire`)과 전 Rust
/// 소비처의 단일 술어(**부작용 있음** — env 3회 판독). 규약은 순수 코어
/// [`gate_pending_axis_effective_from`].
///
/// ★(BLOCK-3 잔여분 수리 · 2026-08-24) 종전 이 함수는 `CYS_GATE_PENDING` **하나만** 읽었다.
/// 그래서 마스터 스위치(`CYS_BOOT_GATES=0`)를 눌러도 **데몬에는 닿지 않아** 이미 실린
/// gate_pending 표식이 TTL(30분)까지 계속 직렬화됐다 — CLI 는 종전 판정으로 돌아갔는데
/// 데몬은 여전히 좌석을 보류로 내보내는 **반쪽 롤백**이고, 그 조합이 정확히 BLOCK-4 가
/// 없앤 "관측은 하는데 귀결은 없는" 상태다. 사고 순간에 쥐는 손잡이 하나가 데몬까지 닿아야
/// '되돌렸다'가 참이 된다.
pub fn gate_pending_axis_enabled() -> bool {
    gate_pending_axis_effective_from(
        std::env::var(ENV_BOOT_GATES).ok().as_deref(),
        std::env::var(ENV_GATE_PENDING_CLOSE).ok().as_deref(),
        std::env::var(ENV_GATE_PENDING).ok().as_deref(),
    )
}

/// 축 노출 판정의 **순수 코어** — 세 스위치의 접기.
///
/// 규약: **축 노출 ⟺ 보류 장치가 켜져 있다.** 마스터(`CYS_BOOT_GATES=0`)·강등
/// (`CYS_GATE_PENDING_CLOSE=1`)·축(`CYS_GATE_PENDING=0`) 어느 하나를 눌러도 **동시에** 꺼진다.
/// ★불변식은 여기서 다시 쓰지 않고 [`gate_axes_from`] 의 산출값을 뒤집어 쓴다 — 조건을 두 벌로
/// 적는 순간 한 벌만 고쳐지고, 그 갈라짐이 이 저장소가 반복해서 맞은 사본 드리프트다.
/// (판정 축 노브들은 이 축과 무관하므로 `None` 을 먹인다 — `gate_pending_close` 는 그것들에
/// 의존하지 않는다. U-17 이 축을 하나 더 얹었어도 같다: 축 노브는 자기 축만 끈다.)
pub fn gate_pending_axis_effective_from(
    master_env: Option<&str>,
    close_env: Option<&str>,
    axis_env: Option<&str>,
) -> bool {
    !gate_axes_from(master_env, None, None, None, close_env, axis_env, None).gate_pending_close
}

/// **축 노브 하나만** 보는 순수 코어 — `"0"` 만 끈다.
///
/// ★이것은 축 술어가 아니다(그것은 [`gate_pending_axis_effective_from`]). 여기는 노브 한 개의
/// 판독 규약이고, 마스터·강등 스위치와의 합류는 [`gate_axes_from`] 이 소유한다.
///
/// ★극성이 왜 '기본 켜짐 · "0" 만 끔' 인가: 이 단위에서 축이 켜져 있어도 **생산자가 없어
/// 값은 항상 null** 이므로 기본값이 곧 종전 동작이다. 반대로 '기본 꺼짐 + "1" 로 켬' 으로
/// 두면 U-11 착지 시 켜는 것을 잊는 순간 보류 좌석이 다시 `already_alive` 로 접힌다 —
/// 잊어서 위험해지는 방향이 아니라 **잊어도 오늘과 같은** 방향으로 배치한다.
/// `"false"`·`"off"`·빈 값 같은 느슨한 falsy 는 끄지 않는다(형제 게이트와 같은 엄격 비교 —
/// 오타로 안전장치가 조용히 꺼지는 사고 방지).
pub fn gate_pending_axis_enabled_from(env_val: Option<&str>) -> bool {
    env_val != Some("0")
}

/// wire 값(`null` | object) → **관문 보류인가**. 전 Rust 소비처의 단일 술어(**부작용 있음** —
/// 킬스위치 env 판독). 판정 규약 자체는 순수 코어 `gate_pending_from_wire_with` 에 있다.
pub fn gate_pending_from_wire(v: &Value) -> bool {
    gate_pending_from_wire_with(gate_pending_axis_enabled(), v)
}

/// 관문 보류 술어의 **순수 코어** — 킬스위치 상태와 wire 값만 받아 판정한다.
///
/// 계약: `enabled ∧ object` → 보류(true) · 그 밖(off · null · 키 부재 · 비 object) →
/// **무신호**(false = 종전 판정으로 흐름).
/// ★비 object non-null(스큐·손상)을 'gated' 로 접지 않는 이유: 판정불가를 미충족으로 만들면
/// 부트가 영원히 재시도하는 라이브락(A1 클래스)이 된다. 이 축의 fail-open 방향은
/// **"오늘보다 나빠지지 않는다"** 로 고정한다.
pub fn gate_pending_from_wire_with(enabled: bool, v: &Value) -> bool {
    enabled && v.is_object()
}

// ─────────────────────────────────────────────────────────────────────────────
// ★(U-11) readiness 실패 귀결의 재정의 — 만료 규약 · 롤백 킬스위치 · 종료코드
// ─────────────────────────────────────────────────────────────────────────────
//
// U-10 이 좌석 **자리**(제4 등급)를 만들었고, 여기서 그 자리에 값이 실린다. 값이 실리는 순간
// 세 가지가 **동시에** 정해져야 한다 — 그렇지 않으면 새 사망 경로·라이브락이 열린다:
//   ① **만료**: 보류 표식은 사람이 관문을 통과시켜도 저절로 사라지지 않는다. 지우는 계약이
//      없으면 좌석이 영원히 미충족 → `check` 영구 실패 → 부트 라이브락(A1 클래스)이다.
//      U-10 이 이 규약을 **명시적으로 U-11 에 인계**했다(state.rs `gate_pending` 필드 doc).
//   ② **롤백**: 보류는 '닫지 않는다' 는 결정이다. 그 결정이 틀린 기계에서 좌석이 고이면
//      env 하나로 **종전(무조건 close)** 으로 돌아갈 수 있어야 한다.
//   ③ **종료코드**: 보류는 성공(0)도 실패(1)도 아니다. 둘 중 하나로 접으면 소비부가
//      '팀을 세웠다' 또는 '기동이 깨졌다' 로 오독한다(G11 계열의 의미 융합).

/// 관문 보류 표식의 **수명 상한**(초). 이 나이를 넘긴 표식은 데몬의 단일 직렬화 지점
/// (`Surface::gate_pending_wire`)에서 **null 로 접힌다** = 전 소비자가 종전 판정으로 복귀한다.
///
/// ★왜 만료가 **필수**인가: 표식을 지우는 유일한 능동 경로는 "그 좌석에서 readiness 가 다시
/// 확정될 때"인데, 보류 좌석은 `run_boot` 이 **관측만 하고 건너뛰므로**(U-10) 재확정 기회가
/// 오지 않는다. 사람이 화면에서 관문을 통과시켜도 표식만 남아 좌석이 영구 미충족이 된다 —
/// 그 자체가 부트 라이브락이다. 만료는 그 라이브락의 **상한**이다.
///
/// ★왜 이 방향이 안전한가: 만료의 귀결은 "축이 없던 것처럼 = **정확히 오늘의 동작**" 이다
/// (관문 좌석이 다시 `AlivePresumed` → `already_alive`). 즉 만료는 새 위험을 만들지 않고
/// 관측을 잃을 뿐이다. 반대 방향(무기한 보류)만이 새 고장을 만든다.
///
/// ★값 30분: 사람이 pane 을 보고 관문 한 번을 통과시키는 현실적 창. 짧으면 관측이 일찍
/// 사라지고(=오늘로 복귀), 길면 라이브락 상한이 길어진다. **새 leaf 다** — 기존 예산 leaf
/// (`BUDGET_*` · `javis_budget.LEAF_FLOORS`)는 어느 것도 건드리지 않는다.
pub const GATE_PENDING_TTL_SECS: f64 = 1800.0;

/// 표식이 아직 유효한가 — **순수 코어**(시계·env 비의존, 진리표 테스트 대상).
///
/// 계약: `since` 가 미래거나(시계 되돌림·스큐) NaN 이면 **유효**로 본다. 판정불가를 만료로
/// 접으면 관측이 조용히 사라지고, 그 방향의 실수는 "보류를 already_alive 로 되돌리는" 것 —
/// 이 단위가 없애려는 바로 그 상태다. 반대로 유효로 두면 최악이 TTL 만큼의 지연이다.
pub fn gate_pending_fresh(since: f64, now: f64, ttl: f64) -> bool {
    let age = now - since;
    !(age.is_finite() && age > ttl)
}

/// 보류 귀결의 **롤백 킬스위치** env 이름(단일 지점).
///
/// `CYS_GATE_PENDING_CLOSE=1` 이면 `GatePending` 판정을 **`LaunchFailed` 로 강등**한다 →
/// 좌석은 종전처럼 즉시 close 되고, 표식도 실리지 않는다(= 이 단위 착지 이전과 동일).
/// 강등은 CLI 의 **판정 반환 지점 한 곳**에서만 일어난다(호출부 3곳이 각자 분기하면 그 순간
/// 롤백이 3지점이 되고, 한 곳이라도 빠지면 롤백이 거짓말이 된다).
pub const ENV_GATE_PENDING_CLOSE: &str = "CYS_GATE_PENDING_CLOSE";

/// 롤백 판정의 **단일 진입점**(부작용 있음 — env 2회). 규약은 순수 코어
/// `gate_pending_close_override_from`.
///
/// ★두 스위치를 **여기서 합류**시키는 이유(반쪽 롤백 차단): U-10 의 축 스위치
/// (`CYS_GATE_PENDING=0`)는 "데몬이 이 축을 직렬화하지 않는다" 는 뜻이다. 그 상태에서 CLI 만
/// 보류를 계속하면 **pane 은 남는데 좌석은 여전히 `already_alive` 로 읽히는** 반쪽 상태가 된다
/// (관측 없는 보류 = 허위 READY). 롤백은 '반쯤 되돌아가는' 것이 가장 위험하다 — 어느 스위치를
/// 눌러도 기능 **전체**가 꺼지게 한다.
pub fn gate_pending_close_override() -> bool {
    gate_axes().gate_pending_close
}

/// 롤백 합류의 **순수 코어** — 두 스위치 중 하나라도 켜지면 종전(즉시 close)으로 돌아간다.
///
/// ★(BLOCK-4 · 2026-08-24) 이 함수는 **축 스위치 둘만** 본다. 마스터 스위치와의 합류,
/// 그리고 "엄격 판정 + 즉시 close 는 성립 불가" 불변식은 [`gate_axes_from`] 이 소유한다.
pub fn gate_pending_close_override_from(close_env: Option<&str>, axis_env: Option<&str>) -> bool {
    gate_pending_close_from(close_env) || !gate_pending_axis_enabled_from(axis_env)
}

// ─────────────────────────────────────────────────────────────────────────────
// ★(BLOCK-3 · BLOCK-4 · 2026-08-24) 부트 관문 판정 축의 **단일 접기 지점**
// ─────────────────────────────────────────────────────────────────────────────
//
// ## BLOCK-3 — 문서화된 탈출구가 작동하지 않았다
//
// U-13 은 "문제 생기면 `CYS_READINESS_V1=1` 로 되돌려라" 라고 보고했지만 **단독으로는
// 무효**였다(리뷰어 4칸 진리표 전수): V1=1 로 ready 가 나도 부트 사전 가드와 `inject_text`
// 가드가 다시 잡아 여전히 rc 78 · 미주입이었고, 종전 동작 복귀의 유일한 조합은
// `CYS_READINESS_V1=1` **AND** `CYS_INJECT_GATE_GUARD=0` 이었다. 축마다 노브가 따로 있고
// 조합해야 복귀하는 구조는 **사고 순간에 사람이 쓸 수 없다**. 그래서 상위
// **마스터 스위치**([`ENV_BOOT_GATES`]`=0`)를 둔다 — 하나를 끄면 이 캠페인이 추가한 판정 축이
// **전부 동시에** 종전 동작으로 복귀한다. 개별 노브는 축 단위 조정용으로 남는다.
//
// ## BLOCK-4 — 롤백 스위치 하나가 기저보다 파괴적인 상태를 만들었다
//
// `CYS_GATE_PENDING=0` **단독**이 "**엄격 판정 + 즉시 close**" 를 만들었다:
// `gate_pending_close_override_from(None, Some("0")) == true`(합류 OR) 인데
// `readiness::legacy_v1_from(None) == false`(엄격 유지) → 관문 화면이 `GateHeld` 로 잡혀
// 영원히 ready 가 아니고 → readiness 타임아웃 → `boot_verdict_effective(_, true)` 가
// **LaunchFailed 로 강등** → 호출부가 `surface.close`. 문서화된 스위치 하나로 전 pane 사망이다.
//
// **불변식**: `보류 장치가 꺼졌다(close 강등) ⇒ 이 캠페인이 추가한 판정 축은 전부 종전(느슨)`.
// 엄격화와 보류는 한 몸이다 — 보류라는 안전한 귀결이 없는 상태에서 엄격해질 권리는 없다.
// 이 불변식은 [`gate_axes_from`] **한 곳**에서만 성립하고, 진리표
// `strict_judgment_and_immediate_close_is_unreachable_in_every_env_combination` 이
// 관련 env 전 조합으로 전수 집행한다.

/// ★마스터 롤백 스위치의 env 이름. `0` → 이 캠페인이 추가한 판정 축 **전부** 종전 복귀.
///
/// 개별 노브(`CYS_READINESS_V1`·`CYS_INJECT_GATE_GUARD`·`CYS_TRUST_RETURN_V1`·
/// `CYS_GATE_PENDING_CLOSE`·`CYS_GATE_PENDING`·`CYS_PROFILE_GATE_OBSERVE_ONLY`)는 축 단위
/// 조정용으로 남지만, **사고 순간에 사람이 쥐는 손잡이는 이것 하나**다.
///
/// ★새 판정 축은 **태어날 때 여기에 접는다**(U-17 이 그 규율의 첫 적용). 축마다 노브를 따로
/// 두고 나중에 합치는 순서는 BLOCK-3 에서 이미 실패했다 — 사고 순간에는 조합을 못 만든다.
pub const ENV_BOOT_GATES: &str = "CYS_BOOT_GATES";

/// 마스터 스위치 판정의 **순수 코어** — `"0"` 만 끈다.
///
/// 형제 게이트(`gate_pending_axis_enabled_from`)와 같은 엄격 비교다. 기본(미설정)이 신동작인
/// 이유도 같다: 잊어서 위험해지는 방향이 아니라 **잊으면 새 안전장치가 켜져 있는** 방향.
pub fn boot_gates_master_off_from(env_val: Option<&str>) -> bool {
    env_val == Some("0")
}

/// 이 캠페인이 추가한 판정 축의 **최종 유효값**. 축이 여럿이라 bool 을 따로 들고 다니면
/// 어느 호출부가 하나를 빠뜨렸는지 알 수 없다 — 한 타입으로 묶어 **동시에** 결정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateAxes {
    /// readiness 관문 AND 항이 꺼졌는가(= U-13 이전 판정).
    pub readiness_legacy: bool,
    /// 주입·제출 가드가 **관측 전용**으로 강등됐는가(= U-14 이전 전송).
    pub inject_guard_off: bool,
    /// 폴더신뢰 Return 정책이 종전(재전송 상한 · 하드코딩 needle)인가(= U-15 이전).
    pub trust_legacy: bool,
    /// 보류 판정을 즉시 close 로 강등하는가(= U-11 이전 귀결).
    pub gate_pending_close: bool,
    /// ★(U-17) 프로필 인증 전제 판정기가 **관측·보고 전용**으로 강등됐는가(= 차단 안 함).
    ///
    /// 이 축이 여기 있는 이유: 판정 축을 새로 만들 때마다 노브가 따로 생기면 사고 순간에
    /// 사람이 조합을 기억해야 한다(BLOCK-3 이 그 값을 치렀다). 새 축은 태어날 때 마스터에
    /// 접는다 — `CYS_BOOT_GATES=0` 하나로 이 판정기도 함께 종전(차단 없음)으로 돌아간다.
    pub profile_gate_observe_only: bool,
}

/// ★판정 축의 **순수 접기 함수**(진리표 대상). env 는 하나도 읽지 않는다.
///
/// 규약:
///   ① 마스터(`CYS_BOOT_GATES=0`)는 축을 **전부** 종전으로 되돌린다(BLOCK-3).
///   ② 보류 장치가 꺼지면(`gate_pending_close`) 판정 축도 **전부** 종전으로 되돌아간다
///      — 엄격 판정 + 즉시 close 조합을 구조적으로 불가능하게 한다(BLOCK-4).
///   ③ 개별 노브는 **자기 축만** 끈다(교차 오염 금지 — 신뢰 노브가 킬체인 가드를 열면 안 된다).
pub fn gate_axes_from(
    master_env: Option<&str>,
    readiness_v1_env: Option<&str>,
    guard_env: Option<&str>,
    trust_v1_env: Option<&str>,
    close_env: Option<&str>,
    axis_env: Option<&str>,
    profile_gate_env: Option<&str>,
) -> GateAxes {
    // 보류 장치가 꺼졌는가 = 마스터 ∨ 강등 스위치 ∨ 축 스위치.
    let holding_off = boot_gates_master_off_from(master_env)
        || gate_pending_close_override_from(close_env, axis_env);
    GateAxes {
        readiness_legacy: holding_off || readiness::legacy_v1_from(readiness_v1_env),
        inject_guard_off: holding_off || inject_guard::guard_off_from(guard_env),
        trust_legacy: holding_off || inject_guard::trust_v1_from(trust_v1_env),
        gate_pending_close: holding_off,
        profile_gate_observe_only: holding_off
            || profile_gate::observe_only_from(profile_gate_env),
    }
}

/// 위의 **단일 진입점**(부작용 있음 — env 6회 판독). 규약은 순수 코어 [`gate_axes_from`].
pub fn gate_axes() -> GateAxes {
    gate_axes_from(
        std::env::var(ENV_BOOT_GATES).ok().as_deref(),
        std::env::var(readiness::ENV_V1).ok().as_deref(),
        std::env::var(inject_guard::ENV_GUARD_OFF).ok().as_deref(),
        std::env::var(inject_guard::ENV_TRUST_V1).ok().as_deref(),
        std::env::var(ENV_GATE_PENDING_CLOSE).ok().as_deref(),
        std::env::var(ENV_GATE_PENDING).ok().as_deref(),
        std::env::var(profile_gate::ENV_OBSERVE_ONLY).ok().as_deref(),
    )
}

/// "이 캠페인이 추가한 판정 축을 **전부** 종전으로 되돌려야 하는가" — 마스터·보류 축의 합류.
///
/// 각 축 모듈(`readiness`·`inject_guard`)이 자기 env 를 1지점에서 읽되 이 합류값과 OR 하도록
/// 나눠 둔 이유: 롤백 스위치의 **축별 1지점 규약**(검체가 판독 지점 수를 센다)을 지키면서도
/// 불변식은 이 파일 하나가 소유하기 위해서다.
pub fn gate_axes_forced_legacy() -> bool {
    boot_gates_master_off_from(std::env::var(ENV_BOOT_GATES).ok().as_deref())
        || gate_pending_close_override_from(
            std::env::var(ENV_GATE_PENDING_CLOSE).ok().as_deref(),
            std::env::var(ENV_GATE_PENDING).ok().as_deref(),
        )
}

/// 롤백 판정의 **순수 코어** — `"1"` 만 켠다.
///
/// 형제 게이트(`gate_pending_axis_enabled_from` 의 `"0"` 만 끔)와 **같은 엄격 비교**다:
/// `"true"`·`"yes"`·빈 값 같은 느슨한 truthy 를 받아주면 오타 하나로 안전장치가 조용히
/// 뒤집힌다. 기본(미설정) = 신동작(보류) 이고, 되돌림은 명시 `1` 하나뿐이다.
pub fn gate_pending_close_from(env_val: Option<&str>) -> bool {
    env_val == Some("1")
}

/// `cys launch-agent` 의 **관문 보류 전용 종료코드**(sysexits `EX_CONFIG`).
///
/// 종전 계약은 성공 0 / 그 밖 전부 1 이었다. 보류는 그 1비트에 담기지 않는다 —
/// **surface 는 만들어졌고 에이전트 프로세스는 살아 있으며 stdout 에 ref 도 나갔지만, 사람이
/// 관문을 한 번 통과시키기 전까지는 쓸 수 없다.** 0 을 주면 소비부가 '노드를 세웠다'로 읽어
/// 디렉티브·티켓을 태우고(그 주입이 관문 창의 Return 이 된다 = 실측 킬 스텝), 1 을 주면
/// '기동이 깨졌다'로 읽어 **살아 있는 좌석을 회수·파괴**하려 든다. 둘 다 이 단위가 막으려는
/// 사고다.
///
/// ★왜 2~11 이 아니라 78 인가: 게이트 exit 공간 2~11 은 `javis_bootstrap` 헤더 표가 소유한
/// 만원 공간이다(H-DOC-4 가 유령·결손 양방향 대조). 형제 선례 `EXIT_BOOT_BUSY = 75`
/// (EX_TEMPFAIL)와 같은 sysexits 공간을 쓴다. 78 = `EX_CONFIG`("설정이 완결되지 않았다")가
/// 첫기동 관문의 성격에 정확히 대응한다.
///
/// ★소비 3자 파리티(H-EXIT-11 이 기계 대조): 이 상수(정본) ↔ `javis_bootstrap.py`
/// `CYS_LAUNCH_EXIT_GATE_PENDING` ↔ GUI `src-tauri` 분기. 구 바이너리는 78 을 내지 않으므로
/// (0/1 만) 이 분기는 신 바이너리에서만 발동한다 — 스큐 안전.
pub const EXIT_GATE_PENDING: i32 = 78;

/// 동봉 runtime PATH 선두 주입(RC-5 · 공용 — cysd PTY 자식·GUI 직스폰이 공유, 중복 구현 금지).
/// `exe_dir`(바이너리 폴더) + Windows 자기완결 설치의 `<install>\runtime\{python, git\cmd, git\usr\bin}`
/// 중 **실재하는** 디렉토리를 `current_path` 앞에 (중복 제거) 얹은 새 PATH를 반환. 얹을 게 없으면
/// None(기존 동작 무변경). current_path를 인자로 받아 순수 함수(테스트 가능·env 비의존).
/// 근거: GUI(Finder/Explorer) 기동 프로세스는 PATH가 빈곤해 bash/python3 lookup 실패(RC-5 ＋부서 무반응).
/// 동봉 runtime의 bin 디렉토리들(디스크에 실재하는 것만) — OS별 레이아웃. 반환 순서 = PATH 선두 우선순위.
/// Windows(RC-5): exe 형제 `runtime/`(python·git/cmd·git/usr/bin).
/// macOS(RC-18·T6b): 앱 번들은 실행바이너리=Contents/MacOS·리소스(runtime/)=Contents/Resources →
///   `exe_dir/../Resources/runtime`(python/bin·git/bin·uv·node/bin). 개발 빌드(exe 형제 runtime/)도 폴백.
/// runtime_prefixed_path(PATH 선두주입)와 state.rs `-lc` 재선두주입(D8 — 로그인셸 path_helper 강등 회피)이 공유.
pub fn runtime_bin_dirs(exe_dir: &Path) -> Vec<PathBuf> {
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(unused_mut))]
    let mut dirs: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        let rt = exe_dir.join("runtime");
        for d in [
            rt.join("python"),
            rt.join("git").join("cmd"),
            rt.join("git").join("usr").join("bin"),
            rt.join("node"), // ★T6b 파리티: node.exe·npm·npx (mac runtime/node/bin 대칭 — win은 top-level)
        ] {
            if d.is_dir() {
                dirs.push(d);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        // 앱 번들 리소스 경로 우선, 개발 빌드(형제 runtime/) 폴백. 첫 유효 루트만 사용.
        let roots = [
            exe_dir.parent().map(|p| p.join("Resources").join("runtime")),
            Some(exe_dir.join("runtime")),
        ];
        for rt in roots.into_iter().flatten() {
            if !rt.is_dir() {
                continue;
            }
            for d in [
                rt.join("python").join("bin"),
                rt.join("git").join("bin"),
                rt.join("uv"),
                rt.join("node").join("bin"),
            ] {
                if d.is_dir() {
                    dirs.push(d);
                }
            }
            break;
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = exe_dir;
    }
    dirs
}

/// pane/자식 프로세스에 물릴 PATH 를 계산 — 결과가 현행과 다르면 Some(주입값), 무변경이면 None.
/// **이중 의미론**: unix = exe_dir + 번들 runtime bins 선두 주입 + `~/.local/bin` 말미 append(나머지 보존) —
/// 로그인 셸(-l) 프로파일이 PATH 를 복원하나 claude native 설치기가 rc 를 수정하지 않음이 실측 확인되어
/// `~/.local/bin` 은 belt-and-braces 로 직접 append 한다(compose_unix_pane_path 참조).
/// Windows = 레지스트리에서 신선 PATH 를 재합성한다. 데몬은 자기 기동 시점 PATH 스냅샷을 자식에 물리는데,
/// 실행 중엔 WM_SETTINGCHANGE 를 못 받아 레지스트리 PATH 변경(claude 사후 설치 등)이 반영되지 않는다 →
/// spawn 마다 HKLM/HKCU 에서 신선 PATH 를 읽어 재합성 = 새 PowerShell 창과 등가(compose_pane_path 참조).
pub fn runtime_prefixed_path(exe_dir: &Path, current_path: &str) -> Option<String> {
    let sep = if cfg!(windows) { ';' } else { ':' };
    let mut prefixes: Vec<String> = vec![exe_dir.to_string_lossy().into_owned()];
    for d in runtime_bin_dirs(exe_dir) {
        prefixes.push(d.to_string_lossy().into_owned());
    }
    #[cfg(windows)]
    {
        // ★Windows stale PATH: 데몬은 WM_SETTINGCHANGE 를 못 받아 레지스트리 PATH 변경(claude 사후 설치)이
        // 프로세스 PATH 스냅샷에 반영 안 됨 → spawn 마다 레지스트리에서 신선 PATH 재합성 = 새 PowerShell 등가.
        // 어떤 실패도 현행보다 나빠지지 않게(fail-open): fresh=None 이면 아래 compose 가 프로세스 PATH 폴백.
        let fresh = windows_registry_path();
        let home = home_dir();
        let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
        let user_bins: Vec<String> = windows_user_bin_dirs(&home, appdata.as_deref())
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let composed = compose_pane_path(&prefixes, fresh.as_deref(), &user_bins, current_path, sep);
        // 기존 계약 유지: 결과가 현행과 같으면 None(무주입). 레지스트리 재합성이 보통 값을 바꾼다.
        if composed == current_path {
            return None;
        }
        return Some(composed);
    }
    #[cfg(not(windows))]
    {
        // unix: 로그인 셸(-l) 프로파일이 PATH 를 복원하지만, claude native 설치기(2.1.207)가 rc 를
        // 수정하지 않음이 실측 확인되어 ~/.local/bin 은 belt-and-braces 로 무조건 append 한다.
        let _ = sep; // unix 합성은 compose_unix_pane_path 가 ':' 고정 사용.
        compose_unix_pane_path(&prefixes, &home_dir(), current_path)
    }
}

/// Claude Code 가 **Windows 에서 훅·셸 실행에 쓸 bash** 를 지정하는 벤더 env 키(U-20).
///
/// 왜 이 키가 부트 안전 문제인가: 시스템 Git 이 없는 Windows 기계에서 이 값이 비어 있으면
/// Claude Code 는 훅을 **한 번도 실행하지 못한다**. 훅이 죽으면 각성·부트 체인이 통째로
/// 무음이 되고, 증상은 "아무 일도 안 일어난다" 뿐이라 원인 추적이 극히 어렵다.
/// 우리는 PortableGit 을 동봉하므로 그 기계에도 bash 는 **실재한다** — 벤더에게 그 자리를
/// 알려주기만 하면 된다.
pub const ENV_CLAUDE_CODE_GIT_BASH_PATH: &str = "CLAUDE_CODE_GIT_BASH_PATH";

/// 동봉 PortableGit **런처** bash 의 설치 루트 상대 경로 세그먼트(`runtime\git\bin\bash.exe`).
///
/// ★`git\bin` 이지 `git\usr\bin` 이 **아니다**(B-12) — 근거 셋:
///   ① 릴리스 레인이 이미 이 경로를 하드 단언한다. `.github/workflows/windows-build.yml`:
///      "Claude Code 는 훅(hook) 실행에 PortableGit 의 **런처** bash(`runtime\git\bin\bash.exe`)
///      를 쓴다 — MSYS 실 바이너리인 `usr\bin\bash.exe` 와 **다른 파일**" + 부재 시 FATAL 종료.
///   ② `git\bin\bash.exe` 는 MSYS2 런타임과 Git 환경을 세워 주는 런처다. `usr\bin\bash.exe` 를
///      직접 부르면 그 준비가 없는 raw MSYS bash 가 뜬다.
///   ③ 하필 이 구분이 위험한 이유: 데몬 셸 탐지의 기존 후보 순서
///      (`schedule.rs windows_bash_candidates` = [`runtime_bin_dirs`] + `git\bin` **덧댐**)는
///      [`runtime_bin_dirs`] 의 Windows 순서(python → git\cmd → **git\usr\bin** → node) 때문에
///      `usr\bin\bash.exe` 를 **먼저** 집는다. 그 순서는 스케줄 잡의 살아있는 계약이라
///      건드리지 않고(순서 교정은 별도 티켓), **이 키만 `git\bin` 으로 단일 고정**한다.
pub const BUNDLED_GIT_BASH_REL: [&str; 4] = ["runtime", "git", "bin", "bash.exe"];

/// 동봉 bash 절대경로 해소(**부작용: 파일 stat 1회** — OS 는 인자로 받는다).
///
/// `os` 가 `"windows"` 가 아니면 **무조건 None**: mac/linux 에는 이 키를 얹지 않는다.
/// 형제 선례 [`d5_gate_for_os`] 와 같은 꼴로 `cfg!` 대신 OS 문자열을 인자로 받는 이유는,
/// 회귀 핀이 **다른 플랫폼 CI 에서도 Windows 분기를 실제로 밟게** 하기 위해서다
/// (`std::env::consts::OS` 는 타깃별 컴파일 상수라 실효는 `cfg(windows)` 와 같다).
///
/// 실재하지 않으면 None = **fail-open**. 없는 경로를 벤더에게 통보하는 쪽이 미통보보다 나쁘다
/// (벤더가 그 경로로 spawn 을 시도하다 실패하면 진단이 더 어려워진다).
pub fn bundled_git_bash_path_for(exe_dir: &Path, os: &str) -> Option<PathBuf> {
    if os != "windows" {
        return None;
    }
    let mut p = exe_dir.to_path_buf();
    for seg in BUNDLED_GIT_BASH_REL {
        p.push(seg);
    }
    if p.is_file() {
        Some(p)
    } else {
        None
    }
}

/// `CLAUDE_CODE_GIT_BASH_PATH` 기본값 주입의 **순수 코어**(env 도 디스크도 보지 않는다).
///
/// 불가침 계약 넷([`inject_claude_alt_screen_default_for`] 의 3계약과 동형 + 롤백 1):
///   ① `user_env_set` — 사용자가 이미 값을 가졌으면 **절대 덮지 않는다**(프로세스 env 관측은
///      호출부의 몫이다. 그 값이 우리 것보다 나쁘더라도 사용자 소유물이다).
///   ② 이미 쌓인 쌍에 같은 키가 있으면 손대지 않는다(later-wins 뒤집기·중복 금지).
///   ③ `resolved == None` 이면 아무것도 얹지 않는다 — **fail-open**. 이 축의 최악값은
///      "종전과 동일"이지 "더 나빠짐"이 아니다.
///   ④ `master_off`(= `CYS_BOOT_GATES=0`)면 주입하지 않는다 = 종전 동작 완전 복귀.
///      새 판정 축은 태어날 때 마스터에 접는다 — 사고 순간에 사람은 노브를 조합하지 못한다
///      (BLOCK-3 이 그 값을 이미 치렀다). 이 축 전용 노브는 **만들지 않는다**.
pub fn inject_claude_code_git_bash_path_for(
    env_pairs: &mut Vec<(String, String)>,
    resolved: Option<&Path>,
    user_env_set: bool,
    master_off: bool,
) {
    if master_off || user_env_set {
        return;
    }
    let Some(p) = resolved else { return };
    if env_pairs
        .iter()
        .any(|(k, _)| k == ENV_CLAUDE_CODE_GIT_BASH_PATH)
    {
        return;
    }
    env_pairs.push((
        ENV_CLAUDE_CODE_GIT_BASH_PATH.to_string(),
        p.to_string_lossy().into_owned(),
    ));
}

/// 자식 프로세스(pane 스폰·스케줄 발화·GUI 직스폰)에 얹을 env 주입 쌍(OS 무관 컴파일).
///
/// ★'순수' 표기 정정(U-20 · 2026-08-24): 이 자리엔 오래 "(순수 — 회귀 핀·OS 무관 컴파일)"
/// 이라고 적혀 있었지만 **사실이 아니었다**. ①은 [`runtime_prefixed_path`] 를 거쳐 디렉터리
/// `is_dir()` stat 을 돌리고, Windows 에서는 레지스트리(HKLM/HKCU `Environment\Path`)까지
/// 읽는다. ⑤가 여기에 파일 stat 1회와 프로세스 env 판독 2회를 더 얹으므로 지금 정정한다 —
/// 이 함수는 **인자로 받는 것(exe_dir·PATH·HOME·USERPROFILE)에 대해서만 결정론**이고,
/// 나머지는 디스크·레지스트리·프로세스 env 를 본다. 회귀 핀이 이 함수를 직접 호출해도 되는
/// 이유는 '순수해서'가 아니라 **핀이 실재하지 않는 exe_dir 를 넘겨 디스크 축을 고정**하기
/// 때문이다(`/nonexistent-exe-dir-for-pin`).
///
/// ① PATH: 동봉 runtime 선두 주입. 데몬은 GUI(Explorer/Finder) 기동이라 PATH 가 빈곤해
///    python3·printf·tail 을 못 찾는다 — office-bridge/auto-restore 와 **동일 SOT**
///    (`runtime_prefixed_path`)를 재사용한다(중복 구현 금지). 무변경이면 쌍 없음.
/// ② HOME: Windows 의 비로그인 셸(`bash -c`)·순수 cmd 스폰은 HOME 이 없어 페이로드의
///    `${CYS_PACK_DIR:-$HOME/.cys/pack}` 이 `/.cys/pack` 으로 붕괴한다 → **미설정일 때만**
///    USERPROFILE 로 채운다. HOME 이 이미 있으면 무접촉(unix 는 항상 이 경로 → 무변경).
/// ③ PYTHONDONTWRITEBYTECODE(★SEAL-1): ①이 PATH 선두에 **번들 python** 을 꽂는 바로 그 자리다 —
///    그래서 이 자식들이 부르는 `python3` 는 곧 앱 번들 안의 인터프리터이고, 그게 `__pycache__`
///    를 번들에 쓰면 코드서명 봉인이 깨져 다음 실행이 Gatekeeper 에 차단된다(2026-08-01 실사고).
///    pane·스케줄 잡·훅은 셸을 거치므로 `python_command` 팩토리를 못 탄다 → **상속**으로 덮는
///    유일한 지점이 여기다. 항상 얹는다(무조건 쌍 1개 추가 — PATH 무변경이어도 이건 나간다).
///    근거·대안 비교는 `ENV_PY_NO_BYTECODE` 상수 주석.
/// ④ PYTHONUTF8(감사 blocker #4 · W-B2): 한국어 Windows(cp949)에서 이 규약으로 스폰되는 python 의
///    stdio·open() 기본 인코딩이 ANSI 코드페이지로 붕괴해 부트 체인(`javis_orchestra.py check`)이
///    UnicodeEncodeError 로 즉사한다. pane 스폰은 state.rs literal(RC-6)로 이미 막았지만 스케줄
///    발화·GUI 직스폰은 무보호였다 → 공용 규약에 편입해 세 소비 경로가 같이 덮인다. ③과 같이
///    **항상** 얹는다(무조건 쌍 — PATH 무변경이어도 나간다). state.rs literal 과의 중복 주입이
///    무해한 근거(같은 값 "1"·later-wins)는 `ENV_PY_UTF8` 상수 주석.
/// ⑤ CLAUDE_CODE_GIT_BASH_PATH(U-20 · **Windows 전용**): 시스템 Git 이 없는 Windows 기계에서
///    Claude Code 는 훅 실행용 bash 를 못 찾아 훅을 **한 번도 실행하지 못한다**(각성·부트
///    체인이 통째로 무음). 동봉 PortableGit 의 런처 bash 가 실재할 때만, 그리고 **사용자가
///    이미 값을 갖고 있지 않을 때만** 그 절대경로를 얹는다. ③④와 달리 **조건부 쌍**이다 —
///    조건 미충족 시 아무것도 얹지 않아 종전과 완전히 동일하다(fail-open).
///    ★Windows 실기 검증은 이 커밋 범위 밖이다(mac 개발기에서 실기 재현 불가). 여기서
///    증명된 것은 ⓐ경로 선택 규약 ⓑ불가침 계약 ⓒ타 플랫폼 미주입 셋뿐이고, 실제 훅이 뜨는가는
///    `feat/**` 브랜치 windows-health 잡과 실기 재현의 몫이다 — 과장하지 않는다.
///
/// ★T-0147-7 W1a(A17): 이 함수는 `schedule.rs` 의 private fn 이었다. **pane 스폰 경로
///   (state.rs)에는 같은 backfill 이 없어** Windows 에서 pane 속 훅·python 이 `$HOME` 붕괴로
///   발화 무산됐다(감사 A17: "state.rs pane 스폰에 backfill 부재 — schedule.rs:882 에는 있음").
///   사본을 늘리는 대신 lib 공용 함수로 승격해 **두 스폰 경로가 같은 규약을 소비**하게 한다
///   (RC4 '규약 산재' 소멸 · 검체 H-WIN-8). GUI 직스폰(src-tauri `inject_runtime_path`)의
///   편입은 W-B2 에서 완료 — 세 스폰 경로(pane·스케줄 발화·GUI 직스폰)가 이 규약 하나를
///   소비한다(회귀 핀: src-tauri `gui_spawn_env_matches_pane_spawn_env`).
pub fn spawn_env_pairs(
    exe_dir: &Path,
    current_path: &str,
    home: Option<&str>,
    userprofile: Option<&str>,
) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Some(newp) = runtime_prefixed_path(exe_dir, current_path) {
        env.push(("PATH".to_string(), newp));
    }
    if home.map(|h| h.is_empty()).unwrap_or(true) {
        if let Some(up) = userprofile.filter(|u| !u.is_empty()) {
            env.push(("HOME".to_string(), up.to_string()));
        }
    }
    // ③ SEAL-1: 셸 경유 python(pane·훅·스케줄 잡)이 번들에 .pyc 를 못 쓰게 상속으로 봉인.
    env.push((
        ENV_PY_NO_BYTECODE.to_string(),
        PY_NO_BYTECODE_ON.to_string(),
    ));
    // ④ 감사 blocker #4(W-B2): cp949 콘솔 상속 python 의 UnicodeEncodeError 즉사 봉인 — UTF-8
    //    모드 강제(③처럼 무조건 쌍). 값 규약("1"만 유효·그 외 기동 fatal)은 ENV_PY_UTF8 주석.
    env.push((ENV_PY_UTF8.to_string(), PY_UTF8_ON.to_string()));
    // ⑤ U-20: Windows 동봉 bash 를 벤더 훅 실행기에 알린다. 판정은 순수 코어 두 개가 소유하고
    //    여기서는 **관측만** 한다 — OS·디스크(해소기) · 사용자 env(1회 판독) · 마스터 롤백
    //    스위치(1회 판독). 조건 미충족이면 쌍이 하나도 늘지 않는다(종전 동작 그대로).
    inject_claude_code_git_bash_path_for(
        &mut env,
        bundled_git_bash_path_for(exe_dir, std::env::consts::OS).as_deref(),
        std::env::var_os(ENV_CLAUDE_CODE_GIT_BASH_PATH).is_some(),
        boot_gates_master_off_from(std::env::var(ENV_BOOT_GATES).ok().as_deref()),
    );
    env
}

/// `spawn_env_pairs` 를 **현재 프로세스 env** 로 계산한다(호출부 3중 복붙 제거).
/// 반환 쌍을 각 스폰 빌더(`tokio::process::Command`·portable-pty `CommandBuilder`)에 얹는다.
pub fn spawn_env_pairs_from_process(exe_dir: &Path) -> Vec<(String, String)> {
    let path = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").ok();
    let userprofile = std::env::var("USERPROFILE").ok();
    spawn_env_pairs(exe_dir, &path, home.as_deref(), userprofile.as_deref())
}

/// unix pane PATH 합성(순수·테스트 가능 · **unix 전용 의미론** · OS 무관 컴파일). 순서:
/// `[prefixes: exe_dir + 번들 runtime bins] 중 신규분 선두` ; `current_path 전체(순서 그대로 보존)` ;
/// `~/.local/bin(claude native 설치 위치) 신규분 말미 append`. claude 설치기가 ~/.zshrc 를 수정하지
/// 않아(수동 안내만 출력·실측) 프로파일 복원만으론 claude 가 영원히 미발견 → windows_user_bin_dirs 의
/// belt-and-braces 를 unix 로 대칭 확장, is_dir 게이트 없이 무조건 append(셸은 없는 PATH 항목을 무시).
/// append 인 이유는 MAJ#1 과 동일: 발견이 목적 — 기존 항목의 precedence 를 강등하지 않는다.
/// dedup 은 unix 관례대로 case-sensitive 단순 비교. 결과가 현행과 같으면 None(무변경 계약 유지).
pub fn compose_unix_pane_path(prefixes: &[String], home: &Path, current_path: &str) -> Option<String> {
    let sep = ':';
    let add: Vec<&str> = prefixes
        .iter()
        .map(String::as_str)
        .filter(|p| !current_path.split(sep).any(|e| e == *p))
        .collect();
    let local_bin = home.join(".local").join("bin").to_string_lossy().into_owned();
    let append_local = !current_path.split(sep).any(|e| e == local_bin)
        && !add.iter().any(|p| *p == local_bin);
    if add.is_empty() && !append_local {
        return None;
    }
    let mut composed = String::new();
    if !add.is_empty() {
        composed.push_str(&add.join(&sep.to_string()));
        composed.push(sep);
    }
    composed.push_str(current_path);
    if append_local {
        composed.push(sep);
        composed.push_str(&local_bin);
    }
    Some(composed)
}

/// Windows `%VAR%` 전개(ExpandEnvironmentStrings 의미론 근사) — OS 무관 컴파일(순수·테스트 가능).
/// 짝을 이룬 `%..%` 만 치환하고, **미지 변수·미종결 %·빈 이름(%%)은 원문 그대로 보존**(안전 폴백:
/// 레지스트리 REG_EXPAND_SZ 의 %USERPROFILE% 등을 프로세스 env 로 전개하되 못 푼 건 깨뜨리지 않는다).
pub fn expand_windows_env(s: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' {
            if let Some(rel) = chars[i + 1..].iter().position(|&c| c == '%') {
                let close = i + 1 + rel;
                let name: String = chars[i + 1..close].iter().collect();
                if !name.is_empty() {
                    if let Some(val) = lookup(&name) {
                        out.push_str(&val); // %VAR% → 값
                    } else {
                        out.extend(chars[i..=close].iter()); // 미지 변수: %NAME% 원문 유지
                    }
                    i = close + 1;
                    continue;
                }
                // 빈 이름(%%): 첫 % 만 리터럴로 밀어 원문(%%)을 보존.
                out.push('%');
                i += 1;
                continue;
            }
            // 미종결 %: 리터럴.
            out.push('%');
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Windows 사용자 bin 후보(belt-and-braces) — claude native 설치 위치 `%USERPROFILE%\.local\bin` 와
/// npm 전역 `%APPDATA%\npm`. 설치기가 레지스트리 등록을 빠뜨려도 잡히게 is_dir 게이트 없이 무조건 포함
/// (셸은 없는 PATH 항목을 무시·claude 설치 직후 재시작 없이 발견). OS 무관 컴파일(순수·테스트 가능).
pub fn windows_user_bin_dirs(home: &Path, appdata: Option<&Path>) -> Vec<PathBuf> {
    let mut v = vec![home.join(".local").join("bin")];
    if let Some(a) = appdata {
        v.push(a.join("npm"));
    }
    v
}

/// pane PATH 최종 합성(순서·dedup 규칙 · **Windows 전용 의미론** · OS 무관 컴파일). 순서:
/// `[prefixes: exe_dir + 번들 runtime bins]` ; `process_path 전체(현행 순서 그대로 보존)` ;
/// `fresh_base(신선 레지스트리 머신;유저) 중 신규분만 append` ; `user_bins 중 신규분만 append`.
/// ★MAJ#1(fail-open): 부모가 프로세스 PATH 선두에 의도 주입한 항목(pyenv/nvm shim 등, 레지스트리에 없는
/// 것)의 precedence 를 강등하지 않는다 — 버그 본질은 claude 디렉터리 '발견 가능'이지 '선두'가 아니라,
/// 레지스트리 신규분은 append 로 충분(프로세스 PATH 에 claude 가 없으니 append 로 발견됨). 레지스트리가
/// 새로 주는 게 없으면 composed==current 가 더 자주 성립 → None(무주입) 계약도 더 잘 보존된다.
/// 단, prefixes(동봉 runtime)는 항상 선두 — 레지스트리 유입 python 등에 절대 밀리지 않는다.
/// ★MAJ#2(dedup): Windows PATH 는 case-insensitive 이므로 비교 키를 **소문자 정규화 + 후행 경로구분자
/// (`\`·`/`) 트림**으로 만든다(출력은 원문=최초 등장 형태 유지). casing 변형·후행 슬래시 중복이 잔존해
/// env 블록 32767자 한계를 넘겨 CreateProcess 가 실패(pane 미기동)하는 꼬리위험을 막는다. 빈 항목 제거.
/// fresh_base=None 이면 base=프로세스 PATH(현행 폴백).
pub fn compose_pane_path(
    prefixes: &[String],
    fresh_base: Option<&str>,
    user_bins: &[String],
    process_path: &str,
    sep: char,
) -> String {
    // Windows 비교 키: 소문자 + 후행 '\'·'/' 트림. 출력엔 안 쓰고 dedup 판정에만.
    fn norm_key(s: &str) -> String {
        s.trim_end_matches(['\\', '/']).to_lowercase()
    }
    let mut items: Vec<&str> = Vec::new();
    for p in prefixes {
        items.push(p.as_str());
    }
    for e in process_path.split(sep) {
        items.push(e);
    }
    if let Some(fb) = fresh_base {
        for e in fb.split(sep) {
            items.push(e);
        }
    }
    for u in user_bins {
        items.push(u.as_str());
    }
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<&str> = Vec::new();
    for it in items {
        if it.is_empty() {
            continue;
        }
        if seen.insert(norm_key(it)) {
            out.push(it);
        }
    }
    out.join(&sep.to_string())
}

/// Windows 레지스트리에서 신선 PATH 재합성(머신;유저) — 실행 중 데몬이 못 받는 WM_SETTINGCHANGE 를 우회.
/// HKLM `...\Session Manager\Environment` + HKCU `Environment` 의 `Path` 를 관례대로 머신;유저 순 결합,
/// REG_EXPAND_SZ 의 %VAR% 는 프로세스 env 로 수동 전개. 성공 hive 만 사용·둘 다 실패면 None(fail-open).
#[cfg(windows)]
pub fn windows_registry_path() -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let machine = read_registry_string(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        "Path",
    );
    let user = read_registry_string(HKEY_CURRENT_USER, "Environment", "Path");
    // %VAR% 는 로그인 후 불변인 프로세스 env(USERPROFILE 등)로 전개 — pane 이 볼 값과 등가.
    let expand = |raw: String| expand_windows_env(&raw, |k| std::env::var(k).ok());
    match (machine, user) {
        (Some(m), Some(u)) => Some(format!("{};{}", expand(m), expand(u))),
        (Some(m), None) => Some(expand(m)),
        (None, Some(u)) => Some(expand(u)),
        (None, None) => None,
    }
}

/// 레지스트리 문자열 값 읽기(REG_SZ·REG_EXPAND_SZ, NOEXPAND 로 원문 취득 후 상위에서 수동 전개).
/// 2-pass: 1) 필요한 바이트 크기 질의 → 2) 버퍼 채움. 실패(값 부재 등)면 None(fail-open).
#[cfg(windows)]
fn read_registry_string(
    hkey: windows_sys::Win32::System::Registry::HKEY,
    subkey: &str,
    value: &str,
) -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    };
    let sub: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let val: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    // 1-pass: 크기 질의(데이터 포인터 NULL).
    let mut size: u32 = 0;
    let rc = unsafe {
        RegGetValueW(
            hkey,
            sub.as_ptr(),
            val.as_ptr(),
            flags,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if rc != ERROR_SUCCESS || size == 0 {
        return None;
    }
    // 2-pass: u16 버퍼(바이트→워드, 널 여유 +1).
    let mut buf: Vec<u16> = vec![0u16; (size as usize / 2) + 1];
    let mut size2 = (buf.len() * 2) as u32;
    let rc2 = unsafe {
        RegGetValueW(
            hkey,
            sub.as_ptr(),
            val.as_ptr(),
            flags,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            &mut size2,
        )
    };
    if rc2 != ERROR_SUCCESS {
        return None;
    }
    let n = (size2 as usize / 2).min(buf.len());
    let slice = &buf[..n];
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    let s = String::from_utf16_lossy(&slice[..end]);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 홈 디렉토리(RC-7 공용). Windows는 HOME 미설정이 기본이라 `env::var("HOME")`은 빈값으로 폴백돼
/// `~/.cys/...` 경로를 CWD 상대경로로 붕괴시킨다(부서목록·프로파일·pending-restore 오지정). dirs::home_dir()
/// (Windows=USERPROFILE/HOMEDRIVE 기반·unix=$HOME)로 OS중립 해소. 코어(cys)·GUI(src-tauri) 공유.
pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// (W1) claude CLAUDE_CONFIG_DIR 결정론 해소 — agents.json의 `${CYS_ACCOUNT_DIR:-$HOME/.cys/claude}`와
/// 동일 규칙을 **현재 프로세스 env**로 전개한다. pane 셸(=데몬 자식)이 실제로 해소하는 값과 일치하려면
/// 실제 전개 주체인 **데몬 프로세스에서 호출**하는 것이 권위다(state.rs의 CYS_ACCOUNT_DIR 전파와 정합).
/// discover 스캔(usage.rs)이 ~/.cys/claude를 원리적으로 못 보므로, config_dir 권위는 이 결정론 해소뿐이다.
pub fn resolve_claude_config_dir() -> String {
    std::env::var("CYS_ACCOUNT_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| home_dir().join(".cys").join("claude").to_string_lossy().into_owned())
}

/// ★D5(v4 수리 — 문제2 env 방어층): claude 가 alternate screen(fullscreen TUI)으로 뜨면 휠
/// 보고가 앱으로 들어가 프롬프트 히스토리를 오염시킨다(스펙 §D5). 이 키를 "1" 로 주입해
/// fullscreen 진입 자체를 막되, **사용자가 agents.json env 에 이미 값을 적었으면(특히
/// "0" 옵트아웃) 절대 덮지 않는다** — 주입은 '키 부재 시에만'이 계약이다.
/// ★주입 여부는 OS 게이트가 정한다: **macOS = 기본 주입 · Windows = 옵트인 시에만 · 그 외 =
/// 미주입**(`d5_gate_for_os` — 그 함수 doc 이 강등 근거와 승격 절차의 정본).
pub const ENV_CLAUDE_NO_ALT_SCREEN: &str = "CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN";

/// D5 주입 헬퍼 — **두 소비처(cys.rs `boot_agent_on_surface` 인라인 재조립·`run_launch_agent_opts`
/// surface.create env)가 모두 이 함수를 경유**한다(사본 금지 — lib 에 있는 이유는 `cargo test --lib`
/// 레인이 회귀 핀을 상주 실행하기 때문. cys.rs 테스트 모듈은 CI 0회 실행이라 핀이 죽는다).
///
/// 게이트 = `d5_gate_for_os(std::env::consts::OS, d5_win_opt_in())` ∧ `bin == "claude"`
/// (호출부가 cys.rs 의 `fn extract_bin` 으로 env-prefix 를 걷어낸 실제 바이너리 토큰을 넘긴다 —
/// 어댑터 키 개명 내성). OS 매핑은 **macOS = 무조건 주입 · Windows = 옵트인했을 때만 ·
/// 그 외 = 미주입**이다. 왜 Windows 만 옵트인인지(앵커 ④ · 실기 스모크 B-5 미수행)와
/// **기본 on 승격 절차**는 `d5_gate_for_os` 의 doc 이 정본이다 — 여기서 중복 서술하지 않는다.
///
/// ★이 doc 의 상호참조는 **줄번호를 쓰지 않는다**(적대검증 2R minor 수리). 종전 판은
/// `extract_bin`(:1081)·`boot_agent_on_surface`(:6358)·`8889-8893`·`main.rs:382-395` 네 곳을
/// 줄번호로 인용했는데 **네 개가 전부 어긋나 있었다**(각각 실제 :1125 · :6328 · 무관한 묘비
/// skip 코드 · 383-396). 병렬 편집이 몇 줄만 밀어도 즉시 거짓이 되는 인용 형식이라, 다음
/// 조사자를 엉뚱한 코드로 보낸다 — 이 저장소가 "주석을 계약으로 취급"하는 이상 그 자체가
/// 결함이다. 그래서 **심볼명 또는 grep 가능한 마커**로만 가리킨다.
///
/// ★Windows 를 사정권에 넣은 근거(v0.14 품질 라인 — 반증된 전제 정정): 종전 게이트가 mac
/// 단독이었던 이유는 "Windows 의 claude 는 기본 inline 이라 fullscreen 이 미발현"이라는
/// 전제였는데, 그 전제는 실측으로 반증됐다. Claude Code 2.1.233 의 fullscreen 판정 함수 `ra()`
/// 에는 순수 Windows→inline 분기가 없고 Windows 관련 분기는 `Windows ∧ SSH` 하나뿐이며,
/// settings 의 `tui` 키가 부재하면 최종 판정을 서버측 기능게이트가 한다 — 즉 **OS 가 아니라
/// 계정·롤아웃이 결정한다**. 저장소 안에 그 전제를 뒷받침하는 버전 핀·감지 코드는 0건이었다.
/// ∴ Windows 에서도 fullscreen 은 **뜰 수 있다**(그래서 UI 휠 가드가 본체 방어로 들어갔다).
/// ★단, 그 사실이 곧 "그러니 env 를 기본 주입하자"는 아니다 — 주입의 최악 결과가 앵커 ④
/// (전 pane 사망)라서, 2026-08-17 에 Windows 는 **기본 주입에서 옵트인으로 강등**됐다.
/// 강등 근거·승격 절차는 `d5_gate_for_os` doc.
///
/// ★정직 주석(이것은 '벨트'이고 '본체'가 아니다 — 다음 조사자가 같은 함정을 밟지 않도록):
/// 이 env 가 **옵트인한 Windows 사용자에게** 실제로 도달하는 경로는 `cys launch-agent` 가
/// **새 surface 를 만들며 기동한 pane** 하나뿐이다(`run_launch_agent_opts` 의 `surface.create`
/// env 주입 경로). 옵트인하지 않았다면 도달 경로는 **0건**이다(게이트가 거짓이라 주입 자체가
/// 없다 — Windows 기본 동작은 강등 전 출고본과 동일하다).
/// · `fn boot_agent_on_surface`(src/bin/cys.rs)는 Windows 에서 `render_launch` 의 env 를
///   그대로 폐기한다 — `let (send, _send_env) = render_launch(&cmd, &env_pairs);` 로 send 문자열만
///   취하고, Windows 는 인라인 `KEY="val" cmd` 를 쓰지 않으므로 env 가 pane 에 실리지 않는다.
///   저장소 자신이 같은 구멍(빈 셸 pane 은 env 가 비어 Windows 는 계정 격리가 깨진다)을
///   restore 의 계정격리 가드에 문서화하고 있다 — src/bin/cys.rs 에서
///   grep `★계정격리 가드(E8)`.
/// · Tauri GUI 의 `fn create_surface`(src-tauri/src/main.rs)는 `surface.create` RPC 에
///   cwd/title/rows/cols 만 실어 보내고 env 는 **아예 넘기지 않는다**.
/// ∴ 기존 pane 에 붙는 기동·GUI 기동에는 이 벨트가 닿지 않는다. Windows 휠 오염의 **본체 방어는
/// UI 가드**(`ui/src/wheelgate.ts` 의 Windows 전용 억제 술어)이고, 이 함수는 그 위에 덧대는
/// 벨트일 뿐이다. 여기를 고쳤다고 Windows 문제가 닫혔다고 판단하지 마라.
///
/// ★CI 실행 경로(2026-08-17 갱신 — 종전의 "Windows 레인 0건" 고지는 **해소됐다**):
/// 적대검증 2R 이 major 로 지목한 "Windows 전용 신규 코드가 Windows 러너에서 한 줄도
/// 컴파일·실행되지 않는다"를 워크플로 변경으로 닫았다.
/// · `.github/workflows/release.yml` — Windows 레그(`if: matrix.platform == 'windows-latest'`)의
///   rust 스텝이 `cargo test --lib factory_reset::` 에 더해 **`cargo test --lib claude_alt_screen`**
///   를 실행한다(태그 경로 = 사용자에게 도달하는 유일한 경로). 비용 0 — lib 은 이미 컴파일돼
///   있고 D5 핀은 순수 매핑이다.
/// · `.github/workflows/windows-health.yml` — **`cargo test --bin cys d5_env_injection`** 스텝을
///   추가했다. 아래 ②-win 파이프라인 핀(`#[cfg(windows)]`)을 **처음으로 컴파일·실행하는 곳**이
///   여기다. 태그 레인이 아니라 health 레인에 둔 이유: 그 테스트 하네스는 이 저장소에서 한 번도
///   Windows 컴파일된 적이 없어 D5 와 무관한 이유로 깨질 수 있고, 그 발견은 태그를 만드는
///   순간이 아니라 그 전에 나야 하기 때문이다.
/// · 여전한 한계(정직): windows-health 는 `push: branches: ['feat/**']` + workflow_dispatch 로만
///   돈다. 릴리스 브랜치에서는 **사람이 workflow_dispatch 로 1회 기동해야** 이 핀이 돈다 —
///   릴리스 체크리스트(docs/plans/v0.14.16-release-notes.md 의 Windows 스모크 절)에 그것을
///   조건으로 적어 두었다.
/// ∴ 지금 Windows 러너가 증명하는 것은 (a) OS × 옵트인 매핑(`d5_gate_for_os` — 기본 미주입 행
/// 포함)과 (b) health 레인을 돌렸다면 두 소비처 파이프라인의 env 산출물(기본·옵트인 두 행)이다. 그러나 **claude 가 이 env 를 실제로 존중하는가**
/// (=화면 모드가 바뀌는가)는 어느 자동 테스트도 증명하지 않는다 — 그것은 실기 스모크의 몫이다.
/// ★그 미증명 축이 바로 Windows 강등의 이유다(2026-08-17): 자동 레인이 닿지 못하는 곳에
/// 앵커 ④(전 pane 사망)가 있고 실기 스모크 B-5 를 수행할 기계가 없었으므로, 기본값을
/// 미주입으로 돌려 **자동 레인이 증명하는 범위 안으로 위험을 축소**했다. 옵트인한 사용자만
/// 그 미증명 축을 밟는다(그리고 그 사용자는 스위치를 지우면 즉시 되돌릴 수 있다).
///
/// ★함정 주석(스펙 명문): `agent_env_pairs` 는 내부에서 이미 `v.sort()` 를 끝냈다 — 여기서
/// **append 후 재정렬을 하면 안 된다**. CLAUDE_CODE_… 는 사전순으로 CLAUDE_CONFIG_DIR 보다
/// 앞이라, 재정렬은 무해해 '보이지만' 미래에 같은 키의 사용자 "0" 쌍과 기본 "1" 쌍이 공존하는
/// 버그가 생겼을 때 정렬이 **사용자 "0" 을 "1" 뒤로 뒤집어** 셸 전개 순서상 기본값이 이기게
/// 만든다. 그래서 계약은 정렬이 아니라 **contains 검사 후 부재 시에만 append** 다.
pub fn inject_claude_alt_screen_default(env_pairs: &mut Vec<(String, String)>, bin: &str) {
    inject_claude_alt_screen_default_for(
        env_pairs,
        bin,
        d5_gate_for_os(std::env::consts::OS, d5_win_opt_in()),
    );
}

/// D5 Windows 옵트인 스위치의 **env 이름**(형제 게이트와 동형 — `CYS_…`, 값 `"1"` 만 참).
///
/// ★이름 극성 주의(왜 `CYS_WIN_ALT_SCREEN_OFF` 가 아닌가): 이 저장소의 `_OFF` 접미는
/// `CYS_WIN_WHEEL_GUARD_OFF` 처럼 **'우리 기능을 끈다'**(킬스위치) 극성으로 이미 쓰이고 있어,
/// 같은 `~/.cys/` 안에 `win-wheel-guard-off`(보호를 끔)와 `win-alt-screen-off`(보호를 켬)가
/// 나란히 놓이면 사용자·후임 조사자 모두 극성을 오독한다. 게다가 기본 on 승격 뒤에 정말 필요한
/// 것은 롤백 킬스위치이고 그 자리의 자연스러운 이름이 바로 `CYS_WIN_ALT_SCREEN_OFF` 다 —
/// 지금 그 이름을 반대 극성으로 태워 버리면 승격 시점에 이름을 잃는다. 그래서 긍정형으로
/// 짓되 **주입되는 키 이름(`ENV_CLAUDE_NO_ALT_SCREEN`)과 같은 어휘**를 써 대응을 자명하게 했다.
pub const D5_WIN_OPT_IN_ENV: &str = "CYS_WIN_NO_ALT_SCREEN";

/// D5 Windows 옵트인 스위치의 **파일 경로**(홈 기준 상대 — 형제: `.cys/ime-debug`,
/// `.cys/allow-app-mouse`, `.cys/win-wheel-guard-off`).
pub const D5_WIN_OPT_IN_FILE: &str = ".cys/win-no-alt-screen";

/// D5 Windows 옵트인 판독(**부작용 있음** — env 1회 + 파일 stat 1회).
///
/// 형제 게이트(`src-tauri/src/main.rs` 의 `ime_debug_enabled`·`app_mouse_enabled`·
/// `win_wheel_guard_disabled`)와 **동형**이다: `env == "1"` 또는 파일 존재. 그 **판정 규약
/// 자체는 순수 코어 `d5_win_opt_in_from` 에 있고**, 이 함수는 두 채널을 관측해 먹이기만 하는
/// 얇은 래퍼다(그렇게 쪼갠 이유는 그 함수의 doc — 판독기에 회귀 핀을 걸기 위해서다).
///
/// ★왜 Tauri 커맨드가 아니라 여기(코어 lib)인가: D5 를 소비하는 것은 GUI 가 아니라 **Rust CLI
/// 의 부트 경로**(`cys launch-agent` → `run_launch_agent_opts` / `boot_agent_on_surface`)다.
/// Tauri 커맨드는 UI(TS)가 invoke 로 묻는 채널이라 이 경로에서 부를 수 없다. 그래서 형제들과
/// **판독 규약은 같게**, 소재지는 CLI·GUI 가 함께 링크하는 `cys` lib 로 두었다.
///
/// ★두 수단의 적용 시점(사용자 문서와 같은 내용 — USER-MANUAL env 표):
/// · 파일 — `cys launch-agent` 는 **호출마다 새로 뜨는 단명 프로세스**라 매 기동에 stat 한다
///   → **다음 기동부터 즉시** 반영. Windows 권장 수단.
/// · env — 이 프로세스가 **상속한 값**만 보인다. Windows 의 pane 은 GUI→데몬→pane 순으로
///   env 를 물려받으므로 `setx` 만으로는 이미 떠 있는 GUI 계보에 반영되지 않는다(GUI 재시작
///   필요). 그래서 정본 안내는 파일이다.
///
/// ★비용: macOS 는 게이트가 옵트인 값과 무관하게 참이라 이 판독이 낭비다(호출부가 인자를
/// eager 하게 평가한다). 그러나 pane 기동당 stat 1회이고, 게이트를 **순수 함수로 유지**해
/// mac 호스트에서 `("windows", false)`·`("windows", true)` 두 행을 함께 핀으로 박는 값이
/// 그 비용보다 크다.
pub fn d5_win_opt_in() -> bool {
    d5_win_opt_in_from(
        std::env::var(D5_WIN_OPT_IN_ENV).ok().as_deref(),
        home_dir().join(D5_WIN_OPT_IN_FILE).exists(),
    )
}

/// D5 옵트인 **판독 규약의 순수 코어** — 두 채널의 관측값만 받아 판정한다
/// (`env_val` = `CYS_WIN_NO_ALT_SCREEN` 의 값 · `file_exists` = `~/.cys/win-no-alt-screen` 존재).
/// 규약은 형제 게이트와 동형인 **`env == "1"` ∨ 파일 존재**이고, `"true"`·`"yes"`·빈 값 같은
/// 느슨한 truthy 는 **참이 아니다**(형제들과 같은 엄격 비교 — 오타로 켜지는 사고 방지).
///
/// ★왜 쪼갰는가(고친 결함 · 2026-08-17 적대검증 2R minor): 강등의 안전 사슬에서 **판독기만
/// 핀이 없었다**. 게이트 매핑(`d5_gate_for_os`)·삽입 배선·래퍼 라우팅은 모두 고정돼 있었지만
/// '**런타임 기본값이 거짓인가**'를 단언하는 테스트가 저장소 전체에 0건이라, 종전 본문의
/// `unwrap_or(false)` → `unwrap_or(true)` 급 1글자 퇴행이 **전 레인 초록인 채로** Windows 를
/// 기본 on 으로 되돌릴 수 있었다(그 최악이 앵커 ④ — 전 pane 사망). 부작용을 래퍼에 남기고
/// 판정만 순수 함수로 떼면 그 축이 `claude_alt_screen_win_opt_in_reader_pins` 로 고정된다.
/// 판독 규약을 바꾸려면 그 핀을 **함께 뒤집는 의도적 행위**여야 한다.
pub fn d5_win_opt_in_from(env_val: Option<&str>, file_exists: bool) -> bool {
    env_val == Some("1") || file_exists
}

/// D5 OS 게이트의 **단일 진리원** — `macOS = 무조건 · Windows = 옵트인 시에만 · 그 외 = 비대상`.
///
/// ★왜 Windows 가 '기본 on' 이 아니라 '옵트인' 인가(2026-08-17 강등 · 오너 부재 중 대리 판단):
/// D5 확장은 v0.14 품질 라인 diff 에서 **부트 체인에 닿는 유일한 변경**이었고, 그 최악 결과가
/// 릴리스 위험표의 유일한 '고' 등급 R2 = **앵커 ④(전 pane 사망)** 이다 — 이 env 가 Windows 의
/// claude 를 깨뜨리면 `cys boot` 로 뜬 4종 노드가 **전부** 죽고, 오너가 부재면 복구 수단이 없다.
/// 그 가능성을 배제하는 유일한 증거는 **Windows 실기 스모크 B-5**(`cys boot` 4종 노드 정상 +
/// pane 안 env 실림)인데, 물리 Windows 기계가 없어 **수행하지 못했다**. 자동 레인은 이 축을
/// 대신하지 못한다(레인이 증명하는 것은 OS→게이트 매핑과 env 산출물까지이고, claude 가 그 env 를
/// 존중하는가는 실기의 몫이다 — 위 `inject_claude_alt_screen_default` doc 의 CI 실행 경로 절).
/// 결정을 강제한 것은 **비대칭**이다: 기본 on 의 최악은 '전 pane 사망·복구 불가', 옵트인의
/// 최악은 '부트 관측성이 오늘과 동일'(= Windows 회귀 0). 오너 지침("Windows 설치파일
/// 업데이트에 신중에 신중")과도 같은 방향이다. **조건을 건너뛴 것이 아니라, 조건이 걸린 변경
/// 자체를 무장 해제한 것**이다.
///
/// ★기본 on 승격 절차(다음 사람은 이 주석만 읽고 승격할 수 있어야 한다):
///  · 조건 — Windows 실기에서 **B-5 1회 통과**. 즉 옵트인(`~/.cys/win-no-alt-screen` 생성)
///    상태로 `cys boot` 가 4종 노드를 정상 기동하고, 각 pane 에서
///    `$env:CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN` 이 `1` 이며 claude 가 정상 표시될 것.
///  · 바꿀 한 줄 — 아래 match 의 `"windows" => win_opt_in,` → `"windows" => true,`.
///  · 동반 개정(빠뜨리면 거짓 산문이 남는다 — ④⑤ 는 2026-08-17 적대검증 2R 이 **누락을 지적해**
///    추가된 자리다. 목록을 줄이지 마라) —
///    ① 이 파일의 핀 `claude_alt_screen_env_injection_pins` 의 ④-b·⑤-b(미옵트인=미주입) 행과
///       `src/bin/cys.rs` 의 `d5_env_injection_covers_both_consumers` 의 `#[cfg(windows)]` 블록.
///    ② `USER-MANUAL.md` env 표의 `CYS_WIN_NO_ALT_SCREEN` 행 · `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN` 행.
///    ③ `docs/plans/v0.14.16-release-notes.md` 의 'Windows D5 는 옵트인' 문단과 B-5 의 지위.
///    ④ `src/bin/cysd/state.rs` 의 `env_injected` 주석과 그 회귀 핀
///       (`create_surface_with_env_records_env_injected_flag` 의 D5 절). 둘 다 'D5 한 쌍만으로
///       `env_injected` 가 **격리 키 없이** 참이 되는' 조합을 서술하는데, 승격하면 그 조합의
///       조건에서 **옵트인 항이 사라진다**(Windows 전부가 해당). 조건을 지우지 않으면 산문이
///       실제보다 좁게 읽혀 in-seat 가드(`src/bin/cys.rs` grep `★계정격리 가드(E8)`)를 조사하는
///       사람이 그 경로를 배제한다.
///    ⑤ `src/bin/cys.rs` 의 `alt_screen_notice` **Windows 힌트 문안** — '①Windows 는 이 env
///       주입이 기본 off(옵트인) 입니다' 가 통째로 거짓이 되고, 사용자에게 없는 절차를 시킨다.
///       진리표 핀은 `hint` 토큰만 보므로 **자동 검출되지 않는다**(문안은 사람이 지켜야 한다).
///  · 승격하면 이 옵트인 스위치는 무의미해지고, 그때 필요한 것은 **롤백 킬스위치**다 —
///    그 이름으로 `CYS_WIN_ALT_SCREEN_OFF`(파일 `~/.cys/win-alt-screen-off`)를 비워 두었다
///    (형제 `CYS_WIN_WHEEL_GUARD_OFF` 와 같은 `_OFF` = '우리 기능 끄기' 극성).
///    그 교체를 하면 `d5_win_opt_in`·`d5_win_opt_in_from` 과 그 핀
///    (`claude_alt_screen_win_opt_in_reader_pins`)은 **함께 킬스위치 쪽으로 옮겨야 한다** —
///    극성이 뒤집히므로 핀의 ①(기본=거짓)은 ①(기본=**참**)이 된다. 판독기를 핀 없이 남기지
///    마라: 그 핀이 없던 것이 이번 라운드에 지적된 결함이다(그 함수의 doc 참조).
///
/// ★왜 `cfg!` 가 아니라 문자열 매핑인가(가짜 핀 제거 — v0.14 품질 라인):
/// `cfg!` 는 컴파일 시각에 상수로 접힌다. 그래서 mac 호스트에서는 확장 **전**(`cfg!(macos)`)과
/// **후**(`cfg!(macos) || cfg!(windows)`)가 둘 다 참이라, 어떤 단언을 써도 두 상태를 갈라내지
/// 못했다 — 종전의 'windows 도 삽입된다' 핀이 감시력 0 의 장식이었던 구조적 원인이 이것이다.
/// OS 판정을 순수 함수로 빼면 **어느 호스트에서든 세 OS 를 동시에 조회**할 수 있어, 누군가
/// 매핑을 흔들면(mac 단독 회귀·Windows 무조건 승격 모두) mac CI 에서 즉시 빨개진다.
///
/// ★두 번째 인자가 bool 인 이유(강등 후에도 그 판별력을 지키기 위해): 옵트인 판독은 env·파일
/// stat = **부작용**이라 이 함수 **바깥**(`d5_win_opt_in`)에서 하고, 여기는 순수 매핑으로
/// 남긴다. 그래야 mac 호스트에서 `("windows", false)`·`("windows", true)` 두 행을 함께 조회해
/// **'기본 미주입'과 '옵트인 시 주입'을 각각** 핀으로 박을 수 있다.
///
/// 값 동치: `std::env::consts::OS` 는 컴파일 대상의 상수 문자열("macos"/"windows"/"linux"…)이라
/// `cfg!(target_os = …)` 과 같은 것을 가리킨다. 그 동치는 아래 테스트가 옵트인 축을 **상수로
/// 고정**해 대조한다 — `(OS, true)` ↔ `cfg!(macos) || cfg!(windows)`,
/// `(OS, false)` ↔ `cfg!(macos)`(현 빌드 OS 1개 한정).
///
/// ★적용 범위 주의: 이 함수는 '이 OS·옵트인 조합이 D5 env 주입 대상인가'만 답한다. 그 env 가
/// 실제로 pane 까지 실리는지는 OS별 파이프라인의 문제이고, Windows 쪽 한계는 위 doc 의 정직
/// 주석을 보라.
pub fn d5_gate_for_os(os: &str, win_opt_in: bool) -> bool {
    match os {
        "macos" => true,
        "windows" => win_opt_in,
        _ => false,
    }
}

/// D5 순수 코어 — OS 게이트를 인자로 받아 **어느 호스트에서든 양 분기를 테스트**할 수 있게 한다
/// (compose_pane_path 등 이 파일의 'windows 로직을 mac에서 검증' 관례와 동일).
///
/// `gated` 는 '이 기동이 D5 주입 대상인가'다 — 즉 `d5_gate_for_os(OS, 옵트인)` 의 결과다
/// (구 이름 `is_macos` — Windows 확장으로 의미가 어긋나 개명했다. 호출부는 위치 인자라 시그니처
/// 호환은 유지된다). 이 함수는 OS 도 옵트인도 모른다: **불가침 3계약**(키 부재 시에만 append ·
/// 사용자 값 불가침 · 재정렬 금지)만 지킨다. 게이트를 넓히든 좁히든 이 셋은 건드리지 마라.
pub fn inject_claude_alt_screen_default_for(
    env_pairs: &mut Vec<(String, String)>,
    bin: &str,
    gated: bool,
) {
    if !gated || bin != "claude" {
        return;
    }
    if env_pairs.iter().any(|(k, _)| k == ENV_CLAUDE_NO_ALT_SCREEN) {
        return; // 사용자 값(특히 fullscreen 을 되살리는 "0") 절대 불가침 — 부재 시에만 기본값.
    }
    env_pairs.push((ENV_CLAUDE_NO_ALT_SCREEN.to_string(), "1".to_string()));
}

/// Claude Code projects/ 디렉터리명 munge — 실측: '/'와 특수문자가 '-'로 치환된다.
/// ASCII 영숫자·'-'만 보존하는 보수 구현. resume 사전검증 게이트(cys.rs)와 usage 휴리스틱이 공유한다.
pub fn claude_project_component(cwd: &str) -> String {
    cwd.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// 부서 데몬 소켓/파이프 경로(RC-4 · 공용 — GUI(src-tauri)·cys fleet가 공유, 규약 단일화).
/// Windows: named pipe `\\.\pipe\cys-dept-<name>`(기본 데몬 `\\.\pipe\cys`와 대칭 · RC-13 state_dir
/// 슬러그 `cys-dept-<name>`과 정합). unix: `~/.local/state/cys-dept-<name>/cys.sock`(cys-dept 규약).
/// HOME 미설정 함정(RC-7) 회피 — unix도 dirs::home_dir() 사용.
pub fn dept_socket_path(name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(format!(r"\\.\pipe\cys-dept-{name}"))
    }
    #[cfg(not(windows))]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/state")
            .join(format!("cys-dept-{name}"))
            .join("cys.sock")
    }
}

/// 이 소켓/파이프 경로가 부서(dept) 데몬의 것인가 — 부서 규약 `cys-dept-<name>`(dept_socket_path와 정합).
/// 채널은 메인 cysd 단독 소유(DESIGN §2.5)이므로 부서 데몬의 브리지 스폰을 구조적으로 거부하는 데 쓴다.
/// 판별: 경로 컴포넌트(unix 부모 디렉토리 `cys-dept-<name>` / windows 파이프명 `cys-dept-<name>`) 중
/// `cys-dept-` 접두 이름이 있으면 부서. 메인 데몬(슬러그 `cys`·`cys.sock`)은 오판하지 않는다
/// (`cys`는 `cys-dept-` 접두가 아님 — 오탐 시 채널 전면 불능이라 접두를 정확히 요구).
pub fn is_dept_socket(socket_path: &std::path::Path) -> bool {
    socket_path
        .to_string_lossy()
        .split(|c| c == '/' || c == '\\')
        .any(|comp| comp.starts_with("cys-dept-"))
}

/// Parse a surface reference: "surface:31", "31", or 31 → 31.
pub fn parse_surface_ref(s: &str) -> Option<u64> {
    let t = s.trim();
    let t = t.strip_prefix("surface:").unwrap_or(t);
    t.parse::<u64>().ok()
}

pub fn surface_ref(id: u64) -> String {
    format!("surface:{id}")
}

/// Map a named key name to the byte sequence
/// written to the PTY. Supports C- (ctrl), M- (alt/meta) prefixes.
pub fn key_to_bytes(key: &str) -> Option<Vec<u8>> {
    // Modifier prefixes
    if let Some(rest) = key.strip_prefix("C-") {
        // 단일 문자일 때만 ctrl 비트 변환 — "C-Space"의 'S'가 0x13(XOFF, 출력 동결)으로
        // 잘못 변환되어 Space 분기가 사문화되는 것을 차단
        if rest.chars().count() == 1 {
            let c = rest.chars().next()?;
            let lower = c.to_ascii_lowercase();
            if lower.is_ascii_lowercase() {
                return Some(vec![(lower as u8) & 0x1f]);
            }
        }
        return match rest {
            "Space" | "space" => Some(vec![0x00]),
            _ => None,
        };
    }
    if let Some(rest) = key.strip_prefix("M-") {
        let mut b = vec![0x1b];
        b.extend_from_slice(rest.as_bytes());
        return Some(b);
    }
    let seq: &[u8] = match key {
        "Return" | "Enter" => b"\r",
        "Tab" => b"\t",
        "BTab" | "BackTab" => b"\x1b[Z",
        "Space" => b" ",
        "Escape" | "Esc" => b"\x1b",
        "Backspace" => b"\x7f",
        "Delete" | "DC" => b"\x1b[3~",
        "Up" => b"\x1b[A",
        "Down" => b"\x1b[B",
        "Right" => b"\x1b[C",
        "Left" => b"\x1b[D",
        "Home" => b"\x1b[H",
        "End" => b"\x1b[F",
        "PageUp" | "PPage" => b"\x1b[5~",
        "PageDown" | "NPage" => b"\x1b[6~",
        "F1" => b"\x1bOP",
        "F2" => b"\x1bOQ",
        "F3" => b"\x1bOR",
        "F4" => b"\x1bOS",
        "F5" => b"\x1b[15~",
        "F6" => b"\x1b[17~",
        "F7" => b"\x1b[18~",
        "F8" => b"\x1b[19~",
        "F9" => b"\x1b[20~",
        "F10" => b"\x1b[21~",
        "F11" => b"\x1b[23~",
        "F12" => b"\x1b[24~",
        _ => {
            // Single literal character passes through
            if key.chars().count() == 1 {
                return Some(key.as_bytes().to_vec());
            }
            return None;
        }
    };
    Some(seq.to_vec())
}

/// ★A9(v4 수리 — 문제2 데몬측 예외 1건): xterm 마우스 보고 시퀀스 판별 — **TS 이식본**.
///
/// 정본은 `ui/src/mousefilter.ts` 의 `classifyMouseReport`(:29-112)이고, 이 모듈은 그 규칙을
/// 바이트 단위로 동일하게 Rust 로 옮긴 것이다(TS 대응 좌표를 각 항목에 병기). 두 구현의
/// 패리티는 공유 코퍼스 `src/testdata/mouse_report_corpus.json` 이 기계 고정한다 — 규칙을
/// 고칠 땐 **양쪽 + 코퍼스**를 함께 고쳐라(한쪽만 고치면 코퍼스 테스트가 빨간불).
///
/// 왜 데몬에 필요한가: GUI 는 mac 에서 비-휠 마우스 보고(클릭·모션)를 앱에 forward 하는데,
/// 그 경로가 `send_input`(human=true) → `surface.send_text` 라서 보고가 **사람 타이핑으로
/// 위장**된다 — 오너가 pane 을 스크롤해 읽는 동안 `--queued` 배달이 무기 연기되고(큐 적체
/// 앵커 위반) seat 판정이 오염된다(R2 SIM 발견 8). 데몬은 '수신 텍스트 **전체**가 마우스
/// 보고의 연접'일 때만 last_human_input 갱신을 생략한다(`is_pure_mouse_report`).
///
/// ★B1(0.14.24) 범위 확장 고지: 이 모듈은 이제 마우스 보고 **바깥**의 터미널 자동 응답까지
/// 판별한다(`is_pure_terminal_autoreply`). 그 추가분은 **TS 쌍둥이가 없다** — GUI 는 자동
/// 응답을 그대로 forward 할 뿐이고 판정은 데몬 단독이다(단일 정의처). 위의 '양쪽+코퍼스를
/// 함께 고쳐라' 계약은 **마우스 규칙에만** 적용된다(코퍼스도 마우스만 고정한다).
pub mod mousereport {
    /// mousefilter.ts:21-24 `MouseVerdict` 동형. `Wheel.dir`: -1=위로, +1=아래로(scrollLines 규약).
    #[derive(Debug, PartialEq, Eq)]
    pub enum MouseVerdict {
        Pass,
        Drop,
        Wheel { dir: i8, count: u32 },
    }

    /// mousefilter.ts:34 — urxvt·X10 은 좌표/버튼을 32 오프셋으로 싣는다(제어문자 회피).
    const OFFSET: u16 = 32;

    /// bracketed paste 개시 시퀀스 — A9 계약: 이 접두가 붙은 텍스트는 **무조건 비면제**
    /// (2004-off 상태의 ESC 붙여넣기 등 극저확률 역방향 창까지 명시적으로 봉인 — R3 RISK 4-①).
    pub const BRACKETED_PASTE_PREFIX: &str = "\u{1b}[200~";

    /// mousefilter.ts:40-46 `wheelDelta` 동형 — 휠 비트 64·확장버튼 128 배제(0xC0 마스크),
    /// 하위 2비트만 방향(0=업, 1=다운, 2/3=가로휠→0). button 은 JS `Number(..) & 0xc0` 의
    /// ToInt32 절단과 등가가 되도록 u64 → u32 wrapping 캐스트를 거친다(2^53 초과는 JS f64
    /// 반올림과 갈릴 수 있으나 실보고 값 범위 밖 — 이론 한계 주석).
    fn wheel_delta(button: u64) -> i32 {
        let b = button as u32;
        if (b & 0xc0) != 0x40 {
            return 0;
        }
        match b & 3 {
            0 => -1, // 휠업 → 위로 스크롤
            1 => 1,  // 휠다운 → 아래로 스크롤
            _ => 0,
        }
    }

    /// mousefilter.ts:48-51 `Hit` 동형 — end = 보고가 끝나는 인덱스(다음 스캔 시작점).
    struct Hit {
        end: usize,
        wheel: i32,
    }

    /// `u[i..]` 에서 십진 숫자(≥1자리)를 읽는다. JS `\d`(ASCII 0-9 한정) 동형.
    /// 값은 u64 wrapping 누적 — wheel_delta 의 ToInt32 절단 주석 참조.
    fn read_digits(u: &[u16], mut i: usize) -> Option<(u64, usize)> {
        let start = i;
        let mut v: u64 = 0;
        while i < u.len() && (0x30..=0x39).contains(&u[i]) {
            v = v.wrapping_mul(10).wrapping_add((u[i] - 0x30) as u64);
            i += 1;
        }
        if i == start {
            None
        } else {
            Some((v, i))
        }
    }

    /// mousefilter.ts:53-85 `matchReport` 동형 — u[from] 에서 시작하는 보고 하나를 매칭.
    /// 시도 순서(SGR → X10 → urxvt)와 각 분기의 non-match/null 반환 지점까지 동일하다.
    /// 입력은 UTF-16 코드유닛 열 — TS 의 charCodeAt/정규식 lastIndex 의미론과 정확히 겹친다.
    fn match_report(u: &[u16], from: usize) -> Option<Hit> {
        let esc = 0x1b_u16;
        // RE_SGR(ts:29): ESC [ < d+ ; d+ ; d+ [Mm]  — SGR(1006)과 SGR-Pixels(1016) 공통
        // (픽셀 좌표는 대형 십진수 — 자릿수 제한 없음).
        if u.len() > from + 2 && u[from] == esc && u[from + 1] == b'[' as u16 && u[from + 2] == b'<' as u16 {
            if let Some((b, i)) = read_digits(u, from + 3) {
                if u.get(i) == Some(&(b';' as u16)) {
                    if let Some((_x, i)) = read_digits(u, i + 1) {
                        if u.get(i) == Some(&(b';' as u16)) {
                            if let Some((_y, i)) = read_digits(u, i + 1) {
                                if u.get(i) == Some(&(b'M' as u16)) || u.get(i) == Some(&(b'm' as u16)) {
                                    return Some(Hit { end: i + 1, wheel: wheel_delta(b) });
                                }
                            }
                        }
                    }
                }
            }
        }
        // RE_X10(ts:31): ESC [ M + 원시 3 코드유닛. ts:60-72 — 정규식은 자릿수만 세므로
        // 값 하한(셋 다 OFFSET 이상)을 별도 검증하고, 매치 후 하한 미달이면 **null 반환**
        // (urxvt 재시도 없이 — TS 의 return null 과 동일 구조. ESC[M 은 urxvt 문법과
        // 겹치지 않아 행동 차이는 없다).
        if u.len() > from + 5 && u[from] == esc && u[from + 1] == b'[' as u16 && u[from + 2] == b'M' as u16 {
            let (b, x, y) = (u[from + 3], u[from + 4], u[from + 5]);
            if b >= OFFSET && x >= OFFSET && y >= OFFSET {
                return Some(Hit { end: from + 6, wheel: wheel_delta((b - OFFSET) as u64) });
            }
            return None;
        }
        // RE_URXVT(ts:30): ESC [ d+ ; d+ ; d+ M — 접두가 평범해 오탐 여지(ts:78-80):
        // 버튼(원시-32 ≥ 0)·좌표(≥ OFFSET) 하한을 만족할 때만 마우스로 인정.
        if u.len() > from + 1 && u[from] == esc && u[from + 1] == b'[' as u16 {
            if let Some((braw, i)) = read_digits(u, from + 2) {
                if u.get(i) == Some(&(b';' as u16)) {
                    if let Some((x, i)) = read_digits(u, i + 1) {
                        if u.get(i) == Some(&(b';' as u16)) {
                            if let Some((y, i)) = read_digits(u, i + 1) {
                                if u.get(i) == Some(&(b'M' as u16))
                                    && braw >= OFFSET as u64
                                    && x >= OFFSET as u64
                                    && y >= OFFSET as u64
                                {
                                    return Some(Hit { end: i + 1, wheel: wheel_delta(braw - OFFSET as u64) });
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// mousefilter.ts:96-112 `classifyMouseReport` 동형 — 청크 **전체가 보고로만 구성**될 때만
    /// Drop/Wheel, 그 외(혼합·절단·빈 청크)는 전부 Pass. 혼합을 Pass 로 두는 비대칭 근거
    /// (오폐기=입력 소실 > 유출 1회)는 TS 머리 주석(ts:89-95) 참조.
    pub fn classify_mouse_report(text: &str) -> MouseVerdict {
        if text.is_empty() {
            return MouseVerdict::Pass; // 빈 청크는 손대지 않는다(ts:97)
        }
        let u: Vec<u16> = text.encode_utf16().collect();
        let mut i = 0usize;
        let mut reports = 0u32;
        let mut net: i64 = 0; // 휠 노치 순증감(+아래/-위) — 배칭 청크를 한 번의 스크롤로 접는다
        while i < u.len() {
            let Some(hit) = match_report(&u, i) else {
                return MouseVerdict::Pass; // 마우스 아닌 유닛이 하나라도 있으면 통째로 통과(ts:104)
            };
            reports += 1;
            net += hit.wheel as i64;
            i = hit.end;
        }
        if reports == 0 {
            return MouseVerdict::Pass; // 도달 불가(빈 문자열은 위에서 컷) — 방어(ts:109)
        }
        if net == 0 {
            return MouseVerdict::Drop; // 휠 아님, 또는 상쇄(ts:110)
        }
        MouseVerdict::Wheel {
            dir: if net > 0 { 1 } else { -1 },
            count: net.unsigned_abs().min(u32::MAX as u64) as u32,
        }
    }

    /// A9 면제 술어 — cysd `surface.send_text` human 경로가 소비한다.
    /// 참 = '수신 텍스트 전체가 마우스 보고 시퀀스의 연접'(classify != Pass) ∧ bracketed
    /// paste 접두 아님. **참일 때만** last_human_input 갱신을 생략한다(순수 보고=미갱신·
    /// 비순수(혼합·절단·paste 래퍼)=갱신 — 회귀 핀은 코퍼스+handlers 테스트).
    pub fn is_pure_mouse_report(text: &str) -> bool {
        if text.starts_with(BRACKETED_PASTE_PREFIX) {
            return false; // 무조건 비면제(스펙 §D4+A9 명문)
        }
        classify_mouse_report(text) != MouseVerdict::Pass
    }

    /// `u[i]` 를 ASCII 바이트로 — 비-ASCII 코드유닛·범위 밖은 None.
    /// (자동 응답 문법은 전부 ASCII 다 — 비-ASCII 가 섞였다는 건 사람 글자라는 뜻이다.)
    fn ascii_at(u: &[u16], i: usize) -> Option<u8> {
        match u.get(i).copied() {
            Some(c) if c < 0x80 => Some(c as u8),
            _ => None,
        }
    }

    /// `u[i..]` 가 ASCII 리터럴 `lit` 로 시작하는가(경계 초과는 거짓 — 절단 방어).
    fn starts_with_ascii(u: &[u16], i: usize, lit: &[u8]) -> bool {
        lit.iter().enumerate().all(|(k, &b)| ascii_at(u, i + k) == Some(b))
    }

    /// CSI 응답의 파라미터 열 `d+(;d+)*` 를 읽는다 → (파라미터 **개수**, 다음 인덱스).
    /// 값은 판별에 쓰지 않으므로 버린다. 숫자를 하나도 못 읽거나 `;` 뒤가 숫자가 아니면
    /// None — 빈 파라미터(`ESC[?;c` 같은 형태)는 **미인식**으로 접어 사람 입력 쪽에 둔다.
    fn read_params(u: &[u16], from: usize) -> Option<(usize, usize)> {
        let (_, mut i) = read_digits(u, from)?;
        let mut n = 1usize;
        while ascii_at(u, i) == Some(b';') {
            let (_, j) = read_digits(u, i + 1)?;
            i = j;
            n += 1;
        }
        Some((n, i))
    }

    /// 터미널 자동 응답 **하나**를 `u[from]` 에서 매칭 → 성공 시 끝 인덱스(다음 스캔 시작점).
    /// `match_report`(마우스)와 같은 계약이다 — 부분 일치·의심스러운 형태는 전부 None 이다.
    fn match_terminal_autoreply(u: &[u16], from: usize) -> Option<usize> {
        const ESC: u8 = 0x1b;
        const BEL: u8 = 0x07;
        if ascii_at(u, from) != Some(ESC) {
            return None; // 자동 응답은 예외 없이 ESC 로 시작한다
        }
        // ① 마우스 보고 — 정본 매처에 위임(SGR·X10·urxvt 전 인코딩·좌표 하한 검증 포함).
        //    이 위임이 `is_pure_terminal_autoreply` 를 `is_pure_mouse_report` 의 상위 집합으로
        //    만든다(A9 면제는 그대로 살아 있고, 그 위에 자동 응답이 얹힌다).
        if let Some(hit) = match_report(u, from) {
            return Some(hit.end);
        }
        match ascii_at(u, from + 1)? {
            // ② CSI 계열 — ESC [
            b'[' => match ascii_at(u, from + 2) {
                // 포커스 보고(DECSET 1004). Claude Code 가 기동 시 1004h 를 켜므로 pane 을
                // 클릭·이탈할 때마다 흐른다 = 결함3 최다 발생원.
                Some(b'I') | Some(b'O') => Some(from + 3),
                // private 파라미터 응답 — DA1 / DECXCPR / kitty 플래그 / DECRPM 이 여기 모인다.
                Some(b'?') => {
                    let (n, i) = read_params(u, from + 3)?;
                    match ascii_at(u, i)? {
                        // DA1 `ESC[?<n>(;<n>)*c` — 파라미터 개수는 터미널마다 달라 제한 없음.
                        b'c' => Some(i + 1),
                        // DECXCPR `ESC[?<row>;<col>[;<page>]R` — `ESC[?6n` 의 응답.
                        b'R' if (2..=3).contains(&n) => Some(i + 1),
                        // kitty 키보드 프로토콜 플래그 `ESC[?<flags>u` — 파라미터 1개.
                        b'u' if n == 1 => Some(i + 1),
                        // DECRPM `ESC[?<mode>;<value>$y` — DECRQM(모드 질의)의 응답.
                        b'$' if n == 2 && ascii_at(u, i + 1) == Some(b'y') => Some(i + 2),
                        _ => None,
                    }
                }
                // DA2 `ESC[><n>(;<n>)*c` — 2차 장치 속성.
                Some(b'>') => {
                    let (_, i) = read_params(u, from + 3)?;
                    (ascii_at(u, i) == Some(b'c')).then_some(i + 1)
                }
                // CPR `ESC[<row>;<col>R` — `ESC[6n` 의 응답(파라미터 정확히 2개).
                // 숫자로 시작하는 다른 CSI(urxvt 마우스 `…M` 등)는 위 ① 이 이미 걸렀고,
                // 걸리지 않은 것은 종결자 불일치로 여기서 None 이 된다(보수적).
                _ => {
                    let (n, i) = read_params(u, from + 2)?;
                    (n == 2 && ascii_at(u, i) == Some(b'R')).then_some(i + 1)
                }
            },
            // ③ DCS 계열 — ESC P … ESC \  (DA3 / XTVERSION)
            b'P' => {
                let hex_only = if starts_with_ascii(u, from + 2, b"!|") {
                    true // DA3 `ESC P!|<hex>ESC\` — 16진 단말 유닛 ID.
                } else if starts_with_ascii(u, from + 2, b">|") {
                    false // XTVERSION `ESC P>|<이름/버전>ESC\`.
                } else {
                    return None;
                };
                let mut i = from + 4;
                loop {
                    match ascii_at(u, i) {
                        // 본문은 좁게 받는다 — DA3 는 16진만, XTVERSION 은 인쇄가능 ASCII 만.
                        // 임의 바이트를 삼키면 사람 입력이 DCS 뒤에 숨어 면제받을 수 있다.
                        Some(c) if (hex_only && c.is_ascii_hexdigit())
                            || (!hex_only && (0x20..=0x7e).contains(&c)) =>
                        {
                            i += 1
                        }
                        _ => break,
                    }
                }
                if i == from + 4 {
                    return None; // 빈 본문 = 미인식(절단 방어)
                }
                starts_with_ascii(u, i, b"\x1b\\").then_some(i + 2)
            }
            // ④ OSC 색 질의 응답 — ESC ] 1[012] ; <본문> (BEL | ESC \)
            //    10=전경 11=배경 12=커서. 종결자 두 형태를 모두 받는다(터미널마다 다름).
            b']' => {
                if ascii_at(u, from + 2) != Some(b'1')
                    || !matches!(ascii_at(u, from + 3), Some(b'0') | Some(b'1') | Some(b'2'))
                    || ascii_at(u, from + 4) != Some(b';')
                {
                    return None;
                }
                let mut i = from + 5;
                while let Some(c) = ascii_at(u, i) {
                    if !(0x20..=0x7e).contains(&c) {
                        break; // 제어문자(BEL·ESC 포함) = 본문 끝
                    }
                    i += 1;
                }
                if i == from + 5 {
                    return None; // 빈 본문 = 미인식
                }
                match ascii_at(u, i) {
                    Some(BEL) => Some(i + 1),
                    Some(ESC) => starts_with_ascii(u, i, b"\x1b\\").then_some(i + 2),
                    _ => None, // 종결자 없음 = 절단
                }
            }
            _ => None,
        }
    }

    /// ★B1(0.14.24) 터미널 **자동 응답** 술어 — cysd `surface.send_text` human 경로가 소비한다.
    /// `is_pure_mouse_report` 의 **상위 집합**이다(순수 마우스 보고면 이 술어도 참).
    ///
    /// 왜 필요한가(결함3 주범): GUI 는 `term.onData` 로 나오는 **모든** 바이트를
    /// `send_input(human=true)` 로 데몬에 올린다(`ui/src/main.ts`). 그런데 onData 에는 사람이
    /// 친 글자만 오지 않는다 — Claude Code 는 기동 시 포커스 보고(`ESC[?1004h`)를 켜므로 pane 을
    /// 클릭·이탈할 때마다 `ESC[I`/`ESC[O` 가 흐르고(`ui/src/trackfilter.ts` 가 1004 를 보존한다),
    /// 커서위치 질의(`ESC[6n`·`ESC[?6n`)·DA·XTVERSION·DECRPM·kitty 플래그·OSC 색 질의의
    /// **응답**도 같은 경로로 올라온다. 이것들이 `last_human_input` 을 찍으면 이후 3초
    /// (`typing_guard_secs`) 동안 다른 노드의 `send-key Return` 이 `typing_guard` 로 거부된다 —
    /// **오너가 master pane 을 클릭해 보고를 읽는 순간부터 노드 보고의 제출 Enter 가 먹지 않는다.**
    ///
    /// 판정 규약(A9 와 같은 보수성): 청크 **전체**가 아래 시퀀스의 연접일 때만 참이다. 사람
    /// 글자가 하나라도 섞이면(혼합)·시퀀스가 잘렸으면(절단)·빈 문자열이면 거짓이고,
    /// `ESC[200~`(bracketed paste) 접두는 **무조건 거짓**이다(A9 규약 유지). 위험한 방향은
    /// 오폐기가 아니라 **오면제**(사람 입력을 기계로 오인해 가드를 안 켜는 것)이므로 애매하면
    /// 전부 거짓으로 접는다.
    ///
    /// 인식 목록: 마우스 보고(기존 매처 위임) · 포커스 `ESC[I`/`ESC[O` · CPR `ESC[<n>;<n>R` ·
    /// DECXCPR `ESC[?<n>;<n>[;<n>]R` · DA1 `ESC[?<n>(;<n>)*c` · DA2 `ESC[><n>(;<n>)*c` ·
    /// DA3 `ESC P!|<hex>ESC\` · XTVERSION `ESC P>|<텍스트>ESC\` · DECRPM `ESC[?<n>;<n>$y` ·
    /// kitty 키보드 플래그 `ESC[?<n>u` · OSC 10/11/12 색 응답 `ESC]1[012];<본문>(BEL|ESC\)`.
    pub fn is_pure_terminal_autoreply(text: &str) -> bool {
        if text.is_empty() || text.starts_with(BRACKETED_PASTE_PREFIX) {
            return false;
        }
        let u: Vec<u16> = text.encode_utf16().collect();
        let mut i = 0usize;
        while i < u.len() {
            match match_terminal_autoreply(&u, i) {
                // end > i 는 무한 루프 방어 — 매처가 진전 없이 성공하는 일은 없어야 한다.
                Some(end) if end > i => i = end,
                _ => return false, // 자동 응답 아닌 유닛이 하나라도 있으면 통째로 거짓
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {

    // ── ★(U-10) 좌석 제4 등급 `gate_pending` 축: 롤백 킬스위치 + wire 술어 순수 코어 ──
    //   env 를 만지는 래퍼가 아니라 **순수 코어**를 잰다(cargo test 는 스레드 병렬이라
    //   env 변조 테스트는 형제 테스트를 오염시킨다 — d5 판독기 핀과 같은 분리 이유).
    #[test]
    fn gate_pending_kill_switch_only_zero_disables() {
        // 기본(미설정) = 축 노출. 이 단위엔 생산자가 없어 기본값이 곧 종전 동작이다.
        assert!(super::gate_pending_axis_enabled_from(None), "미설정에서 축이 꺼졌다");
        assert!(!super::gate_pending_axis_enabled_from(Some("0")), "\"0\" 이 축을 끄지 못한다(롤백 불능)");
        // 느슨한 falsy 로는 꺼지지 않는다 — 오타 한 글자로 안전장치가 조용히 꺼지는 사고 방지.
        for loose in ["", "false", "off", "no", "1", "00", " 0"] {
            assert!(
                super::gate_pending_axis_enabled_from(Some(loose)),
                "느슨한 값 {loose:?} 가 축을 껐다(엄격 비교 회귀)"
            );
        }
    }

    #[test]
    fn gate_pending_wire_predicate_folds_unknown_shapes_to_no_signal() {
        use serde_json::json;
        let obj = json!({"gate": "disclaimer", "since": 1.0});
        assert!(super::gate_pending_from_wire_with(true, &obj), "object 가 보류로 읽히지 않는다");
        // ★null·키 부재 = '이 축에 대해 말할 것이 없음'(구 데몬). 부정이 아니다 — 종전 판정으로 흐른다.
        for v in [json!(null), json!(true), json!(false), json!(0), json!("gated"), json!([])] {
            assert!(
                !super::gate_pending_from_wire_with(true, &v),
                "비 object 값 {v} 이 보류로 접혔다(판정불가→미충족 = 부트 라이브락 경로)"
            );
        }
        // 킬스위치 off 면 값과 무관하게 무신호(축 통째 생략) — 단일 지점 롤백의 계약.
        assert!(!super::gate_pending_from_wire_with(false, &obj),
                "킬스위치 off 인데 축이 살아 있다(롤백 1지점 계약 붕괴)");
    }

    #[test]
    fn gate_pending_wire_key_is_the_single_name() {
        // 데몬 2메서드·topology·python 미러가 공유하는 키 이름. 바뀌면 축이 조용히 사라진다.
        assert_eq!(super::GATE_PENDING_KEY, "gate_pending");
        assert_eq!(super::ENV_GATE_PENDING, "CYS_GATE_PENDING");
    }

    // ── ★(U-11) 보류 귀결: 만료 규약 · 롤백 킬스위치 · 종료코드 ──
    #[test]
    fn gate_pending_ttl_expires_only_past_the_bound_and_folds_unknown_time_to_fresh() {
        let ttl = super::GATE_PENDING_TTL_SECS;
        assert!(super::gate_pending_fresh(1000.0, 1000.0, ttl), "갓 찍은 표식이 만료로 읽힌다");
        assert!(super::gate_pending_fresh(1000.0, 1000.0 + ttl, ttl), "경계값이 만료로 접혔다");
        assert!(!super::gate_pending_fresh(1000.0, 1000.0 + ttl + 1.0, ttl),
                "TTL 초과 표식이 살아남는다 — 부트 라이브락 상한 붕괴");
        // 시계 되돌림·스큐·NaN = 판정 불가 → **유효**(관측 유지). 만료로 접으면 보류가 조용히
        // already_alive 로 되돌아간다 — 이 단위가 없애려는 바로 그 상태다.
        assert!(super::gate_pending_fresh(2000.0, 1000.0, ttl), "미래 since 가 만료로 접혔다");
        assert!(super::gate_pending_fresh(f64::NAN, 1000.0, ttl), "NaN since 가 만료로 접혔다");
        assert!(super::gate_pending_fresh(1000.0, f64::NAN, ttl), "NaN now 가 만료로 접혔다");
    }

    #[test]
    fn gate_pending_close_override_folds_both_switches_so_rollback_is_never_half() {
        // 기본(둘 다 미설정) = 신동작(보류).
        assert!(!super::gate_pending_close_override_from(None, None),
                "기본값이 종전 close 다(신동작 소실)");
        // 어느 하나만 눌러도 기능 **전체**가 꺼진다 — 반쪽 롤백(pane 은 남는데 좌석은
        // already_alive)이 가장 위험한 상태다.
        assert!(super::gate_pending_close_override_from(Some("1"), None),
                "보류 롤백 스위치가 듣지 않는다");
        assert!(super::gate_pending_close_override_from(None, Some("0")),
                "축 스위치를 껐는데 CLI 가 계속 보류한다 — 관측 없는 보류(허위 READY)");
        assert!(super::gate_pending_close_override_from(Some("1"), Some("0")),
                "두 스위치 동시 사용이 서로를 상쇄했다");
    }

    #[test]
    fn gate_pending_close_switch_only_literal_one_reverts() {
        assert!(!super::gate_pending_close_from(None), "미설정이 종전 close 로 접혔다(신동작 소실)");
        assert!(super::gate_pending_close_from(Some("1")), "\"1\" 이 롤백을 켜지 못한다");
        for loose in ["true", "yes", "on", "", "0", "01", " 1"] {
            assert!(!super::gate_pending_close_from(Some(loose)),
                    "느슨한 값 {loose:?} 이 롤백을 켰다(오타로 안전장치가 뒤집힘)");
        }
    }

    // ── ★(BLOCK-3 · BLOCK-4 · 2026-08-24) 마스터 롤백 스위치 · '엄격+즉시close' 불변식 ──

    /// ★BLOCK-4 전수 진리표 — 관련 env **전 조합**(5값 × 7축 = 78,125)에서
    /// "엄격 판정 + 즉시 close" 가 **성립하지 않음**을 단언한다.
    ///
    /// 이 조합이 성립하면 관문 화면이 영원히 ready 가 아니고 → readiness 타임아웃 →
    /// `LaunchFailed` 강등 → 호출부 `surface.close` 로 **전 pane 이 죽는다**. 문서화된 롤백
    /// 스위치가 기저보다 파괴적인 상태를 만드는 것이 재난④의 정체다.
    ///
    /// ★(U-17) 축이 여섯에서 일곱으로 늘었다 — 핀을 **지우지 않고 이사**시켰다: 기존 세 축
    /// 단언은 그대로 두고 `profile_gate_observe_only` 를 같은 AND 사슬에 **추가**한다.
    /// 새 축만 엄격하게 남으면 마스터 스위치가 다시 거짓말이 되기 때문이다(완화가 아니라 강화).
    #[test]
    fn strict_judgment_and_immediate_close_is_unreachable_in_every_env_combination() {
        const VALS: [Option<&str>; 5] = [None, Some(""), Some("0"), Some("1"), Some("true")];
        let (mut close_seen, mut master_seen) = (0usize, 0usize);
        for m in VALS {
            for r in VALS {
                for g in VALS {
                    for t in VALS {
                        for c in VALS {
                            for a in VALS {
                                for p in VALS {
                                let ax = super::gate_axes_from(m, r, g, t, c, a, p);
                                if ax.gate_pending_close {
                                    close_seen += 1;
                                    assert!(
                                        ax.readiness_legacy
                                            && ax.inject_guard_off
                                            && ax.trust_legacy
                                            && ax.profile_gate_observe_only,
                                        "★재난④ 조합: 보류가 close 로 강등됐는데 판정 축이 \
                                         엄격하게 남았다 — master={m:?} readiness={r:?} \
                                         guard={g:?} trust={t:?} close={c:?} axis={a:?} \
                                         profile={p:?} → {ax:?}"
                                    );
                                }
                                if super::boot_gates_master_off_from(m) {
                                    master_seen += 1;
                                    assert!(
                                        ax.readiness_legacy
                                            && ax.inject_guard_off
                                            && ax.trust_legacy
                                            && ax.gate_pending_close
                                            && ax.profile_gate_observe_only,
                                        "마스터 스위치가 전 축을 되돌리지 못했다 → {ax:?}"
                                    );
                                }
                                }
                            }
                        }
                    }
                }
            }
        }
        // 진리표가 **공허하지 않다**(해당 조합이 실제로 존재한다)는 것까지 확인한다.
        assert!(close_seen > 0 && master_seen > 0,
                "진리표가 대상 조합을 하나도 밟지 않았다(close={close_seen} master={master_seen})");
    }

    /// ★계측 타당성(in-band) — **수리 전 조립**에서는 그 조합이 실제로 성립했다.
    #[test]
    fn pre_fix_composition_reproduced_strict_judgment_with_immediate_close() {
        // 구 조립: close 강등은 두 축 스위치의 OR 였고(`CYS_GATE_PENDING=0` 단독으로 참),
        // readiness 는 **자기 노브만** 봤다(미설정 = 엄격).
        let close = super::gate_pending_close_override_from(None, Some("0"));
        let strict = !crate::readiness::legacy_v1_from(None);
        assert!(
            close && strict,
            "계측 무효: 구 조립에서 '엄격 + 즉시 close' 가 성립하지 않는다면 BLOCK-4 서사가 틀린 것"
        );
        // 지금 조립은 같은 입력에서 판정도 함께 종전으로 푼다.
        let ax = super::gate_axes_from(None, None, None, None, None, Some("0"), None);
        assert!(ax.gate_pending_close && ax.readiness_legacy && ax.inject_guard_off
                    && ax.profile_gate_observe_only,
                "축 스위치 단독이 여전히 엄격 판정을 남긴다 → {ax:?}");
    }

    /// ★BLOCK-3 — 개별 노브는 **자기 축만** 끄고(리뷰어 4칸 진리표의 사실), 종전 동작 전체
    /// 복귀는 **마스터 스위치 하나**로 된다.
    #[test]
    fn master_switch_alone_restores_the_previous_behavior_on_every_axis() {
        // ① `CYS_READINESS_V1=1` 단독 — ready 는 나지만 주입 가드가 그대로다(= 여전히 rc 78).
        let only_v1 = super::gate_axes_from(None, Some("1"), None, None, None, None, None);
        assert!(only_v1.readiness_legacy);
        assert!(
            !only_v1.inject_guard_off,
            "V1 단독이 주입 가드까지 껐다면 리뷰어 4칸 진리표(종전 복귀 조합은 둘)가 틀린 것 — \
             이 대조군이 마스터 스위치의 존재 이유다"
        );
        // ② `CYS_INJECT_GATE_GUARD=0` 단독 — 가드만 열리고 판정은 엄격(관문 화면은 보류).
        let only_guard = super::gate_axes_from(None, None, Some("0"), None, None, None, None);
        assert!(only_guard.inject_guard_off && !only_guard.readiness_legacy);
        // ③ 리뷰어가 찾은 '종전 복귀의 유일한 조합' — 사람이 둘을 동시에 기억해야 했다.
        let both = super::gate_axes_from(None, Some("1"), Some("0"), None, None, None, None);
        assert!(both.readiness_legacy && both.inject_guard_off);
        assert!(!both.gate_pending_close, "노브 둘이 보류 귀결까지 바꾸면 축 경계가 무너진다");
        // ④ ★마스터 하나 = 네 축 전부 종전.
        let master = super::gate_axes_from(Some("0"), None, None, None, None, None, None);
        assert_eq!(
            master,
            super::GateAxes {
                readiness_legacy: true,
                inject_guard_off: true,
                trust_legacy: true,
                gate_pending_close: true,
                profile_gate_observe_only: true,
            },
            "마스터 스위치가 '하나를 끄면 전부 복귀' 계약을 지키지 못한다"
        );
        // ⑤ 엄격 비교 — 오타로 안전장치가 조용히 뒤집히지 않는다(형제 게이트와 같은 규율).
        for loose in [None, Some(""), Some("off"), Some("false"), Some("no"), Some(" 0"), Some("1")] {
            assert!(!super::boot_gates_master_off_from(loose),
                    "마스터 스위치가 느슨한 값 {loose:?} 을 받아들였다");
        }
        assert!(super::boot_gates_master_off_from(Some("0")));
        assert_eq!(super::ENV_BOOT_GATES, "CYS_BOOT_GATES");
    }

    /// ★소스 핀 — 축 노브 셋이 **전부** 상위 접기값을 OR 한다. 하나라도 빠지면 마스터
    /// 스위치가 거짓말이 되고, 그 축만 엄격하게 남아 BLOCK-4 경로가 되살아난다.
    #[test]
    fn every_axis_knob_folds_in_the_master_switch_source_pin() {
        const FOLD: &str = "|| crate::gate_axes_forced_legacy()";
        let r = include_str!("readiness.rs");
        let g = include_str!("inject_guard.rs");
        // ★(U-17) 새 축도 같은 계약을 진다 — 핀을 지우지 않고 항목을 **추가**한다.
        let pg_all = include_str!("profile_gate.rs");
        let pg = &pg_all[..pg_all.find("#[cfg(test)]").expect("profile_gate 테스트 앵커 부재")];
        assert_eq!(r.matches(FOLD).count(), 1, "readiness 축이 상위 접기값을 소비하지 않는다");
        assert_eq!(
            g.matches(FOLD).count(),
            2,
            "inject_guard 의 두 축(가드·신뢰) 중 하나가 상위 접기값을 소비하지 않는다"
        );
        assert_eq!(
            pg.matches(FOLD).count(),
            1,
            "profile_gate 축이 상위 접기값을 소비하지 않는다 — 마스터 스위치를 눌러도 인증 \
             판정기만 엄격하게 남는다"
        );
        // 그리고 축별 env 판독은 여전히 1지점이다(형제 검체가 세는 계약을 깨지 않았다).
        assert_eq!(r.matches("std::env::var(ENV_V1)").count(), 1);
        assert_eq!(g.matches("std::env::var(ENV_GUARD_OFF)").count(), 1);
        assert_eq!(g.matches("std::env::var(ENV_TRUST_V1)").count(), 1);
        assert_eq!(pg.matches("std::env::var(ENV_OBSERVE_ONLY)").count(), 1);
    }

    /// ★(BLOCK-3 잔여분 · 2026-08-24) **마스터 스위치가 데몬까지 닿는다.**
    ///
    /// 데몬의 유일한 직렬화 지점(`cysd/state.rs::gate_pending_wire`)은 술어 하나
    /// (`gate_pending_axis_enabled`)를 본다. 종전엔 그 술어가 `CYS_GATE_PENDING` **만** 읽어,
    /// 마스터(`CYS_BOOT_GATES=0`)를 눌러도 이미 실린 표식이 **TTL 30분까지 계속 직렬화**됐다 —
    /// CLI 는 종전 판정으로 돌아갔는데 데몬은 여전히 좌석을 보류로 내보내는 **반쪽 롤백**이다.
    #[test]
    fn master_switch_reaches_the_daemon_serialization_axis() {
        fn on(m: Option<&str>, c: Option<&str>, a: Option<&str>) -> bool {
            super::gate_pending_axis_effective_from(m, c, a)
        }
        // ① 기본(전 스위치 미설정) = 축 노출(신동작). 여기가 뒤집히면 기능이 통째로 사라진다.
        assert!(on(None, None, None), "기본에서 축이 꺼졌다");
        // ② 세 스위치 **각각 단독**으로 축을 끈다 — 어느 손잡이를 눌러도 기능 전체가 꺼진다.
        assert!(
            !on(Some("0"), None, None),
            "★마스터 스위치가 데몬 직렬화 축에 닿지 않는다 — 눌러도 표식이 TTL 까지 계속 나간다"
        );
        assert!(!on(None, Some("1"), None), "강등 스위치가 축을 끄지 못한다(반쪽 롤백)");
        assert!(!on(None, None, Some("0")), "축 스위치가 축을 끄지 못한다");
        // ③ 엄격 비교 — 오타 하나로 안전장치가 조용히 뒤집히지 않는다(형제 게이트와 같은 규율).
        for loose in [Some(""), Some("false"), Some("off"), Some("no"), Some(" 0"), Some("1")] {
            assert!(on(loose, None, None), "마스터가 느슨한 값 {loose:?} 을 받았다");
        }
        for loose in [Some(""), Some("true"), Some("yes"), Some("on"), Some(" 1")] {
            assert!(on(None, loose, None), "강등 스위치가 느슨한 값 {loose:?} 을 받았다");
            assert!(on(None, None, loose), "축 스위치가 느슨한 값 {loose:?} 을 받았다");
        }
        // ④ **불변식 단일 소유** — 축 노출은 언제나 보류 귀결의 부정이다(전 조합 대조).
        //   갈리는 순간 둘 중 하나다: 관측 없는 보류(허위 READY) 또는 귀결 없는 관측(영구 미충족).
        const VALS: [Option<&str>; 5] = [None, Some(""), Some("0"), Some("1"), Some("true")];
        for m in VALS {
            for c in VALS {
                for a in VALS {
                    let ax = super::gate_axes_from(m, None, None, None, c, a, None);
                    assert_eq!(
                        on(m, c, a),
                        !ax.gate_pending_close,
                        "축 노출과 보류 귀결이 갈렸다: master={m:?} close={c:?} axis={a:?}"
                    );
                }
            }
        }
        // ⑤ 소스 핀 — 데몬 직렬화 지점이 자기 판정을 복제하지 않고 이 술어를 경유한다.
        let st = include_str!("bin/cysd/state.rs");
        assert!(
            st.contains("if !cys::gate_pending_axis_enabled() {"),
            "데몬 직렬화 지점이 축 술어를 경유하지 않는다 — 롤백이 다시 두 갈래가 된다"
        );
        // ⑥ 그리고 래퍼는 세 env 를 **전부** 읽는다 — 하나라도 빠지면 그 손잡이가 데몬에
        //   닿지 않는다(그것이 이 결함의 정체였다: 마스터를 눌러도 데몬은 못 들었다).
        let lib = include_str!("lib.rs");
        let head = &lib[lib
            .find("pub fn gate_pending_axis_enabled() -> bool {")
            .expect("축 술어 래퍼 소실")..];
        let body = &head[..head.find("\n}\n").expect("래퍼 본문 경계 소실")];
        for env in [
            "std::env::var(ENV_BOOT_GATES)",
            "std::env::var(ENV_GATE_PENDING_CLOSE)",
            "std::env::var(ENV_GATE_PENDING)",
        ] {
            assert!(
                body.contains(env),
                "축 술어가 `{env}` 를 읽지 않는다 — 그 손잡이는 데몬에 닿지 않는다"
            );
        }
    }

    #[test]
    fn gate_pending_exit_code_stays_outside_the_gate_exit_space() {
        // 2~11 은 javis_bootstrap 헤더 표가 소유한 만원 공간이다(H-DOC-4 유령·결손 대조).
        assert_eq!(super::EXIT_GATE_PENDING, 78);
        assert!(!(2..=11).contains(&super::EXIT_GATE_PENDING),
                "게이트 exit 공간을 침범했다 — 헤더 표와 충돌");
        assert_ne!(super::EXIT_GATE_PENDING, super::EXIT_BOOT_BUSY, "형제 sysexits 값과 충돌");
        assert_eq!(super::ENV_GATE_PENDING_CLOSE, "CYS_GATE_PENDING_CLOSE");
    }
    use super::*;

    #[test]
    fn surface_refs() {
        assert_eq!(parse_surface_ref("surface:31"), Some(31));
        assert_eq!(parse_surface_ref("31"), Some(31));
        assert_eq!(parse_surface_ref("x"), None);
    }

    /// ★busy 백오프 정책 핀(decorrelated jitter): 어떤 입력에도 결과는
    /// [RETRY_INTERVAL, BACKOFF_CAP] 안이어야 한다 — 0 은 busy spin, 과대는 5s 데드라인 내
    /// 재시도 횟수 고갈. 순수 함수라 비-Windows 에서 정책 전체를 박제한다.
    #[test]
    fn next_busy_delay_stays_within_policy_bounds() {
        use std::time::Duration;
        // 결정 경계: rand01=0 → 하한, rand01=1 → min(prev*3, cap).
        assert_eq!(
            next_busy_delay(PIPE_BUSY_RETRY_INTERVAL, 0.0),
            PIPE_BUSY_RETRY_INTERVAL,
            "rand=0 은 하한(즉시 리듬) — 하한이 무너지면 busy spin"
        );
        assert_eq!(
            next_busy_delay(PIPE_BUSY_RETRY_INTERVAL, 1.0),
            PIPE_BUSY_RETRY_INTERVAL * 3,
            "rand=1 은 prev*3 (decorrelated jitter 상단)"
        );
        assert_eq!(
            next_busy_delay(PIPE_BUSY_BACKOFF_CAP, 1.0),
            PIPE_BUSY_BACKOFF_CAP,
            "prev*3 이 cap 을 넘으면 cap 클램프 — 상한 없는 증가는 재시도 고갈"
        );
        // prev=0(오염)·rand 오염(NaN·음수·>1)도 전부 구간 안 — fail-closed 클램프.
        for prev in [Duration::ZERO, Duration::from_secs(3600)] {
            for r in [f64::NAN, f64::INFINITY, -1.0, 0.5, 2.0] {
                let d = next_busy_delay(prev, r);
                assert!(
                    (PIPE_BUSY_RETRY_INTERVAL..=PIPE_BUSY_BACKOFF_CAP).contains(&d),
                    "구간 이탈: prev={prev:?} rand={r} → {d:?}"
                );
            }
        }
        // 최악 대기(wait slice + cap)로도 데드라인 안에 재시도가 여러 번 돈다(무재시도 회귀 방지).
        assert!(
            (PIPE_BUSY_WAIT_SLICE + PIPE_BUSY_BACKOFF_CAP) * 4 < PIPE_BUSY_RETRY_DEADLINE,
            "슬라이스+캡이 데드라인을 잠식하면 사실상 1회 open 으로 회귀한다"
        );
        // jitter 난수원은 항상 [0,1) — next_busy_delay 클램프와 이중 방어지만 계약은 박제.
        for _ in 0..64 {
            let r = rand01_cheap();
            assert!((0.0..1.0).contains(&r), "rand01_cheap 구간 이탈: {r}");
        }
    }

    /// ★SEAL-1 회귀 핀(2026-08-01 실사고): 번들 python 이 번들 안에 `.pyc` 를 쓰면 코드서명
    /// 봉인이 깨져 다음 실행이 Gatekeeper 에 차단된다. 두 층(직스폰 팩토리·상속 env)이
    /// **같은 상수**로 잠겨 있어야 한다 — 한쪽만 남으면 셸 경유 경로가 다시 새어 사고가 재발한다.
    #[test]
    fn python_spawns_never_write_bytecode_into_the_bundle() {
        // 값 규약: CPython 은 "비어 있지 않으면 참"이라 빈 값은 곧 봉인 파손 복귀다.
        assert_eq!(ENV_PY_NO_BYTECODE, "PYTHONDONTWRITEBYTECODE");
        assert!(!PY_NO_BYTECODE_ON.is_empty(), "빈 값은 CPython 이 '끔'으로 읽는다");

        // 층1 — 직스폰 팩토리(cysd phoenix·office-bridge·cys 헬퍼·GUI 직스폰).
        let cmd = python_command("python3");
        let got = cmd
            .get_envs()
            .find(|(k, _)| *k == std::ffi::OsStr::new(ENV_PY_NO_BYTECODE))
            .and_then(|(_, v)| v)
            .map(|v| v.to_string_lossy().into_owned());
        assert_eq!(
            got.as_deref(),
            Some(PY_NO_BYTECODE_ON),
            "python_command 는 반드시 바이트코드 쓰기를 끈 채로 나와야 한다"
        );

        // 층2 — 상속 env(pane·스케줄 잡·훅: 셸을 거쳐 python 이 도는 경로).
        // PATH 가 무변경이어도(=주입 쌍 없음) 이 쌍은 **항상** 나가야 한다.
        let exe_dir = Path::new("/nonexistent-exe-dir-for-pin");
        for pairs in [
            spawn_env_pairs(exe_dir, "/usr/bin:/bin", Some("/Users/user"), None),
            spawn_env_pairs(exe_dir, "", None, Some("C:\\Users\\me")),
        ] {
            assert_eq!(
                pairs
                    .iter()
                    .find(|(k, _)| k == ENV_PY_NO_BYTECODE)
                    .map(|(_, v)| v.as_str()),
                Some(PY_NO_BYTECODE_ON),
                "셸 경유 python 은 상속으로만 막을 수 있다 — 쌍이 빠지면 pane/훅이 번들을 오염시킨다"
            );
        }
    }

    /// ★W-B2 회귀 핀(감사 blocker #4 · cp949 즉사): `spawn_env_pairs` 는 PYTHONUTF8=1 을
    /// **항상** 실어야 한다 — PATH 무변경·HOME 유무와 무관한 무조건 쌍이다(③ SEAL-1 과 동형).
    /// 이 쌍이 빠지면 한국어 Windows(cp949)에서 스케줄 발화·GUI 직스폰 python(부트 체인
    /// `javis_orchestra.py check`)이 '✓' 한 글자에 UnicodeEncodeError 로 즉사한다(pane 은
    /// state.rs literal 이 이중으로 막지만 나머지 두 경로는 이 규약이 유일한 방어다).
    #[test]
    fn spawn_env_pairs_always_carry_python_utf8() {
        // 값 규약 핀: PYTHONUTF8 은 "1"/"0" 만 유효 — 그 외 비어 있지 않은 값은
        // "Fatal Python error: preconfig_init_utf8_mode" 로 파이썬 기동 자체가 죽고(실측),
        // 빈 문자열은 미설정 취급(끔)이다. 값이 "1" 에서 벗어나는 순간 봉인 해제 또는 전멸이다.
        assert_eq!(ENV_PY_UTF8, "PYTHONUTF8");
        // state.rs pane 스폰 literal("PYTHONUTF8","1" · RC-6)과의 값 파리티 — 중복 주입이
        // 무해하다는 근거가 "양쪽 값 동일"이므로, 이 값이 갈라지면 그 근거가 무너진다.
        // (state.rs 는 bin 크레이트라 lib 테스트가 심볼로 참조할 수 없어 literal 로 핀한다.)
        assert_eq!(PY_UTF8_ON, "1");

        let exe_dir = Path::new("/nonexistent-exe-dir-for-pin");
        for pairs in [
            // PATH 무변경에 가깝고 HOME 이 이미 있는 unix 꼴 — 그래도 쌍은 나가야 한다.
            spawn_env_pairs(exe_dir, "/usr/bin:/bin", Some("/Users/user"), None),
            // HOME 부재 Windows 꼴(backfill 발동) — 조건 조합과 무관하게 쌍은 나가야 한다.
            spawn_env_pairs(exe_dir, "", None, Some("C:\\Users\\me")),
        ] {
            assert_eq!(
                pairs
                    .iter()
                    .find(|(k, _)| k == ENV_PY_UTF8)
                    .map(|(_, v)| v.as_str()),
                Some(PY_UTF8_ON),
                "PYTHONUTF8=1 무조건 쌍이 빠졌다 — cp949 Windows 에서 부트 체인 python 이 즉사한다"
            );
        }
    }

    /// U-20 픽스처: `<install>\runtime\git\{bin,usr\bin}\bash.exe` 를 **둘 다** 깐 가짜 설치본.
    /// 둘 다 까는 것이 핵심이다 — B-12 의 위험은 "bash 가 없다"가 아니라 **엉뚱한 bash 가 먼저
    /// 잡힌다**이므로, 하나만 깔면 오선택을 재현할 수 없다.
    fn u20_fixture(tag: &str) -> std::path::PathBuf {
        // tag 는 호출 테스트마다 고유하고 pid 로 동시 실행을 가른다(병렬 테스트 간섭 방지).
        let base = std::env::temp_dir().join(format!("cys-u20-{}-{}", tag, std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        for rel in [
            ["runtime", "git", "bin"].join(std::path::MAIN_SEPARATOR_STR),
            ["runtime", "git", "usr", "bin"].join(std::path::MAIN_SEPARATOR_STR),
        ] {
            let d = base.join(rel);
            std::fs::create_dir_all(&d).expect("픽스처 디렉터리 생성 실패");
            std::fs::write(d.join("bash.exe"), b"fixture").expect("픽스처 bash 생성 실패");
        }
        base
    }

    /// ★U-20 회귀 핀 ⓐ — 동봉 bash 해소는 **런처**(`git\bin\bash.exe`)를 고른다.
    ///
    /// 왜 이 핀이 필요한가(B-12): 데몬 셸 탐지의 후보 순서(`schedule.rs
    /// windows_bash_candidates` = `runtime_bin_dirs` + `git\bin` 덧댐)는 `runtime_bin_dirs` 의
    /// Windows 순서(python → git\cmd → **git\usr\bin** → node) 때문에 MSYS 실 바이너리인
    /// `usr\bin\bash.exe` 를 **먼저** 집는다. Claude Code 훅이 요구하는 것은 런처 쪽이고
    /// (`.github/workflows/windows-build.yml` 이 그 파일의 실재를 FATAL 로 단언한다), 그 순서를
    /// 그대로 재사용했다면 이 키는 조용히 틀린 값을 실었을 것이다. 그래서 **이 키만 단일 고정**
    /// 하고, 그 선택을 여기서 박제한다.
    #[test]
    fn claude_code_git_bash_path_picks_git_bin_launcher() {
        let base = u20_fixture("launcher");
        let got = bundled_git_bash_path_for(&base, "windows").expect("동봉 bash 를 못 찾았다");
        assert_eq!(
            got,
            base.join("runtime").join("git").join("bin").join("bash.exe"),
            "런처 bash(git\\bin)가 아닌 경로가 선택됐다"
        );
        let s = got.to_string_lossy().into_owned();
        let usr = format!("{sep}usr{sep}", sep = std::path::MAIN_SEPARATOR);
        assert!(
            !s.contains(&usr),
            "MSYS 실 바이너리(git\\usr\\bin)가 선택됐다 — 훅 실행기가 요구하는 런처가 아니다: {s}"
        );

        // 런처가 없으면 **폴백하지 않는다**: usr\bin 이 남아 있어도 None.
        // (틀린 bash 를 통보하는 것보다 미통보가 낫다 — 미통보는 종전과 같은 상태다.)
        std::fs::remove_file(base.join("runtime").join("git").join("bin").join("bash.exe"))
            .expect("픽스처 정리 실패");
        assert_eq!(
            bundled_git_bash_path_for(&base, "windows"),
            None,
            "런처 부재인데 usr\\bin 으로 폴백했다 — 단일 고정 위반"
        );
        std::fs::remove_dir_all(&base).ok();
    }

    /// ★U-20 회귀 핀 ⓑ — 이 키는 **Windows 밖으로 새지 않는다**.
    ///
    /// mac/linux 에 실리면 존재하지도 않는 `…\bash.exe` 를 벤더에게 통보하는 꼴이 된다.
    /// 순수 코어는 OS 문자열로 전수 확인하고, 배선(`spawn_env_pairs`)은 **현재 호스트**에서
    /// 확인한다 — mac CI 에서 이 단언이 곧 '미주입 단언'이다.
    #[test]
    fn claude_code_git_bash_path_never_leaves_windows() {
        let base = u20_fixture("os-gate");
        for os in ["macos", "linux", "freebsd", "ios", "android", ""] {
            assert_eq!(
                bundled_git_bash_path_for(&base, os),
                None,
                "OS {os:?} 에 Windows 전용 bash 경로가 해소됐다"
            );
        }
        assert!(
            bundled_git_bash_path_for(&base, "windows").is_some(),
            "OS 게이트가 windows 까지 막았다 — 기능 자체가 사문이 된다"
        );

        // 배선 축: 실제 스폰 규약이 호스트 OS 규칙을 따르는가.
        let pairs = spawn_env_pairs(&base, "/usr/bin:/bin", Some("/Users/user"), None);
        let got = pairs
            .iter()
            .find(|(k, _)| k == ENV_CLAUDE_CODE_GIT_BASH_PATH);
        if cfg!(windows) {
            // 사용자 기설정·마스터 스위치가 눌린 실행에서는 **미주입이 계약**이므로 그 둘을
            // 배제한 때만 주입을 단언한다(테스트가 계약을 뒤집지 않게).
            if std::env::var_os(ENV_CLAUDE_CODE_GIT_BASH_PATH).is_none()
                && !boot_gates_master_off_from(std::env::var(ENV_BOOT_GATES).ok().as_deref())
            {
                assert!(
                    got.is_some(),
                    "Windows 인데 동봉 bash 가 실재하는 설치본에 키가 실리지 않았다 — 훅 전멸 재발"
                );
            }
        } else {
            assert_eq!(
                got, None,
                "mac/linux 스폰 env 에 Windows 전용 키가 실렸다 — 없는 경로를 벤더에 통보한다"
            );
        }
        std::fs::remove_dir_all(&base).ok();
    }

    /// ★U-20 회귀 핀 ⓒ — 불가침 계약 넷(순수 코어 · env 무접촉이라 병렬 테스트에 안전).
    ///
    /// ①사용자 값 불가침 ②이미 쌓인 쌍 불가침 ③미해소 시 무주입(fail-open)
    /// ④마스터 롤백 스위치(`CYS_BOOT_GATES=0`)로 **즉시 종전 복귀**.
    /// ④가 이 축 전용 노브가 아니라 마스터인 이유: 축마다 노브를 두고 나중에 합치는 순서는
    /// BLOCK-3 에서 이미 실패했다 — 사고 순간에 사람은 노브 조합을 만들지 못한다.
    #[test]
    fn claude_code_git_bash_path_user_value_and_master_switch() {
        let p = std::path::Path::new(r"C:\app\runtime\git\bin\bash.exe");
        let want = r"C:\app\runtime\git\bin\bash.exe";

        // 기본 경로: 해소됐고 사용자 값 없고 마스터 안 눌림 → 얹는다.
        let mut base = Vec::new();
        inject_claude_code_git_bash_path_for(&mut base, Some(p), false, false);
        assert_eq!(
            base.iter()
                .find(|(k, _)| k == ENV_CLAUDE_CODE_GIT_BASH_PATH)
                .map(|(_, v)| v.as_str()),
            Some(want),
            "정상 조건에서 키가 실리지 않았다"
        );

        // ① 사용자 프로세스 env 에 값이 있으면 무접촉(사용자 소유물 — 우리 값이 더 좋아도 덮지 않는다).
        let mut user = Vec::new();
        inject_claude_code_git_bash_path_for(&mut user, Some(p), true, false);
        assert!(user.is_empty(), "사용자 기설정을 덮었다: {user:?}");

        // ② 이미 쌓인 쌍이 있으면 값 보존(later-wins 뒤집기·중복 금지).
        let mut dup = vec![(
            ENV_CLAUDE_CODE_GIT_BASH_PATH.to_string(),
            r"D:\mine\bash.exe".to_string(),
        )];
        inject_claude_code_git_bash_path_for(&mut dup, Some(p), false, false);
        assert_eq!(dup.len(), 1, "같은 키를 중복으로 얹었다: {dup:?}");
        assert_eq!(dup[0].1, r"D:\mine\bash.exe", "먼저 실린 값을 덮었다");

        // ③ 미해소(동봉 bash 부재) → 무주입. 이 축의 최악값은 '종전과 동일'이다.
        let mut none = Vec::new();
        inject_claude_code_git_bash_path_for(&mut none, None, false, false);
        assert!(none.is_empty(), "해소 실패인데 키를 얹었다: {none:?}");

        // ④ 마스터 롤백 스위치 — 눌리면 조건이 전부 충족돼도 얹지 않는다(즉시 종전 복귀).
        let mut off = Vec::new();
        inject_claude_code_git_bash_path_for(&mut off, Some(p), false, true);
        assert!(off.is_empty(), "CYS_BOOT_GATES=0 인데 주입이 살아 있다: {off:?}");
        // 스위치 이름이 갈리면 사고 순간에 손잡이를 못 찾는다 — 마스터 1지점을 못박는다.
        assert!(
            boot_gates_master_off_from(Some("0")),
            "마스터 스위치 판독 규약이 바뀌었다"
        );
        assert_eq!(ENV_BOOT_GATES, "CYS_BOOT_GATES");
    }

    /// ★SEAL-1 층3 회귀 핀: 프로세스 env 봉인이 실제로 **상속 가능한 자리**에 심기는가.
    ///
    /// 층1·층2 는 "우리가 아는 프로그램"만 덮는다 — 임의 명령 스폰(cysd 채널 브리지·계정
    /// cmd 어댑터·`cys run -- …`)은 이 층이 유일한 방어다. 이 핀이 검사하는 것은 **자식이
    /// 실제로 상속하는 것과 같은 원천**(현재 프로세스 env)이다: `Command` 는 명시 `.env()` 가
    /// 없으면 부모 env 를 그대로 물려주므로, 여기서 값이 보이면 자식도 본다.
    ///
    /// env 를 실제로 쓰는 유일한 테스트인데도 안전한 이유: 값이 **멱등**(항상 같은 "1")이고,
    /// 다른 테스트가 이 키를 읽거나 쓰지 않으며, 방향이 하나뿐이라(켜기) 경합해도 결과가 같다.
    #[test]
    fn process_env_seal_is_inheritable_by_arbitrary_children() {
        seal_python_bytecode_in_process();
        assert_eq!(
            std::env::var(ENV_PY_NO_BYTECODE).ok().as_deref(),
            Some(PY_NO_BYTECODE_ON),
            "층3 부재 = 임의 명령 스폰(브리지·계정 cmd)이 번들에 .pyc 를 쓴다"
        );
        // e2e: 사고 경로와 **같은 형태**(임의 명령을 셸로 스폰, 명시 `.env()` 없음)로 실제
        // 자식을 띄워 상속을 확증한다. 단언을 프로세스 env 읽기로만 끝내면 "심었다"까지만
        // 증명하고 "자식이 받는다"는 증명하지 못한다 — 이 층의 존재 이유가 후자다.
        #[cfg(unix)]
        {
            let out = std::process::Command::new("/bin/sh")
                .args(["-c", "printf %s \"$PYTHONDONTWRITEBYTECODE\""])
                .output()
                .expect("/bin/sh 스폰 실패");
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                PY_NO_BYTECODE_ON,
                "임의 명령 셸 자식이 봉인을 상속하지 못했다 — 브리지·계정 cmd 가 그대로 샌다"
            );
        }
    }

    #[test]
    fn dept_socket_path_os_convention() {
        // RC-4 회귀 핀: OS별 부서 소켓 규약. 기본 socket_path와 대칭(둘 다 windows=named pipe).
        let p = dept_socket_path("dept-3");
        let s = p.to_string_lossy();
        #[cfg(windows)]
        assert_eq!(s, r"\\.\pipe\cys-dept-dept-3", "windows named pipe");
        #[cfg(not(windows))]
        {
            assert!(s.ends_with(".local/state/cys-dept-dept-3/cys.sock"), "unix .sock: {s}");
            assert!(!s.starts_with('/') || s.contains("/.local/state/"), "home 기반: {s}");
        }
    }

    #[test]
    fn is_dept_socket_detects_dept_not_main() {
        // H3: dept_socket_path와 정합 — 부서만 true, 메인은 false(오판=채널 전면 불능이라 접두 정확).
        assert!(is_dept_socket(&dept_socket_path("dept-3")), "부서 소켓은 true");
        assert!(is_dept_socket(Path::new("/x/.local/state/cys-dept-future/cys.sock")));
        assert!(is_dept_socket(Path::new(r"\\.\pipe\cys-dept-3")), "windows 파이프");
        // 메인 데몬 — 오판 금지.
        assert!(!is_dept_socket(Path::new("/x/.local/state/cys/cys.sock")), "메인 unix");
        assert!(!is_dept_socket(Path::new(r"\\.\pipe\cys")), "메인 windows");
        assert!(!is_dept_socket(Path::new("/tmp/cys_chan_test_1_tag/cysd.sock")), "테스트 임시");
    }

    #[test]
    fn runtime_prefixed_path_prepends_exe_dir_and_dedups() {
        // RC-5 회귀 핀(양 OS 공통 로직): exe_dir가 PATH에 없으면 선두에 얹는다.
        let sep = if cfg!(windows) { ';' } else { ':' };
        let exe = Path::new("/opt/cysapp/bin");
        let cur = format!("/usr/bin{sep}/bin");
        let got = runtime_prefixed_path(exe, &cur).expect("exe_dir 미포함이면 Some");
        assert!(got.starts_with("/opt/cysapp/bin"), "exe_dir 선두 주입: {got}");
        #[cfg(windows)]
        {
            assert!(got.ends_with(&cur), "기존 PATH 보존(제거 없음): {got}");
            // 이미 PATH에 있으면(중복) 얹지 않는다 → None(무변경). (windows는 runtime 하위 dir가
            // 실재하면 Some일 수 있으나 이 합성 경로엔 없음.)
            let already = format!("/opt/cysapp/bin{sep}/usr/bin");
            assert_eq!(runtime_prefixed_path(exe, &already), None, "중복이면 무변경");
        }
        #[cfg(not(windows))]
        {
            // 갱신 사유: unix 브랜치가 ~/.local/bin 을 무조건 append 하게 되어(claude 설치기 rc
            // 무수정 실측 대응) 합성 결과가 `…:cur:~/.local/bin` — ends_with(&cur) 단정을 교체.
            let local = home_dir().join(".local").join("bin").to_string_lossy().into_owned();
            assert!(
                got.ends_with(&format!("{cur}{sep}{local}")),
                "기존 PATH 보존 + ~/.local/bin 말미 append: {got}"
            );
            // prefixes 도 ~/.local/bin 도 이미 있으면 → None(무변경).
            let already = format!("/opt/cysapp/bin{sep}/usr/bin{sep}{local}");
            assert_eq!(runtime_prefixed_path(exe, &already), None, "중복이면 무변경");
        }
    }

    #[test]
    fn compose_unix_pane_path_appends_local_bin() {
        // Mac 핫픽스 회귀 핀: claude native 설치기(2.1.207)가 rc 무수정임이 실측 확인 →
        // ~/.local/bin 을 is_dir 게이트 없이 무조건 말미 append(발견 목적·기존 precedence 불강등).
        let home = Path::new("/home/fixture-user");
        let local = home.join(".local").join("bin").to_string_lossy().into_owned();
        let prefixes = vec!["/opt/app/bin".to_string()];
        // ① current_path 에 없으면 맨 뒤에 append — prefixes 는 선두.
        let out = compose_unix_pane_path(&prefixes, home, "/usr/bin:/bin").expect("Some");
        assert_eq!(out, format!("/opt/app/bin:/usr/bin:/bin:{local}"));
        // ② 이미 있으면 중복 append 하지 않는다(부정 케이스).
        let cur2 = format!("{local}:/usr/bin");
        let out2 = compose_unix_pane_path(&prefixes, home, &cur2).expect("Some");
        assert_eq!(out2, format!("/opt/app/bin:{cur2}"));
        // ③ prefixes·~/.local/bin 모두 기존재 → None(무변경 계약).
        let cur3 = format!("/opt/app/bin:/usr/bin:{local}");
        assert_eq!(compose_unix_pane_path(&prefixes, home, &cur3), None);
        // ④ append 가 기존 항목 순서를 바꾸지 않는다(선두 의도 주입 강등 금지 — MAJ#1 대칭).
        let out4 = compose_unix_pane_path(&prefixes, home, "/custom/shim:/usr/bin:/bin").expect("Some");
        assert_eq!(out4, format!("/opt/app/bin:/custom/shim:/usr/bin:/bin:{local}"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn runtime_bin_dirs_macos_resolves_bundle_resources_layout() {
        // RC-18(T6b) 회귀 핀: mac 번들 레이아웃(Contents/MacOS·Contents/Resources/runtime)에서
        // python/bin·git/bin·uv·node/bin을 선두 우선순위로 잡는다. 실재 디렉토리만 계상.
        use std::fs;
        let base = std::env::temp_dir().join(format!("cysrt-t6b-{}", std::process::id()));
        let macos = base.join("Contents").join("MacOS");
        let rt = base.join("Contents").join("Resources").join("runtime");
        for d in ["python/bin", "git/bin", "uv", "node/bin"] {
            fs::create_dir_all(rt.join(d)).unwrap();
        }
        fs::create_dir_all(&macos).unwrap();
        let dirs = runtime_bin_dirs(&macos);
        let got: Vec<String> = dirs.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        assert_eq!(got.len(), 4, "4개 runtime bin dir: {got:?}");
        assert!(got[0].ends_with("Resources/runtime/python/bin"), "python 선두: {got:?}");
        assert!(got[1].ends_with("Resources/runtime/git/bin"), "git 2순위: {got:?}");
        assert!(got[2].ends_with("Resources/runtime/uv"), "uv 3순위: {got:?}");
        assert!(got[3].ends_with("Resources/runtime/node/bin"), "node 4순위: {got:?}");
        // PATH 선두주입: runtime dir들이 exe_dir 뒤·기존 PATH 앞.
        let p = runtime_prefixed_path(&macos, "/usr/bin:/bin").expect("Some");
        let py_idx = p.find("Resources/runtime/python/bin").unwrap();
        let usrbin_idx = p.find("/usr/bin").unwrap();
        assert!(py_idx < usrbin_idx, "runtime python이 /usr/bin보다 앞(env 레벨): {p}");
        // 부재 dir는 계상 안 함: uv 제거 후 3개.
        fs::remove_dir_all(rt.join("uv")).unwrap();
        assert_eq!(runtime_bin_dirs(&macos).len(), 3, "uv 부재 시 3개");
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn expand_windows_env_cases() {
        // %VAR% 전개(mac 에서 Windows 로직 검증 — 순수 fn 직접 호출).
        let lk = |k: &str| match k {
            "USERPROFILE" => Some(r"C:\Users\cys".to_string()),
            "APPDATA" => Some(r"C:\Users\cys\AppData\Roaming".to_string()),
            _ => None,
        };
        // 전개.
        assert_eq!(
            expand_windows_env(r"%USERPROFILE%\.local\bin", lk),
            r"C:\Users\cys\.local\bin"
        );
        // 미지 변수는 원문 유지.
        assert_eq!(expand_windows_env(r"%NOPE%\bin", lk), r"%NOPE%\bin");
        // 변수 없음: 그대로.
        assert_eq!(expand_windows_env(r"C:\Windows\System32", lk), r"C:\Windows\System32");
        // 연속 변수(경계 인접).
        assert_eq!(
            expand_windows_env(r"%USERPROFILE%%APPDATA%", lk),
            r"C:\Users\cysC:\Users\cys\AppData\Roaming"
        );
        // 빈 이름(%%)·미종결 %는 원문 보존(안전 폴백).
        assert_eq!(expand_windows_env(r"50%%off", lk), r"50%%off");
        assert_eq!(expand_windows_env(r"tail%USERPROFILE", lk), r"tail%USERPROFILE");
        // 빈 문자열.
        assert_eq!(expand_windows_env("", lk), "");
    }

    #[test]
    fn windows_user_bin_dirs_composition() {
        let home = Path::new(r"C:\Users\cys");
        let appdata = PathBuf::from(r"C:\Users\cys\AppData\Roaming");
        let with = windows_user_bin_dirs(home, Some(&appdata));
        let got: Vec<String> = with.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        // 경로 구분자는 호스트 OS 규약이라 컴포넌트 존재로 검증(mac 에서도 무결).
        assert_eq!(got.len(), 2);
        assert!(got[0].contains(".local") && got[0].ends_with("bin"), "local/bin: {got:?}");
        assert!(got[1].ends_with("npm"), "appdata/npm: {got:?}");
        // APPDATA 부재 시 .local/bin 만.
        let none = windows_user_bin_dirs(home, None);
        assert_eq!(none.len(), 1);
    }

    #[test]
    fn compose_pane_path_rules() {
        let sep = ';';
        let prefixes = vec![r"C:\app\bin".to_string(), r"C:\app\runtime\python".to_string()];
        let user_bins = vec![r"C:\Users\cys\.local\bin".to_string(), r"C:\Users\cys\AppData\Roaming\npm".to_string()];
        // ① 낡은 프로세스 PATH(.local\bin 없음) + 신선 레지스트리(.local\bin 있음) →
        //    새 순서: prefixes ; process ; fresh 신규분 ; user_bins 신규분. .local\bin 정확히 1회.
        let fresh = r"C:\Windows\System32;C:\Users\cys\.local\bin";
        let process = r"C:\Windows\System32;C:\stale";
        let out = compose_pane_path(&prefixes, Some(fresh), &user_bins, process, sep);
        let parts: Vec<&str> = out.split(sep).collect();
        assert_eq!(parts[0], r"C:\app\bin", "prefix 선두: {out}");
        assert_eq!(parts[1], r"C:\app\runtime\python", "runtime 2순위: {out}");
        let local_count = parts.iter().filter(|&&p| p == r"C:\Users\cys\.local\bin").count();
        assert_eq!(local_count, 1, "user bin 정확히 1회(레지스트리·user_bins 중복 dedup): {out}");
        assert!(parts.contains(&r"C:\stale"), "세션 유래 항목 보존: {out}");
        // 세션 유래(process) 항목이 fresh 신규분(.local\bin)보다 앞 — precedence 보존(MAJ#1).
        let stale_idx = parts.iter().position(|&p| p == r"C:\stale").unwrap();
        let local_idx = parts.iter().position(|&p| p == r"C:\Users\cys\.local\bin").unwrap();
        assert!(stale_idx < local_idx, "process 항목이 레지스트리 신규분보다 앞: {out}");
        // System32 도 dedup(레지스트리·프로세스 중복) — 1회.
        assert_eq!(parts.iter().filter(|&&p| p == r"C:\Windows\System32").count(), 1, "전체 dedup: {out}");

        // ② 레지스트리 None → base=프로세스 PATH(현행 동작 등가). prefix 후 프로세스 항목 보존.
        let out2 = compose_pane_path(&prefixes, None, &user_bins, process, sep);
        let parts2: Vec<&str> = out2.split(sep).collect();
        assert_eq!(parts2[0], r"C:\app\bin");
        assert!(parts2.contains(&r"C:\stale") && parts2.contains(&r"C:\Windows\System32"));
        // user_bins 는 여전히 포함(fresh 에 없어도 belt-and-braces).
        assert!(parts2.contains(&r"C:\Users\cys\.local\bin"));

        // ④ 빈 항목 제거·중복 전면 dedup.
        let out3 = compose_pane_path(&prefixes, Some(";;C:\\dup"), &[], "C:\\dup;C:\\app\\bin;", sep);
        let parts3: Vec<&str> = out3.split(sep).collect();
        assert!(!parts3.iter().any(|p| p.is_empty()), "빈 항목 없음: {out3}");
        assert_eq!(parts3.iter().filter(|&&p| p == r"C:\dup").count(), 1, "dup 1회: {out3}");
        assert_eq!(parts3.iter().filter(|&&p| p == r"C:\app\bin").count(), 1, "prefix dup 1회: {out3}");

        // (a) MAJ#1 핀: 프로세스 PATH 선두의 shim(레지스트리에 없음)이 prefixes 바로 다음·레지스트리 항목보다 앞.
        let out_a = compose_pane_path(&prefixes, Some(r"C:\reg\bin"), &[], r"C:\shim\bin;C:\other", sep);
        let pa: Vec<&str> = out_a.split(sep).collect();
        assert_eq!(pa[0], r"C:\app\bin");
        assert_eq!(pa[1], r"C:\app\runtime\python");
        assert_eq!(pa[2], r"C:\shim\bin", "process 선두 shim 이 prefixes 직후: {out_a}");
        let shim_idx = pa.iter().position(|&p| p == r"C:\shim\bin").unwrap();
        let reg_idx = pa.iter().position(|&p| p == r"C:\reg\bin").unwrap();
        assert!(shim_idx < reg_idx, "shim 이 레지스트리 항목보다 앞: {out_a}");

        // (b) MAJ#2 핀: casing·후행 '\' 변형은 동일 항목으로 dedup(Windows case-insensitive) — 1회만.
        let out_b = compose_pane_path(&[], Some(r"C:\WINDOWS\System32"), &[], r"C:\Windows\System32\", sep);
        let pb: Vec<&str> = out_b.split(sep).collect();
        assert_eq!(pb.len(), 1, "casing·후행슬래시 변형 dedup=1개: {out_b}");
        assert_eq!(pb[0], r"C:\Windows\System32\", "출력은 원문(최초 등장=process 형태) 유지: {out_b}");

        // (c) ★순서보장 핀(오너 통찰): fresh_base 에 경쟁 python(C:\Python312)이 있어도 동봉 runtime 이 항상 앞.
        let out_c = compose_pane_path(&prefixes, Some(r"C:\Python312"), &[], "", sep);
        let pc: Vec<&str> = out_c.split(sep).collect();
        let rt_idx = pc.iter().position(|&p| p == r"C:\app\runtime\python").unwrap();
        let py_idx = pc.iter().position(|&p| p == r"C:\Python312").unwrap();
        assert!(rt_idx < py_idx, "동봉 runtime 이 레지스트리 유입 python 보다 앞(절대 밀리지 않음): {out_c}");
    }

    #[test]
    fn keys() {
        assert_eq!(key_to_bytes("Return"), Some(b"\r".to_vec()));
        assert_eq!(key_to_bytes("C-c"), Some(vec![0x03]));
        assert_eq!(key_to_bytes("Up"), Some(b"\x1b[A".to_vec()));
    }

    #[test]
    fn surface_ref_roundtrip_and_edges() {
        // 왕복: id → surface_ref → parse_surface_ref → id
        for id in [0u64, 1, 31, 65535, u64::MAX] {
            assert_eq!(parse_surface_ref(&surface_ref(id)), Some(id));
        }
        // 공백 trim
        assert_eq!(parse_surface_ref("  42  "), Some(42));
        assert_eq!(parse_surface_ref("\tsurface:7\n"), Some(7));
        // prefix는 1회만 제거 — 이중 prefix는 parse 실패
        assert_eq!(parse_surface_ref("surface:surface:31"), None);
        // 음수·비숫자·빈 문자열
        assert_eq!(parse_surface_ref("-5"), None);
        assert_eq!(parse_surface_ref(""), None);
        assert_eq!(parse_surface_ref("surface:"), None);
        assert_eq!(parse_surface_ref("3.5"), None);
        // u64 초과는 None (오버플로 시 silent wrap 금지)
        assert_eq!(parse_surface_ref("18446744073709551616"), None);
    }

    #[test]
    fn key_ctrl_modifier() {
        // C-c == C-C (대소문자 무관, ctrl 비트 0x1f 마스크)
        assert_eq!(key_to_bytes("C-c"), Some(vec![0x03]));
        assert_eq!(key_to_bytes("C-C"), Some(vec![0x03]));
        assert_eq!(key_to_bytes("C-a"), Some(vec![0x01]));
        assert_eq!(key_to_bytes("C-z"), Some(vec![0x1a]));
        // C-Space → NUL (0x00), 'S'가 0x13(XOFF)으로 오변환되지 않음
        assert_eq!(key_to_bytes("C-Space"), Some(vec![0x00]));
        assert_eq!(key_to_bytes("C-space"), Some(vec![0x00]));
        // ctrl + 비-알파벳 단일문자는 매핑 없음
        assert_eq!(key_to_bytes("C-1"), None);
        assert_eq!(key_to_bytes("C-["), None);
        // 다중문자 C- (Space 외)는 ctrl 비트 변환 금지 → None
        assert_eq!(key_to_bytes("C-Foo"), None);
        // C- + 비-ASCII 단일문자(멀티바이트)는 ctrl 매핑 없음 → None
        // (count==1이라 단일문자 분기에 들지만 is_ascii_lowercase=false라 fall-through)
        assert_eq!(key_to_bytes("C-가"), None);
        // C- 단독(빈 rest)은 단일문자도 Space도 아님 → None
        assert_eq!(key_to_bytes("C-"), None);
    }

    #[test]
    fn key_meta_modifier() {
        // M-x → ESC + 'x'
        assert_eq!(key_to_bytes("M-x"), Some(vec![0x1b, b'x']));
        // M-<여러글자>도 ESC 접두 후 그대로 (Alt 시퀀스)
        assert_eq!(
            key_to_bytes("M-Foo"),
            Some([&[0x1b][..], b"Foo"].concat())
        );
        // M- 단독 (빈 rest) → ESC 단독
        assert_eq!(key_to_bytes("M-"), Some(vec![0x1b]));
    }

    #[test]
    fn key_named_and_literal() {
        assert_eq!(key_to_bytes("Enter"), Some(b"\r".to_vec()));
        assert_eq!(key_to_bytes("Tab"), Some(b"\t".to_vec()));
        assert_eq!(key_to_bytes("Escape"), Some(b"\x1b".to_vec()));
        assert_eq!(key_to_bytes("Backspace"), Some(b"\x7f".to_vec()));
        assert_eq!(key_to_bytes("F5"), Some(b"\x1b[15~".to_vec()));
        // 단일 리터럴 문자는 그대로 통과 (멀티바이트 포함)
        assert_eq!(key_to_bytes("a"), Some(b"a".to_vec()));
        assert_eq!(key_to_bytes("가"), Some("가".as_bytes().to_vec()));
        // 알 수 없는 다중문자 키 이름 → None
        assert_eq!(key_to_bytes("Nonsense"), None);
        assert_eq!(key_to_bytes(""), None);
    }

    #[test]
    fn key_function_keys_use_correct_protocol() {
        // F1-F4는 SS3(\x1bO_), F5+는 CSI(\x1b[_~) — 두 인코딩이 갈리는 경계 박제.
        assert_eq!(key_to_bytes("F1"), Some(b"\x1bOP".to_vec()));
        assert_eq!(key_to_bytes("F4"), Some(b"\x1bOS".to_vec()));
        assert_eq!(key_to_bytes("F5"), Some(b"\x1b[15~".to_vec()));
        assert_eq!(key_to_bytes("F12"), Some(b"\x1b[24~".to_vec()));
        // F5와 F6 사이에 16이 건너뛰는 VT 표준(역사적 결번) 보존
        assert_eq!(key_to_bytes("F6"), Some(b"\x1b[17~".to_vec()));
        // 대소문자 민감 — 'f1'은 명명키 아님, 단일문자도 아님(2글자) → None
        assert_eq!(key_to_bytes("f1"), None);
    }

    #[test]
    fn key_navigation_and_aliases() {
        // 화살표(CSI 종결바이트 A-D)
        assert_eq!(key_to_bytes("Up"), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_to_bytes("Down"), Some(b"\x1b[B".to_vec()));
        assert_eq!(key_to_bytes("Right"), Some(b"\x1b[C".to_vec()));
        assert_eq!(key_to_bytes("Left"), Some(b"\x1b[D".to_vec()));
        // 별칭 동치 (Return=Enter 등 호환 어휘)
        assert_eq!(key_to_bytes("Return"), key_to_bytes("Enter"));
        assert_eq!(key_to_bytes("Esc"), key_to_bytes("Escape"));
        assert_eq!(key_to_bytes("BTab"), key_to_bytes("BackTab"));
        assert_eq!(key_to_bytes("Delete"), key_to_bytes("DC"));
        assert_eq!(key_to_bytes("PageUp"), key_to_bytes("PPage"));
        assert_eq!(key_to_bytes("PageDown"), key_to_bytes("NPage"));
        // BTab은 CSI Z (shift-tab)
        assert_eq!(key_to_bytes("BTab"), Some(b"\x1b[Z".to_vec()));
    }

    #[test]
    fn key_meta_with_named_key_is_literal_not_translated() {
        // ★불변식 박제: M- 접두는 rest를 명명키로 재해석하지 않고 '리터럴 바이트'로 붙인다.
        // 즉 M-Enter는 ESC+CR(\x1b\r)이 아니라 ESC + "Enter"(\x1b + 5글자)다.
        // (이 동작에 의존하는 호출부가 있으면 회귀 시 여기서 드러난다)
        assert_eq!(key_to_bytes("M-Enter"), Some([&[0x1b][..], b"Enter"].concat()));
        assert_ne!(key_to_bytes("M-Enter"), Some(vec![0x1b, b'\r']));
        // M-멀티바이트도 UTF-8 바이트 그대로 ESC 뒤에 (Alt+한글)
        assert_eq!(
            key_to_bytes("M-가"),
            Some([&[0x1b][..], "가".as_bytes()].concat())
        );
    }

    #[test]
    fn env_compat_fallback_priority() {
        // 고유 키로 격리 (다른 테스트·환경과 충돌 방지)
        let p = "CYS_ZZUNIQUETEST";
        let j = "JAVIS_ZZUNIQUETEST";
        let a = "AITERM_ZZUNIQUETEST";
        for k in [p, j, a] {
            std::env::remove_var(k);
        }
        // 셋 다 없으면 None
        assert_eq!(env_compat(p), None);
        // AITERM_만 있으면 폴백
        std::env::set_var(a, "aiterm_val");
        assert_eq!(env_compat(p), Some("aiterm_val".to_string()));
        // JAVIS_가 AITERM_보다 우선
        std::env::set_var(j, "javis_val");
        assert_eq!(env_compat(p), Some("javis_val".to_string()));
        // CYS_(primary)가 최우선
        std::env::set_var(p, "cys_val");
        assert_eq!(env_compat(p), Some("cys_val".to_string()));
        // 빈 문자열은 미설정으로 간주 → 다음 폴백
        std::env::set_var(p, "");
        assert_eq!(env_compat(p), Some("javis_val".to_string()));
        for k in [p, j, a] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn env_compat_only_first_cys_token_is_rewritten() {
        // replacen(..,1)이 'CYS_'를 첫 1회만 치환 — primary에 CYS_가 없으면
        // 세 후보 키가 모두 primary와 동일(폴백 무의미)임을 박제.
        let only = "CYS_ZZONLYPRIMARY";
        let javis = "JAVIS_ZZONLYPRIMARY";
        std::env::remove_var(only);
        std::env::remove_var(javis);
        // primary에 CYS_가 없는 키: 폴백 키가 자기 자신과 같아져 primary만 본다
        let nocys = "PLAINKEY_ZZ";
        std::env::remove_var(nocys);
        assert_eq!(env_compat(nocys), None);
        std::env::set_var(nocys, "plain");
        assert_eq!(env_compat(nocys), Some("plain".to_string()));
        std::env::remove_var(nocys);
        // 첫 CYS_만 치환 — 'CYS_'가 값 중간에 또 나와도 1회만
        std::env::set_var(javis, "via_javis");
        assert_eq!(env_compat(only), Some("via_javis".to_string()));
        std::env::remove_var(javis);
    }

    #[test]
    fn claude_project_component_munges_path() {
        // 실측 munge 규칙: '/'와 특수문자 → '-', 영숫자·'-' 보존.
        assert_eq!(
            claude_project_component("/Users/user/Desktop/ProjX"),
            "-Users-user-Desktop-ProjX"
        );
        assert_eq!(claude_project_component("/tmp/a.b_c"), "-tmp-a-b-c");
    }

    #[test]
    fn resolve_claude_config_dir_is_deterministic_env_not_scan() {
        // (W1-2 핵심) config_dir 권위는 결정론 env 해소뿐 — discover 스캔(~/.claude*)을 원리적으로
        // 참조하지 않는다. CYS_ACCOUNT_DIR 설정 시 그 값 그대로, 미설정 시 $HOME/.cys/claude.
        let prev = std::env::var("CYS_ACCOUNT_DIR").ok();
        // (a) 명시 계정 dir → 그 절대경로 그대로 (foreign ~/.claude-* 존재 여부와 무관 = 스캔 안 함)
        std::env::set_var("CYS_ACCOUNT_DIR", "/tmp/zz-acct/.cys/claude");
        assert_eq!(resolve_claude_config_dir(), "/tmp/zz-acct/.cys/claude");
        // (b) 빈 문자열 = 미설정 취급 → 기본 $HOME/.cys/claude
        std::env::set_var("CYS_ACCOUNT_DIR", "");
        let def = resolve_claude_config_dir();
        assert!(def.ends_with("/.cys/claude"), "기본 해소: {def}");
        assert!(
            def.starts_with(&home_dir().to_string_lossy().into_owned()),
            "HOME 기반: {def}"
        );
        // (c) 미설정도 동일 기본
        std::env::remove_var("CYS_ACCOUNT_DIR");
        assert_eq!(resolve_claude_config_dir(), def);
        // 원복
        match prev {
            Some(v) => std::env::set_var("CYS_ACCOUNT_DIR", v),
            None => std::env::remove_var("CYS_ACCOUNT_DIR"),
        }
    }

    /// ★A9 패리티 핀 — 공유 코퍼스(src/testdata/mouse_report_corpus.json)를 소비해
    /// Rust 매처의 판정이 코퍼스 `pure` 와 전건 일치함을 단언한다. 같은 파일을 TS 테스트
    /// (ui/src/mousefilter.test.ts — W3)도 소비하므로, 이 테스트가 green 이면 TS↔Rust
    /// 이중 구현 드리프트가 구조적으로 불가능하다(R3 RISK 4-② 봉합).
    #[test]
    fn mouse_report_corpus_parity() {
        let raw = include_str!("testdata/mouse_report_corpus.json");
        let doc: serde_json::Value = serde_json::from_str(raw).expect("코퍼스는 유효 JSON");
        let cases = doc["cases"].as_array().expect("cases 배열");
        assert!(cases.len() >= 30, "코퍼스 축소 금지(4인코딩×휠/클릭/릴리스/하한/혼합/절단/paste)");
        for c in cases {
            let name = c["name"].as_str().unwrap_or("?");
            let bytes = c["bytes"].as_str().expect("bytes 문자열");
            let pure = c["pure"].as_bool().expect("pure 불리언");
            assert_eq!(
                mousereport::is_pure_mouse_report(bytes),
                pure,
                "코퍼스 케이스 '{name}' 판정 불일치 (bytes={bytes:?}, 기대 pure={pure})"
            );
        }
    }

    /// A9 verdict 세부 핀 — 코퍼스는 pure 만 고정하므로, 휠 방향·배칭 접기·상쇄는 여기서
    /// TS 의미론(ts:110-111)과 대조한다.
    #[test]
    fn mouse_report_verdict_semantics() {
        use mousereport::{classify_mouse_report, MouseVerdict};
        // 휠업 단독 → dir=-1, count=1.
        assert_eq!(
            classify_mouse_report("\u{1b}[<64;10;20M"),
            MouseVerdict::Wheel { dir: -1, count: 1 }
        );
        // 배칭(업2+다운1) → 순증감 -1.
        assert_eq!(
            classify_mouse_report("\u{1b}[<64;1;1M\u{1b}[<64;1;1M\u{1b}[<65;1;1M"),
            MouseVerdict::Wheel { dir: -1, count: 1 }
        );
        // 상쇄 → Drop(스크롤할 것 없음). 클릭·릴리스도 Drop.
        assert_eq!(
            classify_mouse_report("\u{1b}[<64;1;1M\u{1b}[<65;1;1M"),
            MouseVerdict::Drop
        );
        assert_eq!(classify_mouse_report("\u{1b}[<0;5;7m"), MouseVerdict::Drop);
        // X10 인코딩 휠업(b=96 → 64) — 오프셋 복원 경로.
        assert_eq!(
            classify_mouse_report("\u{1b}[M`*%"),
            MouseVerdict::Wheel { dir: -1, count: 1 }
        );
        // paste 래퍼는 classify 로도 Pass 이고, 술어로는 접두 규칙이 이중 봉인한다.
        assert_eq!(
            classify_mouse_report("\u{1b}[200~\u{1b}[<64;10;20M\u{1b}[201~"),
            MouseVerdict::Pass
        );
        assert!(!mousereport::is_pure_mouse_report("\u{1b}[200~\u{1b}[<64;10;20M\u{1b}[201~"));
    }

    /// ★B1 인식 표 박제 — 브리프가 열거한 자동 응답 **전 종류**가 단독으로도, 서로 연접해도
    /// 참이어야 한다. 하나라도 빠지면 그 시퀀스가 흐르는 순간 타이핑 가드가 3초 오염되어
    /// 노드 보고의 제출 Enter 가 거부된다(결함3 재발). 값은 실기에서 나오는 형태로 적었다.
    #[test]
    fn terminal_autoreply_recognizes_every_documented_response_shape() {
        use mousereport::is_pure_terminal_autoreply as pure;
        let shapes: [(&str, &str); 13] = [
            ("포커스 획득(1004)", "\u{1b}[I"),
            ("포커스 상실(1004)", "\u{1b}[O"),
            ("CPR(ESC[6n 응답)", "\u{1b}[24;80R"),
            ("DECXCPR 2파라미터", "\u{1b}[?24;80R"),
            ("DECXCPR 3파라미터(page)", "\u{1b}[?24;80;1R"),
            ("DA1", "\u{1b}[?62;1;6c"),
            ("DA2", "\u{1b}[>0;276;0c"),
            ("DA3", "\u{1b}P!|00000000\u{1b}\\"),
            ("XTVERSION", "\u{1b}P>|XTerm(370)\u{1b}\\"),
            ("DECRPM(모드 질의 응답)", "\u{1b}[?2004;1$y"),
            ("kitty 키보드 플래그", "\u{1b}[?1u"),
            ("OSC 11 배경색(BEL 종결)", "\u{1b}]11;rgb:1e1e/1e1e/1e1e\u{7}"),
            ("OSC 10 전경색(ST 종결)", "\u{1b}]10;rgb:ffff/ffff/ffff\u{1b}\\"),
        ];
        for (name, seq) in shapes {
            assert!(pure(seq), "{name} 미인식: {seq:?} — 이 시퀀스가 타이핑 가드를 오염시킨다");
        }
        // 연접(실기에서는 한 청크에 여러 응답이 함께 온다 — 클릭 직후 포커스+CPR 등).
        let joined: String = shapes.iter().map(|(_, s)| *s).collect();
        assert!(pure(&joined), "자동 응답 연접이 거짓 — 실기 청크는 대개 연접이다");
        assert!(
            pure("\u{1b}[I\u{1b}[24;80R\u{1b}[O"),
            "포커스+CPR+포커스 연접(가장 흔한 실기 조합)이 거짓"
        );
        // OSC 12(커서색)도 코드 표에 있다 — 10/11 만 통과하는 반쪽 구현 방지.
        assert!(pure("\u{1b}]12;rgb:8888/8888/8888\u{7}"));
    }

    /// ★B1 오면제 방어 박제 — 위험한 방향은 오폐기가 아니라 **오면제**다(사람 입력을 기계로
    /// 오인해 가드를 안 켜면, 사람의 미완성 입력에 원격 Return 이 꽂힌다). 혼합·절단·paste
    /// 래퍼·빈 문자열·평문은 전부 거짓이어야 한다.
    #[test]
    fn terminal_autoreply_rejects_mixed_truncated_paste_and_plain_text() {
        use mousereport::is_pure_terminal_autoreply as pure;
        for (why, bad) in [
            ("빈 문자열", ""),
            ("평문", "hello"),
            ("자동응답+사람글자 혼합(뒤)", "\u{1b}[Ix"),
            ("사람글자+자동응답 혼합(앞)", "x\u{1b}[I"),
            ("연접 사이에 사람글자", "\u{1b}[I q \u{1b}[O"),
            ("절단 CSI", "\u{1b}[24;80"),
            ("절단 포커스(ESC[ 만)", "\u{1b}["),
            ("ESC 단독", "\u{1b}"),
            ("절단 DCS(ST 없음)", "\u{1b}P>|XTerm(370)"),
            ("절단 OSC(종결자 없음)", "\u{1b}]11;rgb:1e1e/1e1e/1e1e"),
            ("빈 본문 OSC", "\u{1b}]11;\u{7}"),
            ("빈 본문 DCS", "\u{1b}P>|\u{1b}\\"),
            ("DA3 본문이 16진이 아님", "\u{1b}P!|zz\u{1b}\\"),
            ("허용 밖 OSC 코드(4=팔레트)", "\u{1b}]4;1;rgb:0/0/0\u{7}"),
            ("CPR 파라미터 1개", "\u{1b}[24R"),
            ("DECRPM 파라미터 1개", "\u{1b}[?2004$y"),
            ("kitty 플래그 파라미터 2개", "\u{1b}[?1;2u"),
            ("빈 파라미터", "\u{1b}[?;c"),
            ("종결자 오염(CSI m)", "\u{1b}[24;80m"),
            ("paste 래퍼 안의 자동 응답", "\u{1b}[200~\u{1b}[I\u{1b}[201~"),
            ("paste 개시 접두 단독", "\u{1b}[200~"),
        ] {
            assert!(!pure(bad), "{why} 가 자동 응답으로 오면제됐다: {bad:?}");
        }
        // 개행·CR 은 절대 자동 응답이 아니다 — 이게 참이 되면 제출 Enter 자체가 면제된다.
        for cr in ["\r", "\n", "\r\n", "\u{1b}[I\r"] {
            assert!(!pure(cr), "개행/CR 이 자동 응답으로 판정됐다: {cr:?}");
        }
    }

    /// ★B1 상위 집합 계약 박제 — 새 술어는 A9(마우스 면제)를 **줄이지 않는다**. 공유 코퍼스의
    /// `pure=true` 케이스는 전부 자동 응답으로도 참이어야 하고, `pure=false` 인 paste 래퍼는
    /// 양쪽 다 거짓이어야 한다(A9 규약 유지). 이 핀이 없으면 B1 이 A9 면제를 조용히 좁힐 수 있다.
    #[test]
    fn terminal_autoreply_is_superset_of_pure_mouse_report() {
        let raw = include_str!("testdata/mouse_report_corpus.json");
        let doc: serde_json::Value = serde_json::from_str(raw).expect("코퍼스는 유효 JSON");
        let cases = doc["cases"].as_array().expect("cases 배열");
        let mut pure_seen = 0usize;
        for c in cases {
            let name = c["name"].as_str().unwrap_or("?");
            let bytes = c["bytes"].as_str().expect("bytes 문자열");
            if c["pure"].as_bool().expect("pure 불리언") {
                pure_seen += 1;
                assert!(
                    mousereport::is_pure_terminal_autoreply(bytes),
                    "코퍼스 '{name}' 는 순수 마우스 보고인데 자동 응답 술어가 거짓이다 \
                     (A9 면제 축소 — bytes={bytes:?})"
                );
            }
        }
        assert!(pure_seen >= 10, "코퍼스의 pure 케이스가 너무 적다({pure_seen}) — 상위집합 검증이 공허해진다");
        // paste 래퍼는 두 술어 모두 거짓(A9 명문 규약).
        let wrapped = "\u{1b}[200~\u{1b}[<64;10;20M\u{1b}[201~";
        assert!(!mousereport::is_pure_mouse_report(wrapped));
        assert!(!mousereport::is_pure_terminal_autoreply(wrapped));
    }

    /// ★D5 회귀 핀(--lib 상주 — cys.rs 테스트 모듈은 CI 0회 실행이라 여기 둔다):
    /// ① 사용자 "0"(fullscreen 되살리기) 이 있으면 최종 산출에 "1" 이 **절대 미출현**
    ///    (append+sort 뒤집기 함정 봉인)
    /// ② 게이트 참 ∧ claude 면 기본 "1" 삽입 ③ 게이트 거짓(리눅스 등)·타 에이전트는 미삽입.
    ///
    /// ★2026-08-17 강등 개정(의미가 바뀐 핀): Windows 는 이제 **기본 미주입 · 옵트인 시에만
    /// 주입**이다(근거·승격 절차는 `d5_gate_for_os` doc — 앵커 ④ · 실기 스모크 B-5 미수행).
    /// 그래서 ④ 가 고정하는 것은 OS 축 하나가 아니라 **OS × 옵트인 2축 매트릭스**다
    /// (⑤ 는 그 결론을 산출물 모양으로 되비출 뿐이다 — 아래 '⑤ 의 지위'):
    ///   macos → 주입 · windows∧미옵트인 → **미주입** · windows∧옵트인 → 주입 ·
    ///   windows∧옵트인∧사용자 "0" → **여전히 "0"** · 그 외 OS → 미주입.
    /// 이 다섯 행이 함께 있어야 '기본 off' 와 '옵트인이 실제로 동작함'이 **동시에** 지켜진다 —
    /// 어느 한쪽만 있으면 스위치가 죽은 채(항상 false) 또는 강등이 무효화된 채(항상 true)
    /// 초록이 된다.
    ///
    /// ★①~③ 은 '게이트 bool 이 주어졌을 때의 삽입 규칙'만 본다(OS·옵트인 무지). 그래서
    /// 이들만으로는 'Windows 가 대상인가'를 한 글자도 증명하지 못한다 — 그 구멍을 메우는 것은
    /// **④ 하나**다(순수 매핑표를 세 OS × 옵트인 2축으로 직접 조회한다).
    ///
    /// ★⑤ 의 지위(정직 — 2026-08-17 적대검증 2R 의 과장 지적을 반영해 문안을 낮췄다):
    /// ⑤ 는 ④ 의 결론을 **산출물 형태로 재확인**할 뿐이고 **독립 판별력이 없다**. 실패 집합이
    /// ④ ∧ ①②③ 에 완전히 포함되기 때문이다 — 예로 ⑤-b 는 `d5_gate_for_os("windows", false)`
    /// 가 참이 되거나(그러면 ④-b 가 **먼저** 깨진다) 코어가 gated=false 에서 삽입해야만
    /// (그러면 ③-a 가 먼저 깨진다) 실패한다. 두 순수 함수를 테스트 안에서 합성할 뿐
    /// **프로덕션 배선을 지나지 않기** 때문이며, 실제 배선을 보는 것은 ⑥ 하나다.
    /// 그럼에도 남겨 둔 이유는 문서 가치다: 'mac=주입 · win 기본=미주입 · win 옵트인=주입 ·
    /// 사용자 "0" 불가침' 네 행이 **최종 산출물 모양으로** 한자리에 보인다.
    /// 이 문단을 '판별력의 소재지' 로 되돌려 쓰지 마라 — 그 형태의 과장이 바로 이 저장소가
    /// 직전 라운드에 '가짜 핀'으로 규탄한 것이다(다만 ⑤ 는 중복일 뿐 거짓 초록은 만들지 않는다).
    /// 게이트 값을 리터럴이 아니라 `d5_gate_for_os(...)` 로 먹이는 형식은 유지하라(종전 개정판은
    /// `..., true)` 리터럴이라 ②와 **문자 그대로 동치**여서, 문서 가치조차 없었다).
    #[test]
    fn claude_alt_screen_env_injection_pins() {
        let k = ENV_CLAUDE_NO_ALT_SCREEN;
        // ① 사용자 "0" 불가침 — mac 게이트가 참이어도 덮지 않는다.
        let mut with_zero = vec![
            ("CLAUDE_CONFIG_DIR".to_string(), "/x".to_string()),
            (k.to_string(), "0".to_string()),
        ];
        inject_claude_alt_screen_default_for(&mut with_zero, "claude", true);
        let vals: Vec<&str> = with_zero.iter().filter(|(key, _)| key == k).map(|(_, v)| v.as_str()).collect();
        assert_eq!(vals, ["0"], "사용자 '0' 이 유지되고 '1' 은 미출현이어야 한다: {with_zero:?}");
        // ② 게이트 참(mac) ∧ claude ∧ 키 부재 → "1" 삽입(기존 쌍 순서 불변 — 재정렬 금지 계약).
        let mut absent = vec![("CLAUDE_CONFIG_DIR".to_string(), "/x".to_string())];
        inject_claude_alt_screen_default_for(&mut absent, "claude", true);
        assert_eq!(absent[0].0, "CLAUDE_CONFIG_DIR", "기존 쌍 순서 불변(재정렬 금지)");
        assert_eq!(
            absent.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str()),
            Some("1"),
            "게이트 참 + claude 는 기본 '1' 이 삽입돼야 한다: {absent:?}"
        );
        // ③-a 게이트 거짓(리눅스 등 비대상 OS)은 미삽입 — 순수 코어의 OS-게이트 계약.
        let mut ungated = vec![("CLAUDE_CONFIG_DIR".to_string(), "/x".to_string())];
        inject_claude_alt_screen_default_for(&mut ungated, "claude", false);
        assert!(
            ungated.iter().all(|(key, _)| key != k),
            "게이트 거짓이면 미삽입: {ungated:?}"
        );
        // ③-b 타 에이전트(codex 등)는 게이트가 참이어도 미삽입.
        let mut codex: Vec<(String, String)> = Vec::new();
        inject_claude_alt_screen_default_for(&mut codex, "codex", true);
        assert!(codex.is_empty(), "타 에이전트는 미삽입: {codex:?}");
        // ④ ★OS × 옵트인 매핑표 — **이 테스트의 유일한 판별력 소재지**. `cfg!` 가 아니라 순수
        //    문자열 함수라서 mac 호스트에서도 windows·linux 행을 함께 조회한다.
        //    ④-a macOS 는 옵트인과 **무관하게** 주입 대상(강등은 Windows 축만 건드렸다 —
        //         mac 회귀 0 이 강등 결정의 전제였다).
        assert!(d5_gate_for_os("macos", false), "macOS 는 옵트인 없이도 D5 주입 대상(기본)");
        assert!(d5_gate_for_os("macos", true), "macOS 는 옵트인 여부와 무관하게 주입 대상");
        //    ④-b ★강등 핀: windows ∧ 옵트인 없음 → **비대상**. 누가 `"windows" => true` 로
        //         승격하면(= 실기 스모크 B-5 없이 기본 on 복귀) 이 줄이 **mac CI 에서 깨진다**.
        //         승격은 이 줄을 함께 뒤집는 의도적 행위여야 한다(`d5_gate_for_os` doc 의 승격 절차).
        assert!(
            !d5_gate_for_os("windows", false),
            "Windows 는 옵트인 없이는 비대상이어야 한다(2026-08-17 강등 — 앵커 ④ · 실기 B-5 미수행)"
        );
        //    ④-c ★스위치 생존 핀: windows ∧ 옵트인 → 대상. 누가 `"windows" => false` 로
        //         적으면(= 스위치가 죽은 채 초록) 여기서 깨진다.
        assert!(d5_gate_for_os("windows", true), "Windows 는 옵트인하면 주입 대상이어야 한다");
        assert!(!d5_gate_for_os("linux", false), "linux 는 비대상");
        assert!(!d5_gate_for_os("linux", true), "linux 는 옵트인해도 비대상(스위치는 Windows 전용)");
        assert!(!d5_gate_for_os("freebsd", true), "미지 OS 는 비대상(기본 거짓)");
        //    ④-d ★문서 결합 핀: 옵트인 채널 이름은 사용자 표면이다. 코드에서 이름을 바꾸면
        //         USER-MANUAL env 표·릴리스 노트·cys.rs 의 Windows 힌트 문안이 **한꺼번에**
        //         거짓이 된다 — 그 세 곳을 함께 고치라는 실패 메시지를 남긴다.
        assert_eq!(
            (D5_WIN_OPT_IN_ENV, D5_WIN_OPT_IN_FILE),
            ("CYS_WIN_NO_ALT_SCREEN", ".cys/win-no-alt-screen"),
            "옵트인 채널 이름을 바꿨다면 USER-MANUAL env 표 · v0.14.16 릴리스 노트 · \
             src/bin/cys.rs 의 alt_screen_notice Windows 힌트 문안을 함께 고쳐라"
        );
        // ⑤ 합성 재확인(**독립 판별력 없음** — 위 doc 의 '⑤ 의 지위' 참조): 게이트를 리터럴이
        //    아니라 매핑표에서 뽑아 코어에 먹여, ④ 의 결론을 최종 산출물 모양으로 한자리에
        //    보인다. 여기가 깨지면 ④ 나 ①②③ 이 **먼저** 깨진다.
        //    ⑤-a windows ∧ **옵트인** ∧ claude ∧ 키 부재 → 삽입.
        let mut win = vec![("CLAUDE_CONFIG_DIR".to_string(), "/x".to_string())];
        inject_claude_alt_screen_default_for(&mut win, "claude", d5_gate_for_os("windows", true));
        assert_eq!(
            win.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str()),
            Some("1"),
            "windows 는 옵트인하면 '1' 이 삽입돼야 한다: {win:?}"
        );
        //    ⑤-b 강등 재확인(산출물 쪽): windows ∧ 옵트인 없음 → 산출물에 키 자체가 없다.
        //         ④-b 가 게이트 값을, 여기가 그 값에서 나오는 **산출물 모양**을 보인다
        //         (같은 실패 집합 — 프로덕션 배선을 보는 것은 ⑥ 이다).
        let mut win_default = vec![("CLAUDE_CONFIG_DIR".to_string(), "/x".to_string())];
        inject_claude_alt_screen_default_for(
            &mut win_default,
            "claude",
            d5_gate_for_os("windows", false),
        );
        assert!(
            win_default.iter().all(|(key, _)| key != k),
            "windows 기본(미옵트인)은 미삽입이어야 한다 — 강등 전 출고본과 동일: {win_default:?}"
        );
        //    ⑤-c windows ∧ 옵트인 ∧ 사용자 값 "0" → **여전히 "0"**. 게이트를 어떻게 흔들어도
        //        '키 부재 시에만 append' 불가침 계약은 한 치도 약해지지 않는다.
        let mut win_zero = vec![
            ("CLAUDE_CONFIG_DIR".to_string(), "/x".to_string()),
            (k.to_string(), "0".to_string()),
        ];
        inject_claude_alt_screen_default_for(
            &mut win_zero,
            "claude",
            d5_gate_for_os("windows", true),
        );
        let win_vals: Vec<&str> =
            win_zero.iter().filter(|(key, _)| key == k).map(|(_, v)| v.as_str()).collect();
        assert_eq!(
            win_vals, ["0"],
            "windows 옵트인이라도 사용자 '0' 은 불가침이어야 한다('1' 미출현): {win_zero:?}"
        );
        //    ⑤-d linux 를 매핑표에서 뽑아 먹이면 미삽입 — ③-a(리터럴 false)와 달리 **OS 이름이
        //        게이트 거짓으로 이어지는 배선**까지 함께 본다.
        let mut linux = vec![("CLAUDE_CONFIG_DIR".to_string(), "/x".to_string())];
        inject_claude_alt_screen_default_for(&mut linux, "claude", d5_gate_for_os("linux", true));
        assert!(linux.iter().all(|(key, _)| key != k), "linux 는 미삽입: {linux:?}");
        // ⑥ 래퍼 배선 스모크 — 공개 래퍼가 `d5_gate_for_os(consts::OS, d5_win_opt_in())` 로
        //    라우팅되는지를 핀으로 박는다. 좌변은 래퍼의 **실제 산출물**, 우변은 같은 입력을
        //    **독립 재계산**한 값이다(리터럴 하드코딩이면 어긋난다).
        //    ★강등이 이 핀의 감시력을 **키웠다**(종전 doc 의 정직한 한계 고지가 부분 해소됐다):
        //      종전에는 `--lib` 을 도는 레인(mac·Windows) 둘 다 게이트가 참이라 래퍼의 세 번째
        //      인자를 `true` 로 하드코딩해도 어느 레인에서도 죽지 않았다. 이제 **Windows 러너는
        //      옵트인이 없어 게이트가 거짓**이므로(러너 홈에 `~/.cys/win-no-alt-screen` 도
        //      `CYS_WIN_NO_ALT_SCREEN` 도 없다), 그 하드코딩은 release.yml 의
        //      `cargo test --lib claude_alt_screen` Windows 스텝에서 빨개진다.
        //    ★남은 한계(정직): 여전히 **현재 빌드 OS 한 행**만 본다. 나머지 행은 ④ 가 지킨다.
        let mut wrapped: Vec<(String, String)> = Vec::new();
        inject_claude_alt_screen_default(&mut wrapped, "claude");
        assert_eq!(
            wrapped.iter().any(|(key, _)| key == k),
            d5_gate_for_os(std::env::consts::OS, d5_win_opt_in()),
            "래퍼는 d5_gate_for_os(consts::OS, d5_win_opt_in()) 로 라우팅돼야 한다"
        );
        // ⑦ consts::OS ↔ cfg! 동치 — 옵트인 축을 **상수로 고정**해 두 표현을 대조한다.
        //    (옵트인을 참으로 고정하면 매핑은 정확히 mac ∨ windows, 거짓으로 고정하면 mac 단독.)
        assert_eq!(
            d5_gate_for_os(std::env::consts::OS, true),
            cfg!(target_os = "macos") || cfg!(windows),
            "옵트인 참 고정 시 매핑은 cfg!(macos) ∨ cfg!(windows) 와 일치해야 한다"
        );
        assert_eq!(
            d5_gate_for_os(std::env::consts::OS, false),
            cfg!(target_os = "macos"),
            "옵트인 거짓 고정 시 매핑은 cfg!(macos) 단독이어야 한다(Windows 기본 미주입)"
        );
    }

    /// ★D5 옵트인 **판독기** 핀(2026-08-17 적대검증 2R minor 수리 — 강등 안전 사슬의 유일한
    /// 미핀 고리였다).
    ///
    /// 종전 실측: 게이트 매핑(위 ④)·삽입 재확인(⑤)·래퍼 라우팅(⑥)은 모두 고정돼 있었으나
    /// '**런타임 기본값이 거짓인가**'를 단언하는 테스트가 저장소 전체에 0건이었다. 그래서
    /// `d5_win_opt_in` 의 판정이 참으로 퇴행하면 Windows 가 기본 on 으로 되돌아가
    /// **앵커 ④(전 pane 사망)가 재무장**되는데도 어느 레인도 빨개지지 않았다:
    ///   · mac 레인 — 게이트가 옵트인과 무관하게 참이라 전건 초록.
    ///   · Windows `cargo test --lib claude_alt_screen` 레인 — 위 ⑥ 은 좌·우변이 **같은 판독기를
    ///     재호출**하므로 둘이 함께 참이 되어 여전히 초록(판독기에 대한 판별력 0).
    ///   · windows-health 의 `#[cfg(windows)]` 블록 — 게이트를 `d5_gate_for_os("windows", …)` 로
    ///     명시 주입해 판독기를 **의도적으로 우회**한다.
    /// 그래서 판정 규약을 순수 함수(`d5_win_opt_in_from`)로 떼어 여기서 직접 못박는다.
    ///
    /// ★이름에 `claude_alt_screen` 을 넣은 것은 **의도**다: Windows 레인은 전체가 아니라
    /// `cargo test --lib claude_alt_screen` **필터**로 돈다(release.yml 태그 레인 ·
    /// windows-health 는 `factory_reset::`/`d5_env_injection`). 이 축이 실제로 문제가 되는 OS 의
    /// 레인에서 함께 돌게 하려면 이름이 그 필터에 걸려야 한다 — 개명 시 그 사실을 확인하라.
    #[test]
    fn claude_alt_screen_win_opt_in_reader_pins() {
        // ① ★기본값 핀 — 두 채널 모두 없으면 **거짓**. 이 한 줄이 '강등이 살아 있는가'다.
        assert!(
            !d5_win_opt_in_from(None, false),
            "옵트인 채널이 하나도 없으면 판독은 거짓이어야 한다(Windows 기본 미주입 — 앵커 ④)"
        );
        // ② env 채널은 정확히 "1" 만 참(형제 게이트와 동형 · 느슨한 truthy 금지).
        assert!(d5_win_opt_in_from(Some("1"), false), "CYS_WIN_NO_ALT_SCREEN=1 은 옵트인");
        assert!(!d5_win_opt_in_from(Some("0"), false), "\"0\" 은 옵트인이 아니다");
        assert!(!d5_win_opt_in_from(Some("true"), false), "\"1\" 이외 값은 옵트인이 아니다");
        assert!(!d5_win_opt_in_from(Some(""), false), "빈 값은 옵트인이 아니다");
        // ③ 파일 채널은 단독으로 참 — Windows 정본 안내가 파일인 이유(setx 는 이미 뜬 GUI
        //    계보에 반영되지 않는다)는 `d5_win_opt_in` doc 참조.
        assert!(d5_win_opt_in_from(None, true), "~/.cys/win-no-alt-screen 존재만으로 옵트인");
        // ④ 두 채널은 OR — 파일이 있으면 env 가 "0" 이어도 참이다. env 로 **끄는** 극성은 이
        //    스위치가 아니라 승격 후의 `CYS_WIN_ALT_SCREEN_OFF` 몫이다(`d5_gate_for_os` doc).
        assert!(d5_win_opt_in_from(Some("0"), true), "두 채널은 OR — 파일이 있으면 참");
        // ⑤ 래퍼 배선 스모크 — 부작용 판독을 테스트가 **독립 재현**해 순수 코어에 먹인 값과
        //    래퍼 산출물이 일치해야 한다.
        //    ★판별력의 한계(정직): '읽는 위치'(env 이름·파일 경로)는 래퍼와 테스트가 같은
        //    상수를 공유하므로 이 줄이 아니라 ④-d(문서 결합 핀)가 지킨다. 여기서 잡는 것은
        //    래퍼의 **하드코딩·극성 반전**이다 — 옵트인이 없는 러너·개발기에서
        //    `fn d5_win_opt_in() -> bool { true }` 나 `!d5_win_opt_in_from(...)` 은 즉시 빨개진다.
        let env_now = std::env::var(D5_WIN_OPT_IN_ENV).ok();
        let file_now = home_dir().join(D5_WIN_OPT_IN_FILE).exists();
        assert_eq!(
            d5_win_opt_in(),
            d5_win_opt_in_from(env_now.as_deref(), file_now),
            "d5_win_opt_in 은 (env {D5_WIN_OPT_IN_ENV}, ~/{D5_WIN_OPT_IN_FILE} 존재)를 \
             d5_win_opt_in_from 에 먹이는 얇은 래퍼여야 한다"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// ★U-7 · 자식 스폰 규약 소스 핀 + 실동작 검증
// ════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod spawn_policy_tests {
    use super::{ChildLifetime, SpawnPolicy, ENV_NO_AUTOSTART, NO_AUTOSTART_ON};

    /// `#[cfg(test)]` 가 붙은 **항목만** 걷어낸 프로덕션 슬라이스. cys.rs 처럼 파일 중간에
    /// 인라인 테스트 모듈이 끼어 있는 파일이 있어서 "첫 `#[cfg(test)]` 앞까지" 방식은 쓸 수
    /// 없다(그 방식이면 cys.rs 의 프로덕션 12,000줄이 스캔에서 통째로 빠진다).
    ///
    /// ★[D 수리 · 2026-08-24] 종전 구현은 `#[cfg(test)]` 를 만나면 **열 0 의 `}`** 가 나올
    /// 때까지 무조건 버렸다. 그 항목이 블록이 **아니면**(단일 줄 `static`/`const`/`use`)
    /// 다음 col-0 `}` 는 한참 아래 다른 항목의 것이고, 그 사이 프로덕션이 **임의 분량 통째로**
    /// 사라진 채 핀은 조용히 초록이 된다(계측기 무효화). 실측 절단량: cys.rs 18,113→12,955줄 ·
    /// governance.rs 8,152→4,096줄. 지금은 **항목 단위로 분류**한다 —
    /// 블록(`… {`)이면 col-0 `}` 까지, 비블록(`… ;`)이면 그 항목만, 분류 불가면 **hard fail**.
    /// 조용한 소실은 구조적으로 불가능해졌고, 남은 실패는 전부 큰 소리로 난다.
    fn production_slice(src: &str) -> String {
        production_slice_checked(src).unwrap_or_else(|e| panic!("{e}"))
    }

    /// `production_slice` 의 판정부 — 실패를 `Err` 로 돌려준다(자기검증 테스트가 hard fail
    /// 축을 **직접 겨눌 수 있게**. 패닉만 있으면 검체가 그 경로를 재현할 수 없다).
    fn production_slice_checked(src: &str) -> Result<String, String> {
        /// `#[cfg(test)]` 항목 머리를 몇 줄까지 따라갈 것인가(다중 줄 시그니처 대비).
        const HEAD_SCAN_LIMIT: usize = 40;
        let lines: Vec<&str> = src.lines().collect();
        let mut out = String::with_capacity(src.len());
        let mut i = 0usize;
        while i < lines.len() {
            if lines[i] != "#[cfg(test)]" {
                out.push_str(lines[i]);
                out.push('\n');
                i += 1;
                continue;
            }
            // `#[cfg(test)]` 와 항목 머리 사이의 속성·주석·빈 줄은 그 항목의 일부다.
            let mut j = i + 1;
            while j < lines.len() {
                let t = lines[j].trim_start();
                if t.is_empty() || t.starts_with('#') || t.starts_with("//") {
                    j += 1;
                } else {
                    break;
                }
            }
            if j >= lines.len() {
                return Err("`#[cfg(test)]` 뒤에 항목이 없다(파일 절단 · 핀 무력화)".into());
            }
            // 항목 머리는 여러 줄일 수 있다(다중 줄 fn 시그니처). 먼저 만나는 종결이
            // `{` 면 블록 항목, `;` 면 비블록 항목이다.
            let mut h = j;
            let limit = (j + HEAD_SCAN_LIMIT).min(lines.len());
            while h < limit {
                let t = lines[h].trim_end();
                if t.ends_with('{') {
                    // 블록 항목 — 이 저장소 관례상 열 0 의 `}` 가 닫는다.
                    let mut k = h + 1;
                    while k < lines.len() && lines[k] != "}" {
                        k += 1;
                    }
                    if k >= lines.len() {
                        return Err(format!(
                            "`#[cfg(test)]` 블록(`{}`)을 닫는 열 0 의 `}}` 가 없다 — \
                             파일 끝까지 삼켰다",
                            lines[j].trim()
                        ));
                    }
                    j = k + 1;
                    break;
                }
                if t.ends_with(';') {
                    // 블록이 **아닌** 단일 항목(static/const/use/type) — 그 항목만 버린다.
                    j = h + 1;
                    break;
                }
                h += 1;
            }
            if h >= limit {
                return Err(format!(
                    "분류 불가한 `#[cfg(test)]` 항목 머리(`{}`) — 블록(`{{`)도 단일 항목(`;`)도 \
                     아니다. 종전처럼 열 0 의 `}}` 까지 통째로 버리면 그 사이 프로덕션이 \
                     **조용히** 사라지므로, 분류 불가는 통과가 아니라 실패다.",
                    lines[j].trim()
                ));
            }
            i = j;
        }
        Ok(out)
    }

    /// production_slice 자기 검증(계측 타당성 ①) — 이 도구가 실제로 테스트만 걷어내고
    /// 프로덕션은 남기는지 확인한다. 걷어내기가 과하면(=프로덕션까지 삭제) 아래 핀이
    /// **아무것도 안 보고 초록**이 되어 버린다. 그래서 도구부터 검체를 건다.
    #[test]
    fn production_slice_keeps_production_drops_tests() {
        let sample = "fn keep_me() {}\n#[cfg(test)]\nmod t {\n    fn drop_me() {}\n}\nfn keep2() {}\n";
        let p = production_slice(sample);
        assert!(p.contains("keep_me"), "프로덕션이 삭제됐다: {p}");
        assert!(p.contains("keep2"), "테스트 모듈 뒤 프로덕션이 삭제됐다: {p}");
        assert!(!p.contains("drop_me"), "테스트 모듈이 남았다: {p}");
        // ★[D 축] **블록이 아닌** `#[cfg(test)]` 항목(단일 줄 `static`/`const`/`use`).
        //   종전 구현은 `#[cfg(test)]` 를 보면 무조건 열 0 의 `}` 까지 버렸다 — 그 항목이
        //   블록이 아니면 다음 col-0 `}` 는 **저 아래 다른 항목**의 것이고, 그 사이 프로덕션이
        //   임의 분량 통째로 사라진다(핀이 빈 슬라이스를 보고 조용히 초록 = 계측기 무효화).
        //   실측 절단량: cys.rs 18,113→12,955줄 · governance.rs 8,152→4,096줄 규모.
        let nonblock = "fn keep_me() {}\n#[cfg(test)]\nstatic DROP_ME: u32 = 1;\n\
                        fn keep_after() {}\nmod z {\n    fn inner() {}\n}\nfn keep_last() {}\n";
        let np = production_slice(nonblock);
        assert!(!np.contains("DROP_ME"), "테스트 전용 단일 항목이 남았다: {np}");
        assert!(
            np.contains("keep_after"),
            "블록이 아닌 `#[cfg(test)]` 항목 뒤의 프로덕션이 통째로 사라졌다 — \
             이 절단은 무음이라 핀 전체가 헛돈다: {np}"
        );
        assert!(np.contains("fn inner"), "절단이 다음 블록까지 삼켰다: {np}");
        assert!(np.contains("keep_last"), "절단이 파일 끝까지 번졌다: {np}");
        // 실물에도 걸어 둔다 — 이 파일들의 프로덕션 앵커가 사라지면 핀이 헛도는 것이다.
        let cys = production_slice(include_str!("bin/cys.rs"));
        assert!(
            cys.contains("fn spawn_detached_daemon"),
            "cys.rs 프로덕션 앵커(spawn_detached_daemon) 소실 — 핀이 빈 문자열을 보고 있다"
        );
        assert!(
            cys.contains("fn run_scoped"),
            "cys.rs 프로덕션 앵커(run_scoped) 소실"
        );
        let ch = production_slice(include_str!("bin/cysd/channels.rs"));
        assert!(ch.contains("fn spawn_bridge"), "channels.rs 프로덕션 앵커 소실");
        let sc = production_slice(include_str!("bin/cysd/schedule.rs"));
        assert!(sc.contains("fn launch_via_cli"), "schedule.rs 프로덕션 앵커 소실");
    }

    /// 스폰 규약 스캔 대상 = **CLI·데몬 프로덕션 전량**.
    ///
    /// ★(U-7 결손 · 2026-08-24) 종전엔 6파일 화이트리스트였다. 그래서 `state.rs` 의
    /// **두 번째 정의처**(`HideConsole::hide_console` = `creation_flags` 직접 호출)와 그
    /// 호출부 `usage.rs` 가 통째로 핀 **밖**에 있었고, U-7 이 주장한 "단일 정의처"는
    /// 검체로는 거짓이었다. 목록을 좁게 유지하면 **다음 결손도 같은 방식으로 목록 밖에**
    /// 생긴다 — 스캔 비용이 0 이므로 전량으로 잠근다(화이트리스트 관리 자체가 결함원이다).
    const SPAWN_SCAN: &[(&str, &str)] = &[
        ("src/bin/cys.rs", include_str!("bin/cys.rs")),
        ("src/bin/cysd/accounts.rs", include_str!("bin/cysd/accounts.rs")),
        ("src/bin/cysd/alerts.rs", include_str!("bin/cysd/alerts.rs")),
        ("src/bin/cysd/analytics.rs", include_str!("bin/cysd/analytics.rs")),
        ("src/bin/cysd/approval.rs", include_str!("bin/cysd/approval.rs")),
        ("src/bin/cysd/approval_risk.rs", include_str!("bin/cysd/approval_risk.rs")),
        ("src/bin/cysd/caps.rs", include_str!("bin/cysd/caps.rs")),
        ("src/bin/cysd/channels.rs", include_str!("bin/cysd/channels.rs")),
        ("src/bin/cysd/classifier.rs", include_str!("bin/cysd/classifier.rs")),
        ("src/bin/cysd/cost.rs", include_str!("bin/cysd/cost.rs")),
        ("src/bin/cysd/deadman.rs", include_str!("bin/cysd/deadman.rs")),
        ("src/bin/cysd/delivery.rs", include_str!("bin/cysd/delivery.rs")),
        ("src/bin/cysd/events.rs", include_str!("bin/cysd/events.rs")),
        ("src/bin/cysd/governance.rs", include_str!("bin/cysd/governance.rs")),
        ("src/bin/cysd/handlers.rs", include_str!("bin/cysd/handlers.rs")),
        ("src/bin/cysd/hwmon.rs", include_str!("bin/cysd/hwmon.rs")),
        ("src/bin/cysd/main.rs", include_str!("bin/cysd/main.rs")),
        ("src/bin/cysd/recall.rs", include_str!("bin/cysd/recall.rs")),
        ("src/bin/cysd/schedule.rs", include_str!("bin/cysd/schedule.rs")),
        ("src/bin/cysd/severity.rs", include_str!("bin/cysd/severity.rs")),
        ("src/bin/cysd/skillrun.rs", include_str!("bin/cysd/skillrun.rs")),
        ("src/bin/cysd/state.rs", include_str!("bin/cysd/state.rs")),
        ("src/bin/cysd/undo.rs", include_str!("bin/cysd/undo.rs")),
        ("src/bin/cysd/usage.rs", include_str!("bin/cysd/usage.rs")),
    ];

    /// ★핵심 핀: 프로덕션의 자식 분리는 **전부 `spawn_policy` 를 경유**한다.
    ///
    /// 우회 스폰(`pre_exec`/`creation_flags` 직접 호출)이 하나라도 생기면 적색이다.
    /// 왜 이 핀인가: 이 규약은 "빠뜨리면 조용히 잘못 동작"하는 종류다 —
    /// Windows 에서 `CREATE_NEW_PROCESS_GROUP` 이 빠져도 스폰은 성공하고, 훅/콘솔이
    /// 잘리는 순간에만 자식이 동반 사망한다(PROBE V-c·WIN-3 H4). 런타임 검체로는
    /// 그 순간을 재현하기 어려우므로 **소스 층에서** 규약 이탈을 잡는다.
    #[test]
    fn no_bypass_child_separation_in_production() {
        for (name, src) in SPAWN_SCAN.iter().copied() {
            let prod = production_slice(src);
            // 문자열 조립으로 찾는다 — 이 테스트 소스 자체가 자기 검색어를 리터럴로
            // 담고 있어도 무해하지만(스캔 대상은 bin/*.rs 뿐), 의도를 분명히 한다.
            for needle in ["pre_exec(", "creation_flags("] {
                let n = prod.matches(needle).count();
                assert_eq!(
                    n, 0,
                    "{name} 프로덕션에 `{needle}` 직접 호출 {n}건 — 자식 분리는 \
                     cys::SpawnPolicy::spawn_policy 하나만 경유해야 한다(U-7). \
                     새 스폰 지점이면 ChildLifetime 등급을 먼저 정하고 헬퍼를 써라."
                );
            }
        }
    }

    /// 한 문장 안에서 등급 선언(`spawn_policy(`)과 콘솔 은폐 별칭(`hide_console(`)이
    /// **함께** 쓰였는가 — 그 문장을 돌려준다(빈 벡터 = 병용 없음).
    ///
    /// 왜 문장 단위인가: `creation_flags` 는 누적이 아니라 **덮어쓰기**다. 한 빌더 체인에서
    /// 뒤에 오는 호출이 앞의 flag word 를 통째로 지우므로,
    /// `.spawn_policy(ChildLifetime::GroupScoped).hide_console()` 은
    /// `CREATE_NEW_PROCESS_GROUP` 을 **조용히 지운다**(mac/Linux 무증상 · CI 전부 초록 ·
    /// Windows 에서만 원장 pgid 회수가 무력화되고 부모 콘솔 Ctrl-C 로 자식이 동반 사망).
    /// 두 호출이 별개 문장이면 이 위험이 없으므로 파일 단위가 아니라 문장 단위로 잰다.
    ///
    /// 문장 경계 = `;` `{` `}`. 중괄호까지 끊는 이유는 함수 경계를 넘어 두 호출이 한 조각에
    /// 섞여 **오탐**하는 것을 막기 위해서다(오탐은 반경 밖 파일을 볼모로 잡는다).
    /// 별칭의 **정의처**(`fn hide_console(`)는 호출이 아니므로 제외한다 — 정의는 등급
    /// `Attached` 로의 위임 그 자체이고, 그것이 이 수리의 내용이다.
    /// 알려진 판별력 한계(정직): 두 호출 사이에 중괄호 블록(클로저 등)이 끼면 놓친다.
    fn statements_mixing_lifetime_and_hide_console(prod: &str) -> Vec<String> {
        strip_line_comments(prod)
            .split([';', '{', '}'])
            .filter(|s| {
                s.contains("spawn_policy(")
                    && s.contains("hide_console(")
                    && !s.contains("fn hide_console(")
            })
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    }

    /// ★계측기 자기검증(계측 타당성) — 두 탐지기가 **실제로 탐지하는지**를 합성 표본으로
    /// 확인한다. 이 저장소의 현 트리에는 병용도 봉인 결손도 없어서(= 두 핀이 초록) 탐지기가
    /// 통째로 고장 나 있어도 아무 데서도 드러나지 않는다. 그 침묵을 여기서 깬다.
    #[test]
    fn spawn_pin_detectors_actually_detect() {
        // ① 병용 탐지: 같은 문장이면 잡고, 문장이 갈리면 안 잡는다.
        let mixed = "    cmd.spawn_policy(cys::ChildLifetime::GroupScoped).hide_console();\n";
        assert_eq!(
            statements_mixing_lifetime_and_hide_console(mixed).len(),
            1,
            "병용을 못 잡는다 — 이 핀은 아무것도 지키지 않는다"
        );
        let apart = "    cmd.spawn_policy(cys::ChildLifetime::Attached);\n    other.hide_console();\n";
        assert!(
            statements_mixing_lifetime_and_hide_console(apart).is_empty(),
            "별개 문장을 병용으로 오탐한다 — 오탐은 반경 밖 파일을 볼모로 잡는다"
        );
        // 주석 속 언급은 코드가 아니다(실제로 schedule.rs 가 그 형태다 — 오탐 시 적색이 된다).
        let commented = "    cmd\n        // 종전 `hide_console()` 과 동일한 flag\n        \
                         .spawn_policy(cys::ChildLifetime::Attached);\n";
        assert!(
            statements_mixing_lifetime_and_hide_console(commented).is_empty(),
            "주석을 코드로 읽는다"
        );
        // ② 형제 CLI 스폰 탐지: 봉인이 없는 합성 스폰을 실제로 찾아낸다.
        let unsealed = "let cli = crate::state::sibling_cli_path();\n\
                        let _ = Command::new(cli).arg(\"node-recover\").output();\n";
        let found = cli_spawn_sites("<합성>", unsealed);
        assert_eq!(found.len(), 1, "형제 CLI 스폰을 못 찾는다 — 봉인 핀이 눈이 멀었다");
        assert!(
            !found[0].contains(".no_autostart()"),
            "봉인 결손 표본에서 봉인을 봤다고 한다(탐지기 오작동)"
        );
        // 정의처는 스폰이 아니다 — 이 제외가 깨지면 state.rs 가 매번 오탐된다.
        assert!(
            cli_spawn_sites("<합성>", "pub fn sibling_cli_path() -> PathBuf {\n").is_empty(),
            "정의처를 스폰으로 오탐한다"
        );
        // ③ production_slice hard fail 축 — 분류 불가는 '통과'가 아니라 '실패'다.
        let unclassifiable = "fn keep() {}\n#[cfg(test)]\npub static X: u32 = compute(\n";
        assert!(
            production_slice_checked(unclassifiable).is_err(),
            "분류 불가한 `#[cfg(test)]` 항목을 조용히 통과시킨다 — 무음 절단이 되살아난다"
        );
        assert!(
            production_slice_checked("fn keep() {}\n#[cfg(test)]\nmod t {\n    fn d() {}\n")
                .is_err(),
            "닫히지 않은 테스트 블록이 파일 끝까지 삼켜도 통과한다"
        );
    }

    /// ★등급을 선언한 자식에 콘솔 은폐 별칭을 **이어 붙이지 않는다**(병용 금지).
    ///
    /// `hide_console()` 은 이제 `ChildLifetime::Attached` 의 별칭이므로, 다른 등급 뒤에 오면
    /// 그 등급의 flag word 를 통째로 덮어쓴다 — `GroupScoped` 의 `CREATE_NEW_PROCESS_GROUP` 이
    /// 사라지는 것이 정확히 그 사고다. **현 트리에는 병용이 0 건이고**, 이 핀은 그 0 을 잠근다
    /// (탐지기가 실제로 병용을 잡는다는 증명은 `spawn_pin_detectors_actually_detect` ①).
    #[test]
    fn lifetime_grade_and_hide_console_are_never_mixed() {
        for (name, src) in SPAWN_SCAN.iter().copied() {
            let prod = production_slice(src);
            let mixed = statements_mixing_lifetime_and_hide_console(&prod);
            assert!(
                mixed.is_empty(),
                "{name}: 등급 선언과 `hide_console()` 이 한 문장에 함께 있다 — \
                 `creation_flags` 는 덮어쓰기라 뒤에 오는 쪽이 앞 등급의 flag word 를 \
                 **조용히 지운다**(mac/Linux 무증상). 등급 하나로만 선언하라.\n{mixed:?}"
            );
        }
    }

    /// ★알려진 미편입 1건의 **동결 카운트**(범위 밖이라 빼는 게 아니라, 늘어나면 적색이 되게 건다).
    ///
    /// `src/factory_reset.rs::no_console_win` 은 `CREATE_NO_WINDOW` 한 줄짜리 로컬 헬퍼로,
    /// 등급표상 `Attached` 와 같은 flag 다(분리 없음 — 규약 위반이 아니라 **정의처 미통합**).
    /// U-7 의 대상 파일이 아니라 이번에 옮기지 않았다. 판정 축을 넓히는 대신 **현재값을 동결**해,
    /// 이 파일에 분리 flag 가 새로 생기거나 우회 스폰이 늘어나면 즉시 적색이 되게 한다.
    /// 이것은 완화가 아니다 — 종전에는 이 파일을 아무도 안 봤고, 지금은 1건 초과가 금지된다.
    #[test]
    fn known_unmigrated_separation_sites_are_frozen() {
        let fr = production_slice(include_str!("factory_reset.rs"));
        assert_eq!(
            fr.matches("creation_flags(").count(),
            1,
            "factory_reset.rs 의 창 은폐 flag 는 no_console_win 1건으로 동결돼 있다 —              늘었다면 새 스폰이 규약 밖에서 생긴 것이다(cys::SpawnPolicy 로 편입하라)"
        );
        assert_eq!(
            fr.matches("pre_exec(").count(),
            0,
            "factory_reset.rs 에 세션 분리가 새로 생겼다 — 회수 책임자가 없는 자식이다"
        );
    }

    /// 각 스폰 지점이 **자기 생사 등급을 이름으로 선언**했는지 핀. 등급이 사라지거나
    /// 다른 등급으로 슬쩍 바뀌면 적색 — 특히 `Survivor` 가 `Attached` 로 바뀌면
    /// CLI 가 죽을 때 데몬이 동반 사망하는 회귀다(부팅 전멸).
    #[test]
    fn every_detached_spawn_declares_its_lifetime_class() {
        let cys = production_slice(include_str!("bin/cys.rs"));
        assert!(
            cys.contains("ChildLifetime::Survivor"),
            "cys.rs 의 데몬 기동(spawn_detached_daemon)이 Survivor 등급을 잃었다 — \
             CLI 사망 시 데몬 동반 사망(부팅 전멸) 회귀"
        );
        assert!(
            cys.contains("ChildLifetime::ConsoleScoped"),
            "cys.rs 의 `cys run --` scoped 실행이 ConsoleScoped 등급을 잃었다"
        );
        let ch = production_slice(include_str!("bin/cysd/channels.rs"));
        assert!(
            ch.contains("ChildLifetime::GroupScoped"),
            "channels.rs 브리지가 GroupScoped 등급을 잃었다 — 원장 pgid 회수가 무력화된다"
        );
        let sc = production_slice(include_str!("bin/cysd/schedule.rs"));
        assert!(
            sc.contains("ChildLifetime::Attached"),
            "schedule.rs launch_via_cli 가 Attached 등급을 잃었다"
        );
    }

    /// 줄 주석(`//` 이후)을 지운 **코드만의 사본**. 구조를 재는 핀은 주석을 코드로 오인하면
    /// 안 된다 — 실제로 `governance.rs` 의 스폰은 주석 안의 `.output(` 때문에 스폰식이
    /// 엉뚱한 곳에서 잘렸다(주석 한 줄이 검체의 시야를 자른다).
    /// ★needle **개수**를 세는 핀(`no_bypass_child_separation_in_production`)에는 쓰지 않는다:
    /// 거기서는 주석 속 언급까지 세는 쪽이 안전측(과탐)이다.
    fn strip_line_comments(src: &str) -> String {
        let mut out = String::with_capacity(src.len());
        for line in src.lines() {
            match line.find("//") {
                Some(p) => out.push_str(&line[..p]),
                None => out.push_str(line),
            }
            out.push('\n');
        }
        out
    }

    /// 프로덕션 슬라이스에서 **형제 CLI(`sibling_cli_path()`)를 program 으로 쓰는 스폰**을
    /// 전부 찾아 그 스폰식 본문을 돌려준다. 정의처(`fn sibling_cli_path`)는 스폰이 아니므로 제외.
    ///
    /// 본문의 끝 = 첫 실행 종결자(`.output(` / `.spawn(` / `.status(`). 종결자를 못 찾으면
    /// **패닉**한다 — 검체가 볼 수 없는 형태로 바뀐 것을 조용히 통과시키면 그 순간 눈이 먼다.
    fn cli_spawn_sites(name: &str, prod: &str) -> Vec<String> {
        const NEEDLE: &str = "sibling_cli_path()";
        const TERMINATORS: [&str; 3] = [".output(", ".spawn(", ".status("];
        let code = strip_line_comments(prod);
        let mut out = Vec::new();
        let mut from = 0usize;
        while let Some(rel) = code[from..].find(NEEDLE) {
            let at = from + rel;
            from = at + NEEDLE.len();
            // 정의처(`pub fn sibling_cli_path() -> PathBuf {`)는 호출이 아니다.
            let line_start = code[..at].rfind('\n').map(|p| p + 1).unwrap_or(0);
            if code[line_start..at].contains("fn ") {
                continue;
            }
            let end = TERMINATORS
                .iter()
                .filter_map(|t| code[at..].find(t).map(|p| at + p + t.len()))
                .min()
                .unwrap_or_else(|| {
                    panic!(
                        "{name}: sibling_cli_path() 스폰의 실행 종결자\
                         (.output()/.spawn()/.status())를 못 찾았다 — 검체가 이 스폰을 볼 수 \
                         없다(핀 무력화). 형태가 바뀌었으면 종결자 목록을 먼저 늘려라."
                    )
                });
            out.push(code[at..end].to_string());
        }
        out
    }

    /// 데몬이 낳는 CLI 자식은 **전부** autostart 를 봉인한다(재귀 기동 = 데몬 폭주 ①).
    ///
    /// ★(P3-2 짝 · 2026-08-24) 종전 이 핀은 `schedule.rs::launch_via_cli` **함수 본문 한 곳만**
    /// 스캔했다. 그래서 `governance.rs` 의 `node-recover` 스폰(형제 CLI 를 그대로 띄운다)에
    /// `.no_autostart()` 가 빠진 것을 **보지 못했다** — "전부 봉인한다"는 주장이 검체로는
    /// 한 지점짜리였다. 판정 대상을 "`sibling_cli_path()` 를 program 으로 쓰는 프로덕션 스폰
    /// **전량**"으로 넓힌다(핀 이사 — 단언 수가 1 → 지점 수만큼으로 는다).
    #[test]
    fn daemon_spawned_cli_children_seal_autostart() {
        let mut sites = 0usize;
        for (name, src) in SPAWN_SCAN.iter().copied() {
            let prod = production_slice(src);
            for body in cli_spawn_sites(name, &prod) {
                sites += 1;
                assert!(
                    body.contains(".no_autostart()"),
                    "{name}: 형제 CLI 스폰에 CYS_NO_AUTOSTART 봉인이 없다 — 데몬이 낳은 cys 가 \
                     소켓 실패(데몬 종료 중·소켓 교체 중)를 만나면 `spawn_detached_daemon` 으로 \
                     **라이벌 데몬을 낳는다**(재귀 기동 · 폭주 ①).\n스폰식:\n{body}"
                );
            }
        }
        // 검체가 **아무것도 못 찾고** 초록이 되는 경로 차단(계측 타당성).
        // 현재 실재 지점: schedule.rs::launch_via_cli · governance.rs::node-recover.
        assert!(
            sites >= 2,
            "형제 CLI 스폰을 {sites}곳밖에 못 찾았다 — 탐지가 눈이 멀었다(needle·슬라이스 확인)"
        );
    }

    /// 등급표 자체의 핀 — 값 규약이 조용히 바뀌면 소비측(`cys.rs` autostart 게이트가
    /// `== \"1\"` 로 읽는다)이 무음으로 통과한다.
    #[test]
    fn no_autostart_contract_values() {
        assert_eq!(ENV_NO_AUTOSTART, "CYS_NO_AUTOSTART");
        assert_eq!(NO_AUTOSTART_ON, "1");
        let mut c = std::process::Command::new("true");
        c.no_autostart();
        let got: Vec<_> = c
            .get_envs()
            .filter(|(k, _)| *k == std::ffi::OsStr::new(ENV_NO_AUTOSTART))
            .collect();
        assert_eq!(got.len(), 1, "no_autostart 가 env 를 얹지 않았다");
        assert_eq!(
            got[0].1,
            Some(std::ffi::OsStr::new(NO_AUTOSTART_ON)),
            "값이 계약값이 아니다"
        );
    }

    /// ★실동작 검증(unix): 등급이 실제로 프로세스 그룹을 가르는가.
    /// 소스 핀만으로는 "헬퍼가 no-op 이어도 초록"이 되므로, 진짜 자식을 띄워
    /// `getpgid` 를 재본다 — 분리 등급은 부모와 다른 pgid, `Attached` 는 같은 pgid.
    #[cfg(unix)]
    #[test]
    fn spawn_policy_actually_separates_process_group_on_unix() {
        fn pgid(pid: u32) -> i32 {
            unsafe { libc::getpgid(pid as libc::pid_t) }
        }
        let mine = pgid(std::process::id());
        assert!(mine > 0, "자기 pgid 관측 실패 — 계측기 무효");

        let mut detached = std::process::Command::new("sleep");
        detached.arg("5").spawn_policy(ChildLifetime::GroupScoped);
        let mut d = detached.spawn().expect("분리 자식 spawn 실패");
        let dp = pgid(d.id());

        let mut attached = std::process::Command::new("sleep");
        attached
            .arg("5")
            .spawn_policy(ChildLifetime::Attached)
            .stdout(std::process::Stdio::null());
        let mut a = attached.spawn().expect("동반 자식 spawn 실패");
        let ap = pgid(a.id());

        let _ = d.kill();
        let _ = d.wait(); // 좀비 0
        let _ = a.kill();
        let _ = a.wait();

        assert_eq!(
            dp,
            d.id() as i32,
            "GroupScoped 자식의 pgid 가 자기 pid 와 다르다 — setsid 가 안 걸렸다"
        );
        assert_ne!(dp, mine, "GroupScoped 자식이 부모 그룹에 남았다(분리 무효)");
        assert_eq!(ap, mine, "Attached 자식이 부모 그룹을 벗어났다(동반 사망 계약 파손)");
    }

    /// `Survivor` 만 부모 파이프를 놓는다 — 실제로 stdout 이 상속되지 않는지 자식의
    /// 출력이 부모 캡처에 안 잡히는 것으로 확인한다(파이프 무점유 규약).
    #[cfg(unix)]
    #[test]
    fn survivor_does_not_hold_parent_pipes() {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg("echo LEAKED")
            .spawn_policy(ChildLifetime::Survivor)
            .output()
            .expect("spawn 실패");
        assert!(
            out.stdout.is_empty(),
            "Survivor 자식이 stdout 을 물려받았다 — 부모 파이프를 쥐면 부모 종료가 지연된다: {:?}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}
