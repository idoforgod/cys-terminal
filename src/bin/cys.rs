//! cys — the CYSJavis terminal CLI client. 모든 pane 안의 AI가 이 CLI로 동등 노드가 된다.
//! 예: cys send --surface surface:31 "..." ; cys send-key --surface surface:31 Return

use clap::{Parser, Subcommand};
// ★EXIT_BOOT_BUSY 정본은 lib 상수다(GUI cys-app·python javis_bootstrap 과 3자 공유 — 사본 금지).
use cys::{key_to_bytes, parse_surface_ref, socket_path, surface_ref, ENV_SURFACE_ID, EXIT_BOOT_BUSY};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};

#[derive(Parser)]
#[command(
    name = "cys",
    version,
    about = "cys — the CYSJavis terminal CLI (bidirectional socket, multi-agent OS)"
)]
struct Cli {
    /// Socket path override (default: AITERM_SOCKET or platform default)
    #[arg(long, global = true)]
    socket: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Ping the daemon
    Ping,
    /// Identify daemon + caller (uses AITERM_SURFACE_ID env when inside a surface)
    Identify,
    /// ★W1: 이 cys 바이너리 자신의 identity 3필드(build_id·embedded_pack_hash·protocol_version) JSON 출력.
    /// 데몬 불요(컴파일타임 상수 self-report) — phoenix 폴백이 데몬 status 의 동일 3필드와 교차대조한다.
    #[command(name = "phoenix-identity", hide = true)]
    PhoenixIdentity,
    /// Emit the data-derived command catalog (self-describing index — agents/LLM read this
    /// instead of re-parsing prose tables; the clap definition IS the single source of truth)
    Actions {
        #[arg(long)]
        json: bool,
    },
    /// Create a new surface (PTY session). Prints its surface ref.
    NewSurface {
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        cmd: Option<String>,
        #[arg(long)]
        title: Option<String>,
        /// Register this surface under a role (master/worker/cso/reviewer/...)
        #[arg(long)]
        role: Option<String>,
        #[arg(long, default_value_t = 35)]
        rows: u16,
        #[arg(long, default_value_t = 120)]
        cols: u16,
    },
    /// List surfaces
    List,
    /// Inject text into a surface's stdin (no trailing newline; follow with send-key Return)
    Send {
        #[arg(long)]
        surface: Option<String>,
        /// Address by role name instead of surface ref (e.g. --to master, --to 'reviewer-*')
        #[arg(long)]
        to: Option<String>,
        /// Followup mode: deliver when the target goes quiet (daemon queues + auto-injects with Return)
        #[arg(long)]
        queued: bool,
        /// 입력 버퍼 선정리(Ctrl-U) — launch-agent 등록 에이전트 pane 한정 (TUI별 의미 상이)
        #[arg(long)]
        clear_first: bool,
        /// Text to inject (multiple args are joined with spaces)
        #[arg(required = true)]
        text: Vec<String>,
    },
    /// Inject a named key (Return, Tab, C-c, Up, ...) into a surface's stdin
    SendKey {
        #[arg(long)]
        surface: Option<String>,
        /// Role name; supports glob (e.g. --to 'reviewer-*')
        #[arg(long)]
        to: Option<String>,
        /// Queue the key for quiet-time delivery (Return/Enter only) — typing-guard safe
        #[arg(long)]
        queued: bool,
        #[arg(required = true)]
        keys: Vec<String>,
    },
    /// T1-1 자기보고: 이 에이전트의 상태·컨텍스트%·작업을 데몬에 신고 (화면 파싱 대체)
    SetStatus {
        /// working | waiting | blocked | done
        #[arg(long, default_value = "working")]
        state: String,
        /// 컨텍스트 사용률 % (0-100)
        #[arg(long)]
        context: Option<u8>,
        /// 현재 작업 한 줄
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        surface: Option<String>,
    },
    /// T5 사용량 관측: 이 세션의 트랜스크립트 경로를 pane에 등록 (SessionStart hook 전용 plumbing)
    UsageRegister {
        /// 세션 트랜스크립트 절대경로 (.jsonl)
        #[arg(long)]
        transcript: String,
        #[arg(long)]
        surface: Option<String>,
    },
    /// T5 Phase 2-A: claude statusline stdin JSON을 읽어 usage.report로 push (cys-statusline.sh 전용 plumbing)
    UsageReportStdin {
        #[arg(long)]
        surface: Option<String>,
        /// push만 하고 사람용 statusline 한 줄을 출력하지 않는다 (기존 statusline 체인 보존 시).
        #[arg(long)]
        quiet: bool,
    },
    /// T7 E1-4: PreToolUse/PostToolUse hook stdin을 읽어 usage.event로 push (cys-hook.sh 전용 plumbing)
    UsageEventStdin {
        #[arg(long)]
        surface: Option<String>,
    },
    /// CC v2: 계정 단위 rate limit 뷰(로컬 데몬) — 자원 게이트·스크립트 소비용
    UsageAccounts {
        #[arg(long)]
        json: bool,
    },
    /// CC v2: RSI 학습 체크포인트 push — stdin JSON({round, verdict, stored, harness, discovery})을
    /// learn.checkpoint RPC로 전달 (javis_learn.py 전용 plumbing · 데몬 부재 시 exit 1)
    LearnCheckpoint,
    /// T1-2 통합 관제 보드: 전 노드 상태를 1콜로 (read-screen 폴링 대체)
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Tasks Control Center(CLI): 모든 부서의 모든 노드가 지금 하는 업무를 1콜로 (부서 다중소켓 집계)
    Fleet {
        #[arg(long)]
        json: bool,
    },
    /// T4-15 kill-switch: 큐 배달·스케줄 발화 동결 (직접 send는 통과 — '신경 차단'이지 행동 정지가 아님)
    Pause {
        #[arg(long, default_value = "")]
        reason: String,
    },
    /// kill-switch 해제 — 동결된 큐·스케줄 재개
    Resume,
    /// 업데이트 재시작 전 살아있는 노드에 저장 신호 + 유예 (best-effort drain)
    Drain {
        /// 저장 검증 모드: 노드별 체크포인트(SESSION_STATE) nonce 마커 기입을 결정론 확인 후
        /// 결과 JSON+exit code 반환 (기존 무인자 plain drain은 거동 불변 — best-effort 저장 신호만).
        #[arg(long)]
        verify: bool,
        /// verify 모드 노드별 검증 대기(초) — 전역 하드캡=timeout+마진. plain drain은 무영향.
        #[arg(long, default_value_t = 20)]
        timeout: u64,
    },
    /// preflight 게이트: exit 0 = running, 4 = paused (자율주행 매 action 전 확인용)
    GateCheck,
    /// 미배달 큐 검사·철회 (kill-switch의 짝)
    Queue {
        #[command(subcommand)]
        action: QueueAction,
    },
    /// T2-4 컨텍스트 60% 사이클 집행기: 저장 지시→파일 검증→clear→지침 재주입→재개 포인터
    CycleAgent {
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        /// 2-phase handshake 검증자 역할 — master cycle엔 필수 (self-clear 금지)
        #[arg(long)]
        verifier: Option<String>,
        /// 저장 검증 파일 (반복 가능; 기본: <cwd>/_round/SESSION_STATE.md 자동 탐지)
        #[arg(long = "save-file")]
        save_files: Vec<String>,
        /// clear 명령 override (기본: agents.json clear_cmd)
        #[arg(long)]
        clear_cmd: Option<String>,
        /// 재개 포인터 텍스트 override
        #[arg(long)]
        resume_text: Option<String>,
        #[arg(long, default_value_t = 120)]
        timeout: u64,
        /// 저장 파일 검증 없이 진행 (위험 — 명시 opt-out)
        #[arg(long)]
        force_no_verify: bool,
    },
    /// T2-5 죽은 에이전트를 같은 surface에서 재기동 + 지침 재주입 + 복원 포인터
    NodeRecover {
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        role: Option<String>,
    },
    /// T2-6 조직 복원: 토폴로지 스냅샷의 죽은 역할들을 일괄 재기동·재주입 (작업 재개는 master 판단)
    Restore {
        #[arg(long)]
        cwd: Option<String>,
        /// master 역할도 재기동 대상에 포함 (기본 제외 — restore 실행자가 보통 master)
        #[arg(long)]
        include_master: bool,
        /// 에이전트 resume 플래그(agents.json resume_arg) 미사용
        #[arg(long)]
        no_resume: bool,
    },
    /// T2-7 디렉티브 재주입 (+--check: 각성 핑으로 드리프트 감지 후 필요 시에만 재주입)
    Reinject {
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        /// 각성 확인 핑 먼저 — 응답 없을 때만 재주입
        #[arg(long)]
        check: bool,
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
    /// T3-14 완료 대기: scrollback 라인이 regex에 매칭될 때까지 블로킹 (plain-line 마커 규약)
    Watch {
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        to: Option<String>,
        /// 대기할 regex 패턴
        #[arg(long)]
        until: String,
        #[arg(long, default_value_t = 120)]
        timeout: u64,
        /// 이 라인 커서 이후부터 감시 (기본: 호출 시점 이후)
        #[arg(long)]
        since: Option<u64>,
    },
    /// T4-18 트랜스크립트 해시체인: pin(평가자 외부 보관) / verify(사후 변조 대조)
    Attest {
        #[command(subcommand)]
        action: AttestAction,
    },
    /// 온보딩③: 데몬 상시 가동 등록 — 재부팅 후에도 24/365 (macOS launchd / Windows 작업 스케줄러)
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Read a surface's screen (vt100-accurate) or last N scrollback lines
    ReadScreen {
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        lines: Option<u64>,
        /// T3-14 델타 읽기: 이 라인 커서 이후의 새 라인만 (stderr에 next_cursor 출력)
        #[arg(long)]
        since: Option<u64>,
        #[arg(long, default_value_t = 2000)]
        max_lines: u64,
    },
    /// Resize a surface
    Resize {
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        rows: u16,
        #[arg(long)]
        cols: u16,
    },
    /// Close a surface and force-kill its entire descendant process tree
    CloseSurface {
        surface: String,
        /// ★W2/C6: Reap 사유로 닫는다(묘비 미생성·부활 대상 유지) — 죽은 surface 잔재 회수용.
        /// 기본(플래그 없음)=OwnerClose(묘비 생성·의도적 폐역).
        #[arg(long)]
        reap: bool,
    },
    /// ★G4(W4-C): 죽은(exited) 좌석 수동 회수 — surface.reap RPC(권위 role 전용·7조건 게이트)
    ///
    /// close-surface(자기/생성자 한정)와 별개 계약: master/cso pane 에서 타 노드의 **죽은**
    /// 좌석을 즉시 회수한다(active surface 는 어떤 조합에서도 거부). exit 0=회수 완료 ·
    /// 7=게이트 거부(사유 stderr — claim-role rc=7 계열) · 1=오류.
    ReapSurface {
        surface: String,
    },
    /// ★W2/A-S3: 역할을 topology 묘비에 심는다(의도적 폐역). 데몬이 묘비 유일 작성자(단일 작성자 원칙).
    #[command(name = "tombstone")]
    Tombstone {
        role: String,
        /// 폐역 해제(재편입 가능).
        #[arg(long)]
        remove: bool,
        /// 부서(dept) 묘비 대상 — role 세션이 아니라 부서 데몬의 부활을 차단/해소한다
        /// (BOOTSTRAP_HARDENING WP-3 · dept_tombstone.set RPC). cys-dept가 삭제/재생성 시
        /// phoenix 묘비와 쌍으로 호출한다(한쪽만 있으면 재생성 부서 살해 또는 부활 구멍).
        #[arg(long)]
        dept: bool,
    },
    /// Subscribe to the daemon event stream (push; no polling)
    Events {
        #[arg(long)]
        after_seq: Option<u64>,
        #[arg(long = "name")]
        names: Vec<String>,
        #[arg(long = "category")]
        categories: Vec<String>,
        /// 이름 접두 필터(클라이언트측 뷰 필터) — 예: `--filter channel.` 로 채널 이벤트만 표시.
        /// --name(정확 일치·서버측)과 달리 접두 매칭이라 `channel.outbound.*` 등 계열 구독에 쓴다.
        #[arg(long = "filter")]
        filter: Option<String>,
        /// Auto-reconnect on connection loss
        #[arg(long)]
        reconnect: bool,
        /// 시작 커서를 이 파일에서 읽고(있으면), 매 이벤트마다 seq를 원자적으로 기록
        #[arg(long = "cursor-file")]
        cursor_file: Option<String>,
    },
    /// Mirror a surface's raw output to stdout (read-only tail)
    Attach { surface: String },
    /// Run a command in a new process group, registered in the daemon's process ledger.
    /// On exit the whole group is force-killed — 서버 생명주기 강제 종료.
    Run {
        #[arg(long)]
        surface: Option<String>,
        /// Command and arguments (after --)
        #[arg(required = true, last = true)]
        command: Vec<String>,
    },
    /// Show the process ledger (registered/scoped processes)
    Ps,
    /// Kill a ledger-registered process (group) by pid
    Kill { pid: u32 },
    /// Add a health rule (regex matched against every output line; fires health.alert)
    AddHealthRule {
        name: String,
        pattern: String,
        /// T4-17 조치 바인딩 (opt-in): pause-queue — 60초 창 threshold회 매칭 시 queued 배달 일시정지
        #[arg(long)]
        action: Option<String>,
        #[arg(long, default_value_t = 3)]
        threshold: u32,
        #[arg(long, default_value_t = 300)]
        pause_secs: u64,
    },
    /// List health rules
    HealthRules,
    /// Approval feed — 워커 승인 요청을 한 곳에 모아 처리
    Feed {
        #[command(subcommand)]
        action: FeedAction,
    },
    /// RSI 학습 루프 — 사람 직접 명령(제안 생성) 또는 현재 학습 라운드 상태 조회
    Learn {
        /// 학습 주제 (생략하고 --status면 상태 조회)
        topic: Option<String>,
        /// 현재 학습 라운드 상태(라운드·verdict·채택/rollback·발견)를 조회
        #[arg(long)]
        status: bool,
    },
    /// Install the CYSJavis Pack (multi-agent operating system templates) to ~/.cys/pack
    #[command(name = "init-pack", alias = "init-jarvis")]
    InitPack {
        /// Overwrite existing files (default: preserve user edits)
        #[arg(long)]
        force: bool,
        /// (기본 동작이 됨 — 하위호환용 no-op) SessionStart hook 등록
        #[arg(long, hide = true)]
        install_hook: bool,
        /// SessionStart hook 등록을 건너뛴다 (기본: ~/.claude*/settings.json 자동 탐색·등록)
        #[arg(long)]
        no_install_hook: bool,
        /// Claude settings.json 경로 명시 (생략 시 자동 탐색, 없으면 ~/.claude/settings.json 생성)
        #[arg(long)]
        claude_settings: Option<String>,
    },
    /// 무중단 팩 업데이트(재시작 0) — 서명된 팩을 검증→디스크 반영→살아있는 노드 reinject.
    /// 핵심 경로는 --from(로컬 디렉터리: pack.tar.gz + pack-manifest.json + .minisig).
    PackUpdate {
        /// 로컬 소스 디렉터리(pack.tar.gz + pack-manifest.json + pack-manifest.json.minisig)
        #[arg(long)]
        from: Option<String>,
        /// 원격 manifest URL (부차 — staging에 fetch; 핵심 로직은 --from으로 완전 테스트)
        #[arg(long)]
        manifest_url: Option<String>,
        /// 검증·버전게이트만 수행하고 디스크 반영·reinject는 생략(점검용)
        #[arg(long)]
        dry_run: bool,
    },
    /// 업데이트 드라이런(투명성) — 내장 팩 반영 시 갱신/보존/치유/병합대기/정리를 설치 **전에** 표시(쓰기 0)
    #[command(name = "pack-plan")]
    PackPlan {
        /// force 설치(init-pack --force) 기준으로 판정
        #[arg(long)]
        force: bool,
    },
    /// 커스터마이즈 병합 — 병합 대기 원장(.merge-pending.json)의 신버전(.new)·보존본(.user)을 검토·해소
    #[command(name = "pack-merge")]
    PackMerge {
        /// 대상 팩 상대경로(예: soul.md, directives/MASTER_DIRECTIVE.md). 생략 시 대기 목록 표시
        #[arg(long)]
        file: Option<String>,
        /// vendor 신버전 채택(내 수정 폐기)
        #[arg(long)]
        take_new: bool,
        /// 내 수정 유지(이번 신버전 병합 대기 해소 — vendor 가 또 전진하면 다시 병치)
        #[arg(long)]
        keep_mine: bool,
        /// AI(claude 헤드리스) 3-way 병합 제안 — 사용자 커스텀 의도를 신버전 베이스라인에 재적용
        #[arg(long)]
        ai: bool,
        /// (healed system 파일 전용) 보존본(.user)을 ~/.cys/local 오버레이로 이동(스킬 shadowing)
        #[arg(long)]
        to_local: bool,
        /// 내 수정(.user/현재본)과 vendor 본의 diff 를 제안 patch 파일로 생성(개선 환류 — 자동 전송 없음)
        #[arg(long)]
        propose: bool,
        /// 확인 프롬프트 없이 적용 (헌법 파일 병합·교체는 --yes 여도 확인 필수)
        #[arg(long)]
        yes: bool,
        /// (A12 승격 가드 override · 결정 D8) CEO 승격 중(.pre-ceo 존재)에도 MASTER_DIRECTIVE.md
        /// 를 vendor 본으로 강제 교체 — 승격 파괴를 승인하는 명시 플래그(기본 거부)
        #[arg(long)]
        force_vendor: bool,
        /// 해소를 적용하지 않고 검증·위험 요약·예정 판정까지만 수행(쓰기 0 — --take-new/--keep-mine 전용)
        #[arg(long)]
        dry_run: bool,
        /// (G3-축3 안전핵 게이트 override) 헌법 파일 take-new 가 안전핵 소실을 검출해도 강제 진행 —
        /// 소실 승인 명시 플래그(기본 거부 rc=7 · 검사·감사 원장 기록은 계속 수행)
        #[arg(long)]
        force_unsafe_core: bool,
    },
    /// 팩 상대경로의 소유권 등급(system|user|seed-once) 판정 — pack-guard hook·스크립트용 결정론 조회
    #[command(name = "pack-ownership")]
    PackOwnership {
        /// 팩 상대경로(예: bin/javis_learn.py, soul.md)
        rel: String,
        /// 등급 문자열만 출력(스크립트 소비용)
        #[arg(long)]
        quiet: bool,
    },
    /// 공용(~/.cys/claude)·개인(~/.claude*) settings.json 에서 **지정 팩을 가리키는 훅만** 제거 —
    /// 부서 teardown 잔존 치유의 단일 진입점(G3 축1 · 제거 엔진 단일). 경로 접두가 곧 소유 ID.
    #[command(name = "hooks-prune")]
    HooksPrune {
        /// 소유 팩 디렉터리(훅 명령 문자열에 박힌 절대경로 접두로 판정)
        #[arg(long)]
        pack_dir: String,
        /// 제거 대상·건수만 표시하고 아무것도 쓰지 않는다(관측 우선)
        #[arg(long)]
        dry_run: bool,
        /// base 팩(비 pack-dept-*) 대상 허용 — 기본은 게이트 거부(exit 7 · base 훅 오제거 fail-closed)
        #[arg(long)]
        allow_base: bool,
    },
    /// 직전 설치 보존본(<pack>.prev)에서 파일 단위 복원 — 업데이트 직후 "잃었다" 순간의 원커맨드 되돌리기
    #[command(name = "pack-rollback")]
    PackRollback {
        /// 복원할 팩 상대경로. 생략 시 .prev 와 현재 팩의 차이 목록 표시
        #[arg(long)]
        file: Option<String>,
        /// 확인 프롬프트 없이 적용
        #[arg(long)]
        yes: bool,
        /// (A12 승격 가드 override · 결정 D8) CEO 승격 중(.pre-ceo 존재)에도 MASTER_DIRECTIVE.md
        /// 를 보존본으로 강제 후진 — 승격 파괴를 승인하는 명시 플래그(기본 거부)
        #[arg(long)]
        force_vendor: bool,
        /// (G3-축3 안전핵 게이트 override · take-new 대칭) 헌법 파일 복원이 현재본 대비 안전핵
        /// 소실을 검출해도 강제 진행 — 소실 승인 명시 플래그(기본 거부 rc=7)
        #[arg(long)]
        force_unsafe_core: bool,
    },
    /// pro 라이선스("열쇠") 관리 — 검증·설치·typed 진단 (DESIGN-pro-license.md §7)
    License {
        #[command(subcommand)]
        action: LicenseAction,
    },
    /// pro 팩 설치를 free(내장 팩)로 강등 — 유일한 pro→free 경로 (license-aware·v3 §5)
    #[command(name = "pack-downgrade-to-free")]
    PackDowngradeToFree {
        /// 실제 실행(생략 시 현재 상태·계획만 출력)
        #[arg(long)]
        yes: bool,
        /// 유효 pro 라이선스가 실재해도 강행(기본 거부 — pro 앱 기능 ↔ free 팩 불일치 방지)
        #[arg(long)]
        override_valid_license: bool,
    },
    /// .pack-state.json(채널 상태) 진단·복구 — 권위 = accepted 기록 + pro 파일 증거 (v4 §5)
    #[command(name = "pack-repair-channel")]
    PackRepairChannel {
        /// 복구 대상 채널(free|pro). 생략 시 진단만 출력
        #[arg(long)]
        to: Option<String>,
        /// 실제 실행(생략 시 진단만)
        #[arg(long)]
        yes: bool,
        /// 증거 없는 전환 강행(전문가 전용 — loud 경고 동반)
        #[arg(long)]
        expert_override: bool,
    },
    /// 임베드 PACK+PACK_SKILLS에서 권위 manifest(pack-manifest.json)를 stdout JSON으로 방출.
    /// CI(release.yml)가 standalone 팩 manifest의 단일 SOT로 쓴다(임베드 콘텐츠→tree 동일성 게이트).
    #[command(name = "pack-manifest")]
    PackManifest {
        /// 서명 key_id 주입(미지정 시 생략 — CI 서명단계가 채움)
        #[arg(long)]
        key_id: Option<String>,
        /// 서명 발행 시각 Unix epoch 초(미지정 시 생략)
        #[arg(long)]
        signed_at: Option<i64>,
        /// 서명 만료 시각 Unix epoch 초(미지정 시 생략)
        #[arg(long)]
        expires_at: Option<i64>,
        /// 이 팩이 요구하는 최소 바이너리 버전(기본 빈 문자열=제약 없음)
        #[arg(long, default_value = "")]
        min_binary_version: String,
        /// pack_version 오버라이드(팩-only 릴리스 레인 — 미지정 시 CARGO_PKG_VERSION).
        /// 바이너리 범프 없이 팩만 전진시킬 때 CI(pack-release.yml)가 pack-v* 태그에서 주입한다.
        #[arg(long)]
        pack_version: Option<String>,
    },
    /// 시스템 자기진단·수리(§3.4) — pack 스큐·stale lock·고아 소켓·hook·채널 DB 무결 진단.
    /// --fix: stale lock·고아 소켓·staging 잔재 제거 + hook 재등록(사용자 데이터·pack 본체·DB 미삭제).
    Doctor {
        /// 감지된 문제를 자동 수리(안전 항목만). 생략 시 진단(읽기전용)만 수행한다.
        #[arg(long)]
        fix: bool,
        /// 진단 결과를 JSON으로 출력.
        #[arg(long)]
        json: bool,
        /// 커스터마이즈 실태 리포트 생성(로컬 파일 — 자발 제출용·자동 전송 없음)
        #[arg(long)]
        custom_report: bool,
    },
    /// 완전 초기화(팩토리 리셋) — 연습·사용 흔적 전부(부서·세션·대화기억·상태·훅·스킬링크)를
    /// cys-trash 로 격리하고 "설치 초기 상태"로 되돌린다. 라이선스·미등록 파일(오너 배치 *.env 등)은
    /// 보존, 복구는 격리 폴더 manifest.json 역방향 mv(14일 후 reap 자동 소거). DESIGN-factory-reset.md
    FactoryReset {
        /// 쓰기 0 프리뷰 — 격리·보존·해제 계획만 표시하고 아무것도 바꾸지 않는다.
        #[arg(long)]
        plan: bool,
        /// 확인 문구 입력 생략(스크립트용). 대화식에선 "완전 초기화" 정확 입력이 필요하다.
        #[arg(long)]
        yes: bool,
        /// 계획·결과를 JSON으로 출력.
        #[arg(long)]
        json: bool,
        /// 라이선스 파일도 격리한다(기본은 보존 — 구매물).
        #[arg(long)]
        purge_license: bool,
        /// 직접 만든 오버레이(~/.cys/local — 지침 append·스킬·훅)도 격리한다(기본은 보존).
        #[arg(long)]
        purge_local: bool,
        /// 사용자 프로젝트 폴더 안의 작업기억(_round)까지 격리한다(기본은 고지만 하고 남긴다).
        #[arg(long)]
        purge_round: bool,
        /// 계획 목록을 접지 않고 전량 표시한다(기본은 큰 항목 8건 + 분류별 소계).
        #[arg(long)]
        verbose: bool,
        /// 되돌리기 — 격리 폴더(cys-trash/factory-reset-*)의 복구 지도로 원위치 복원.
        /// `--plan` 과 함께 쓰면 무엇이 복구되는지만 표시한다(쓰기 0).
        #[arg(long, value_name = "TRASH_DIR")]
        undo: Option<String>,
    },
    /// Search the persistent transcript memory of ALL agents' terminal activity (FTS)
    Recall {
        /// Search text (substring matching via trigram FTS)
        query: String,
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        /// Only results from the last N days
        #[arg(long)]
        days: Option<f64>,
        #[arg(long, default_value_t = 20)]
        limit: u64,
    },
    /// Skill library — 경험을 스킬로 영속하고 재사용 (쓸수록 똑똑해지는 루프)
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// 노드 페르소나·운영 노브 커스터마이즈 (안전핵은 잠김). `cys persona list-params`로 노브 확인
    Persona {
        #[command(subcommand)]
        action: PersonaAction,
    },
    /// Heartbeat scheduler — 정해진 시각에 반복 업무를 자동 발화 (24/365 상주 데몬)
    Schedule {
        #[command(subcommand)]
        action: ScheduleAction,
    },
    /// D3: 비용·효율 eval baseline — tier 라우팅 도입 전후 '비용↓·품질불변' 검증(producer≠evaluator)
    #[command(name = "cost-baseline")]
    CostBaseline {
        #[command(subcommand)]
        action: CostBaselineAction,
    },
    /// Register the current (or given) surface under a role — for sessions started without launch-agent
    ClaimRole {
        /// Role: master / worker / cso / reviewer
        role: String,
        #[arg(long)]
        surface: Option<String>,
        /// ★SEAT: 보유자가 '빈 좌석'(agent 없는 셸·자손 프로세스 0·최근 입력 없음)이면 승계한다.
        /// 데몬이 그 좌석의 공허를 결정론으로 재판정하므로 요청일 뿐 강제가 아니다 — agent 가 붙은
        /// 정당한 보유자는 종전대로 거부된다. 부트 체인(javis_bootstrap ③)이 빈 셸에 막혀
        /// '유령 master' 데드엔드에 빠지던 경로를 푼다.
        #[arg(long = "takeover-empty-seat")]
        takeover_empty_seat: bool,
    },
    /// Mark a surface quiescing(=채널 inbox 주입 보류) or release it (§2.2 S5) — cycle-agent가 clear 전후 호출
    Quiesce {
        /// 대상 surface(role 주소는 --to 대신 surface ref). 미지정 시 자기 surface.
        #[arg(long)]
        surface: Option<String>,
        /// quiescing 해제(기본은 설정).
        #[arg(long)]
        off: bool,
    },
    /// Launch an AI agent in a new role surface and auto-inject its directive
    LaunchAgent {
        /// Role: master / worker / cso / reviewer
        #[arg(long)]
        role: String,
        /// Agent: claude / gemini(=Antigravity CLI agy) / codex / grok (defined in agents.json)
        #[arg(long)]
        agent: String,
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Boot the standard node set — 설치된 CLI만 자동 감지·기동·지침 주입. 표준 편성 4종(CSO 먼저 + worker claude + reviewer agy/codex) + 선택 grok
    Boot {
        /// Working directory for launched nodes
        #[arg(long)]
        cwd: Option<String>,
        /// 기계 판독 결과를 stdout 마지막 줄에 JSON 으로 낸다(B1·G11·G29·B8):
        /// `{"roles":[{"role","agent","outcome","mandatory","install_hint"}],"summary":{…}}`.
        /// outcome ∈ launched | already_alive | busy | missing | failed | recovered |
        /// skipped_unconfirmed.
        /// ★(W4) bare exit 계약: **0 = Fatal 없음(Degrade-only 포함) · 1 = Fatal 실패(mandatory 역할의
        /// failed·missing) · 75 = busy(다른 boot 가 락 보유 — 무스폰 skip)**. 세 의미가 분리돼 있으므로
        /// 소비부는 `!success()` 하나로 판정하지 말고 75 를 별도 분기해야 한다(위경보 방지).
        #[arg(long)]
        json: bool,
    },
    /// 어댑터 CLI 설치 감지 — agents.json 의 각 어댑터가 **지금 기동 가능한가**(바이너리 실재+실행권).
    /// 부트·python(detect_reviewer)·GUI 가 공유하는 단일 오라클(CS-1③). 데몬 무의존.
    #[command(name = "agent-detect")]
    AgentDetect {
        /// 기계 판독 JSON:
        /// `{"agents":{"<agent>":{"installed","bin","resolved","reason","hint"}}}`
        #[arg(long)]
        json: bool,
    },
    /// Print (creating if absent) this surface's role-specific TODO file path — 복수 워커가 같은 파일을 공유하지 않도록 역할별 고유 경로를 결정론적으로 산출.
    /// 새로 만드는 파일에는 선언 블록 v1 한 줄이 **자동 동봉**된다(집계기는 파일명이 아니라 이 선언으로 귀속을 판정한다).
    TodoPath {
        /// 다른 역할의 todo 경로/선언을 산출한다 — **경로 산출 전용**(파일 생성·기록 없음).
        /// 신원 게이트 우회 통로가 되지 않도록 남의 역할 파일은 절대 만들지 않는다(설계 R7).
        #[arg(long)]
        role: Option<String>,
        /// 경로 대신 **선언 한 줄**을 출력한다(손기재 오작성을 줄이는 기계 생성기 · 설계 §4-4 P1).
        #[arg(long)]
        emit_decl: bool,
    },
    /// Print this surface's cysd-authoritative role (one word) — PreToolUse capability-gate hook용.
    /// CYS_SURFACE_ID로 자기 surface를 찾아 데몬 roles 맵의 role을 출력(미등록 시 빈 줄·exit 0).
    SurfaceRole,
    /// HMAC signed-prefix 승인 — 위험명령 prefix를 1회 서명하면 이후 자동 통과(guard.sh 연동)
    Approval {
        #[command(subcommand)]
        action: ApprovalAction,
    },
    /// C0 채널 계층 — Slack·Discord 브리지 수명주기·인바운드·아웃바운드(브리지가 쓰는 thin RPC 래퍼, --json 출력)
    Channel {
        #[command(subcommand)]
        action: ChannelAction,
    },
    /// ★(U-22) 훅 결정 프런트도어 — 단명 훅의 판정을 데몬 인메모리 1왕복으로 옮긴다(근본원인 R2).
    ///
    /// stdout 에는 **아무것도 쓰지 않는다**(훅의 stdout 계약은 셸이 소유한다). 판정은 exit code
    /// 로만 전달하고 사유는 stderr 로 남긴다 — 자세한 계약은 [`run_hook_user_prompt_submit`].
    Hook {
        #[command(subcommand)]
        event: HookEvent,
    },
    /// ★(P2) 부트 인텐트 프런트도어 — 훅의 직접 spawn 을 데몬 감독자 스폰으로 이관하는 입구.
    ///
    /// 훅(role-bootstrap.sh)이 게이트 사슬(role→detect→machine-origin→선행 claim rc0)을 전부
    /// 통과한 뒤 부른다. RPC `boot.enqueue` 1왕복으로 인텐트를 데몬 스풀에 기록하고 즉시
    /// 돌아온다 — 실제 스폰은 데몬 감독자 소관이다. stdout 에는 **아무것도 쓰지 않는다**
    /// (훅의 stdout 계약은 셸이 소유한다). 판정은 stderr 토큰 1차 + exit 보조 —
    /// 자세한 계약은 [`run_boot_intent`].
    BootIntent,
}

/// `cys hook <event>` — 훅 이벤트별 결정 요청. 현재 UserPromptSubmit 하나뿐이다.
///
/// ★새 이벤트를 붙일 때 규율: 이 CLI 는 **판정을 하지 않는다**. 판정은 데몬(`hook.decide`)이
/// 인메모리 사실로 내리고, 여기서는 그 결과를 exit code 로 환원할 뿐이다. 판별 사본을 여기
/// 만들면 반드시 데몬과 갈린다(팩 규율 "판별 사본은 반드시 낡는다").
#[derive(Subcommand)]
enum HookEvent {
    /// UserPromptSubmit 훅(`hooks/role-bootstrap.sh`)의 role 게이트 위임.
    UserPromptSubmit,
}

#[derive(Subcommand)]
enum LicenseAction {
    /// 열쇠 번들(디렉터리 또는 파일 경로 + 형제 .minisig) 전건 검증 후 설치 — 실패 시 기존 무손상
    Install { path: String },
    /// typed 진단: free|pro|expired|revoked|invalid|key-expired + 서명키 잔여 수명 상시 병기
    Status,
}

#[derive(Subcommand)]
enum DaemonAction {
    /// 로그인 시 자동 기동 + 죽으면 자동 재기동(launchd KeepAlive) 등록
    Install {
        /// 가동 중인 기존 데몬을 정지하고 launchd에 소유권 이관 (세션 소멸 — 주의)
        #[arg(long)]
        takeover: bool,
    },
    /// 등록 해제 (가동 중인 데몬도 정지)
    Uninstall,
    /// 등록·가동 상태 확인
    Status,
}

#[derive(Subcommand)]
enum QueueAction {
    /// List undelivered queued messages (all surfaces or one)
    List {
        #[arg(long)]
        surface: Option<String>,
        /// RPC entries 원문(JSON 배열) 그대로 출력 — 텍스트 열 파싱 없이 기계 소비
        #[arg(long)]
        json: bool,
    },
    /// Drop all undelivered queued messages for a surface
    Clear { surface: String },
    /// ★G1(W2-E) 운영자 강제 배달 — **단건 전용**(--all 드레인 없음: 반복 강제는 틱당 1건
    /// 페이싱을 뚫는 유일 경로라 v1 제외 — 성찰 BLOCKER). 강제 = quiet 대기 생략만이며
    /// 안전 게이트(kill-switch pause·ACL·빈 좌석·사람 입력·헬스 pause·출력 quiet 1s 하한)는
    /// 전부 유지된다. exit: 0=배달 · 7=게이트 거부(사유 stderr · claim-role rc=7 선례 계열,
    /// 예약 {0,1,2,64} 비충돌) · 1=오류(대상/항목 없음·경합·통신 오류).
    Deliver {
        surface: String,
        /// 조준할 큐 항목 id (`cys queue list` 의 id 열) — 생략 시 머리 항목
        #[arg(long)]
        id: Option<String>,
        /// 조준 항목이 머리가 아닐 때 머리로 끌어올려 배달(순서 변경 명시 — queue.reordered 발행)
        #[arg(long)]
        allow_reorder: bool,
    },
}

/// ★G1(W2-E) queue.deliver 게이트 거부 exit — 예약 규약({0,1,2,64} 충돌 금지 · clap
/// 사용오류=2)과 분리해, 신 팩+구 바이너리 스큐에서 '명령/플래그 부재(clap 2)'와 '게이트
/// 거부'가 소비 스크립트에서 구분되게 한다. 값 7 = claim-role 정당거부(rc=7)·
/// EXIT_UNSAFE_CORE_REFUSED 선례 계열(타입드 거부 — 브리프 확정).
const EXIT_QUEUE_GATE_REFUSED: i32 = 7;

/// queue.deliver 거부 exit 판정(순수) — request() 에러 문자열("code: message")의 code 접두로
/// '안전 게이트 거부'(exit 7)와 '오류'(exit 1)를 가른다. 게이트 코드 목록은 데몬
/// governance::ForceDeliverDenied::code() + handlers "queue.deliver" 게이트 ①②와 1:1 계약 —
/// 목록 밖(대상/항목 없음·경합·통신 오류)은 전부 일반 오류(1)다(fail-closed 아님: 거부는
/// 데몬이 이미 확정했고 여기는 표기 층 분류만 한다).
fn queue_deliver_exit_code(err: &str) -> i32 {
    const GATE_CODES: [&str; 6] = [
        "paused",
        "acl_denied",
        "empty_seat",
        "typing_guard",
        "queue_paused",
        "output_busy",
    ];
    if GATE_CODES.iter().any(|c| err.starts_with(&format!("{c}:"))) {
        EXIT_QUEUE_GATE_REFUSED
    } else {
        1
    }
}

/// ★G4(W4-C) reap-surface 거부 exit 판정(순수) — request() 에러 문자열("code: message")의
/// code 접두로 '게이트 거부(reap_denied)'=exit 7 과 '오류(통신·not_found·invalid)'=exit 1 을
/// 가른다(queue_deliver_exit_code 관례 동형 · rc=7 = claim-role 정당거부·EXIT_UNSAFE_CORE_
/// REFUSED 선례 계열). 사유 코드(caller_unresolved|caller_role_forbidden|active_surface|
/// agent_still_alive|queue_not_empty|daemon_ancestor|grace_not_elapsed|state_changed)는
/// 메시지에 실려 소비 스크립트(javis_reap_exited.py)가 사유별 분기한다.
fn reap_surface_exit_code(err: &str) -> i32 {
    if err.starts_with("reap_denied:") {
        EXIT_QUEUE_GATE_REFUSED
    } else {
        1
    }
}

/// ★G4(W4-C) `cys reap-surface <ref>` — surface.reap RPC 호출. exit 0=회수 완료 ·
/// 7=게이트 거부(사유 코드 stderr) · 1=오류. close-surface 의미(자기/생성자 한정)는
/// 불변 유지 — 소비자 계약 보존(별도 동사·별도 RPC).
fn run_reap_surface(surface: &str) -> i32 {
    let Some(sid) = parse_surface_ref(surface) else {
        eprintln!("error: invalid surface ref: {surface}");
        return 1;
    };
    match request("surface.reap", json!({"surface_id": sid})) {
        Ok(r) => {
            println!(
                "reaped {} (manual reclaim{})",
                surface,
                r["role"]
                    .as_str()
                    .map(|role| format!(", role {role} released"))
                    .unwrap_or_default()
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            reap_surface_exit_code(&e)
        }
    }
}

/// ★G1(W2-B): `cys queue list` 텍스트 행 렌더 — 열 계약의 단일 소유자.
///
/// **열 위치 계약(절대 불변)**: 탭 구분 `surface_ref \t [index] \t <bytes>B \t preview`
/// 4열의 위치는 고정이다 — javis_boot_node.classify_delivery 가 cols[3]=preview 파싱으로
/// '재전송 금지'(wakeup 홍수 방어 게이트)를 판정한다(javis_boot_node.py:645-675). 신규 열
/// (id·age)은 반드시 preview **뒤** 말미에만 추가한다(중간 삽입 = pending 을
/// delivered_no_ack 로 오판 → 멱등 재주입 발동).
///
/// **preview 탭 미포함 불변식**: preview 본문의 탭·개행은 공백으로 치환한다 — 본문에 탭이
/// 실리면 열 파서가 뒤 열을 preview 로 오독해 위 계약이 무너진다(개행은 행 자체를 쪼갠다).
///
/// 신규 열 결손(restored 항목 등 id/age 부재)은 "-" 자리표시 — 열 개수는 항상 6으로 일정.
fn queue_list_row(e: &Value) -> String {
    let preview: String = e["preview"]
        .as_str()
        .unwrap_or("")
        .chars()
        .map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let id = e["id"].as_str().unwrap_or("-");
    let age = e["age_secs"]
        .as_u64()
        .map(|a| format!("{a}s"))
        .unwrap_or_else(|| "-".to_string());
    format!(
        "{}\t[{}]\t{}B\t{}\t{}\t{}",
        e["surface_ref"].as_str().unwrap_or("?"),
        e["index"],
        e["bytes"],
        preview,
        id,
        age,
    )
}

#[cfg(test)]
mod queue_list_row_tests {
    use super::*;

    /// 열 위치 회귀 핀 — cols[3]=preview 는 javis_boot_node 파싱 계약(위 doc comment).
    /// 신규 열(id·age)은 말미(cols[4]·cols[5])에만 있다.
    #[test]
    fn queue_list_row_pins_column_positions() {
        let e = serde_json::json!({
            "surface_ref": "surface:7", "index": 2, "bytes": 12, "preview": "보고 본문",
            "id": "q1a2b.3", "seq": 3, "age_secs": 45
        });
        let row = queue_list_row(&e);
        let cols: Vec<&str> = row.split('\t').collect();
        assert_eq!(cols.len(), 6, "열 개수 고정(4 기존 + id·age 말미 2)");
        assert_eq!(cols[0], "surface:7");
        assert_eq!(cols[1], "[2]");
        assert_eq!(cols[2], "12B");
        assert_eq!(cols[3], "보고 본문", "cols[3]=preview — javis_boot_node 파싱 계약");
        assert_eq!(cols[4], "q1a2b.3", "신규 id 열은 preview 뒤 말미");
        assert_eq!(cols[5], "45s", "신규 age 열은 최말미");
    }

    /// preview 탭·개행 미포함 불변식 — 본문에 탭이 실려도 열 경계를 침범하지 않는다.
    #[test]
    fn queue_list_row_preview_never_contains_tab_or_newline() {
        let e = serde_json::json!({
            "surface_ref": "surface:1", "index": 0, "bytes": 5, "preview": "a\tb\nc\rd",
            "id": "qx.1", "age_secs": 0
        });
        let row = queue_list_row(&e);
        let cols: Vec<&str> = row.split('\t').collect();
        assert_eq!(cols.len(), 6, "본문 탭이 열을 쪼개면 안 된다");
        assert_eq!(cols[3], "a b c d", "탭·개행·CR 은 공백 치환");
        assert_eq!(cols[4], "qx.1", "id 열 위치가 본문 탭에 밀리지 않는다");
        assert!(!row.contains('\n'), "행은 항상 한 줄");
    }

    /// 구형(restored 등) id/age 결손 항목 — "-" 자리표시로 열 개수 불변(파서 보호).
    #[test]
    fn queue_list_row_missing_new_fields_use_placeholder() {
        let e = serde_json::json!({
            "surface_id": 3, "restored": true, "mid": "qmm", "bytes": 4, "preview": "복원"
        });
        let row = queue_list_row(&e);
        let cols: Vec<&str> = row.split('\t').collect();
        assert_eq!(cols.len(), 6);
        assert_eq!(cols[0], "?", "surface_ref 부재 = 기존 '?' 동작 유지");
        assert_eq!(cols[1], "[null]", "index 부재 = 기존 null 표기 유지(무회귀)");
        assert_eq!(cols[3], "복원", "cols[3]=preview 는 결손 항목에서도 불변");
        assert_eq!(cols[4], "-");
        assert_eq!(cols[5], "-");
    }
}

#[derive(Subcommand)]
enum AttestAction {
    /// Print the current chain pin "count:hash" — 평가자가 SESSION_STATE 등 외부에 보관
    Pin {
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },
    /// Verify a previously saved pin against the stored transcript (exit 0=match, 2=mismatch)
    Verify {
        /// "count:hash" (pin 출력 그대로)
        pin: String,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },
}

#[derive(Subcommand)]
enum ApprovalAction {
    /// 명령이 서명된 prefix에 매칭하는지 확인 (exit 0=서명됨/통과, 비0=미서명/차단). guard.sh가 호출.
    Check {
        /// 검사할 전체 명령 문자열
        #[arg(long)]
        command: String,
        /// 명령 실행 cwd (생략 시 미지정 — 레코드가 cwd 무관이면 매칭)
        #[arg(long)]
        cwd: Option<String>,
    },
    /// 위험명령 prefix를 서명·영속 (master role surface에서만 허용 — 위조 서명 차단)
    Sign {
        /// 승인할 명령 prefix (공백 구분 토큰, 예: "git push")
        #[arg(long)]
        prefix: String,
        /// 승인 범위를 고정할 cwd (생략 시 cwd 무관 승인)
        #[arg(long)]
        cwd: Option<String>,
    },
}

/// C0 채널 서브명령 — 전부 channel.* RPC의 thin wrapper. 결과는 JSON 한 줄로 출력(브리지 소비용).
#[derive(Subcommand)]
enum ChannelAction {
    /// 브리지 스폰(cysd 자식·1회용 토큰) + desired-state enabled=1. 첫 기동엔 --cmd 필수.
    Start {
        channel: String,
        /// 브리지 실행 명령(sh -c 로 스폰). 첫 start에 등록되면 이후 재사용·재스폰.
        #[arg(long)]
        cmd: Option<String>,
    },
    /// 브리지 정지 + enabled=0(desired-state).
    Stop { channel: String },
    /// 채널 상태 스냅샷(alive·enabled·registered·outcome 분포·inbox 카운트·allowlist 수).
    Status,
    /// 브리지 자기등록(토큰+pid 이중검증). 응답에 pending outbound 전량 동봉.
    /// 토큰은 --token 대신 **env `CYS_CHANNEL_TOKEN`**(스폰 시 이미 주입됨)에서 읽는 것을 권장한다
    /// (M10: argv 노출=ps 유출 위험). --token 없으면 env로 폴백(argv는 하위호환).
    Register {
        channel: String,
        #[arg(long)]
        token: Option<String>,
        #[arg(long)]
        caps: Option<String>,
        #[arg(long = "bridge-ver")]
        bridge_ver: Option<String>,
    },
    /// 인바운드 메시지 제출(브리지→cysd). inbox-first 퍼널 판정. kind=interaction이면 승인 버튼 검증.
    Inbound {
        channel: String,
        #[arg(long = "sender-id")]
        sender_id: String,
        #[arg(long = "sender-kind", default_value = "user")]
        sender_kind: String,
        #[arg(long)]
        peer: Option<String>,
        /// 메시지 본문(kind=message일 때). interaction은 생략 가능.
        #[arg(long, default_value = "")]
        text: String,
        #[arg(long)]
        ts: Option<f64>,
        #[arg(long = "msg-ref")]
        msg_ref: Option<String>,
        /// 멱등 키: `<channel>:<플랫폼 msg id>` (Slack=ts·Discord=message id).
        #[arg(long = "idempotency-key")]
        idempotency_key: String,
        #[arg(long = "body-hash")]
        body_hash: Option<String>,
        /// 메시지 종류: message(기본) | interaction(승인 버튼 클릭).
        #[arg(long, default_value = "message")]
        kind: String,
        /// interaction 전용 — 대상 feed 항목 id.
        #[arg(long = "feed-id")]
        feed_id: Option<String>,
        /// interaction 전용 — 버튼 nonce(단회 hex).
        #[arg(long)]
        nonce: Option<String>,
        /// interaction 전용 — allow | deny.
        #[arg(long)]
        decision: Option<String>,
    },
    /// 아웃바운드 발신 요청(단조 상태기계·at-least-once). owner allowlist 대상만.
    Outbound {
        channel: String,
        #[arg(long)]
        target: String,
        #[arg(long, default_value = "message")]
        kind: String,
        #[arg(long)]
        body: String,
        #[arg(long = "reply-to")]
        reply_to: Option<String>,
        #[arg(long = "idempotency-key")]
        idempotency_key: String,
        #[arg(long = "retry-of")]
        retry_of: Option<i64>,
    },
    /// 아웃바운드 receipt 보고(브리지→cysd). 단조 전이·terminal 후는 late_receipt 화해.
    Receipt {
        #[arg(long = "outbound-id")]
        outbound_id: i64,
        /// sent|suppressed|partial_failed|failed|unknown
        #[arg(long)]
        outcome: String,
        #[arg(long = "platform-ref")]
        platform_ref: Option<String>,
        #[arg(long)]
        detail: Option<String>,
    },
    /// inbox 항목 ack(master가 처리 후 호출). state=acked·본문 소거.
    Ack { inbox_id: i64 },
    /// owner allowlist에 sender 추가(fail-closed 게이트 통과 대상).
    Allow { channel: String, sender_id: String },
    /// Tier C 원격 승인 기간 한정 opt-in(기본 OFF). --for 8h|30m|45s|1d, --off로 즉시 닫기.
    #[command(name = "allow-remote-approve")]
    AllowRemoteApprove {
        /// 여는 기간(예: 8h, 30m, 45s, 1d). --off와 상호배타.
        #[arg(long = "for")]
        duration: Option<String>,
        /// 즉시 닫기(기간 무시).
        #[arg(long)]
        off: bool,
    },
    /// owner allowlist에서 sender 제거(탈취 개별 철회).
    Revoke { channel: String, sender_id: String },
    /// 긴급 잠금 — 전 채널 브리지 즉시 정지·인바운드 전면 차단(터미널 1명령).
    Lockdown,
    /// lockdown 해제 — 인바운드 차단·reconcile 보류를 푼다(터미널 전용·H2). 채널 재개는 start로.
    Unlock,
}

#[derive(Subcommand)]
enum SkillAction {
    /// Create a new skill from experience (SKILL.md, 4-칸 본문 템플릿).
    /// 기본 생성 위치 = ~/.cys/local/skills (업데이트·치유가 절대 건드리지 않는 사용자 영역)
    New {
        /// kebab-case skill name
        name: String,
        #[arg(long)]
        description: String,
        /// 팩 디렉터리(vendor 영역)에 생성 — 개발 기계에서 upstream 승격 예정인 스킬 전용
        #[arg(long)]
        pack: bool,
    },
    /// List skill covers (name + description)
    List,
    /// Print a skill's full SKILL.md
    Show { name: String },
    /// D5: 보이는 일회용 워커로 스킬 1회 실행 (schedule --fresh 얇은 래퍼·invisible -p 금지)
    Run {
        /// 카탈로그의 skill name
        name: String,
        /// task-prompt 티켓 본문(javis_orchestra가 생성). 빈 값이면 거부(무계약 차단)
        #[arg(long)]
        ticket: String,
        /// 실행 워커 에이전트(agents.json 키)
        #[arg(long, default_value = "claude")]
        agent: String,
        /// fresh surface TTL(초). 미지정=schedule.rs 기본 TTL
        #[arg(long)]
        close_after: Option<u64>,
        /// CC v2: 보드 run 생애주기 추적 id(make_ticket이 발급·산출물 dir과 동일). 미지정=추적 없음
        #[arg(long)]
        run_id: Option<String>,
    },
}

#[derive(Subcommand)]
enum CostBaselineAction {
    /// 현재 7d 분포를 ~/.cys/_round/cost_baseline.json에 박제(sha256 핀·locked_at)
    Lock,
    /// 현재 vs 박제본 비교 → IMPROVED/REGRESSED/FLAT 판정(rework 초과는 reward-hack 차단)
    Diff,
}

#[derive(Subcommand)]
enum PersonaAction {
    /// 현 오버라이드 + 조립 미리보기 출력
    Show {
        #[arg(long, default_value = "master")]
        role: String,
    },
    /// 노브(--param key=val) 또는 페르소나(--persona "...") 저장 (둘 다 가능)
    Set {
        #[arg(long, default_value = "master")]
        role: String,
        #[arg(long)]
        param: Option<String>,
        #[arg(long)]
        persona: Option<String>,
    },
    /// 오버라이드 파일 삭제 → 정식 기본 복귀
    Reset {
        #[arg(long, default_value = "master")]
        role: String,
    },
    /// 튜닝 가능 노브·범위·기본값 표
    ListParams,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum ScheduleAction {
    /// Add a job to ~/.cys/pack/schedule.json (daemon hot-reloads)
    Add {
        #[arg(long)]
        id: String,
        /// "HH:MM" local time (반복 job — --in/--every와 택일)
        #[arg(long)]
        time: Option<String>,
        /// 주기 발화 간격(분) — 마지막 발화 후 N분마다 반복 (예: 5 = 5분 주기 보고 하트비트)
        #[arg(long)]
        every: Option<u64>,
        /// T3-10 원샷: 상대시간 후 1회 발화하고 job 자동 삭제 (예: 90s, 20m, 2h, 1h30m)
        #[arg(long = "in")]
        in_dur: Option<String>,
        /// fresh surface를 발화 후 N초 뒤 자동 close (원샷+fresh 누수 차단; --fresh 전용)
        #[arg(long)]
        close_after: Option<u64>,
        /// Comma-separated days (mon,tue,...). Omit for every day.
        #[arg(long)]
        days: Option<String>,
        /// Push this text to a role's stdin at the scheduled time
        #[arg(long)]
        text: Option<String>,
        /// Target role for --text (e.g. master)
        #[arg(long)]
        to: Option<String>,
        /// Run a shell command instead of pushing text
        #[arg(long)]
        command: Option<String>,
        /// If the target role is absent, launch it first (requires --agent)
        #[arg(long)]
        if_absent_launch: bool,
        /// Launch a NEW surface for every fire (isolation; requires --agent)
        #[arg(long)]
        fresh: bool,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        cwd: Option<String>,
    },
    /// List jobs and last-fired times
    List,
    /// Remove a job by id
    Remove { id: String },
    /// Fire a job immediately (verification; does not affect the schedule)
    RunNow { id: String },
}

#[derive(Subcommand)]
enum FeedAction {
    /// Push an item. --wait blocks until a decision arrives (exit 0=allow, 2=deny, 3=timeout)
    Push {
        #[arg(long, default_value = "permission")]
        kind: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "")]
        body: String,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long)]
        wait: bool,
        #[arg(long, default_value_t = 120)]
        timeout_secs: u64,
        /// 승인 tier(a|b|c|d). 채널 미러는 tier≤C만·무태그=D(미러 금지·fail-closed·§2.4-3).
        #[arg(long)]
        tier: Option<String>,
    },
    /// List feed items
    List {
        #[arg(long)]
        status: Option<String>,
    },
    /// Resolve a pending item (decision: allow / deny / free text)
    Reply {
        request_id: String,
        decision: String,
        /// 결재 사유(W3.3 감사 기록용). 한글·공백은 셸에서 단일 인용으로 감싼다.
        #[arg(long)]
        reason: Option<String>,
    },
}

fn main() {
    // ★SEAL-1 층3: 스레드 생성 전 프로세스 env 봉인 — 이 CLI 가 띄우는 **모든** 자손
    // (`cys run -- <임의명령>`·`launch-agent` 로 뜨는 pane·팩 python 헬퍼)이 상속으로 덮인다.
    // 층1(python_command)·층2(spawn_env_pairs)가 못 닿는 임의 명령 경로의 바닥(lib.rs SOT).
    cys::seal_python_bytecode_in_process();
    // 파이프(head 등)로 출력이 끊겨도 패닉하지 않도록 SIGPIPE 기본 동작 복원
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    if let Some(s) = &cli.socket {
        std::env::set_var(cys::ENV_SOCKET, s);
    }
    // 순수 프로브 명령은 자동 기동 금지 — "데몬이 떠 있는가"라는 질문의 답을 바꾸면 안 된다
    if matches!(
        cli.command,
        Command::Ping
            | Command::Daemon {
                action: DaemonAction::Status
            }
            // 완전 초기화는 데몬을 죽이는 명령 — 어떤 경로로도 자동 기동을 발화하면 안 된다.
            | Command::FactoryReset { .. }
    ) {
        AUTOSTART.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    let code = run(cli.command);
    std::process::exit(code);
}

fn target_surface(explicit: &Option<String>, to_role: &Option<String>) -> Result<u64, String> {
    if let Some(role) = to_role {
        let r = request("system.resolve_role", json!({"role": role}))?;
        return r["surface_id"]
            .as_u64()
            .ok_or_else(|| format!("role '{role}' resolved to invalid surface"));
    }
    if let Some(s) = explicit {
        return parse_surface_ref(s).ok_or_else(|| format!("invalid surface ref: {s}"));
    }
    if let Ok(env) = cys::env_compat(ENV_SURFACE_ID).ok_or(std::env::VarError::NotPresent) {
        if let Some(id) = parse_surface_ref(&env) {
            return Ok(id);
        }
    }
    Err("no --surface/--to given and CYS_SURFACE_ID is not set".into())
}

/// 명시된 --surface가 잘못된 형식이면 에러. 미지정(None)은 그대로 통과시켜
/// 호출처가 의미를 정한다 (env 폴백 또는 전체 검색).
fn parse_explicit_surface(surface: &Option<String>) -> Result<Option<u64>, String> {
    match surface {
        Some(s) => parse_surface_ref(s)
            .map(Some)
            .ok_or_else(|| format!("invalid surface ref: {s}")),
        None => Ok(None),
    }
}

/// T3-11 역할 글롭: '*'만 와일드카드 (reviewer-* 등)
fn cli_glob_match(pattern: &str, value: &str) -> bool {
    fn inner(p: &[char], v: &[char]) -> bool {
        match p.first() {
            None => v.is_empty(),
            Some('*') => {
                (0..=v.len()).any(|i| inner(&p[1..], &v[i..]))
            }
            Some(c) => v.first() == Some(c) && inner(&p[1..], &v[1..]),
        }
    }
    inner(
        &pattern.chars().collect::<Vec<_>>(),
        &value.chars().collect::<Vec<_>>(),
    )
}

/// T3-11: --to에 글롭이 오면 매칭되는 살아있는 역할 전부로 확장 (브로드캐스트)
fn resolve_targets(explicit: &Option<String>, to: &Option<String>) -> Result<Vec<u64>, String> {
    if let Some(role_pat) = to {
        if role_pat.contains('*') {
            let r = request("surface.list", json!({}))?;
            let ids: Vec<u64> = r["surfaces"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter(|s| !s["exited"].as_bool().unwrap_or(true))
                .filter(|s| {
                    s["role"]
                        .as_str()
                        .map(|x| cli_glob_match(role_pat, x))
                        .unwrap_or(false)
                })
                .filter_map(|s| s["surface_id"].as_u64())
                .collect();
            if ids.is_empty() {
                return Err(format!("no live roles match '{role_pat}'"));
            }
            return Ok(ids);
        }
    }
    target_surface(explicit, to).map(|sid| vec![sid])
}

/// surface.list에서 한 surface의 항목 조회 (agent 메타·role·cwd 확인용)
fn surface_entry(sid: u64) -> Result<Value, String> {
    let r = request("surface.list", json!({}))?;
    r["surfaces"]
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|s| s["surface_id"].as_u64() == Some(sid))
                .cloned()
        })
        .ok_or_else(|| format!("surface {sid} not found"))
}

/// cmd 문자열의 env-prefix(KEY=VAL 토큰) 판별 — boot의 바이너리 존재 검사가 env 대입을
/// 바이너리명으로 오판하지 않게 한다. 값에 공백이 없는 단순 대입만 가린다(현 어댑터 cmd 한정).
fn is_env_assignment(tok: &str) -> bool {
    match tok.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name
                    .chars()
                    .next()
                    .map_or(false, |c| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// cmd에서 env-prefix(KEY=VAL)를 건너뛴 실제 바이너리 토큰을 고른다 — boot 설치판정과
/// agent_bin 메타등록이 공유하는 단일 진실(한 곳만 고쳐 다른 곳이 누락되던 codex R1 회귀 차단).
/// 한계(agy R1 지적2): split_whitespace 기반이라 값에 공백이 든 따옴표 대입(KEY="a b")은
/// 미지원 — 현 어댑터 cmd 3종은 공백 없는 env 값이라 영향 없다(범위 한정).
fn extract_bin<'a>(cmd: &'a str, fallback: &'a str) -> &'a str {
    cmd.split_whitespace()
        .find(|t| !is_env_assignment(t))
        .unwrap_or(fallback)
}

/// 데몬이 낸 오류가 **타이핑 가드 거부**인가 — 판정 근거는 lib 단일 소스 문구다.
/// (와이어는 `error.message` 만 전달하므로 코드 문자열은 클라이언트에 도달하지 않는다 —
///  그래도 둘 다 보는 이유는 미래에 코드가 전달되도록 바뀌어도 이 분기가 살아있게 하려는 것.)
fn is_typing_guard_err(e: &str) -> bool {
    e.contains(cys::MSG_TYPING_GUARD) || e.contains(cys::ERR_TYPING_GUARD)
}

/// ★B3(0.14.24) `cys send-key` 가 타이핑 가드 거부를 **큐로 1회 전환**해야 하는가(순수 판정).
///
/// 왜 필요한가: 노드 보고 경로는 `cys send --to master "<본문>"` + `cys send-key --to master
/// Return` 두 프로세스다. 그런데 `Command::SendKey` 는 `request(...)?` 로 에러를 그대로
/// 올려보냈다 — 타이핑 가드에 걸리면 **Return 이 그냥 소실**된다(본문만 남고 미제출).
/// 같은 저장소의 `inject_text` 는 이미 T-0147-6 으로 `--queued` 1회 전환을 갖고 있었는데,
/// CLI 표면에만 그 폴백이 없던 비대칭이 결함3 의 마지막 층이다.
///
/// 조건: 이미 `--queued` 면 전환할 것이 없고, Return/Enter 가 아니면 텍스트 큐에 실을 수
/// 없으며(데몬 계약), 타이핑 가드가 아닌 거부(ACL·종료·큐 만석…)는 **절대** 큐로 바꾸지
/// 않는다 — 오폴백은 거부를 성공으로 위장한다.
fn should_queue_fallback_send_key(queued: bool, key: &str, err: &str) -> bool {
    !queued && matches!(key, "Return" | "Enter") && is_typing_guard_err(err)
}

/// ★B3 `cys send` 본문의 큐 1회 전환 판정(순수) — send-key 와 같은 근거·같은 보수성.
///
/// `clear_first` 를 제외하는 이유: 원자 clear+paste+submit 은 **직접 전달 전용**이고 데몬이
/// `--queued` 와의 결합을 invalid_params 로 거부한다(handlers send_text). 그 조합을 폴백으로
/// 만들면 안내 대신 두 번째 오류를 낳는다.
fn should_queue_fallback_send(queued: bool, clear_first: bool, err: &str) -> bool {
    !queued && !clear_first && is_typing_guard_err(err)
}

/// ★B3 보조 — 큐 전환 직후 데몬이 pause 중이면 한 줄 경고한다.
///
/// pause 중에는 큐 배달이 동결되므로(kill-switch 의미론 — 이 코드는 그 의미론을 **바꾸지
/// 않는다**) 전환은 성공했어도 실제 제출은 resume 후다. 그 사실을 말해주지 않으면 호출자가
/// "보냈는데 왜 조용하지"로 오해한다. `cys status` 와 같은 RPC(org.status)를 쓴다.
/// best-effort: 조회 실패는 침묵한다 — 폴백 자체는 이미 성공했고 여기서 exit 코드를 바꾸지
/// 않는다(경고 채널이 주 경로를 망치면 안 된다).
fn warn_if_daemon_paused() {
    if let Ok(r) = request("org.status", json!({})) {
        if r["paused"].as_bool() == Some(true) {
            eprintln!("[queue] 데몬 pause 중 — 큐는 resume 후 배달됩니다");
        }
    }
}

/// 좌석의 **첫 각성 ack** 관측(생애 창) — `Some(true)`=각성함 · `Some(false)`=아직 · `None`=판정 불가.
///
/// ★'부재 ≠ 부정' 규약(래치·`gate_pending` 축과 동형): 구 데몬은 `awakened_at` **키 자체를
///   내보내지 않는다**. 그 부재를 "아직 각성 안 함(창 열림)" 으로 접으면 살아서 일하는 노드
///   전부가 관문 스캔 대상이 되고, 그중 하나가 화면에 관문 문면을 출력하는 순간(감사 문서·
///   `first_run_gates.rs` 를 `cat` 하는 순간이 그렇다) 그 노드로 가는 주입이 영구 거부된다
///   — 치명위험 ①(오탐 폭주). 그래서 **키 부재는 `None`**(가드 비활성 = 종전 동작)이고,
///   신 데몬이 내보내는 `null`(= 래치 미설정)만 '창 열림' 이다.
///
/// ★(P4-4) 이 판정은 **이미 손에 든 `surface.list` 행**을 받는다(종전 `surface_awakened(sid)`
///   는 자기가 왕복을 쳤다). 가드는 각성 창과 **좌석의 어댑터**를 함께 필요로 하는데, 둘을
///   각자 `surface.list` 로 조회하면 왕복이 두 번이고 그 사이에 값이 갈리면 "창은 열렸는데
///   어댑터는 다른 좌석의 것" 이라는 **찢어진 관측**이 판정에 실린다. 한 왕복의 스냅샷을 나눠
///   쓰는 것이 그 표면 자체를 없앤다.
fn surface_awakened_in(rows: &[Value], sid: u64) -> Option<bool> {
    let row = rows.iter().find(|s| s["surface_id"].as_u64() == Some(sid))?;
    let v = row.get("awakened_at")?; // 키 부재(구 데몬) = 판정 불가
    if v.is_null() {
        return Some(false); // 신 데몬 · 래치 미설정 = 첫 부트 창
    }
    Some(v.as_f64().unwrap_or(0.0) > 0.0)
}

/// 좌석에 등록된 어댑터 이름(`surface.set_meta` 의 `agent`). `None` = 미등록(맨 셸·구 데몬).
fn surface_agent_in(rows: &[Value], sid: u64) -> Option<String> {
    rows.iter()
        .find(|s| s["surface_id"].as_u64() == Some(sid))
        .and_then(|s| s["agent"].as_str())
        .filter(|a| !a.is_empty())
        .map(|a| a.to_string())
}

/// 화면의 마지막 비공백 줄 `n` 개(진단 문안 전용). 보류 처방·에러 본문이 공유한다 — 이 계산을
/// 호출부마다 다시 쓰면 "어떤 경로에서 보고된 꼬리인가"에 따라 길이·필터가 갈린다.
fn screen_tail_lines(screen: &str, n: usize) -> String {
    let lines: Vec<&str> = screen.lines().filter(|l| !l.trim().is_empty()).collect();
    lines
        .iter()
        .rev()
        .take(n)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

/// pane 의 현재 화면(vt100 그리드) 관측. `None` = **관측 실패**(RPC 실패·응답 스키마 스큐).
///
/// ★fail-open 방향의 근거(무변): 이 가드는 종전에 없던 **추가** 안전망이다. 관측 실패의 귀결이
///   '새 차단' 이면, 데몬 왕복 한 번이 흔들릴 때마다 오늘 되던 주입이 안 되게 된다.
///   반대로 접었을 때 잃는 것은 '가드가 한 틱 눈을 감는 것' 뿐이고, 그 자리는 U-13 의 ready
///   판정이 이미 같은 코퍼스로 한 번 막고 있다(2중 그물).
///
/// ★그러나 fail-**silent** 는 끝낸다(P4-6 · 2026-08-24 이종 리뷰어). 종전 반환은 `String` 이고
///   실패는 `.unwrap_or_default()` 로 `""` 였다 — 그 순간 **'관측 실패' 와 '빈 화면(관문 없음)'
///   이 같은 값**이 되어 `identify` 가 `None` 을 내고 `Decision::Send` 로 흘렀다. **로그 0.**
///   스키마 스큐 한 번이면 U-14 그물 전체가 **무증상 통과**한다 — 그물이 없는 것과 그물이
///   있는데 눈을 감은 것은 밖에서 구별되지 않고, 그래서 아무도 고치지 않는다.
///   두 사실을 **타입으로 가르고**, 접는 자리에서 반드시 소리를 낸다
///   (이 저장소가 `agent.observe_dropped` 에서 이미 세운 규율).
fn gate_guard_screen(sid: u64) -> Option<String> {
    request("surface.read_text", json!({"surface_id": sid}))
        .ok()
        .and_then(|r| r["text"].as_str().map(|s| s.to_string()))
}

/// 관측 실패를 **소리 내어** fail-open 으로 접는 단일 지점(P4-6).
///
/// 진단 문안 전용 호출(보류 처방의 화면 꼬리)은 여기를 쓰지 않는다 — 그쪽은 판정이 아니라
/// 에러 본문이라 조용한 빈 문자열이 정확하다.
fn gate_guard_screen_or_warn(sid: u64, stage: &str) -> String {
    match gate_guard_screen(sid) {
        Some(text) => text,
        None => {
            eprintln!(
                "[inject-guard] ⚠ 화면 관측 실패(surface.read_text 무응답 또는 응답 스키마 스큐) \
                 — 관문 판정을 **건너뛰고** 종전대로 {stage} 를 보낸다(fail-open) surface={sid}. \
                 ★이것은 '관문 없음' 이 아니라 **가드가 눈을 감은 것**이다"
            );
            String::new()
        }
    }
}

/// ★(P4-4) 관문 코퍼스 해소의 **단일 소스**. 프로덕션에서 `resolve_from_spec` 을 부르는 지점은
/// 여기 하나다(소스 핀 `gate_corpus_has_a_single_production_source_pin` 이 집행).
///
/// 【무엇이 틀렸었는가 — 리뷰어 격리 실행】 부트 폴링·`adapter_ready` 는 어댑터 스펙에서
/// 해소한 코퍼스(`resolve_from_spec` — `agents.json` override 봉투가 도달한다)를 썼는데,
/// **주입 직전 그물**(`gate_guard_check`)과 부서 소켓 판은 `builtin()` 을 썼다. 그래서 벤더
/// 드리프트로 빌트인 관문이 오탐하면 운영자가 문서대로 `agents.json` 봉투로 고쳐도 **그물은
/// 계속 막았다** — BLOCK-3("문서화된 탈출구가 듣지 않았다")과 **같은 형태**다. 소스가 두 벌인
/// 한 override 는 거짓말이다.
///
/// 【판독 실패의 귀결 — 조용한 '관문 부재' 로 접지 않는다(결함 4와 같은 축)】
/// `agents.json` 이 없거나 손상되면 **코드 정본으로 되돌리고 시끄럽게 남긴다.** 빈 코퍼스로
/// 접으면 그 순간 관문 축이 통째로 사라지는데(=관문 화면에 주입), 그것은 판독 실패가 만들
/// 수 있는 결과 중 가장 나쁘다. `first_run_gates::resolve_with` ③(빈 코퍼스는 '관문 없음' 이
/// 아니라 '눈을 감음')과 **같은 규율**이다.
///
/// 【캐시】 이 함수는 주입 경로에서 반복 호출된다. 해소는 파일 읽기 + JSON 파싱 + 자기규칙
/// 집행이라 틱마다 돌리면 비싸고, 더 나쁘게는 **판정 재료가 도중에 바뀐다**(관측의 일관성
/// 상실 — 부트 폴링이 코퍼스를 루프 밖에서 1회만 해소하는 이유와 같다). 프로세스 수명 동안
/// 어댑터당 1회로 고정한다.
fn resolve_gate_corpus(agent: &str) -> cys::first_run_gates::Resolved {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, cys::first_run_gates::Resolved>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    // poison 내성 — 진단 캐시가 판정 경로를 죽이지 않는다(U-18 과 같은 패턴).
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = map.get(agent) {
        return hit.clone();
    }
    let resolved = match load_agent_spec(agent) {
        Ok(spec) => cys::first_run_gates::resolve_from_spec(&spec),
        Err(e) => {
            eprintln!(
                "[inject-guard] ⚠ 어댑터 '{agent}' 스펙 판독 실패({e}) — 관문 코퍼스를 \
                 **코드 정본으로** 되돌린다(override 봉투는 이번 프로세스에서 도달하지 않는다). \
                 ★'관문 없음'으로 접지 않는다: 빈 코퍼스는 관측이 아니라 맹목이다"
            );
            cys::first_run_gates::Resolved {
                gates: cys::first_run_gates::builtin(),
                notes: vec![format!("어댑터 스펙 판독 실패({e}) — 코드 정본 폴백")],
                source: cys::first_run_gates::Source::Builtin,
            }
        }
    };
    // 진단은 **해소당 1회**만 낸다(캐시 히트에서 반복 인쇄하면 로그가 판정을 덮는다).
    for n in &resolved.notes {
        eprintln!("[inject-guard] 관문 코퍼스({agent}): {n}");
    }
    map.insert(agent.to_string(), resolved.clone());
    resolved
}

/// 좌석의 어댑터로 해소한 관문 코퍼스 — 어댑터 **미상**이면 코드 정본이다.
///
/// ★'미상' 과 '판독 실패' 는 다른 사실이다(위 [`resolve_gate_corpus`] 참조). 어댑터가 등록되지
///   않은 좌석(맨 셸·구 데몬)은 봉투가 걸릴 자리 자체가 없으므로 코드 정본이 곧 정답이고,
///   그것은 종전 동작이기도 하다 — 여기서 소리를 내면 정상 경로가 시끄러워져 진짜 실패가 묻힌다.
fn gate_corpus_for_seat(agent: Option<&str>) -> Vec<cys::first_run_gates::Gate> {
    match agent {
        Some(a) => resolve_gate_corpus(a).gates,
        None => cys::first_run_gates::builtin(),
    }
}

/// ★U-14 주입·제출 가드 — **부트 경로 전용**(생애 창을 상수로 연다 · 코퍼스는 해소본).
///
/// 부트 폴링·주입은 정의상 첫 각성 ack **이전**에 도는 코드다. 창 여부를 데몬에 묻지 않는
/// 이유가 여기 있다 — 구 데몬은 `awakened_at` 키가 없어 `surface_awakened` 가 `None` 을 내고,
/// 그러면 **가드가 가장 필요한 자리에서 조용히 꺼진다**.
///
/// ★통과 예외(`decide_allowing` 의 구멍)를 인자로 받지 않는다: 이 자리에서 통과시켜도 되는
///   관문은 **없다**(디렉티브를 어느 관문 창에 넣어도 옳지 않다). 유일한 예외 사용처인
///   폴더신뢰 자동확인은 이미 화면(`text`)을 손에 들고 있어 이 함수의 화면 RPC 를 다시 칠
///   이유가 없고, 그래서 그쪽은 `decide_allowing` 을 직접 부른다. 늘 `None` 인 인자를 남기면
///   다음 읽는 사람이 "여기도 구멍이 있다"고 오독한다.
fn gate_guard_decide_in_boot(
    sid: u64,
    gates: &[cys::first_run_gates::Gate],
) -> cys::inject_guard::Decision {
    let screen = gate_guard_screen_or_warn(sid, "부트 주입");
    cys::inject_guard::decide(&cys::inject_guard::Observed {
        screen: &screen,
        gates,
        awakened: Some(false), // 부트 창은 상수다(위 doc 참조)
        guard_off: cys::inject_guard::guard_off(),
    })
}

/// ★U-14 주입·제출 가드 — **전 주입 경로 공통 그물**(`inject_text`/`inject_text_on` 안쪽 1지점).
///
/// 여기 한 번 걸면 디렉티브 주입·`[RECOVER]`·`[DRAIN]`·cycle 재주입·pack-update 재주입·
/// 복원 디렉티브·각성 확인 핑이 **동시에** 덮인다. 각 호출부에 가드를 흩으면 새 경로가 생길
/// 때마다 빠뜨리고, 이 저장소에서 살아남는 결함은 전부 그런 이음매에 있다.
///
/// **반환 `Err` 는 보류이지 파괴 근거가 아니다** — 머리표 `inject_guard::HOLD_TOKEN` 이 그
/// 계약이고, 부트 경로는 이 머리표를 보고 `BootVerdict::GatePending`(좌석 보존)으로 흐른다.
fn gate_guard_check(sid: u64, stage: &str) -> Result<(), String> {
    // ① 생애 창 — 닫혔거나 재지 못했으면 화면 RPC 자체를 생략한다(비용 0 · 오탐 0).
    //   ★한 왕복의 스냅샷에서 창과 좌석 어댑터를 **함께** 읽는다(P4-4 — 찢어진 관측 금지).
    let rows = fetch_surfaces();
    let awakened = surface_awakened_in(&rows, sid);
    if awakened != Some(false) {
        return Ok(());
    }
    let screen = gate_guard_screen_or_warn(sid, stage);
    // ★(P4-4) 코퍼스는 부트 폴링·`adapter_ready` 와 **같은 소스**에서 온다 — 종전의
    //   `builtin()` 직호출은 override 봉투가 이 그물에만 도달하지 않는 탈출구의 거짓말이었다.
    let gates = gate_corpus_for_seat(surface_agent_in(&rows, sid).as_deref());
    let decision = cys::inject_guard::decide(&cys::inject_guard::Observed {
        screen: &screen,
        gates: &gates,
        awakened,
        guard_off: cys::inject_guard::guard_off(),
    });
    match decision {
        cys::inject_guard::Decision::Send => Ok(()),
        cys::inject_guard::Decision::SendObserved(hit) => {
            eprintln!(
                "[inject-guard] ⚠ 가드 강등({}=0 또는 마스터 {}=0) — 관문({} · {})을 \
                 관측했으나 종전대로 {stage} 를 보낸다 surface={}",
                cys::inject_guard::ENV_GUARD_OFF,
                cys::ENV_BOOT_GATES,
                hit.id,
                hit.title,
                surface_ref(sid)
            );
            Ok(())
        }
        cys::inject_guard::Decision::Hold(hit) => Err(gate_hold_message(sid, &hit, stage)),
    }
}

/// 보류 사유 + 처방(에러 본문). 머리표로 시작한다 — 호출부의 유일한 분류 근거다.
///
/// 처방에 면책 창의 기본 포커스를 **반드시** 적는다: 그 한 줄이 없으면 사람이 pane 을 보고
/// Return 을 눌러 스스로 노드를 종료시킨다(rc 1) — 처방이 곧 킬 스텝이 된다(실측).
fn gate_hold_message(sid: u64, hit: &cys::inject_guard::GateHit, stage: &str) -> String {
    format!(
        "{} 관문 보류(gate={}) — {stage} 를 **보내지 않았다**(좌석 보존 · close 0 · kill 0). \
         화면에 '{}' 가 떠 있어 지금 붙여넣기·Return 을 보내면 그 키가 관문 위젯의 버튼을 누른다.\n\
         \x20 · {}\n\
         \x20 · 사람 조치: `cys read-screen --surface {}` 로 확인 → ★면책(Bypass) 창의 기본 \
         포커스는 `No, exit` 다(그대로 Return 하면 노드가 종료된다 — 아래 1회 뒤 Return 또는 숫자 `2`).\n\
         \x20 · 통과 뒤 같은 좌석을 그대로 쓴다 — 새 pane 을 만들지 마라.\n\
         \x20 · ★종전 동작으로 되돌리려면 **이 스위치 하나**: {}=0 \
         (이 캠페인이 추가한 판정 축 전부 복귀 · 축 하나만 끄는 {}=0 은 readiness 축이 남아 \
         여전히 보류된다)",
        cys::inject_guard::HOLD_TOKEN,
        hit.id,
        hit.title,
        if hit.human_only {
            "이 관문은 **사람이 1회** 해야 통과한다(로그인·OAuth는 기계가 대신할 수 없다)"
        } else {
            "이 관문은 사람이 한 번 눌러 주면 그대로 진행된다"
        },
        surface_ref(sid),
        cys::ENV_BOOT_GATES,
        cys::inject_guard::ENV_GUARD_OFF,
    )
}

/// 지침·과업 텍스트의 표준 주입: bracketed paste → 0.8s → Return
///
/// ★T-0147-6(사람 입력 경합 · W4): `authoritative:true` 는 타이핑 가드를 면제하지만 **무조건이
/// 아니다** — 데몬은 호출자 신원(`authoritative_caller_ok`)까지 확인하고, 그 확인이 안 되면
/// (비-restore-root 자손·신원 미검증 경로) 가드가 그대로 집행돼 `typing_guard` 로 거부한다.
/// 종전엔 그 거부가 그대로 Err 로 올라가 **주입 자체가 유실**됐다(오너가 그 순간 pane 에 타이핑
/// 중이었다는 이유만으로 디렉티브가 안 들어갔다). 이제 **`--queued` 로 1회 전환**한다:
///   · 큐 배달(`deliver_queued`)은 출력 조용 + 사람 입력 냉각 후 `Inject{cr_delay}` 로 넣는다 —
///     즉 사람의 미완성 입력에 이어붙이거나 제출하는 최악 경로가 **구조적으로** 불가능하다.
///   · 그래서 별도 Return 을 보내지 않는다(큐 배달이 CR 을 포함한다 — 이중 제출 금지).
///   · 재시도는 **정확히 1회**다. 실패해도 여기서 더 밀지 않는다 — 멱등 재주입은 상위
///     (`javis_boot_node` VERIFY 3분기)의 책임이고, 여기서 반복하면 중복 주입이 된다.
/// ★W2 ack 검증과의 결합: 큐 전환은 '배달 예약'이므로 ack(awakened_at 래치)가 늦어질 수 있다.
///   호출부(launch-agent)는 미확인을 **치명으로 올리지 않고** `directive_verified=false` 로
///   상태화하므로(B14), 이 전환이 새 실패를 만들지 않는다. 중복 주입 0 은 위 '1회' 규칙이 보장.
fn inject_text(sid: u64, text: &str) -> Result<(), String> {
    // ★U-14 관문 가드 ①(붙여넣기 직전). 이 한 줄이 `inject_text` 를 부르는 모든 경로를 덮는다.
    gate_guard_check(sid, "디렉티브 주입")?;
    let wrapped = format!("\x1b[200~{text}\x1b[201~");
    // authoritative: 디렉티브·과업 주입은 타이핑 가드를 면제한다 — 막 기동한 에이전트
    // pane에 사람 미완성 입력이 없고, GUI 활성 pane의 사람-입력 잔향이 주입을 영구
    // 차단하던 경로(human is typing 무한)를 끊는다. ACL은 데몬에서 그대로 집행된다.
    match request(
        "surface.send_text",
        json!({"surface_id": sid, "text": wrapped, "quiet": true, "authoritative": true}),
    ) {
        Ok(_) => {}
        Err(e) if is_typing_guard_err(&e) => {
            eprintln!(
                "[inject] 사람 입력 감지 — 입력을 멈추면 큐가 배달합니다(--queued 1회 전환, \
                 {}) surface={}",
                surface_ref(sid),
                surface_ref(sid)
            );
            // ★래핑 주의: 큐 배달은 `WriteReq::Inject` 라 데몬이 **bracketed paste 를 스스로**
            //   씌운다(state.rs writer arm). 여기서 `wrapped` 를 넣으면 이중 래핑이 되어 제어열이
            //   본문에 섞인다 — 직접 경로(`Data`, 클라이언트가 래핑)와 규약이 다르므로 **원문**을
            //   보낸다(`cys send --queued` 가 원문을 보내는 것과 동일).
            request(
                "surface.send_text",
                json!({"surface_id": sid, "text": text, "queued": true,
                       "from": "inject(typing_guard fallback)"}),
            )?;
            return Ok(()); // 큐 배달이 CR 을 포함한다 — 별도 Return 금지(이중 제출 방지)
        }
        Err(e) => return Err(e),
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    // ★U-14 관문 가드 ②(제출 Return 직전). **실측 킬 스텝이 정확히 이 Return 이다** —
    //   붙여넣기와 이 Return 사이의 800ms 동안 관문이 새로 뜰 수 있으므로 화면을 다시 본다.
    //   창이 닫힌 좌석(각성 완료)에서는 `surface_awakened` 가 즉시 반환하므로 추가 비용은
    //   surface.list 왕복 하나뿐이고, 그것도 첫 부트 창에서만 화면 RPC 로 이어진다.
    //   ★보류 시 본문은 이미 pane 에 들어가 있다(제출만 막혔다) — 그래서 처방 문안이
    //     "사람이 관문을 통과시키면 그 입력이 그대로 살아 있다"는 상태를 전제로 쓰였다.
    gate_guard_check(sid, "제출 Return")?;
    match request(
        "surface.send_key",
        json!({"surface_id": sid, "key": "Return", "authoritative": true}),
    ) {
        Ok(_) => Ok(()),
        Err(e) if is_typing_guard_err(&e) => {
            // 본문은 이미 직접 들어갔고 **제출만** 막혔다 — Return 만 큐로 1회 전환한다
            // (send_key --queued 는 Return/Enter 전용이라 계약상 안전).
            eprintln!(
                "[inject] 사람 입력 감지 — 입력을 멈추면 큐가 배달합니다(제출 Return --queued 1회 전환) surface={}",
                surface_ref(sid)
            );
            request(
                "surface.send_key",
                json!({"surface_id": sid, "key": "Return", "queued": true}),
            )?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// "90s" / "20m" / "2h" / "1h30m" → 초
fn parse_duration_secs(s: &str) -> Result<u64, String> {
    let mut total: u64 = 0;
    let mut num = String::new();
    let mut any = false;
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
        } else {
            let n: u64 = num
                .parse()
                .map_err(|_| format!("invalid duration '{s}'"))?;
            num.clear();
            any = true;
            // checked 산술: 거대한 입력(예: 9999999999999999d)이 debug에서 패닉,
            // release에서 silent wrap(엉뚱한 발화 시각)으로 새는 경로를 차단한다.
            let mult = match ch {
                's' => 1,
                'm' => 60,
                'h' => 3600,
                'd' => 86400,
                _ => return Err(format!("invalid duration unit '{ch}' in '{s}'")),
            };
            let add = n
                .checked_mul(mult)
                .ok_or_else(|| format!("duration overflow in '{s}'"))?;
            total = total
                .checked_add(add)
                .ok_or_else(|| format!("duration overflow in '{s}'"))?;
        }
    }
    if !num.is_empty() || !any {
        return Err(format!(
            "invalid duration '{s}' (expected e.g. 90s, 20m, 2h, 1h30m)"
        ));
    }
    Ok(total)
}

fn sha256_file(path: &str) -> Option<String> {
    use sha2::{Digest, Sha256};
    std::fs::read(path).ok().map(|b| {
        let mut h = Sha256::new();
        h.update(&b);
        h.finalize().iter().map(|x| format!("{x:02x}")).collect()
    })
}

// ---------- transport ----------

#[cfg(unix)]
fn connect_raw() -> Result<std::os::unix::net::UnixStream, String> {
    let path = socket_path();
    std::os::unix::net::UnixStream::connect(&path)
        .map_err(|e| format!("cannot connect to cysd at {}: {e}", path.display()))
}

/// ERROR_PIPE_BUSY(231) 한정 bounded 재시도로 named pipe 를 연다. 그 외 오류(파이프 부재
/// ERROR_FILE_NOT_FOUND = 데몬 다운 등)는 즉시 반환 — autostart 판단은 호출부 몫.
/// 231을 데몬 다운으로 오판하면 connect()의 sibling cysd autostart 까지 헛발동한다
/// (2026-07-10 Windows 실사고). 정책(상수·jitter·커널 대기)은 GUI(cys-app)와 공용 단일 진실인
/// lib(cys::PIPE_BUSY_* · next_busy_delay · wait_named_pipe) — 근거·계약은 그 정의부 주석 참조.
/// 비-Windows 테스트가 정책 불변을 박제한다.
#[cfg(windows)]
fn open_pipe_busy_retry(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let deadline = std::time::Instant::now() + cys::PIPE_BUSY_RETRY_DEADLINE;
    let mut delay = cys::PIPE_BUSY_RETRY_INTERVAL;
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(path) {
            Err(e)
                if e.raw_os_error() == Some(cys::PIPE_BUSY_ERROR)
                    && std::time::Instant::now() < deadline =>
            {
                // busy 한정 분기(비-busy 오류는 위 가드에 안 걸려 즉시 반환 유지). 커널 대기가
                // 인스턴스 가용을 알리면 즉시 재-open(창을 놓치면 남이 채간다), 타임아웃이면
                // jitter 백오프로 재시도 위상을 분산한다. wait 의 false(파이프 소멸 포함)는
                // 판정이 아니다 — 최종 판정은 다음 open 이 내린다.
                if !cys::wait_named_pipe(path, cys::PIPE_BUSY_WAIT_SLICE) {
                    delay = cys::next_busy_delay(delay, cys::rand01_cheap());
                    std::thread::sleep(delay);
                }
            }
            other => return other,
        }
    }
}

#[cfg(windows)]
fn connect_raw() -> Result<std::fs::File, String> {
    let path = socket_path();
    open_pipe_busy_retry(&path)
        .map_err(|e| format!("cannot connect to cysd pipe {}: {e}", path.display()))
}

/// 온보딩④: 자동 기동 허용 — ping(순수 프로브)·daemon status는 main()에서 끈다.
static AUTOSTART: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// 한 CLI 실행에서 spawn 시도는 1회만
static AUTOSTART_TRIED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn sibling_daemon_path() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) { "cysd.exe" } else { "cysd" };
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|d| d.join(name))
        .filter(|p| p.exists())
}

// ── Windows 진짜 KeepAlive 패리티(작업 스케줄러 RestartOnFailure) 헬퍼 (mac launchd KeepAlive 대응) ──
// schtasks 명령줄 플래그엔 RestartOnFailure(사망 시 재기동)가 없어 태스크 XML 로만 설정 가능하다.
// 아래 함수는 전부 #[cfg(windows)] — mac 빌드에선 컴파일되지 않는다(dead_code 없음).

#[cfg(windows)]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// 현재 사용자 식별자("DOMAIN\\User") — 태스크 principal/trigger 의 UserId. whoami 우선(정확), env 폴백.
#[cfg(windows)]
fn current_user_id() -> Option<String> {
    if let Ok(out) = std::process::Command::new("whoami").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    let user = std::env::var("USERNAME").ok()?;
    match std::env::var("USERDOMAIN") {
        Ok(d) if !d.is_empty() => Some(format!("{d}\\{user}")),
        _ => Some(user),
    }
}

/// cysd 작업 스케줄러 태스크 XML. LogonTrigger(현재 사용자) + RestartOnFailure(PT1M×10) +
/// ★ExecutionTimeLimit PT0S(무제한 — 기본 72h 제한이 장수 데몬을 죽인다) + IgnoreNew(중복 인스턴스 억제) +
/// 배터리 제약 해제 + StartWhenAvailable + LeastPrivilege(=schtasks /RL LIMITED 대응).
#[cfg(windows)]
fn cysd_task_xml(daemon: &std::path::Path, user_id: &str) -> String {
    let cmd = xml_escape(&daemon.display().to_string());
    let uid = xml_escape(user_id);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>CYSJavis terminal daemon (cysd) — 로그온 자동기동 + 사망 시 자동 재기동(RestartOnFailure)</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{uid}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{uid}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>10</Count>
    </RestartOnFailure>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{cmd}</Command>
    </Exec>
  </Actions>
</Task>"#
    )
}

/// XML 을 UTF-16LE(BOM 포함)로 기록 — schtasks /Create /XML 이 요구하는 인코딩(UTF-16 관례).
#[cfg(windows)]
fn write_utf16le_bom(path: &std::path::Path, s: &str) -> std::io::Result<()> {
    let mut bytes: Vec<u8> = vec![0xFF, 0xFE]; // UTF-16LE BOM
    for u in s.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(path, bytes)
}

/// schtasks /Query /XML 출력에서 RestartOnFailure 존재 여부(=KeepAlive 켜짐). null 바이트 제거로
/// UTF-16/UTF-8 출력 모두에서 ASCII 태그를 안정 검출(UTF-16LE 는 ASCII 사이에 0x00 이 낀다).
#[cfg(windows)]
fn task_has_restart_on_failure(task: &str) -> bool {
    std::process::Command::new("schtasks")
        .args(["/Query", "/TN", task, "/XML"])
        .output()
        .map(|o| {
            let raw: Vec<u8> = o.stdout.iter().copied().filter(|&b| b != 0).collect();
            String::from_utf8_lossy(&raw).contains("RestartOnFailure")
        })
        .unwrap_or(false)
}

/// 소켓 경로 → 그 레인의 팩 경로(결정론 유도 · G34).
/// ★W4: 구현은 **lib 단일 소스**(`cys::pack::lane_pack_for_socket`)로 승격됐다 — GUI(cys-app)도
/// 같은 유도를 소비해야 하기 때문이다(main.rs `start_dept_master` = G34 의 GUI 지점).
/// 여기 남은 얇은 별칭은 이 파일의 기존 호출부·회귀 테스트를 그대로 유지하기 위한 것이다.
fn lane_pack_for_socket(socket: &std::path::Path) -> Option<std::path::PathBuf> {
    cys::pack::lane_pack_for_socket(socket)
}

/// ★G34(W3): (소켓, 팩) **쌍 보증** — 부서 소켓으로 데몬을 띄우면서 본부 팩을 물려주는 것을 막는다.
///
/// 결함(재감사 §1.2 G34 · P1): 부서 소켓+**본부 팩** 조합의 데몬이 생기면 ①부서 마스터 선언이
/// `javis_bootstrap` 의 레인↔팩 가드에 걸려 **exit 8 로 영구 차단**되고 ②본부 팩을 교차 서빙해
/// F1 계정·레인 격리가 붕괴하며 schedule 이 중복 발화한다. 쌍 보증이 `cys-dept` 3지점에만 있었고
/// CLI autostart 는 env 를 무스크럽 상속해 그 조합을 만들 수 있었다(부서 데몬 사망 후 임의 cys 명령).
///
/// 판정(base 소켓은 무동작 — 기존 동작 100% 보존):
///  · 팩 env 미설정 → 소켓에서 레인 팩을 유도해 **주입**(선택지 ①). 유도한 팩 디렉터리가 없으면
///    그 부서는 실재하지 않는 것이므로 거부(선택지 ② — 새 부서 팩을 자동 창설하지 않는다).
///  · 팩 env 설정 + 유도값과 불일치 → **거부**(선택지 ②): 명시 오설정이며, 그대로 띄우면 위 ①②가 확정된다.
/// 거부는 조용하지 않다 — 호출부가 에러를 삼키므로 사유를 여기서 stderr 로 낸다.
fn ensure_daemon_lane_pack(cmd: &mut std::process::Command) -> std::io::Result<()> {
    let socket = cys::socket_path();
    if !cys::is_dept_socket(&socket) {
        return Ok(());
    }
    let derived = match lane_pack_for_socket(&socket) {
        Some(p) => p,
        None => {
            let msg = format!(
                "부서 소켓({})에서 부서명을 유도할 수 없다(불량 레인) — autostart 거부. \
                 부서 데몬은 `cys-dept launch <name>` 으로 기동하세요.",
                socket.display()
            );
            eprintln!("[cys] {msg}");
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, msg));
        }
    };
    let env_pack = cys::pack::PACK_DIR_ENV_KEYS
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
    match env_pack {
        None => {
            if !derived.is_dir() {
                let msg = format!(
                    "부서 소켓({})의 팩({})이 없어 autostart 를 거부한다(본부 팩으로 부서 데몬을 \
                     띄우면 레인↔팩 불일치로 부서 부트가 영구 차단되고 격리가 붕괴한다) — \
                     부서 데몬은 `cys-dept launch <name>` 으로 기동하세요.",
                    socket.display(),
                    derived.display()
                );
                eprintln!("[cys] {msg}");
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, msg));
            }
            eprintln!(
                "[cys] 부서 소켓 감지 — 레인 팩 주입(CYS_PACK_DIR={})",
                derived.display()
            );
            cmd.env(cys::pack::ENV_PACK_DIR, &derived);
            Ok(())
        }
        Some(p) => {
            let same = std::fs::canonicalize(&p)
                .ok()
                .zip(std::fs::canonicalize(&derived).ok())
                .map(|(a, b)| a == b)
                .unwrap_or_else(|| std::path::Path::new(&p) == derived.as_path());
            if same {
                return Ok(());
            }
            let msg = format!(
                "레인↔팩 불일치로 autostart 거부: 소켓={} 팩={}(기대 {}). 이 조합의 데몬은 \
                 부서 부트를 영구 차단하고 본부 팩을 교차 서빙한다 — 부서 데몬은 \
                 `cys-dept launch <name>` 으로 기동하세요.",
                socket.display(),
                p,
                derived.display()
            );
            eprintln!("[cys] {msg}");
            Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, msg))
        }
    }
}

/// 데몬을 분리 세션으로 기동 — CLI가 Ctrl-C로 죽어도 데몬은 살아남는다.
///
/// ★U-7: 분리 규약은 `cys::SpawnPolicy`(단일 정의처) 경유. 등급 `Survivor` 가
/// ①unix `setsid` ②Windows `CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW`
/// ③stdin/stdout/stderr 무점유를 **한 값으로** 결정한다.
///
/// ★Windows 에서 이것은 **행동 변경**이다(문서-코드 정합 정정 · 2026-08-24). 종전 arm 은
/// `CREATE_NO_WINDOW` **단독**이었고 지금은 `CREATE_NEW_PROCESS_GROUP` 이 **새로 걸린다** —
/// U-7 을 "정의처만 이동, 값·행동 무변경"으로 적은 것은 이 지점에서 사실이 아니었다.
/// 무해한 방향의 변경이지만 무변경은 아니다.
///
/// ★그리고 그 flag 가 **무엇을 지키고 무엇을 못 지키는지** 정직하게: 지키는 것은
/// **부모 콘솔의 Ctrl-C/Ctrl-Break 전파 차단** 하나다(unix `setsid` 의 부분 대응물).
/// 지키지 **못하는** 것 — Windows 의 트리 종료는 **Job object**(부모가 쥔 job 의 자식은 job
/// 종료 시 함께 죽는다)와 **`taskkill /T`**(스냅샷의 부모-자식 링크를 따라 내려간다)로 일어나고,
/// **둘 다 프로세스 그룹과 무관하다.** 따라서 이 flag 는 "훅이 트리를 죽일 때 데몬이 살아남는다"를
/// 보장하지 않는다. unix 쪽은 다르다 — `setsid` 는 세션·그룹을 실제로 갈라 시그널 전파를 끊는다.
///
/// 종전 arm 이 `channels.rs::spawn_bridge`(둘 다 건다)와 **비대칭**이었던 것은 사실이고,
/// 실측(PROBE_RESULTS.md V-c · PROBE_RESULTS_WINDOWS.md WIN-3 H4)에서 훅/부모가 잘릴 때
/// 그룹에 남은 자식이 함께 죽는 것도 확인됐다(그 관측의 결정적 축은 unix 경로다).
/// `setsid`(mac 부재)·`setsid.exe`(동봉 PortableGit 부재)로는 우회할 수 없어 **스폰 flag 가
/// 유일한 수단**이라는 것도 그대로다. 다만 Windows 에서 트리 종료로부터의 생존이 필요하다면
/// 다음 수단은 이 flag 가 아니라 **job object 비상속**(`CREATE_BREAKAWAY_FROM_JOB` 등)이며,
/// 그것은 이 단위의 범위 밖이고 **실기 검증 없이 손대지 않는다**(이 저장소는 Windows 크로스
/// 타입체크조차 불가능하다 — 검증 없는 flag 추가는 개선이 아니라 미검증 변경이다).
fn spawn_detached_daemon(path: &std::path::Path) -> std::io::Result<()> {
    use cys::SpawnPolicy;
    let mut cmd = std::process::Command::new(path);
    // ★G34: 스폰 전 (소켓,팩) 쌍 보증 — 거부 시 스폰 자체를 하지 않는다.
    ensure_daemon_lane_pack(&mut cmd)?;
    cmd.spawn_policy(cys::ChildLifetime::Survivor);
    cmd.spawn().map(|_| ())
}

/// socket-ready 실측 폴링(최대 4초=40×100ms). launchd kickstart·sibling spawn 양 경로가 공유.
fn poll_socket_ready() -> Option<ConnStream> {
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Ok(s) = connect_raw() {
            return Some(s);
        }
    }
    None
}

/// 온보딩④: 연결 실패 시 형제 cysd를 자동 기동 후 재시도 — 신규 머신 zero-setup.
/// 옵트아웃: CYS_NO_AUTOSTART=1. (데몬 중복 기동은 cysd 자체의 flock이 차단)
/// ★W3: macOS에서 launchd가 cysd를 소유(적재)하면 sibling spawn 대신 launchctl kickstart로
/// 위임한다 — 구형 CLI가 자기 옆 구형 cysd를 띄워 startup lock을 선점하고 launchd 신형과
/// crashloop 하는 경로를 원천 차단. kickstart 실패·폴링 타임아웃 시에만 sibling fallback(개발 환경).
fn connect() -> Result<ConnStream, String> {
    match connect_raw() {
        Ok(s) => Ok(s),
        Err(first) => {
            let opted_out = cys::env_compat("CYS_NO_AUTOSTART")
                .map(|v| v == "1")
                .unwrap_or(false);
            if opted_out
                || !AUTOSTART.load(std::sync::atomic::Ordering::Relaxed)
                || AUTOSTART_TRIED.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(first);
            }
            // ★A3(성찰 확정): 다른 프로세스가 완전 초기화를 **진행 중**이면 데몬을 되살리지
            // 않는다 — 되살아난 cysd 가 격리와 경합하고(살아있는 DB 이동) phoenix 로 조직까지
            // 복원해 "설치 초기 상태" 계약을 깬다. 판정은 fail-open(TTL·pid 생존)이라 리셋이
            // 죽어도 다음 기동을 영구히 막지 않는다(부트 체인 불가침).
            if cys::factory_reset::reset_in_progress() {
                return Err(
                    "완전 초기화가 진행 중이라 데몬을 기동하지 않는다 — 끝난 뒤 다시 실행하라".into(),
                );
            }
            // launchd 위임 우선(macOS·적재 시). 실패 시 아래 sibling 경로로 폴백.
            #[cfg(target_os = "macos")]
            {
                if cys::launchd::should_delegate_autostart(cys::launchd::is_loaded()) {
                    eprintln!("[cys] cysd not serving — delegating to launchd (launchctl kickstart)");
                    if cys::launchd::kickstart() {
                        if let Some(s) = poll_socket_ready() {
                            return Ok(s);
                        }
                        eprintln!(
                            "[cys] launchd kickstart did not yield a socket within 4s — falling back to sibling spawn"
                        );
                    } else {
                        eprintln!("[cys] launchctl kickstart failed — falling back to sibling spawn");
                    }
                }
            }
            let Some(daemon) = sibling_daemon_path() else {
                return Err(format!("{first} (no sibling cysd to autostart)"));
            };
            eprintln!("[cys] cysd not running — autostarting {}", daemon.display());
            if spawn_detached_daemon(&daemon).is_err() {
                return Err(first);
            }
            poll_socket_ready()
                .ok_or_else(|| format!("{first} (autostarted cysd did not come up within 4s)"))
        }
    }
}

#[cfg(unix)]
type ConnStream = std::os::unix::net::UnixStream;
#[cfg(windows)]
type ConnStream = std::fs::File;

// ═══════════════ U-6 · RPC 왕복 상한(양방향 · Windows arm 실동작) ═══════════════
//
// 【고친 결함】 `request()` 에는 read/write 타임아웃이 **없었다**. 데몬이 accept 한 뒤 응답하지
// 못하는 상태(동기 `handlers::dispatch` 가 락에 걸림·핸들러 교착)가 되면 CLI 는 **영구 대기**한다.
// 그 CLI 가 훅 자식이면 훅 timeout 이 프로세스 트리를 kill 할 때까지 pane 이 멈춘다.
// 타임아웃판(`request_on_timeout`)이 이미 있었으나 (a) `drain --verify` fan-out 전용이고
// (b) **Windows arm 이 `request_on` 위임 = no-op** 이었다(주석이 "범위 한정"으로 자인).
//
// 【상한의 축 — 기구는 무진행, 실효는 총 상한】(문서-코드 정합 정정 · 2026-08-24)
// 기구 자체는 **무진행(한 바이트도 오지 않는) 구간**을 잰다. 총 데드라인으로 자르면 정상 전송
// 중인 큰 응답(수 MB scrollback · 수백 KB 지침 에코)이 잘리는데, 그건 hang 보다 나쁜 실패다
// (멀쩡한 왕복을 죽인다). 1바이트라도 진행하면 상한을 재장전한다.
//
// ★그러나 **즉답 메서드에서는 이것이 사실상 총 상한 40초다.** cysd 는 응답을 `write_line`
// **한 번에** 쓰므로(cysd/main.rs) 부분 진행이라는 것이 존재하지 않는다 — 핸들러가 붙들고 있는
// 동안은 0바이트이고, 그 구간이 40초를 넘으면 응답 직전이어도 끊긴다. 지금은 40초를 넘는 동기
// 핸들러가 없어 안전하지만(그래서 이 문장은 오늘의 사실이지 구조적 보증이 아니다), 그런 핸들러가
// **하나만 생겨도 조용히 끊긴다**. 새 동기 핸들러를 40초 이상으로 만들 일이 생기면 그 메서드를
// `rpc_server_wait_secs` 의 '서버가 의도적으로 대기하는' 목록에 넣어 상한을 파생시켜야 한다.
// (스트림·블로킹 메서드는 아래 【장기 경로 제외】가 이미 그렇게 다룬다.)
//
// 대가의 반대쪽: 데몬이 무한히 찔끔거리면 유계가 아니다. cysd 의 단발 `write_line` 계약상 그
// 상태는 만들 수 없고, 어느 방향이든 종전(무타임아웃)보다 나빠지지 않는다.
//
// 【장기 경로 제외】 응답이 아니라 **스트림**이 흐르는 연결 승격 경로에는 어떤 상한도 걸지 않는다.
// 판별 근거는 데몬의 `Reply` enum(cysd/handlers.rs) 이다 — `Reply::EventStream`(`events.stream`)
// 과 `Reply::Attach`(`surface.attach`)가 연결을 승격하고, `Reply::FeedWait`(`feed.push` wait=true)
// 와 `Reply::WaitFor`(`surface.wait_for`)는 **응답 전에 서버가 의도적으로 대기**한다. 앞 둘은
// 상한 면제, 뒤 둘은 그 대기값에서 상한을 파생한다(아래 `rpc_server_wait_secs`).
//
// 【롤백】 `CYS_RPC_TIMEOUT_SECS=0` = 상한 전면 해제(개정 전 무한 대기 거동). 양수면 그 값(초)이
// 기본 상한을 대체한다. 코드 revert 없이 무력화 가능해야 한다는 단위 계약(§3-2 U-6)의 집행부다.

/// 기본 무진행 상한(초). **즉답 메서드에서는 실효가 총 상한**이다 — 근거는 위 블록의
/// 【상한의 축】(cysd 가 응답을 `write_line` 한 번에 쓰므로 부분 진행이 없다).
///
/// ★값 근거(넉넉한 쪽으로 고른다 — 짧으면 정상 부트 왕복이 잘려 팀이 깨진다):
/// ① 살아있는 데몬 실측 왕복 = `cys ping` 0.007s · `cys status --json` 0.029s
///    (2026-08-23 mac, 3회 중앙값). 정상 왕복은 상한의 1/1000 규모다.
/// ② 냉시작(데몬 자동기동 + 소켓 바인드 + 프로세스 표 refresh)을 포함한 **최악 왕복의 실측 하한**은
///    팩 예산이 12~15초로 박제한다 — `javis_budget.LEAF_FLOORS`: `CYS_STATUS_TIMEOUT_S=12`,
///    `CYS_PING_TIMEOUT_S`/`CYS_LIST_TIMEOUT_S`/`CYS_CLAIM_TIMEOUT_S=15`.
/// ③ 그 최악치(15)의 2배 + 여유 = **40초**. 부트 사슬의 상위 호출자는 자기 서브프로세스
///    timeout(12~15초)으로 **언제나 먼저** 자르므로, 이 상한이 정상 부트 왕복을 자르는 경로는 없다.
/// ④ 이 상수는 부트 readiness 예산(`BUDGET_*` ↔ `LEAF_FLOORS`) 과 **다른 축**이라 그 이름공간에
///    넣지 않는다(H-TIME-1 파리티 대상 아님 — 예산 leaf 무변경 계약 준수).
const RPC_IDLE_TIMEOUT_SECS: u64 = 40;

/// 서버가 스스로 붙잡는 메서드에 얹는 여유(초) — 서버 자신의 timeout 이 먼저 만료해 응답이
/// 오는 것이 정상 귀결이고, 이 마진은 그 응답이 도착하는 데 드는 왕복분이다.
const RPC_SERVER_WAIT_MARGIN_SECS: u64 = 30;

/// 서버측 하드 캡(cysd/handlers.rs: `feed.push` `.min(3600)` · `surface.wait_for` `.min(600)`).
/// 클라이언트가 params 에서 큰 값을 읽어도 이 이상은 기다리지 않는다.
const RPC_SERVER_WAIT_CAP_SECS: u64 = 3600;

/// 롤백 스위치 env 이름(0 = 상한 해제).
const ENV_RPC_TIMEOUT: &str = "CYS_RPC_TIMEOUT_SECS";

/// 노브(`CYS_RPC_TIMEOUT_SECS`)가 받아들이는 **상한**(초 · 1일).
///
/// ★왜 필요한가(실사고 클래스): 이 상한값은 Windows 워치독에서 곧바로
/// `std::time::Instant::now() + timeout` 이 된다(`RpcWatchdog::new`·`touch`). std `Instant::add`
/// 는 오버플로에서 **패닉**하므로, 노브에 대략 9.2e18 이상이 들어오면 **CLI 가 패닉**한다 —
/// 상한을 늘리려던 손동작이 명령 자체를 죽인다. 음수·비숫자는 이미 안전하다(parse 실패 →
/// 기본값 · 진리표가 박제). 뚫린 것은 **거대 양수** 한 축뿐이었다.
/// 값 1일: 그보다 긴 무진행을 기다리는 운용은 없고, 진짜 무제한이 필요하면 계약대로 `0`
/// (= 상한 해제)을 쓰면 된다 — 노브의 표현력이 줄지 않는다.
const RPC_IDLE_TIMEOUT_MAX_SECS: u64 = 86_400;

/// 연결을 **스트림으로 승격**하는 메서드 — 상한 면제(구독이 끊긴다).
/// 지금 이 둘은 `request()` 를 타지 않고 각자 `connect()` 를 쓰지만(`stream_events`·`attach`),
/// 목록을 한 곳에 둬 훗날 `request()` 로 합쳐져도 상한이 따라붙지 않게 한다.
const RPC_STREAMING_METHODS: [&str; 2] = ["events.stream", "surface.attach"];

/// 서버가 **응답 전에 의도적으로 대기**하는 메서드의 선언 대기값(초). None = 즉답 메서드.
fn rpc_server_wait_secs(method: &str, params: &Value) -> Option<u64> {
    // 둘 다 서버 기본값이 120(handlers.rs `param_u64(...).unwrap_or(120)`)이라 여기서도 120으로
    // 맞춘다 — 클라이언트가 값을 안 실어도 서버 대기보다 짧게 자르지 않기 위함.
    // ★서버와 **같은 관용**으로 읽는다(cysd/handlers.rs `param_u64`: `as_u64` 실패 시
    //   `as_str().parse()`). 종전엔 여기만 `as_u64()` 뿐이라, 외부 소비자가
    //   `{"wait":true,"timeout_secs":"3600"}`(문자열)로 보내면 **서버는 3600초 대기 · 클라이언트는
    //   기본 40초**가 됐다 — 오너가 승인하기 전에 클라이언트가 먼저 끊는다(느린 승인이 곧
    //   '실패'로 보이는 오진). 현 트리 호출부는 전부 숫자라 미발현이었을 뿐, 비대칭 자체가 결함이다.
    //   ★관용을 **넓히는** 방향만 맞춘다: 서버가 못 읽는 형태를 클라이언트가 읽으면 반대 비대칭
    //   (클라이언트가 서버보다 오래 기다림 = 무진행 상한이 헐거워짐)이 생긴다.
    let declared = || {
        params
            .get("timeout_secs")
            .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(120)
    };
    match method {
        "surface.wait_for" => Some(declared()),
        // `wait` 가 참일 때만 블로킹(거짓이면 즉시 pending 응답 — 즉답 메서드다).
        "feed.push"
            if params
                .get("wait")
                .and_then(|v| v.as_bool())
                .unwrap_or(false) =>
        {
            Some(declared())
        }
        _ => None,
    }
}

/// 이 왕복에 걸 무진행 상한(None = 무제한).
fn rpc_idle_timeout(method: &str, params: &Value) -> Option<std::time::Duration> {
    rpc_idle_timeout_with(
        method,
        params,
        cys::env_compat(ENV_RPC_TIMEOUT).as_deref(),
    )
}

/// `rpc_idle_timeout` 의 순수 판정부 — env 를 인자로 받아 테스트가 프로세스 전역 env 를 흔들지
/// 않게 한다(병렬 테스트 간 env 경합 = 계측기 자체가 불안정해지는 경로).
fn rpc_idle_timeout_with(
    method: &str,
    params: &Value,
    env_raw: Option<&str>,
) -> Option<std::time::Duration> {
    if RPC_STREAMING_METHODS.contains(&method) {
        return None;
    }
    let base = match env_raw.and_then(|v| v.trim().parse::<u64>().ok()) {
        Some(0) => return None, // 롤백: 상한 해제
        // 거대값 클램프 — 결과가 `Instant::now() + d` 가 되므로 오버플로는 곧 CLI 패닉이다.
        Some(v) => v.min(RPC_IDLE_TIMEOUT_MAX_SECS),
        None => RPC_IDLE_TIMEOUT_SECS,
    };
    // 블로킹 메서드는 '서버 대기 + 마진'과 기본 상한 중 **큰 쪽**. env 로 기본을 낮춰도
    // 정상 대기를 잘라 먹지 않는다(롤백 노브가 새 사망 경로를 열지 않게).
    let secs = match rpc_server_wait_secs(method, params) {
        Some(w) => w
            .min(RPC_SERVER_WAIT_CAP_SECS)
            .saturating_add(RPC_SERVER_WAIT_MARGIN_SECS)
            .max(base),
        None => base,
    };
    Some(std::time::Duration::from_secs(secs))
}

/// Windows `CancelIoEx` 로 취소된 동기 I/O 가 돌려주는 코드. unix errno 공간에 없는 값이라
/// 플랫폼 게이트 없이 검사해도 오탐이 없다(이 판정을 mac 에서 시험할 수 있게 하는 이유).
const WIN_ERROR_OPERATION_ABORTED: i32 = 995;

/// 이 I/O 오류가 '상한 만료'인가 — unix 는 `SO_RCVTIMEO`/`SO_SNDTIMEO` 의 WouldBlock/TimedOut,
/// Windows 는 워치독 `CancelIoEx` 가 만드는 ERROR_OPERATION_ABORTED(995).
fn is_rpc_timeout_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ) || e.raw_os_error() == Some(WIN_ERROR_OPERATION_ABORTED)
}

/// 상한 만료의 귀결 문안 — **조용한 실패 금지**. 무엇이 왜 끊겼는지와 다음 손동작까지 적는다.
fn rpc_timeout_message(method: &str, waited: std::time::Duration) -> String {
    format!(
        "rpc_timeout: cysd 에 연결은 됐지만 '{method}' 응답이 {}초 동안 한 바이트도 오지 않았다(데몬 wedge 의심).\n\
         처방 ① `cys ping` 으로 데몬 응답성을 확인한다.\n\
         처방 ② `cys status --json` 으로 좌석 표가 나오는지 확인한다.\n\
         처방 ③ 여전히 무응답이면 데몬을 재기동한다(macOS: `launchctl kickstart -k gui/$UID/com.cysjavis.cysd`,\n\
         그 외: cysd 프로세스를 종료하면 다음 cys 명령이 자동 기동한다).\n\
         처방 ④ 이 상한 자체가 문제라면 `{ENV_RPC_TIMEOUT}=<초>` 로 늘리거나 `0` 으로 해제한다(0 = 개정 전 무한 대기).",
        waited.as_secs()
    )
}

/// 무진행 워치독 — `timeout` 동안 `touch()` 가 없으면 `on_expire` 를 호출하고, **재장전해
/// 감시를 계속한다**(1회 발화가 아니다 — `Drop` 까지 반복 가능하다).
///
/// ★문서-구현 정합(P2-2): 종전 문안은 "**한 번** 호출한다"였으나 구현은 `F: Fn()` 을 받아
/// 만료마다 재장전한다(`RpcWatchdog::new` 루프의 `g.deadline = now + g.timeout` → `continue`).
/// 그 재장전이 의도적 설계이고 이유는 `new` 의 주석에 있다 — 문서를 구현에 맞춘다.
/// 호출 횟수 계약: **무진행 상한 1회분당 최대 1회**, `Drop`(=`stop`) 이후 0회.
///
/// Windows 전용 기구다(unix 는 커널 소켓 타임아웃이 같은 일을 한다). 다만 `cfg(test)` 에서도
/// 컴파일해 **로직 자체는 mac CI 에서 검증**한다 — Windows 크로스 타입체크가 이 저장소에서
/// 불가능(libsqlite3-sys 의 C 빌드가 호스트 툴체인으로 크로스되지 않음)하므로, 검증 불가한 부분을
/// 취소 FFI 호출부(`CancelIoEx`/`CancelSynchronousIo`)로 최소화하는 것이 설계 의도다.
#[cfg(any(windows, test))]
struct RpcWatchdog {
    state: std::sync::Arc<(std::sync::Mutex<WatchdogState>, std::sync::Condvar)>,
    joiner: Option<std::thread::JoinHandle<()>>,
}

#[cfg(any(windows, test))]
struct WatchdogState {
    timeout: std::time::Duration,
    deadline: std::time::Instant,
    stop: bool,
}

#[cfg(any(windows, test))]
impl RpcWatchdog {
    /// `on_expire` 가 `FnOnce` 가 아니라 `Fn` 인 이유: 만료 후에도 **감시를 계속**한다.
    /// 1회 발화 후 감시자가 죽으면, `touch()` 와 만료 판정이 미세하게 경합해 헛발화한 경우
    /// (만료 직후 마이크로초 단위로 데이터가 도착) 그 뒤 왕복이 **무보호**로 남는다 —
    /// 상한이 있는 척하는 상태가 상한이 없는 것보다 나쁘다. 재장전하며 계속 지키는 쪽을 택한다.
    /// 대가는 그 헛발화 창(무진행 상한 1회분당 최대 1회)이며, 귀결은 명시적 rpc_timeout 오류다.
    fn new<F: Fn() + Send + 'static>(timeout: std::time::Duration, on_expire: F) -> Self {
        let state = std::sync::Arc::new((
            std::sync::Mutex::new(WatchdogState {
                timeout,
                deadline: std::time::Instant::now() + timeout,
                stop: false,
            }),
            std::sync::Condvar::new(),
        ));
        let shared = std::sync::Arc::clone(&state);
        let joiner = std::thread::spawn(move || {
            let (m, cv) = &*shared;
            // 락 poison 관용(into_inner) — 이 저장소의 명시 정책. 감시자가 poison 으로 죽으면
            // 상한이 조용히 사라진다(= 개정 전 무한 대기 부활)이라 관용이 안전측이다.
            let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if g.stop {
                    return;
                }
                let now = std::time::Instant::now();
                if now >= g.deadline {
                    // 재장전하고(다음 창을 계속 지킨다) 락을 놓은 뒤 발화한다 —
                    // on_expire 안에서 touch() 가 불려도 교착하지 않는다.
                    g.deadline = now + g.timeout;
                    drop(g);
                    on_expire();
                    g = m.lock().unwrap_or_else(|e| e.into_inner());
                    continue;
                }
                let wait = g.deadline - now;
                let (next, _) = cv
                    .wait_timeout(g, wait)
                    .unwrap_or_else(|e| e.into_inner());
                g = next;
            }
        });
        Self {
            state,
            joiner: Some(joiner),
        }
    }

    /// 진행이 있었다 — 상한을 재장전한다.
    fn touch(&self) {
        let (m, cv) = &*self.state;
        let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
        g.deadline = std::time::Instant::now() + g.timeout;
        drop(g);
        cv.notify_all();
    }
}

#[cfg(any(windows, test))]
impl Drop for RpcWatchdog {
    fn drop(&mut self) {
        {
            let (m, cv) = &*self.state;
            let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
            g.stop = true;
            drop(g);
            cv.notify_all();
        }
        // ★join 필수: 감시자가 살아있는 채로 스트림이 닫히면 `CancelIoEx` 가 이미 닫힌(혹은
        // 재사용된) 핸들에 나간다. 호출부는 스트림보다 **먼저** 이 값을 drop 해야 한다.
        if let Some(j) = self.joiner.take() {
            let _ = j.join();
        }
    }
}

/// 한 왕복 동안 상한을 쥐고 있는 값 — drop 되면 상한이 풀린다(unix 는 소켓 옵션 원복 불요,
/// Windows 는 감시 스레드 정지·join).
struct RpcDeadline {
    idle: Option<std::time::Duration>,
    #[cfg(windows)]
    watchdog: Option<RpcWatchdog>,
}

impl RpcDeadline {
    /// unix: 커널 소켓 타임아웃이 곧 무진행 상한이다(매 read/write 마다 타이머가 새로 돈다 =
    /// 진행 재장전이 커널에서 공짜로 된다).
    #[cfg(unix)]
    fn arm(stream: &ConnStream, idle: Option<std::time::Duration>) -> Result<Self, String> {
        stream
            .set_read_timeout(idle)
            .map_err(|e| format!("set_read_timeout: {e}"))?;
        stream
            .set_write_timeout(idle)
            .map_err(|e| format!("set_write_timeout: {e}"))?;
        Ok(Self { idle })
    }

    /// Windows: named pipe 에는 `SO_RCVTIMEO` 대응물이 **없다**(`SetCommTimeouts` 는 통신 리소스=
    /// 직렬포트 전용이라 파이프 핸들에 걸리지 않는다). 그래서 블록된 동기 `ReadFile`/`WriteFile` 을
    /// **다른 스레드에서 깨우는** 두 API 를 함께 쏜다 — 이 저장소는 Windows 크로스 타입체크·실행이
    /// 불가능하므로(호스트 툴체인이 libsqlite3-sys 의 C 를 크로스 빌드하지 못한다) "한쪽이 안 먹으면
    /// 상한이 통째로 no-op" 인 배치를 피하는 것이 설계 의도다:
    ///   ① `CancelIoEx(hFile, NULL)` — 그 **핸들**의 미처리 I/O 를 프로세스 내 어느 스레드가
    ///      발행했든 취소한다(Vista+).
    ///   ② `CancelSynchronousIo(hThread)` — **동기** I/O 취소의 정식 API. 대상 스레드는 이 왕복을
    ///      실행 중인 호출 스레드다(arm 시점의 `GetCurrentThreadId`). 필요 권한은 THREAD_TERMINATE.
    /// 취소된 호출은 ERROR_OPERATION_ABORTED(995)로 즉시 반환하고, 그 코드를
    /// `is_rpc_timeout_error` 가 만료로 판정한다. 둘 다 '취소할 것이 없으면' ERROR_NOT_FOUND 로
    /// 실패할 뿐 부작용이 없다.
    ///
    /// ★역할 분담(문서 근거 — Microsoft "Canceling Pending I/O Operations"): **동기** I/O 는
    /// `CancelSynchronousIo`, **비동기(overlapped)** I/O 는 `CancelIoEx` 가 지목된 API 다. 여기서
    /// 블록되는 것은 동기 `ReadFile`/`WriteFile` 이므로 ②가 정공법이고, ①은 그 파이프 핸들에
    /// 걸린 미처리 I/O 를 함께 훑는 **헤지**다. "동기 I/O 를 밖에서 깨우는 수단이 `CancelIoEx`
    /// 하나뿐" 이라는 단정은 사실이 아니며(그 단정은 이 코드가 ②를 함께 쏘는 이유도 설명하지
    /// 못한다), 둘을 병행하는 이유는 "한쪽이 안 먹는 환경에서 상한이 통째로 no-op 이 되는 것"의
    /// 회피다.
    ///
    /// ★★**Windows 실기 미검증**: 이 arm 은 이 저장소의 CI(mac/ubuntu)에서 **컴파일조차 되지
    /// 않는다**. `windows_arm_is_not_a_noop_source_pin` 은 소스에 두 호출이 **문자열로 존재하는지**만
    /// 확인할 뿐 동작을 확인하지 않는다 — 취소가 실제로 블록된 호출을 깨우는지, 995 가 실제로
    /// 올라오는지는 Windows 실기 검증 전까지 **미확인**이다. 과장하지 않는다.
    /// ★이 arm 은 no-op 이 아니다 — 구 `request_on_timeout` 의 Windows arm 이 바로 그 결함이었다.
    #[cfg(windows)]
    fn arm(stream: &ConnStream, idle: Option<std::time::Duration>) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::System::Threading::{
            GetCurrentThreadId, OpenThread, THREAD_TERMINATE,
        };
        use windows_sys::Win32::System::IO::{CancelIoEx, CancelSynchronousIo};
        let Some(d) = idle else {
            return Ok(Self {
                idle,
                watchdog: None,
            });
        };
        // 원시 핸들·스레드 id 만 넘긴다(스트림 소유권과 무관한 정수). 감시자는 Drop 에서 join 되고,
        // 호출부는 스트림보다 먼저 RpcDeadline 을 drop 하므로 닫힌 핸들에 호출되지 않는다.
        let raw = stream.as_raw_handle() as isize;
        // SAFETY: 인자 없는 순수 조회.
        let tid = unsafe { GetCurrentThreadId() };
        let watchdog = RpcWatchdog::new(d, move || unsafe {
            // SAFETY: raw 는 살아있는 파이프 핸들(위 수명 논증). 반환값(BOOL)은 의도적으로 무시한다 —
            // '취소할 I/O 없음'(ERROR_NOT_FOUND)은 정상 귀결이지 오류가 아니다.
            CancelIoEx(raw as HANDLE, std::ptr::null());
            let th = OpenThread(THREAD_TERMINATE, 0, tid);
            if !th.is_null() {
                CancelSynchronousIo(th);
                CloseHandle(th);
            }
        });
        Ok(Self {
            idle,
            watchdog: Some(watchdog),
        })
    }

    /// 진행 재장전(Windows 만 실동작 — unix 는 커널이 알아서 한다).
    fn touch(&self) {
        #[cfg(windows)]
        if let Some(w) = &self.watchdog {
            w.touch();
        }
    }

    fn idle(&self) -> std::time::Duration {
        self.idle.unwrap_or_default()
    }
}

/// 왕복 I/O 실패의 3분류 — 상한 만료는 처방 문안이 다르므로 다른 오류와 섞지 않는다.
#[derive(Debug, PartialEq)]
enum RpcIoFail {
    Timeout,
    Eof,
    Io(String),
}

/// 개행까지 한 줄을 읽되 **진행이 있을 때마다 상한을 재장전**한다.
/// `request()` 는 연결당 요청 1건이라 개행 뒤 잔여 바이트가 존재하지 않는다(단발 왕복 계약) —
/// 그래서 개행 뒤를 버려도 손실이 없다. 줄 길이 상한은 두지 않는다(개정 전 `read_line` 과 동일:
/// 새 실패 유형을 만들지 않는다).
fn read_frame_line<R: Read>(reader: &mut R, dl: &RpcDeadline) -> Result<String, RpcIoFail> {
    let mut out: Vec<u8> = Vec::with_capacity(4096);
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Err(RpcIoFail::Eof),
            Ok(n) => {
                dl.touch(); // 진행 있음 → 재장전
                if let Some(p) = buf[..n].iter().position(|&b| b == b'\n') {
                    out.extend_from_slice(&buf[..p]);
                    return Ok(String::from_utf8_lossy(&out).into_owned());
                }
                out.extend_from_slice(&buf[..n]);
            }
            // ★순서 주의: 만료 판정이 Interrupted 재시도보다 **먼저** 와야 한다.
            // Windows 의 ERROR_OPERATION_ABORTED(995)를 std 가 어떤 ErrorKind 로 접든(현행
            // rustc 는 TimedOut 그룹) 여기서 먼저 걸린다. 뒤에 두면 995 가 Interrupted 로 접히는
            // 순간 **무한 재시도**가 된다 — 워치독은 재장전돼도 이미 취소를 보냈고 읽기는 다시
            // 블록되므로, 상한이 있는데도 영구 대기하는 최악의 형태가 된다.
            Err(e) if is_rpc_timeout_error(&e) => return Err(RpcIoFail::Timeout),
            // unix EINTR(errno 4)는 위 판정에 걸리지 않으므로 여기서 정상 재시도된다.
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(RpcIoFail::Io(e.to_string())),
        }
    }
}

/// 상한이 장전된 스트림 위의 단발 왕복 본체 — `request()`·`request_on_timeout` 공용.
fn rpc_roundtrip<S: Read + Write>(
    stream: &mut S,
    dl: &RpcDeadline,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let req = json!({"id": 1, "method": method, "params": params});
    let mut line = serde_json::to_string(&req).unwrap();
    line.push('\n');
    let write_err = |e: std::io::Error| -> String {
        if is_rpc_timeout_error(&e) {
            rpc_timeout_message(method, dl.idle())
        } else {
            e.to_string()
        }
    };
    stream.write_all(line.as_bytes()).map_err(write_err)?;
    stream.flush().map_err(write_err)?;
    dl.touch(); // 요청 전송 완료 = 진행 → 응답 대기 상한을 새로 장전
    let resp_line = read_frame_line(stream, dl).map_err(|f| match f {
        RpcIoFail::Timeout => rpc_timeout_message(method, dl.idle()),
        RpcIoFail::Eof => format!(
            "cysd 가 '{method}' 응답 없이 연결을 끊었다(요청은 전달됐을 수 있다 — 비멱등 명령은 재시도 전 상태를 먼저 확인하라)."
        ),
        RpcIoFail::Io(m) => format!("{method}: {m}"),
    })?;
    // T1-6: 디코더 대칭검증 — declared `_flen`/`_pv` 형제 메타가 있으면 트렁케이션/버전스큐를
    // 검출한다. additive 계약이라 반환은 top-level 응답 객체 그대로(아래 resp["ok"] 호환 유지).
    // 메타 없는 legacy peer 프레임은 graceful 수용. LenMismatch는 트렁케이션이므로 거부.
    let resp: Value = cys::wire::parse_frame(resp_line.trim()).map_err(|e| format!("abi: {e:?}"))?;
    if resp["ok"].as_bool() == Some(true) {
        Ok(resp["result"].clone())
    } else {
        Err(format!(
            "{}: {}",
            resp["error"]["code"].as_str().unwrap_or("error"),
            resp["error"]["message"].as_str().unwrap_or("unknown error")
        ))
    }
}

fn request(method: &str, params: Value) -> Result<Value, String> {
    // ★선언 순서 = drop 순서의 역순. `deadline` 을 `stream` 뒤에 선언해야 감시자가 스트림보다
    //   먼저 정지·join 된다(닫힌 핸들에 CancelIoEx 금지). 아래 명시 drop 은 그 계약의 이중 보증.
    // ★상한은 **연결 성립 이후**의 왕복에만 건다. connect() 자체는 이미 자기 유계 경로다
    //   (autostart 시 socket-ready 폴링 최대 4초 — `poll_socket_ready`). 최악 총소요는 그 4초 +
    //   본 상한이며, 두 축을 하나로 합치지 않는 이유는 실패 원인이 다르기 때문이다
    //   ('데몬이 없다' vs '데몬이 응답하지 않는다' — 처방이 갈린다).
    let mut stream = connect()?;
    let deadline = RpcDeadline::arm(&stream, rpc_idle_timeout(method, &params))?;
    let out = rpc_roundtrip(&mut stream, &deadline, method, params);
    drop(deadline);
    out
}

// ---------- commands ----------

fn run(command: Command) -> i32 {
    let result = match command {
        Command::Ping => request("system.ping", json!({})).map(|r| println!("{}", r.as_str().unwrap_or("pong"))),
        Command::PhoenixIdentity => {
            // 데몬 접속 없이 이 바이너리 자신의 3필드를 출력(phoenix 폴백 identity 대조의 self-report 측).
            println!(
                "{}",
                json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "build_id": cys::pack::build_id(),
                    "embedded_pack_hash": cys::pack::embedded_pack_hash(),
                    "protocol_version": cys::pack::PHOENIX_PROTOCOL_VERSION,
                })
            );
            return 0;
        }

        Command::Identify => {
            let caller = cys::env_compat(ENV_SURFACE_ID).ok_or(std::env::VarError::NotPresent)
                .ok()
                .and_then(|s| parse_surface_ref(&s))
                .map(|id| json!({"surface_id": id, "surface_ref": surface_ref(id)}))
                .unwrap_or(Value::Null);
            request("system.identify", json!({"caller": caller}))
                .map(|r| println!("{}", serde_json::to_string_pretty(&r).unwrap()))
        }

        Command::Actions { json } => {
            // 데이터 파생 명령 카탈로그 — clap 정의가 단일 진실원천(self-describing). 에이전트/LLM
            // 노드가 산문 표(CLAUDE.md) 재파싱 대신 이 기계 출력을 읽는다(eval-driven: 기계 산출만이 사실).
            let app = <Cli as clap::CommandFactory>::command();
            let mut actions: Vec<Value> = Vec::new();
            for sub in app.get_subcommands() {
                if sub.get_name() == "help" {
                    continue;
                }
                let args: Vec<Value> = sub
                    .get_arguments()
                    .filter(|a| a.get_id() != "help")
                    .map(|a| {
                        json!({
                            "name": a.get_id().as_str(),
                            "long": a.get_long(),
                            "required": a.is_required_set(),
                            "positional": a.is_positional(),
                        })
                    })
                    .collect();
                let subs: Vec<String> =
                    sub.get_subcommands().map(|s| s.get_name().to_string()).collect();
                actions.push(json!({
                    "name": sub.get_name(),
                    "about": sub.get_about().map(|s| s.to_string()),
                    "args": args,
                    "subcommands": subs,
                }));
            }
            let out = json!({"count": actions.len(), "actions": actions});
            if json {
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
            } else {
                for a in &actions {
                    println!(
                        "{:<22} {}",
                        a["name"].as_str().unwrap_or(""),
                        a["about"].as_str().unwrap_or("")
                    );
                }
            }
            Ok(())
        }

        Command::NewSurface { cwd, cmd, title, role, rows, cols } => {
            request(
                "surface.create",
                json!({"cwd": cwd, "cmd": cmd, "title": title, "role": role, "rows": rows, "cols": cols}),
            )
            .map(|r| println!("{}", r["surface_ref"].as_str().unwrap_or("?")))
        }

        Command::List => request("surface.list", json!({})).map(|r| {
            for s in r["surfaces"].as_array().cloned().unwrap_or_default() {
                println!(
                    "{}\trole={}\tpid={}\texited={}\t{}\t{}",
                    s["surface_ref"].as_str().unwrap_or("?"),
                    s["role"].as_str().unwrap_or("-"),
                    s["pid"],
                    s["exited"],
                    s["title"].as_str().unwrap_or(""),
                    s["cwd"].as_str().unwrap_or(""),
                );
            }
        }),

        Command::Send { surface, to, queued, clear_first, text } => {
            resolve_targets(&surface, &to).and_then(|sids| {
                let from = cys::env_compat(ENV_SURFACE_ID).and_then(|s| parse_surface_ref(&s));
                let multi = sids.len() > 1;
                let body = text.join(" ");
                for sid in sids {
                    let tag = if multi { format!(" → surface:{sid}") } else { String::new() };
                    // T3-13 권위 전달: clear_first는 데몬이 원자적으로(Ctrl-U 선정리 → paste → CR)
                    // 집행한다. 클라측 C-u·150ms sleep·게이트는 제거 — 비원자 split·race를 없앤다.
                    // agent 등록 pane 게이트는 데몬 send_text가 집행(clear_first_unsupported).
                    let r = match request(
                        "surface.send_text",
                        json!({"surface_id": sid, "text": body, "from": from, "queued": queued, "clear_first": clear_first}),
                    ) {
                        Ok(r) => r,
                        // ★B3: 타이핑 가드 거부 → `--queued` 1회 전환(inject_text T-0147-6 동형).
                        //   종전엔 여기서 에러가 그대로 올라가 **본문이 소실**됐다. 큐 배달은
                        //   출력 조용 + 사람 입력 냉각 후 `Inject{cr_delay}` 로 넣으므로 사람의
                        //   미완성 입력에 이어붙는 최악 경로가 구조적으로 불가능하다.
                        //   ★큐 배달은 CR 을 **포함**한다 — 이 명령 뒤에 오는 관례적
                        //     `cys send-key Return` 은 빈 프롬프트의 Enter 라 무해하다.
                        //   재시도는 정확히 1회다(반복하면 중복 주입).
                        Err(e) if should_queue_fallback_send(queued, clear_first, &e) => {
                            let r2 = request(
                                "surface.send_text",
                                json!({"surface_id": sid, "text": body, "from": from, "queued": true}),
                            )?;
                            let depth = r2["depth"].as_u64().unwrap_or(0);
                            eprintln!(
                                "[send] 사람 입력 감지 — 본문을 큐로 전환(QUEUED depth {depth}) surface={}",
                                surface_ref(sid)
                            );
                            warn_if_daemon_paused();
                            println!("QUEUED (depth {depth}){tag}");
                            continue;
                        }
                        Err(e) => return Err(e),
                    };
                    if queued {
                        println!("QUEUED (depth {}){tag}", r["depth"]);
                    } else {
                        println!("OK{tag}");
                    }
                }
                Ok(())
            })
        }

        Command::SendKey { surface, to, queued, keys } => {
            resolve_targets(&surface, &to).and_then(|sids| {
                for key in &keys {
                    if key_to_bytes(key).is_none() {
                        return Err(format!("unknown key: {key}"));
                    }
                    if queued && !matches!(key.as_str(), "Return" | "Enter") {
                        return Err(format!(
                            "--queued supports only Return/Enter (got: {key}) — \
                             다른 키는 quiet-time 텍스트 큐에 실을 수 없다"
                        ));
                    }
                }
                let multi = sids.len() > 1;
                // ★B3: 큐로 전환된 키가 하나라도 있으면 이 실행의 결론은 "QUEUED" 다 —
                //   뒤에 "OK" 를 덧붙이면 첫 줄만 읽는 소비 스크립트가 직접 제출로 오독한다.
                let mut any_fallback = false;
                for sid in sids {
                    let mut sid_fallback = false;
                    for key in &keys {
                        let r = match request(
                            "surface.send_key",
                            json!({"surface_id": sid, "key": key, "queued": queued}),
                        ) {
                            Ok(r) => r,
                            // ★B3: 제출 Return 이 타이핑 가드에 막히면 소실시키지 않고 큐로
                            //   1회 전환한다(inject_text T-0147-6 동형 · 데몬 계약상
                            //   send_key --queued 는 Return/Enter 전용이라 안전).
                            //   이것이 없어서 노드 보고의 Enter 가 조용히 사라졌다.
                            Err(e) if should_queue_fallback_send_key(queued, key, &e) => {
                                let r2 = request(
                                    "surface.send_key",
                                    json!({"surface_id": sid, "key": key, "queued": true}),
                                )?;
                                let depth = r2["depth"].as_u64().unwrap_or(0);
                                eprintln!(
                                    "[send-key] 사람 입력 감지 — Return 을 큐로 전환(QUEUED depth {depth}) surface={}",
                                    surface_ref(sid)
                                );
                                warn_if_daemon_paused();
                                println!("QUEUED (depth {depth})");
                                sid_fallback = true;
                                any_fallback = true;
                                continue;
                            }
                            Err(e) => return Err(e),
                        };
                        if queued {
                            match r["depth"].as_u64() {
                                Some(d) => println!("QUEUED (depth {d})"),
                                // 구 데몬은 queued 파라미터를 모르고 즉시 주입한다 —
                                // "QUEUED"로 오표시하지 않는다(skew의 결정론 신호).
                                None => eprintln!(
                                    "[send-key] 경고: 데몬이 --queued를 지원하지 않아 \
                                     직접 주입됨(구버전 cysd — 재기동으로 갱신하라)"
                                ),
                            }
                        }
                    }
                    if multi && !sid_fallback {
                        println!("OK → surface:{sid}");
                    }
                }
                if !multi && !queued && !any_fallback {
                    println!("OK");
                }
                Ok(())
            })
        }

        Command::SetStatus { state, context, task, surface } => {
            target_surface(&surface, &None).and_then(|sid| {
                request(
                    "status.set",
                    json!({"surface_id": sid, "state": state, "context": context, "task": task}),
                )
                .map(|_| println!("OK"))
            })
        }

        Command::UsageRegister { transcript, surface } => {
            target_surface(&surface, &None).and_then(|sid| {
                request(
                    "usage.register",
                    json!({"surface_id": sid, "transcript": transcript}),
                )
                .map(|_| println!("OK"))
            })
        }

        Command::UsageReportStdin { surface, quiet } => {
            return run_usage_report_stdin(&surface, quiet)
        }

        Command::UsageEventStdin { surface } => return run_usage_event_stdin(&surface),

        Command::UsageAccounts { json: as_json } => request("usage.accounts", json!({}))
            .map(|r| {
                if as_json {
                    println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
                } else {
                    for a in r["accounts"].as_array().into_iter().flatten() {
                        let label = a["label"].as_str().unwrap_or("?");
                        let provider = a["provider"].as_str().unwrap_or("?");
                        let rate: Vec<String> = a["rate"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|w| {
                                format!(
                                    "{} {:.0}%",
                                    w["label"].as_str().unwrap_or("?"),
                                    w["used_pct"].as_f64().unwrap_or(0.0)
                                )
                            })
                            .collect();
                        let obs = if a["updated_at"].is_null() {
                            "관측 없음".to_string()
                        } else {
                            rate.join(" · ")
                        };
                        println!("{provider:<12} {label:<32} {obs}");
                    }
                }
            }),

        Command::LearnCheckpoint => {
            let mut buf = String::new();
            if std::io::stdin().read_to_string(&mut buf).is_err() || buf.trim().is_empty() {
                eprintln!("error: stdin JSON 필요 ({{round, verdict, …}})");
                return 1;
            }
            let Ok(v) = serde_json::from_str::<Value>(&buf) else {
                eprintln!("error: stdin JSON 파싱 실패");
                return 1;
            };
            request("learn.checkpoint", v).map(|r| {
                println!("OK round={}", r["round"].as_str().unwrap_or("?"));
            })
        }

        Command::Status { json: as_json } => return run_status(as_json),
        Command::Fleet { json: as_json } => return run_fleet(as_json),

        Command::Pause { reason } => request("system.pause", json!({"reason": reason}))
            .map(|_| println!("PAUSED — 큐 배달·스케줄 발화 동결 (이미 실행 중인 에이전트 행동은 계속된다; cys resume로 해제)")),

        Command::Resume => request("system.resume", json!({}))
            .map(|_| println!("RESUMED — 동결된 큐·스케줄 재개")),

        Command::Drain { verify, timeout } if verify => {
            return run_drain_verify(timeout);
        }

        Command::Drain { .. } => {
            // 업데이트 재시작 전 살아있는 역할 노드에 저장 신호를 보내고 짧게 유예한다(best-effort).
            // 노드(LLM) 협조 의존이라 무손실 보장은 아니며, 주 복원 경로는 재시작 후 resume이다.
            // ★hard watchdog: 데몬 무응답으로 RPC(read_line)가 hang해도 12s 내 무조건 종료해,
            // 호출처(install_update)가 영구 정지하지 않게 한다.
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(12));
                std::process::exit(0);
            });
            let mut n = 0;
            if let Ok(topo) = request("system.topology", json!({})) {
                for e in topo["live"].as_array().cloned().unwrap_or_default() {
                    let Some(role) = e["role"].as_str() else { continue };
                    if let Ok(r) = request("system.resolve_role", json!({"role": role})) {
                        if let Some(sid) = r["surface_id"].as_u64() {
                            // ★(N7) 방향은 그대로 fail-open 이되 **침묵하지 않는다**.
                            //   여기서 통째로 버려지던 것은 관문 Hold 처방 전문이고, 이 신호를
                            //   못 받은 노드는 업데이트 재시작 전에 상태를 저장하지 못한다.
                            if let Err(e) = inject_text(sid, "[DRAIN] 업데이트 재시작이 임박했다. 승인 프롬프트 대기 중이면 이 메시지는 무시하라. 아니면 지금 _round/SESSION_STATE.md와 자기 TODO를 저장하고 작업을 멈춰라. 작업 재개는 복원 후 master 지시를 기다린다.") {
                                eprintln!(
                                    "[drain] {role} {} 저장 신호 미전달(계속 진행) — {e}",
                                    surface_ref(sid)
                                );
                            }
                            n += 1;
                        }
                    }
                }
            }
            if n > 0 {
                std::thread::sleep(std::time::Duration::from_secs(8));
            }
            println!("drained {n} node(s)");
            return 0;
        }

        Command::GateCheck => {
            return match request("system.gate_check", json!({})) {
                Ok(r) => {
                    if r["paused"].as_bool() == Some(true) {
                        println!("PAUSED (reason: {})", r["reason"].as_str().unwrap_or(""));
                        4
                    } else {
                        println!("running");
                        0
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            };
        }

        Command::Queue { action } => {
            return match action {
                QueueAction::List { surface, json: as_json } => parse_explicit_surface(&surface)
                    .and_then(|sid| request("queue.list", json!({"surface_id": sid})))
                    .map(|r| {
                        let entries = r["entries"].as_array().cloned().unwrap_or_default();
                        // --json: RPC entries 원문 — 텍스트 열 계약과 무관한 기계 소비 경로.
                        if as_json {
                            println!("{}", Value::Array(entries));
                            return 0;
                        }
                        if entries.is_empty() {
                            println!("(queue empty)");
                        }
                        for e in entries {
                            // ★G1(W2-B): 행 렌더는 queue_list_row 단일 소유 — 열 위치
                            // 계약(cols[3]=preview)과 회귀 핀은 그 정의부에 있다.
                            println!("{}", queue_list_row(&e));
                        }
                        0
                    })
                    .unwrap_or_else(|e| {
                        eprintln!("error: {e}");
                        1
                    }),
                QueueAction::Clear { surface } => parse_surface_ref(&surface)
                    .ok_or_else(|| format!("invalid surface ref: {surface}"))
                    .and_then(|sid| request("queue.clear", json!({"surface_id": sid})))
                    .map(|r| {
                        println!("cleared {} queued message(s)", r["cleared"]);
                        0
                    })
                    .unwrap_or_else(|e| {
                        eprintln!("error: {e}");
                        1
                    }),
                // ★G1(W2-E): 단건 강제 배달 — 게이트 거부는 exit 7(사유 stderr), 오류는 1.
                // 드레인 루프는 CLI 에도 없다(단건만 — 반복 강제로 페이싱을 뚫지 않는다).
                QueueAction::Deliver { surface, id: entry_id, allow_reorder } => {
                    parse_surface_ref(&surface)
                        .ok_or_else(|| format!("invalid surface ref: {surface}"))
                        .and_then(|sid| {
                            let mut p = json!({"surface_id": sid});
                            if let Some(eid) = entry_id {
                                p["entry_id"] = json!(eid);
                            }
                            if allow_reorder {
                                p["allow_reorder"] = json!(true);
                            }
                            request("queue.deliver", p)
                        })
                        .map(|r| {
                            println!(
                                "delivered {} (seq {}, forced, remaining {})",
                                r["queue_entry_id"].as_str().unwrap_or("?"),
                                r["seq"],
                                r["remaining"]
                            );
                            0
                        })
                        .unwrap_or_else(|e| {
                            eprintln!("error: {e}");
                            queue_deliver_exit_code(&e)
                        })
                }
            };
        }

        Command::CycleAgent {
            role,
            surface,
            verifier,
            save_files,
            clear_cmd,
            resume_text,
            timeout,
            force_no_verify,
        } => {
            return run_cycle_agent(
                role, surface, verifier, save_files, clear_cmd, resume_text, timeout,
                force_no_verify,
            )
        }

        Command::NodeRecover { surface, role } => return run_node_recover(surface, role),

        Command::Restore { cwd, include_master, no_resume } => {
            return run_restore(cwd, include_master, no_resume)
        }

        Command::Reinject { role, surface, check, timeout } => {
            return run_reinject(role, surface, check, timeout)
        }

        Command::Watch { surface, to, until, timeout, since } => {
            return match target_surface(&surface, &to).and_then(|sid| {
                request(
                    "surface.wait_for",
                    json!({"surface_id": sid, "pattern": until,
                           "timeout_secs": timeout, "since_line": since}),
                )
            }) {
                Ok(r) => {
                    if r["matched"].as_bool() == Some(true) {
                        println!("{}", r["line"].as_str().unwrap_or(""));
                        eprintln!("[matched line {} — next_cursor={}]", r["line_no"], r["next_cursor"]);
                        0
                    } else {
                        eprintln!("[no match: {} — next_cursor={}]",
                            r["reason"].as_str().unwrap_or("?"), r["next_cursor"]);
                        3
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            };
        }

        Command::Daemon { action } => return run_daemon_cmd(action),

        Command::Attest { action } => {
            return match action {
                AttestAction::Pin { surface, to } => target_surface(&surface, &to)
                    .and_then(|sid| request("attest.pin", json!({"surface_id": sid})))
                    .map(|r| {
                        println!("{}:{}", r["count"], r["hash"].as_str().unwrap_or("?"));
                        eprintln!("[이 pin을 SESSION_STATE 등 외부에 보관하라 — 검증 지평: anchor {} 이후]",
                            r["verification_horizon"]["anchor_count"]);
                        0
                    })
                    .unwrap_or_else(|e| {
                        eprintln!("error: {e}");
                        1
                    }),
                AttestAction::Verify { pin, surface, to } => {
                    let Some((count_s, hash)) = pin.split_once(':') else {
                        eprintln!("error: pin must be \"count:hash\"");
                        return 1;
                    };
                    let Ok(count) = count_s.parse::<u64>() else {
                        eprintln!("error: bad count in pin");
                        return 1;
                    };
                    match target_surface(&surface, &to).and_then(|sid| {
                        request(
                            "attest.verify",
                            json!({"surface_id": sid, "hash": hash, "count": count}),
                        )
                    }) {
                        Ok(r) => {
                            if r["match"].as_bool() == Some(true) {
                                println!("MATCH — transcript intact ({} lines)", count);
                                0
                            } else {
                                println!(
                                    "MISMATCH — {}",
                                    r["reason"].as_str().unwrap_or("hash differs (변조 또는 유실)")
                                );
                                2
                            }
                        }
                        Err(e) => {
                            eprintln!("error: {e}");
                            1
                        }
                    }
                }
            };
        }

        Command::Approval { action } => {
            return match action {
                // exit 0 = 서명됨(통과) / 비0 = 미서명·차단. cysd 미가용 시 fail-closed(비0).
                ApprovalAction::Check { command, cwd } => {
                    let cwd = cwd.or_else(|| {
                        std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string())
                    });
                    match request(
                        "approval.check",
                        json!({"command": command, "cwd": cwd}),
                    ) {
                        Ok(r) => {
                            if r["approved"].as_bool() == Some(true) {
                                0 // 서명된 prefix — guard.sh가 우회 통과
                            } else {
                                2 // 미서명 — 차단 유지
                            }
                        }
                        // cysd 미가용·RPC 실패 = fail-closed(미서명 취급, 자동 통과 금지)
                        Err(e) => {
                            eprintln!("[approval] check failed (fail-closed): {e}");
                            2
                        }
                    }
                }
                ApprovalAction::Sign { prefix, cwd } => {
                    let tokens: Vec<String> =
                        prefix.split_whitespace().map(|s| s.to_string()).collect();
                    if tokens.is_empty() {
                        eprintln!("error: --prefix must be a non-empty command prefix");
                        return 1;
                    }
                    let cwd = cwd.or_else(|| {
                        std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string())
                    });
                    match request(
                        "approval.sign",
                        json!({"command_prefix": tokens, "cwd": cwd}),
                    ) {
                        Ok(r) => {
                            println!("signed: {}", r["id"].as_str().unwrap_or("?"));
                            0
                        }
                        Err(e) => {
                            eprintln!("error: {e}");
                            1
                        }
                    }
                }
            };
        }

        Command::ReadScreen { surface, to, lines, since, max_lines } => {
            target_surface(&surface, &to).and_then(|sid| {
                if let Some(s) = since {
                    return request(
                        "surface.read_text",
                        json!({"surface_id": sid, "since_line": s, "max_lines": max_lines}),
                    )
                    .map(|r| {
                        let text = r["text"].as_str().unwrap_or("");
                        if !text.is_empty() {
                            println!("{text}");
                        }
                        eprintln!(
                            "[next_cursor={} latest={} truncated={}]",
                            r["next_cursor"], r["latest_cursor"], r["truncated"]
                        );
                    });
                }
                request("surface.read_text", json!({"surface_id": sid, "lines": lines}))
                    .map(|r| println!("{}", r["text"].as_str().unwrap_or("")))
            })
        }

        Command::InitPack { force, install_hook: _, no_install_hook, claude_settings } => {
            return run_init_pack(force, no_install_hook, claude_settings);
        }

        Command::PackUpdate { from, manifest_url, dry_run } => {
            return run_pack_update(from, manifest_url, dry_run);
        }
        Command::PackPlan { force } => return run_pack_plan(force),
        Command::PackMerge {
            file, take_new, keep_mine, ai, to_local, propose, yes, force_vendor, dry_run,
            force_unsafe_core,
        } => {
            return run_pack_merge(
                file, take_new, keep_mine, ai, to_local, propose, yes, force_vendor, dry_run,
                force_unsafe_core,
            );
        }
        Command::PackOwnership { rel, quiet } => {
            // 결정론 조회 전용(쓰기 0) — 분류 SOT 는 pack::ownership() 한 곳(pack-guard hook 이 소비).
            // ★effective 등급: 치유·prune 은 임베드/매니페스트 파일에만 작용하므로, 임베드에 없는
            // 자작 신규 파일은 등급과 무관하게 불가침 — "custom" 으로 구분해 hook 오탐을 차단한다.
            // ★G3 축2: 스코프 인지 분류 — dept 팩(pack-dept-*)의 soul.md 는 seed-once(base 헌장
            // 승계 후 불가침). --quiet 어휘 4종({system,user,seed-once,custom})은 그대로이며
            // pack-guard.sh 는 `= "system"` 비교만 하므로 user→seed-once 전이는 훅 거동 무변.
            let dir = cys::pack::pack_dir();
            let embedded = cys::pack::PACK_ALL.iter().any(|(r, _)| *r == rel.as_str());
            let name =
                if embedded { cys::pack::ownership_name_scoped(&rel, &dir) } else { "custom" };
            if quiet {
                println!("{name}");
            } else {
                let dept_soul = cys::pack::dept_scope_of(&dir).is_some()
                    && (rel == "soul.md" || rel.ends_with("/soul.md"));
                let meaning = if dept_soul && name == "seed-once" {
                    "부서 soul — base 헌장 승계(최초 1회 시드), 존재하면 force 여도 불가침"
                } else {
                    match name {
                        "custom" => "비출하 자작 파일 — 업데이트·치유·정리 전부 불가침(생존 보증 대상)",
                        "user" => "사용자 소유 — 업데이트가 절대 덮지 않음(vendor 전진은 .new 병치)",
                        "seed-once" => "런타임 상태 — 부재 시에만 시드, 존재하면 불가침",
                        _ => "vendor 소유 — 수정본은 다음 설치 스윕에 치유(수정 전 .user 보존). 자작은 새 파일로",
                    }
                };
                println!("{rel}: {name} — {meaning}");
            }
            return 0;
        }
        Command::PackRollback { file, yes, force_vendor, force_unsafe_core } => {
            return run_pack_rollback(file, yes, force_vendor, force_unsafe_core);
        }
        Command::HooksPrune { pack_dir, dry_run, allow_base } => {
            return run_hooks_prune(&pack_dir, dry_run, allow_base);
        }

        Command::PackManifest { key_id, signed_at, expires_at, min_binary_version, pack_version } => {
            return run_pack_manifest(key_id, signed_at, expires_at, &min_binary_version, pack_version);
        }

        Command::Doctor { fix, json, custom_report } => {
            if custom_report {
                return run_doctor_custom_report();
            }
            return run_doctor(fix, json);
        }
        Command::FactoryReset {
            plan, yes, json, purge_license, purge_local, purge_round, verbose, undo,
        } => {
            if let Some(dir) = undo {
                return run_factory_reset_undo(&dir, plan, yes, json);
            }
            return run_factory_reset(plan, yes, json, purge_license, purge_local, purge_round, verbose);
        }

        Command::License { action } => {
            let now = chrono::Utc::now().timestamp();
            match action {
                LicenseAction::Install { path } => {
                    match cys::license::install(std::path::Path::new(&path), now) {
                        Ok(msg) => {
                            println!("{msg}");
                            return 0;
                        }
                        Err(e) => {
                            eprintln!("error: {e}");
                            return 1;
                        }
                    }
                }
                LicenseAction::Status => {
                    println!("{}", cys::license::render_status(now));
                    return 0;
                }
            }
        }

        Command::PackDowngradeToFree { yes, override_valid_license } => {
            return run_pack_downgrade_to_free(yes, override_valid_license);
        }

        Command::PackRepairChannel { to, yes, expert_override } => {
            return run_pack_repair_channel(to, yes, expert_override);
        }

        Command::Quiesce { surface, off } => target_surface(&surface, &None).and_then(|sid| {
            request("surface.quiesce", json!({"surface_id": sid, "on": !off})).map(|_| {
                println!(
                    "surface:{sid} quiescing={}",
                    if off { "off" } else { "on" }
                );
            })
        }),

        Command::ClaimRole { role, surface, takeover_empty_seat } => {
            return run_claim_role(&role, surface, takeover_empty_seat)
        }

        Command::LaunchAgent { role, agent, cwd } => return run_launch_agent(&role, &agent, cwd),
        Command::Boot { cwd, json } => return run_boot(cwd, json),
        Command::AgentDetect { json } => return run_agent_detect(json),
        Command::TodoPath { role, emit_decl } => return run_todo_path(role, emit_decl),

        Command::SurfaceRole => return run_surface_role(),

        Command::Hook { event } => return run_hook(event),
        Command::BootIntent => return run_boot_intent(),

        Command::Resize { surface, rows, cols } => target_surface(&surface, &None).and_then(|sid| {
            request("surface.resize", json!({"surface_id": sid, "rows": rows, "cols": cols}))
                .map(|_| println!("OK"))
        }),

        Command::CloseSurface { surface, reap } => parse_surface_ref(&surface)
            .ok_or_else(|| format!("invalid surface ref: {surface}"))
            .and_then(|sid| {
                // ★W2/C6: --reap → cause="reap"(묘비 미생성). 기본=OwnerClose(묘비).
                let params = if reap {
                    json!({"surface_id": sid, "cause": "reap"})
                } else {
                    json!({"surface_id": sid})
                };
                request("surface.close", params).map(|r| {
                    println!("closed {} (descendants killed{})", surface,
                             if reap { ", reap" } else { "" });
                    let _ = r;
                })
            }),

        // ★G4(W4-C): 수동 좌석 회수 — 게이트 거부는 exit 7(사유 stderr), 오류는 1.
        Command::ReapSurface { surface } => return run_reap_surface(&surface),

        Command::Tombstone { role, remove, dept } => {
            if dept {
                request("dept_tombstone.set", json!({"name": role, "remove": remove})).map(|r| {
                    let n = r["dept_tombstones"].as_array().map(|a| a.len()).unwrap_or(0);
                    println!(
                        "dept tombstone {} {} (총 {n}개)",
                        role,
                        if remove { "removed" } else { "set" }
                    );
                })
            } else {
                request("tombstone.set", json!({"role": role, "remove": remove})).map(|r| {
                    let rev = r["tombstones_rev"].as_u64().unwrap_or(0);
                    println!(
                        "tombstone {} {} (rev={rev})",
                        role,
                        if remove { "removed" } else { "set" }
                    );
                })
            }
        }

        Command::Events { after_seq, names, categories, filter, reconnect, cursor_file } => {
            stream_events(after_seq, names, categories, filter, reconnect, cursor_file)
        }

        Command::Attach { surface } => parse_surface_ref(&surface)
            .ok_or_else(|| format!("invalid surface ref: {surface}"))
            .and_then(attach),

        Command::Run { surface, command } => {
            // 자식의 종료 코드를 그대로 프로세스 exit code로 전달
            return match run_scoped(surface, command) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            };
        }

        Command::Ps => request("ledger.list", json!({})).map(|r| {
            let entries = r["entries"].as_array().cloned().unwrap_or_default();
            if entries.is_empty() {
                println!("(ledger empty)");
            }
            for e in entries {
                println!(
                    "pid={}\tpgid={}\tscoped={}\tsurface={}\t{}",
                    e["pid"],
                    e["pgid"],
                    e["scoped"],
                    e["surface_id"],
                    e["cmd"].as_str().unwrap_or("")
                );
            }
        }),

        Command::Kill { pid } => {
            request("ledger.kill", json!({"pid": pid})).map(|_| println!("killed {pid}"))
        }

        Command::AddHealthRule { name, pattern, action, threshold, pause_secs } => {
            request(
                "health.add_rule",
                json!({"name": name, "pattern": pattern, "action": action,
                       "threshold": threshold, "pause_secs": pause_secs}),
            )
            .map(|_| println!("OK"))
        }

        Command::HealthRules => request("health.list_rules", json!({})).map(|r| {
            for rule in r["rules"].as_array().cloned().unwrap_or_default() {
                println!(
                    "{}\t{}",
                    rule["name"].as_str().unwrap_or("?"),
                    rule["pattern"].as_str().unwrap_or("")
                );
            }
        }),

        Command::Feed { action } => return run_feed(action),

        Command::Learn { topic, status } => {
            if status {
                request("learn.status", json!({}))
                    .map(|r| println!("{}", serde_json::to_string_pretty(&r).unwrap()))
            } else if let Some(t) = topic {
                request("learn.propose", json!({"reason": "manual", "topic": t}))
                    .map(|r| println!("{}", serde_json::to_string_pretty(&r).unwrap()))
            } else {
                Err("usage: cys learn <topic> | cys learn --status".to_string())
            }
        }

        Command::Schedule { action } => return run_schedule(action),
        Command::CostBaseline { action } => return run_cost_baseline(action),

        Command::Recall { query, role, surface, days, limit } => {
            parse_explicit_surface(&surface)
                .and_then(|sid| request(
                    "recall.search",
                    json!({"query": query, "role": role, "surface_id": sid, "days": days, "limit": limit}),
                ))
            .map(|r| {
                let matches = r["matches"].as_array().cloned().unwrap_or_default();
                if matches.is_empty() {
                    println!("(no matches — indexed lines: {})", r["indexed_lines"]);
                }
                for m in matches {
                    let ts = m["ts"].as_f64().unwrap_or(0.0) as i64;
                    let when = chrono_fmt(ts);
                    println!(
                        "[{}] surface:{}({}) {} | {}",
                        when,
                        m["surface_id"],
                        m["role"].as_str().unwrap_or("-"),
                        m["title"].as_str().unwrap_or(""),
                        m["line"].as_str().unwrap_or(""),
                    );
                }
            })
        }

        Command::Skill { action } => return run_skill(action),
        Command::Persona { action } => return run_persona(action),
        Command::Channel { action } => return run_channel(action),
    };

    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn run_feed(action: FeedAction) -> i32 {
    let result: Result<i32, String> = match action {
        FeedAction::Push { kind, title, body, surface, request_id, wait, timeout_secs, tier } => {
            parse_explicit_surface(&surface)
                .and_then(|explicit| {
                    let sid = explicit
                        .or_else(|| cys::env_compat(ENV_SURFACE_ID).and_then(|s| parse_surface_ref(&s)));
                    request(
                        "feed.push",
                        json!({"kind": kind, "title": title, "body": body, "surface_id": sid,
                               "request_id": request_id, "wait": wait, "timeout_secs": timeout_secs,
                               "tier": tier}),
                    )
                })
            .map(|r| {
                if wait {
                    let status = r["status"].as_str().unwrap_or("");
                    let decision = r["decision"].as_str().unwrap_or("");
                    println!("{}", if status == "timeout" { "timeout" } else { decision });
                    match (status, decision) {
                        ("timeout", _) => 3,
                        (_, "allow") | (_, "yes") | (_, "approve") => 0,
                        _ => 2,
                    }
                } else {
                    println!("{}", r["request_id"].as_str().unwrap_or("?"));
                    0
                }
            })
        }
        FeedAction::List { status } => request("feed.list", json!({"status": status})).map(|r| {
            let items = r["items"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                println!("(feed empty)");
            }
            for i in items {
                println!(
                    "{}\t[{}]\t{}\t{}\tdecision={}",
                    i["request_id"].as_str().unwrap_or("?"),
                    i["status"].as_str().unwrap_or("?"),
                    i["kind"].as_str().unwrap_or("?"),
                    i["title"].as_str().unwrap_or(""),
                    i["decision"].as_str().unwrap_or("-"),
                );
            }
            0
        }),
        FeedAction::Reply { request_id, decision, reason } => {
            // reason은 Some일 때만 실어 보낸다(None=키 부재 → 데몬에서 null 처리).
            request(
                "feed.reply",
                json!({"request_id": request_id, "decision": decision, "reason": reason}),
            )
            .map(|_| {
                println!("OK");
                0
            })
        }
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// (2c) 재연결해도 되는 일시적 오류인가? cmux isTransientEventStreamError(Events.swift:105-134) 포팅.
/// ★실측 정렬: cys connect()는 `cannot connect to cysd at {path}: {e}`를 반환하고 {e}는 OS 에러
/// Display라 누락 소켓="No such file or directory (os error 2)"·거부="Connection refused (os error 61)",
/// read half-open="Broken pipe (os error 32)"/"Connection reset by peer (os error 54)"로 나온다.
/// 서버가 (2a) slow_consumer로 스트림을 종료한 케이스도 재연결 대상. 그 외(invalid_params 등)는 비-transient.
fn is_transient_event_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    const MARKERS: &[&str] = &[
        "no such file or directory", // cys connect_raw: 누락 소켓(ENOENT) — 데몬 재기동 중
        "connection refused",        // 데몬 부팅 직전(ECONNREFUSED)
        "connection reset",          // half-open read(ECONNRESET)
        "broken pipe",               // write/read 단절(EPIPE)
        "event stream closed",       // 정상 EOF — 재연결로 이어붙임
        "slow_consumer",             // 서버가 (2a)로 종료한 케이스
        "cannot connect to cysd",    // connect_raw 래퍼 문구(autostart 실패 포함)
        "os error 32",
        "os error 35",
        "os error 54",
        "os error 57",
        "os error 60",
        "os error 61",
    ];
    MARKERS.iter().any(|k| m.contains(k))
}

/// Subscribe to the push event stream and print NDJSON lines.
fn stream_events(
    after_seq: Option<u64>,
    names: Vec<String>,
    categories: Vec<String>,
    filter: Option<String>,
    reconnect: bool,
    cursor_file: Option<String>,
) -> Result<(), String> {
    // (3) 시드: --after_seq 미지정이면 cursor-file에서 읽는다(cmux Events.swift:25-27).
    let mut last_seq = after_seq.or_else(|| {
        cursor_file
            .as_ref()
            .and_then(|p| read_event_cursor(p).ok().flatten())
    });
    // ★P1-3 ⑥: 완전 초기화 중이면 구독이 조용히 끊긴 것처럼 보인다(원인은 "데몬을 일부러 껐다").
    // 사유를 한 번만 알리고, --reconnect 는 그대로 재시도해 초기화 후 자동 복귀시킨다.
    let mut reset_notified = false;
    loop {
        if cys::factory_reset::reset_in_progress() && !reset_notified {
            eprintln!("[events] 완전 초기화가 진행 중 — 데몬이 없습니다. 끝나면 자동 재연결합니다.");
            reset_notified = true;
        }
        let attempt = (|| -> Result<(), String> {
            let mut stream = connect()?;
            let req = json!({
                "id": 1, "method": "events.stream",
                "params": {"after_seq": last_seq, "names": names, "categories": categories},
            });
            let mut line = serde_json::to_string(&req).unwrap();
            line.push('\n');
            stream
                .write_all(line.as_bytes())
                .map_err(|e| e.to_string())?;
            let reader = BufReader::new(stream);
            for read in reader.lines() {
                let l = read.map_err(|e| e.to_string())?;
                // (2c) 에러 프레임을 행동으로 연결: slow_consumer/replay_gap을 Err로 격상해
                // 재시도 게이트가 transient 판정을 거치게 한다. 출력 중복을 막으려 should_return
                // 플래그를 세우고 println은 루프 말미 한 곳에서만 한다.
                let mut should_return: Option<String> = None;
                // --filter 접두 뷰 필터: 이벤트 이름이 접두와 안 맞으면 출력만 건너뛴다(커서는
                // 전 이벤트에 대해 전진 — 뷰 필터라 replay/커서 단조성은 불변).
                let mut suppress_print = false;
                if let Ok(v) = serde_json::from_str::<Value>(&l) {
                    match v["type"].as_str() {
                        Some("event") => {
                            if let Some(seq) = v["seq"].as_u64() {
                                last_seq = Some(seq);
                                if let Some(cf) = &cursor_file {
                                    write_event_cursor(cf, seq)?; // (3) 매 이벤트 원자적 갱신
                                }
                            }
                            if let Some(prefix) = filter.as_deref() {
                                let name = v["name"].as_str().unwrap_or("");
                                if !name.starts_with(prefix) {
                                    suppress_print = true;
                                }
                            }
                        }
                        Some("ack") if last_seq.is_none() => {
                            // 첫 이벤트 수신 전 끊겨도 재접속이 구체적 커서로 replay 경로를 타게 시드
                            last_seq = v["latest_seq"].as_u64();
                        }
                        Some("heartbeat") => { /* keepalive — 출력만, 커서 영향 없음 */ }
                        Some("error") if v["ok"] == false => {
                            let code = v["error"]["code"].as_str().unwrap_or("stream_error");
                            should_return = Some(code.to_string());
                        }
                        _ => {}
                    }
                }
                if !suppress_print {
                    println!("{l}");
                }
                if let Some(c) = should_return {
                    return Err(c);
                }
            }
            Err("event stream closed".into())
        })();
        match attempt {
            // (2c) transient만 재연결 — 비-transient는 즉시 반환(무한루프 차단)
            Err(e) if reconnect && is_transient_event_error(&e) => {
                eprintln!("[events] {e}; reconnecting in 1s...");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            other => return other,
        }
    }
}

/// (3) cmux readEventCursor(Events.swift:206-222): 없으면 None, 비숫자면 Err.
fn read_event_cursor(path: &str) -> Result<Option<u64>, String> {
    let p = expand_tilde(path);
    match std::fs::read_to_string(&p) {
        Ok(s) => s
            .trim()
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("bad cursor in {path}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// (3) cmux writeEventCursor(Events.swift:224-231): 디렉터리 생성 + 원자적 쓰기(tmp+rename).
/// std::fs::write 직접보다 tmp+rename으로 쓰기 도중 프로세스가 죽어도 커서가 절반 상태로 남지 않게 한다.
fn write_event_cursor(path: &str, seq: u64) -> Result<(), String> {
    let p = expand_tilde(path);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, format!("{seq}\n")).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

/// Mirror raw PTY output to stdout.
fn attach(sid: u64) -> Result<(), String> {
    let mut stream = connect()?;
    let req = json!({"id": 1, "method": "surface.attach", "params": {"surface_id": sid}});
    let mut line = serde_json::to_string(&req).unwrap();
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| e.to_string())?;
    // First line is the JSON ack; everything after is raw bytes.
    let mut reader = BufReader::new(stream);
    let mut ack = String::new();
    reader.read_line(&mut ack).map_err(|e| e.to_string())?;
    let ack_v: Value = serde_json::from_str(ack.trim()).unwrap_or(Value::Null);
    if ack_v["ok"].as_bool() != Some(true) {
        return Err(format!("attach failed: {}", ack.trim()));
    }
    eprintln!("[attached surface:{sid} — Ctrl-C to detach]");
    let mut stdout = std::io::stdout();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                stdout.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                stdout.flush().ok();
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn chrono_fmt(epoch: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let dt = UNIX_EPOCH + Duration::from_secs(epoch.max(0) as u64);
    // 로컬 포맷은 데몬이 epoch만 주므로 간단 표기 (ISO-ish, 로컬 오프셋 미적용 시 UTC)
    match std::process::Command::new("date")
        .args(["-r", &epoch.to_string(), "+%m-%d %H:%M"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => format!("{:?}", dt),
    }
}

/// C0 채널 서브명령 디스패처 — 전부 channel.* RPC의 thin wrapper. 결과 JSON을 한 줄로 출력
/// (브리지가 파싱해 소비). 에러도 JSON({"ok":false,...})으로 stdout에 내보내되 exit는 비0.
fn run_channel(action: ChannelAction) -> i32 {
    // Tier C opt-in은 duration 파싱 실패를 깔끔히 보고해야 하므로 조기 처리.
    if let ChannelAction::AllowRemoteApprove { duration, off } = &action {
        let secs = if *off {
            0
        } else {
            match duration.as_deref().map(parse_duration_secs) {
                Some(Ok(n)) => n,
                _ => {
                    println!("{}", json!({"ok": false,
                        "error": "invalid --for duration (use 8h|30m|45s|1d) or --off"}));
                    return 1;
                }
            }
        };
        return match request("channel.allow-remote-approve", json!({"duration_secs": secs})) {
            Ok(r) => {
                println!("{}", serde_json::to_string(&r).unwrap_or_default());
                0
            }
            Err(e) => {
                println!("{}", json!({"ok": false, "error": e}));
                1
            }
        };
    }
    // M10: register 토큰은 argv 대신 env `CYS_CHANNEL_TOKEN`(스폰 시 주입) 우선 — ps 노출 회피.
    // --token 있으면 그것을, 없으면 env로 폴백. 둘 다 없으면 명확히 보고.
    if let ChannelAction::Register { channel, token, caps, bridge_ver } = &action {
        let token = token
            .clone()
            .or_else(|| std::env::var("CYS_CHANNEL_TOKEN").ok().filter(|s| !s.is_empty()));
        let Some(token) = token else {
            println!("{}", json!({"ok": false,
                "error": "no token — pass --token or set CYS_CHANNEL_TOKEN env"}));
            return 1;
        };
        return match request(
            "channel.register",
            json!({"channel": channel, "token": token, "caps": caps, "bridge_ver": bridge_ver}),
        ) {
            Ok(r) => {
                println!("{}", serde_json::to_string(&r).unwrap_or_default());
                0
            }
            Err(e) => {
                println!("{}", json!({"ok": false, "error": e}));
                1
            }
        };
    }
    let (method, params): (&str, Value) = match action {
        ChannelAction::Start { channel, cmd } => (
            "channel.start",
            json!({"channel": channel, "cmd": cmd}),
        ),
        ChannelAction::Stop { channel } => ("channel.stop", json!({"channel": channel})),
        ChannelAction::Status => ("channel.status", json!({})),
        // 위에서 조기 return으로 처리됨(env 토큰 폴백 경로).
        ChannelAction::Register { .. } => unreachable!(),
        ChannelAction::Inbound {
            channel, sender_id, sender_kind, peer, text, ts, msg_ref, idempotency_key, body_hash,
            kind, feed_id, nonce, decision,
        } => (
            "channel.inbound",
            json!({"channel": channel, "sender_id": sender_id, "sender_kind": sender_kind,
                   "peer": peer, "text": text, "ts": ts, "msg_ref": msg_ref,
                   "idempotency_key": idempotency_key, "body_hash": body_hash,
                   "kind": kind, "feed_id": feed_id, "nonce": nonce, "decision": decision}),
        ),
        ChannelAction::Outbound {
            channel, target, kind, body, reply_to, idempotency_key, retry_of,
        } => (
            "channel.outbound",
            json!({"channel": channel, "target": target, "kind": kind, "body": body,
                   "reply_to": reply_to, "idempotency_key": idempotency_key, "retry_of": retry_of}),
        ),
        ChannelAction::Receipt { outbound_id, outcome, platform_ref, detail } => (
            "channel.receipt",
            json!({"outbound_id": outbound_id, "outcome": outcome,
                   "platform_ref": platform_ref, "detail": detail}),
        ),
        ChannelAction::Ack { inbox_id } => ("channel.ack", json!({"inbox_id": inbox_id})),
        ChannelAction::Allow { channel, sender_id } => (
            "channel.allow",
            json!({"channel": channel, "sender_id": sender_id}),
        ),
        ChannelAction::Revoke { channel, sender_id } => (
            "channel.revoke",
            json!({"channel": channel, "sender_id": sender_id}),
        ),
        ChannelAction::Lockdown => ("channel.lockdown", json!({})),
        ChannelAction::Unlock => ("channel.unlock", json!({})),
        // 위에서 조기 return으로 처리됨(duration 파싱 보고 경로).
        ChannelAction::AllowRemoteApprove { .. } => unreachable!(),
    };
    match request(method, params) {
        Ok(r) => {
            println!("{}", serde_json::to_string(&r).unwrap_or_default());
            0
        }
        Err(e) => {
            println!("{}", json!({"ok": false, "error": e}));
            1
        }
    }
}

/// 스킬 라이브러리: jarvis/skills/<name>/SKILL.md (frontmatter 표지 + 4칸 본문).
/// D3 비용·효율 eval baseline (producer≠evaluator) — lock=박제·diff=회귀 판정.
/// 채점은 master(LOCKED ref launcher)가 직접 — producer(워커)가 자기채점 못 함(eval-driven 무결성).
fn run_cost_baseline(action: CostBaselineAction) -> i32 {
    // baseline 박제 위치 — pack 밖·로컬·gitignore(~/.cys는 repo 밖). _round 컨벤션.
    let path = match dirs::home_dir() {
        Some(h) => h.join(".cys/_round/cost_baseline.json"),
        None => {
            eprintln!("home_dir 해소 실패 — baseline 경로 불가");
            return 2;
        }
    };
    // baseline canonical json → sha256 핀(사후 변조 차단).
    let sha_of = |v: &Value| -> String {
        use sha2::{Digest, Sha256};
        let canon = serde_json::to_string(v).unwrap_or_default();
        let mut h = Sha256::new();
        h.update(canon.as_bytes());
        h.finalize().iter().map(|x| format!("{x:02x}")).collect()
    };
    match action {
        CostBaselineAction::Lock => {
            let resp = match request("control.cost_baseline", json!({"window": "7d"})) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("control.cost_baseline 실패: {e}");
                    return 1;
                }
            };
            let baseline = resp["baseline"].clone();
            let sha = sha_of(&baseline);
            let locked = json!({
                "baseline": baseline,
                "sha256": sha,
                "locked_at": resp["now"].clone(),
                "window": resp["window"].clone(),
            });
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!("디렉터리 생성 실패 {}: {e}", parent.display());
                    return 2;
                }
            }
            match std::fs::write(&path, serde_json::to_string_pretty(&locked).unwrap_or_default()) {
                Ok(_) => {
                    println!("baseline locked: {} (sha256 {}…)", path.display(), &sha[..12.min(sha.len())]);
                    0
                }
                Err(e) => {
                    eprintln!("baseline 쓰기 실패: {e}");
                    2
                }
            }
        }
        CostBaselineAction::Diff => {
            let locked_raw = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => {
                    eprintln!("박제본 없음 — 먼저 `cys cost-baseline lock` 실행: {}", path.display());
                    return 2;
                }
            };
            let locked: Value = match serde_json::from_str(&locked_raw) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("박제본 파싱 실패: {e}");
                    return 2;
                }
            };
            // 변조 검증(retention gate): 저장된 sha256 vs baseline 재계산 대조.
            let lb = locked["baseline"].clone();
            if locked["sha256"].as_str() != Some(sha_of(&lb).as_str()) {
                eprintln!("⚠ 박제본 sha256 불일치 — 사후 변조 의심. 판정 중단(retention gate).");
                return 1;
            }
            let cur = match request("control.cost_baseline", json!({"window": "7d"})) {
                Ok(r) => r["baseline"].clone(),
                Err(e) => {
                    eprintln!("control.cost_baseline 실패: {e}");
                    return 1;
                }
            };
            let f = |v: &Value| v.as_f64().unwrap_or(0.0);
            let cps_old = f(&lb["cost_per_session"]);
            let cps_new = f(&cur["cost_per_session"]);
            let rw_old = f(&lb["rework"]["global_rework_rate"]);
            let rw_new = f(&cur["rework"]["global_rework_rate"]);
            let band = 0.05; // ±5% noise band (설계 §8.6 — 1차 보수값)
            let verdict = if rw_new > rw_old + 1e-9 {
                "REGRESSED" // 비용↓라도 재작업률 상승 = 품질저하(reward-hack 차단·품질절대우선)
            } else if cps_old > 0.0 && cps_new < cps_old * (1.0 - band) {
                "IMPROVED"
            } else if cps_old > 0.0 && cps_new > cps_old * (1.0 + band) {
                "REGRESSED"
            } else {
                "FLAT"
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "verdict": verdict,
                    "cost_per_session": {"locked": cps_old, "current": cps_new},
                    "global_rework_rate": {"locked": rw_old, "current": rw_new},
                    "note": "REGRESSED=비용↑ 또는 재작업률↑(reward-hack 차단). 판정=master LOCKED ref 직접(producer≠evaluator).",
                }))
                .unwrap_or_default()
            );
            0
        }
    }
}

fn run_skill(action: SkillAction) -> i32 {
    let skills_dir = cys::pack::pack_dir().join("skills");
    let result: Result<(), String> = match action {
        SkillAction::New { name, description, pack } => (|| {
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err("name must be kebab-case ascii (a-z0-9-)".into());
            }
            // ★W-G1(커스텀 생존 설계 2026-07-17): 자작 스킬의 기본 거처 = local 오버레이(업데이트
            // 불가침 — §12.7 문서와 CLI 의 비대칭 해소). --pack 은 upstream 승격 예정 전용이며,
            // vendor 임베드와 동명이면 다음 스윕에 치유(교체)되므로 생성 자체를 거부한다.
            let vendor_rel = format!("skills/{name}/SKILL.md");
            let vendor_exists = cys::pack::PACK_ALL.iter().any(|(r, _)| *r == vendor_rel);
            let root = if pack {
                if vendor_exists {
                    return Err(format!(
                        "vendor 스킬 '{name}' 이 이미 출하됨 — 팩 안 동명 생성은 다음 스윕에 치유(소실)됩니다. \
                         오버레이 생성(기본값·shadowing) 또는 다른 이름을 쓰세요."
                    ));
                }
                skills_dir.clone()
            } else {
                cys::pack::local_dir().join("skills")
            };
            let dir = root.join(&name);
            let path = dir.join("SKILL.md");
            if path.exists() {
                return Err(format!("skill '{name}' already exists: {}", path.display()));
            }
            // 반대편 루트 중복도 고지(생성은 허용 — shadowing 은 정당한 사용).
            if !pack && vendor_exists {
                println!("[주의] 동명 vendor 스킬 존재 — 이 오버레이 스킬이 shadowing 으로 이깁니다(의도 확인).");
            }
            // ★W-D2 기준점: vendor 동명을 가리는 순간의 임베드 해시를 기록 — 이후 pack-plan 이
            // vendor 전진을 결정론 판정한다. 기록이 없으면 매 실행 "판정 불가" 잡음이 되어
            // 경고 피로(무시 학습)를 부른다 — 승격(--to-local)과 손수 생성 양 경로 모두에 심는다.
            let vendor_base: Option<String> = if !pack {
                cys::pack::PACK_ALL
                    .iter()
                    .find(|(r, _)| *r == vendor_rel)
                    .map(|(_, c)| cys::pack::content_hash_pub(c))
            } else {
                None
            };
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let body = format!(
                "---\nname: {name}\ndescription: {description}\n---\n\n\
                 # {name}\n\n\
                 ## 언제 쓰나\n- \n\n\
                 ## 순서\n1. \n\n\
                 ## 주의할 점 (함정 — 겪을 때마다 한 줄씩 누적하라)\n- \n\n\
                 ## 확인하는 방법 (검증 — 겪을 때마다 한 줄씩 누적하라)\n- \n"
            );
            std::fs::write(&path, body).map_err(|e| e.to_string())?;
            if let Some(h) = vendor_base {
                let _ = cys::pack::write_atomic(&dir.join(".vendor-base"), h.as_bytes());
            }
            println!("created {}", path.display());
            if !pack {
                println!("(업데이트 불가침 영역 — 어떤 패치·재설치에도 보존됩니다)");
            }
            println!("(4칸을 채우고, master 승인이 필요하면 feed push로 보고하라)");
            Ok(())
        })(),
        SkillAction::List => (|| {
            if !skills_dir.exists() {
                return Err(format!(
                    "no skills dir: {} (run cys init-pack)",
                    skills_dir.display()
                ));
            }
            // ① 오버레이 shadowing: 팩 스킬 위에 ~/.cys/local/skills 동명 스킬이 이긴다(업데이트 불가침).
            let mut merged: std::collections::BTreeMap<String, (String, bool)> = Default::default();
            for (root, local) in [(skills_dir.clone(), false), (cys::pack::local_dir().join("skills"), true)] {
                let Ok(entries) = std::fs::read_dir(&root) else { continue };
                for entry in entries.flatten() {
                    let Ok(content) = std::fs::read_to_string(entry.path().join("SKILL.md")) else {
                        continue;
                    };
                    let (mut name, mut desc) = (String::new(), String::new());
                    for line in content.lines().take(10) {
                        if let Some(v) = line.strip_prefix("name:") {
                            name = v.trim().to_string();
                        } else if let Some(v) = line.strip_prefix("description:") {
                            desc = v.trim().to_string();
                        }
                    }
                    if !name.is_empty() {
                        merged.insert(name, (desc, local));
                    }
                }
            }
            for (name, (desc, local)) in &merged {
                println!("{name}\t{desc}{}", if *local { "\t[local]" } else { "" });
            }
            if merged.is_empty() {
                println!("(no skills yet — `cys skill new <name> --description \"...\"`)");
            }
            Ok(())
        })(),
        SkillAction::Show { name } => (|| {
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err("name must be kebab-case ascii (a-z0-9-)".into());
            }
            // ① 오버레이 우선(local shadowing) → 팩 폴백.
            let local = cys::pack::local_dir().join("skills").join(&name).join("SKILL.md");
            let path = if local.exists() { local } else { skills_dir.join(&name).join("SKILL.md") };
            let content = std::fs::read_to_string(&path)
                .map_err(|_| format!("no skill '{name}' ({})", path.display()))?;
            println!("{content}");
            Ok(())
        })(),
        SkillAction::Run { name, ticket, agent, close_after, run_id } => (|| {
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err("name must be kebab-case ascii (a-z0-9-)".into());
            }
            if ticket.trim().is_empty() {
                return Err("ticket 비어 있음 — 무계약 실행 금지(task-prompt 경유 필수)".into());
            }
            // CC v2 WS-B: run 생애주기 등록(best-effort — 실패해도 실행은 막지 않는다).
            // ttl은 아래 close_after 기본(600)과 동일 값 — 데몬이 deadline 산출에 쓴다.
            if let Some(rid) = run_id.as_ref() {
                let _ = request(
                    "skill.run_started",
                    json!({"run_id": rid, "name": name,
                           "ttl_secs": close_after.unwrap_or(600)}),
                );
            }
            // 일회용 격리 실행 = schedule add --fresh 잡(즉발 원샷 + fresh + worker 디렉티브 주입 + 자동 close).
            // invisible `claude -p` 맹목복제 금지(PROMPT_RUNNER_ABSENT) — 보이는 surface + 원장 강제종료.
            // B1 교정: now_epoch()는 cysd 전용 → cys.rs는 chrono로 epoch 취득.
            let job_id = format!("skill-{}-{}", name, chrono::Local::now().timestamp());
            // ★누수 차단(설계 §1 성공기준1·§6 불변식2): 원샷+fresh는 schedule.rs effective_close_ttl이
            // close_after_secs=None이면 None을 반환(반복 fresh만 기본 TTL) → 명시 안 하면 surface 영구 누수.
            // 따라서 미지정 시 보수적 기본 600초를 부여해 worker-fresh-* 가 반드시 자동 close되게 한다.
            let rc = run_schedule(ScheduleAction::Add {
                id: job_id,
                time: None,
                every: None,
                in_dur: Some("0s".into()),   // 즉발 원샷(once:true)
                close_after: Some(close_after.unwrap_or(600)), // fresh 전용 TTL(누수 차단·미지정 600초)
                days: None,
                text: Some(ticket),          // task-prompt 티켓 본문
                to: Some("worker".into()),   // ★raw pane 금지 — worker 디렉티브 주입(compose_directive 폴백)
                command: None,
                if_absent_launch: false,
                fresh: true,                 // 보이는 일회용 surface
                agent: Some(agent),
                cwd: None,                   // 호출 폴더 = 워크플로우 폴더(launch_opts 규칙)
            });
            if rc == 0 {
                Ok(())
            } else {
                Err(format!("schedule add 실패 (rc={rc})"))
            }
        })(),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn run_persona(action: PersonaAction) -> i32 {
    let expert = std::env::var("CYS_OVERRIDE_EXPERT").map(|v| v == "1").unwrap_or(false);
    let result: Result<(), String> = match action {
        PersonaAction::ListParams => {
            println!("튜닝 가능 노브 (안전핵 denylist·recovery·kill-switch는 잠김 — 미표시):");
            for k in cys::overrides::KNOBS {
                println!("  {:<20} {}-{} (기본 {}) — {}", k.key, k.min, k.max, k.default, k.label);
            }
            println!(
                "\n페르소나: cys persona set --persona \"말투·호칭·언어 자유 텍스트\" (최대 {}자)",
                cys::overrides::PERSONA_MAX_LEN
            );
            Ok(())
        }
        PersonaAction::Show { role } => {
            let ov = cys::overrides::load_overrides(&role, expert);
            let path = cys::overrides::override_path(&role);
            println!("# role={role}  file={}", path.display());
            if ov.params.is_empty() && ov.persona.is_empty() {
                println!("(오버라이드 없음 — 정식 기본값 사용)");
            } else {
                for (k, v) in &ov.params {
                    println!("  {k} = {v}");
                }
                if !ov.persona.is_empty() {
                    println!("  persona = {:?}", ov.persona);
                }
            }
            for w in &ov.warnings {
                eprintln!("  ⚠ {w}");
            }
            println!("\n--- 조립 미리보기(오버라이드 블록) ---");
            print!("{}", cys::overrides::render_block(&ov));
            Ok(())
        }
        PersonaAction::Reset { role } => {
            let path = cys::overrides::override_path(&role);
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    println!("삭제 — 정식 기본 복귀: {}", path.display());
                    Ok(())
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("이미 오버라이드 없음: {}", path.display());
                    Ok(())
                }
                Err(e) => Err(format!("삭제 실패 {}: {e}", path.display())),
            }
        }
        PersonaAction::Set { role, param, persona } => (|| {
            if param.is_none() && persona.is_none() {
                return Err("--param key=val 또는 --persona \"...\" 중 최소 하나 필요".into());
            }
            let path = cys::overrides::override_path(&role);
            // 기존 파일 머지 — 검증 통과분만 갱신, 나머지 보존.
            let mut doc = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .unwrap_or_else(|| serde_json::json!({"schema_version": 1}));
            if !doc.is_object() {
                doc = serde_json::json!({"schema_version": 1});
            }
            if let Some(p) = &param {
                let (key, val) = p.split_once('=').ok_or("--param 형식: key=value")?;
                let n: u64 = val.trim().parse().map_err(|_| format!("값이 정수 아님: {val}"))?;
                cys::overrides::validate_knob(key.trim(), n, expert)?; // hard-reject
                // params가 객체가 아니면(부재·수동편집으로 잘못된 타입) 객체로 정규화 —
                // serde_json IndexMut는 비-Object/Null에 인덱싱 시 패닉하므로 fail-closed 정규화.
                if !doc["params"].is_object() {
                    doc["params"] = serde_json::json!({});
                }
                doc["params"][key.trim()] = serde_json::json!(n);
            }
            if let Some(text) = &persona {
                let (clean, warns) = cys::overrides::sanitize_persona(text);
                for w in &warns {
                    eprintln!("  ⚠ {w}");
                }
                doc["persona"] = serde_json::json!(clean);
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let pretty = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
            std::fs::write(&path, pretty).map_err(|e| format!("쓰기 실패 {}: {e}", path.display()))?;
            println!("저장: {}", path.display());
            Ok(())
        })(),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Heartbeat 스케줄 관리: schedule.json은 CLI가 직접 편집(데몬 핫 리로드), 조회·즉발은 RPC.
fn run_schedule(action: ScheduleAction) -> i32 {
    let path = cys::pack::pack_dir().join("schedule.json");
    let result: Result<(), String> = match action {
        ScheduleAction::Add {
            id,
            time,
            every,
            in_dur,
            close_after,
            days,
            text,
            to,
            command,
            if_absent_launch,
            fresh,
            agent,
            cwd,
        } => {
            (|| {
                if text.is_some() == command.is_some() {
                    return Err("exactly one of --text(+--to) or --command is required".into());
                }
                if text.is_some() && to.is_none() {
                    return Err("--text requires --to <role>".into());
                }
                if (if_absent_launch || fresh) && agent.is_none() {
                    return Err("--if-absent-launch/--fresh requires --agent".into());
                }
                if command.is_some()
                    && (to.is_some()
                        || if_absent_launch
                        || fresh
                        || agent.is_some()
                        || cwd.is_some())
                {
                    return Err("--command cannot be combined with --to/--if-absent-launch/--fresh/--agent/--cwd (these apply only to --text push jobs)".into());
                }
                // --time(반복)·--in(원샷)·--every(주기) 정확히 하나
                let mode_count = time.is_some() as u8 + in_dur.is_some() as u8 + every.is_some() as u8;
                if mode_count != 1 {
                    return Err("exactly one of --time (반복) / --in (원샷) / --every (주기) is required".into());
                }
                if let Some(m) = every {
                    if m == 0 {
                        return Err("--every must be >= 1 (minutes)".into());
                    }
                }
                if every.is_some() && days.is_some() {
                    return Err("--every(주기)는 --days와 함께 쓸 수 없다".into());
                }
                if in_dur.is_some() && days.is_some() {
                    return Err("--in(원샷)은 --days와 함께 쓸 수 없다".into());
                }
                if close_after.is_some() && !fresh {
                    return Err("--close-after는 --fresh 전용 (fresh surface TTL)".into());
                }
                // 데몬과 동일 규칙으로 add 시점에 검증 — 잘못된 값이 무음 무발화로 이어지는 것을 차단
                if let Some(t) = &time {
                    chrono::NaiveTime::parse_from_str(t, "%H:%M")
                        .map_err(|_| format!("invalid --time '{t}' (expected HH:MM)"))?;
                }
                let at: Option<i64> = match &in_dur {
                    Some(d) => {
                        let secs = parse_duration_secs(d)?;
                        // R-CLI-2: secs>i64::MAX면 `as i64`가 음수 wrap → now+음수 = 과거 발화 시각.
                        // 안전 캐스트(초과=Err) + saturating_add(i64 오버플로 clamp)로 봉인.
                        let secs_i64 = i64::try_from(secs)
                            .map_err(|_| format!("--in duration too large: {secs}s"))?;
                        Some(chrono::Local::now().timestamp().saturating_add(secs_i64))
                    }
                    None => None,
                };
                let mut root: Value = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_else(|| json!({"jobs": []}));
                let jobs = root
                    .as_object_mut()
                    .ok_or("schedule.json root is not an object")?
                    .entry("jobs")
                    .or_insert(json!([]));
                let arr = jobs.as_array_mut().ok_or("'jobs' is not an array")?;
                if arr.iter().any(|j| j["id"].as_str() == Some(id.as_str())) {
                    return Err(format!("job '{id}' already exists (remove first)"));
                }
                let days_vec: Vec<String> = days
                    .map(|d| d.split(',').map(|s| s.trim().to_lowercase()).collect())
                    .unwrap_or_default();
                const DOW: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
                if let Some(bad) = days_vec.iter().find(|d| !DOW.contains(&d.as_str())) {
                    return Err(format!(
                        "invalid --days token '{bad}' (allowed: mon,tue,wed,thu,fri,sat,sun)"
                    ));
                }
                let mut job = match (&time, at, every) {
                    (Some(t), _, _) => json!({"id": id, "time": t, "days": days_vec}),
                    (None, Some(at), _) => json!({"id": id, "at": at, "once": true}),
                    (None, None, Some(m)) => json!({"id": id, "every_minutes": m}),
                    _ => unreachable!(),
                };
                if let Some(ttl) = close_after {
                    job["close_after_secs"] = json!(ttl);
                }
                if let Some(t) = text {
                    job["action"] = json!("push");
                    job["to"] = json!(to.unwrap());
                    job["text"] = json!(t);
                    if if_absent_launch || fresh {
                        if if_absent_launch {
                            job["if_absent"] = json!("launch");
                        }
                        if fresh {
                            job["fresh"] = json!(true);
                        }
                        job["launch"] =
                            json!({"role": job["to"], "agent": agent.unwrap(), "cwd": cwd});
                    }
                } else {
                    job["action"] = json!("command");
                    job["command"] = json!(command.unwrap());
                }
                arr.push(job);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, serde_json::to_string_pretty(&root).unwrap())
                    .map_err(|e| e.to_string())?;
                println!(
                    "job added to {} (daemon hot-reloads within 30s)",
                    path.display()
                );
                Ok(())
            })()
        }
        ScheduleAction::List => request("schedule.status", json!({})).map(|r| {
            let jobs = r["jobs"].as_array().cloned().unwrap_or_default();
            if jobs.is_empty() {
                println!(
                    "(no jobs — {} )",
                    r["schedule_path"].as_str().unwrap_or("?")
                );
            }
            for j in jobs {
                let lf = r["last_fired"][j["id"].as_str().unwrap_or("")].as_i64();
                let when = j["time"]
                    .as_str()
                    .map(String::from)
                    .or_else(|| j["at"].as_i64().map(|a| format!("once@{}", chrono_fmt(a))))
                    .unwrap_or_else(|| "?".into());
                println!(
                    "{}\t{} {}\t{}\t{}\tlast_fired={}",
                    j["id"].as_str().unwrap_or("?"),
                    when,
                    j["days"]
                        .as_array()
                        .map(|d| if d.is_empty() {
                            "daily".to_string()
                        } else {
                            d.iter()
                                .filter_map(|x| x.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_default(),
                    j["action"].as_str().unwrap_or("?"),
                    j["text"].as_str().or(j["command"].as_str()).unwrap_or(""),
                    lf.map(|t| t.to_string()).unwrap_or_else(|| "-".into()),
                );
            }
        }),
        ScheduleAction::Remove { id } => (|| {
            let mut root: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let arr = root["jobs"]
                .as_array_mut()
                .ok_or("'jobs' is not an array")?;
            let before = arr.len();
            arr.retain(|j| j["id"].as_str() != Some(id.as_str()));
            if arr.len() == before {
                return Err(format!("no job '{id}'"));
            }
            std::fs::write(&path, serde_json::to_string_pretty(&root).unwrap())
                .map_err(|e| e.to_string())?;
            println!("removed {id}");
            Ok(())
        })(),
        ScheduleAction::RunNow { id } => {
            request("schedule.run_now", json!({"job_id": id})).map(|_| println!("fired {id}"))
        }
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// CYSJavis Pack 설치: 임베드된 템플릿을 ~/.cys/pack 에 기록 (기존 파일 보존이 기본).
/// SessionStart hook 등록도 기본 동작이다(절대지침 — 터미널 작동 순간부터 활성화).
/// --no-install-hook으로만 끌 수 있다.
fn run_init_pack(force: bool, no_install_hook: bool, claude_settings: Option<String>) -> i32 {
    let dir = cys::pack::pack_dir();
    // §3.1 팩 atomic swap: 파일별 in-place write(중단 시 반쯤 쓰인 팩) 대신 staging 전개→검증→
    // 원자 rename 교체(pack_dir.prev 1세대 보존). 중단은 기존 팩을 건드리지 않는다.
    // W0-d: cys init-pack CLI 핸들러는 라이브 팩 쓰기 프로덕션 진입점 — 인가 부여.
    // ★G3(--no-install-hook 일관성): 훅 억제는 개인 프로필(~/.claude*)뿐 아니라 격리 config dir
    // 훅 병합까지 일관 적용된다(install_hooks = !no_install_hook — 모든 계급의 훅 등록 억제).
    let (written, kept) = match cys::pack::install_staged(
        force,
        Some(cys::pack::PackWriteAuth::production()),
        !no_install_hook,
    ) {
        Ok(wk) => wk,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    println!(
        "CYSJavis Pack installed at {} ({} written, {} preserved{})",
        dir.display(),
        written,
        kept,
        if force { ", forced" } else { "" }
    );
    println!("다음: cys launch-agent --role master --agent claude  (역할 지침 자동 주입)");

    if no_install_hook {
        return 0;
    }
    let targets = match claude_settings {
        Some(p) => vec![p],
        None => {
            // ★G3 축1(결함2 두 번째 오염 표면): 부서 팩 컨텍스트의 init-pack 이 개인 프로필
            //   (~/.claude*)에 부서 경로 훅을 기록하던 경로 봉인 — 데몬 경로에 이미 있는 base-전용
            //   게이트(pack.rs merge_awakening_hooks_into_personal_profiles :531-534)를 CLI 경로에도
            //   일관 적용한다. --claude-settings 명시 시엔 존중(운영자의 명시 의도 — 위 Some 분기).
            if cys::pack::dept_scope_of(&dir).is_some() {
                println!(
                    "부서 팩 컨텍스트({}) — 개인 프로필(~/.claude*) 훅 무접촉(공용 프로필 무변조 \
                     기본 계약). 부서 훅은 부서 acctdir 시드(cys-dept launch/rotate) 소관이며, \
                     명시 대상은 --claude-settings 로.",
                    dir.display()
                );
                return 0;
            }
            let found = discover_claude_settings();
            if found.is_empty() {
                // 신규 머신: Claude Code 기본 경로에 생성해 "켜는 순간부터 활성화"를 보장.
                vec![dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".claude/settings.json")
                    .to_string_lossy()
                    .into_owned()]
            } else {
                found
            }
        }
    };
    let mut rc = 0;
    for settings_path in targets {
        if let Some(parent) = std::path::Path::new(&settings_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match install_claude_hook(&settings_path, &dir) {
            Ok(msg) => println!("hook[{settings_path}]: {msg}"),
            Err(e) => {
                eprintln!("error: hook install failed for {settings_path}: {e}");
                rc = 1;
            }
        }
    }
    rc
}

/// Claude Code 설정 파일 자동 탐색: $HOME 직하의 `.claude*` **디렉터리**의 settings.json 전부.
/// (멀티 프로필 환경 — 예: .claude / .claude-* — 을 한 번에 커버.)
/// 결정론: 사전순 정렬.
///
/// ★W3 G7: 종전엔 `settings.json` 이 **파일로 존재할 때만** 후보였다 — 프로필 디렉터리는 있는데
/// settings.json 이 아직 없는 상태(신규 프로필·사용자가 파일을 지운 상태)가 **영구 미배선**으로
/// 굳었다(그 프로필의 claude 세션은 훅 없이 돌고, 등록기는 그 프로필을 보지도 않는다). 이제
/// 디렉터리 존재를 기준으로 후보화하고 등록기가 파일을 생성한다(python `discover_claude_settings`
/// 와 동일 규칙 — 두 언어의 home-glob 규칙은 하나여야 한다).
fn discover_claude_settings() -> Vec<String> {
    cys::pack::personal_profile_settings_paths()
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Claude Code settings.json에 **소망 훅 집합**(SessionStart + UserPromptSubmit)을 등록한다.
///
/// ★W3 A9: 종전엔 SessionStart **하나만** 등록했다 — init-pack 을 거친 기계도 각성 훅
/// (role-bootstrap → UserPromptSubmit)은 preflight C28 이 처음 도는 순간까지 미등록이었고,
/// 그 C28 의 유일한 자동 트리거가 바로 그 미등록 훅이었다(닭·달걀). 이제 Rust 시드
/// (`setup_isolated_config_dir`)·init-pack·개인 프로필 병합이 **같은 매니페스트**
/// (`cys::pack::AWAKENING_HOOKS`)를 소비한다 — 소망상태는 한 곳에만 적힌다.
///
/// 멱등·백업·symlink·파싱 거부 규약은 `cys::pack::merge_desired_hooks` 계약에 위임한다
/// (백업은 **실제 write 시에만** — 멱등 재실행이 정상 `.bak-cys` 를 클로버하지 않는다·RC-1 D2).
/// ★T-0147-5(W3): launch-agent 가 기록한 config dir 의 settings.json 에 **각성 훅**이 없으면
/// 기동 로그 경고 + 승인 Feed push 로 1분 내 원인을 가시화한다.
///
/// 왜: 노드는 정상 기동하지만 그 config 계급에 훅이 없으면 ①`/clear` 후 지침 재주입(SessionStart)과
/// ②마스터 선언 부트 발화(UserPromptSubmit)가 **둘 다 사라진다**. 종전엔 어떤 채널에도 신호가 없어
/// "떠 있는데 각성만 안 되는" 침묵 고장이었다(등록≠가동 갭 — A21 재검증).
/// 판정은 preflight C28 의 FAIL 티어와 **같은 매니페스트**(`AWAKENING_HOOKS`)를 소비한다 —
/// 같은 표면·같은 술어여야 두 채널의 보고가 갈리지 않는다.
/// 비치명: 경고만 하고 부트는 계속한다(위경고 모드·부트 봉쇄 회귀 금지 — 금지 방향 ③ 정신).
fn warn_if_awakening_hooks_missing(config_dir: Option<&str>, role: &str, agent: &str) {
    // claude 계열만 대상(agy·codex 는 Claude-config 노드가 아니다 — preflight discover 와 동일 규약).
    if !agent.starts_with("claude") {
        return;
    }
    let Some(cfg) = config_dir else { return };
    let settings = std::path::Path::new(cfg).join("settings.json");
    let root = std::fs::read_to_string(&settings)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or_else(|| json!({}));
    let pack = cys::pack::pack_dir();
    let missing: Vec<&str> = cys::pack::AWAKENING_HOOKS
        .iter()
        .filter(|h| {
            !cys::pack::hook_registered_in(&root, h.event, &cys::pack::hook_command_for(&pack, h.script))
        })
        .map(|h| h.script)
        .collect();
    if missing.is_empty() {
        return;
    }
    // ★★M5(2026-08-24) — **듣지 않는 손잡이를 안내하지 않는다.**
    //   종전 문안은 무조건 "`javis_preflight.py --fix`(C28) 또는 `cys init-pack`" 을 처방했다.
    //   그 두 명령은 **설치 표적**(`pack::config_dir()` = base 레인에서 `pack.parent()/claude`)에
    //   쓰는데, 이 경고가 보는 dir 는 데몬이 기록한 **실소비 config dir**
    //   (`${CYS_ACCOUNT_DIR:-$HOME/.cys/claude}`)다. 팩이 `~/.cys/pack` 이 아니면 두 값이 갈리고,
    //   회전2 격리 주행에서 처방된 두 명령을 **완주시켰는데도 같은 경고가 재현**됐다
    //   (BLOCK-2 와 같은 부류의 재발). 어긋난 경우에는 그 사실을 문안이 먼저 말한다.
    let install_target = cys::pack::config_dir();
    let mismatch = install_target
        .as_deref()
        .and_then(|t| cys::pack::config_target_mismatch(Some(t), std::path::Path::new(cfg)));
    let action = match &mismatch {
        None => format!(
            "조치: `python3 {}/bin/javis_preflight.py --fix`(C28) 또는 `cys init-pack`.",
            pack.display()
        ),
        Some((target, consumed)) => format!(
            "★이 상태에서는 `cys init-pack` 도 `javis_preflight.py --fix` 도 **이 경고를 해소하지 \
             못합니다** — 두 명령의 설치 표적은 {}(팩 위치에서 파생)인데, 이 노드가 실제로 읽는 \
             dir 는 {}({} 해소)라 서로 다른 폴더입니다. 조치: ①`cys doctor`(config-dir-target 항목)로 \
             어긋남을 확인하고 ②`CYS_ACCOUNT_DIR` 을 설치 표적과 같은 값으로 맞추거나 팩을 \
             `~/.cys/pack` 으로 되돌린 뒤 ③위 두 명령 중 하나를 실행하십시오.",
            target.display(),
            consumed.display(),
            "${CYS_ACCOUNT_DIR:-$HOME/.cys/claude}"
        ),
    };
    let body = format!(
        "role={role} agent={agent} 의 config dir({})에 각성 훅이 없습니다: {}. \
         이 노드는 떠도 /clear 후 지침 재주입(SessionStart)·마스터 선언 부트 발화(UserPromptSubmit)가 \
         발동하지 않습니다. {action}",
        settings.display(),
        missing.join(", "),
    );
    eprintln!("[launch-agent] ⚠ 각성 훅 미등록 — {body}");
    // best-effort: 데몬 부재·거부여도 기동은 계속한다(경고 채널 실패가 부트를 죽이지 않는다).
    let _ = request(
        "feed.push",
        json!({"kind": "hook-missing", "title": "각성 훅 미등록(노드 기동)", "body": body}),
    );
}

/// ★★M5(2026-08-24) — 각성 훅 **설치 표적 ≠ 실소비 SOT** 의 loud WARN(기동 1회).
///
/// 판정은 순수 [`cys::pack::config_target_mismatch`] 가 소유하고 여기서는 실 경로 두 개를 먹여
/// 보고만 한다. 기본 경로에서는 두 값이 **우연히 일치**하므로 조용하다 — 어긋난 기계에서만
/// 나오고, 그 기계에서는 `/clear` 후 지침 재주입(SessionStart)·마스터 선언 부트 발화
/// (UserPromptSubmit)가 영구히 발동하지 않는다(팀 미기동).
///
/// **여기서 고치지 않는다**(태그 전 진단 전용 — 표적 통일은 부서 레인·기존 설치본까지 건드리는
/// 반경이라 별도 웨이브다). 수리 처방은 `cys doctor` 의 `config-dir-target` 항목이 소유한다.
fn warn_if_config_target_mismatch() {
    let Some(target) = cys::pack::config_dir() else {
        return; // 부서 스코프 + CYS_ACCOUNT_DIR 부재 = 시드 표적 없음(별도 진단 항목이 받는다).
    };
    let consumed = cys::resolve_claude_config_dir();
    let Some((target, consumed)) =
        cys::pack::config_target_mismatch(Some(&target), std::path::Path::new(&consumed))
    else {
        return;
    };
    eprintln!(
        "[boot] ★★각성 훅 설치 표적과 실소비 dir 가 다르다 — 훅이 **아무도 읽지 않는 폴더**에 \
         설치된다.\n\
         \x20 · 설치 표적(cys init-pack · javis_preflight --fix): {}\n\
         \x20 · 노드 실소비(CLAUDE_CONFIG_DIR = ${{CYS_ACCOUNT_DIR:-$HOME/.cys/claude}}): {}\n\
         \x20 귀결: 노드는 떠도 /clear 후 지침 재주입(SessionStart)·마스터 선언 부트 발화\
         (UserPromptSubmit)가 발동하지 않는다(팀 미기동).\n\
         \x20 ★이 상태에서는 `cys init-pack` 도 `javis_preflight.py --fix` 도 해소하지 못한다 — \
         두 명령은 설치 표적에 쓴다. 진단: `cys doctor`(config-dir-target)",
        target.display(),
        consumed.display()
    );
}

fn install_claude_hook(settings_path: &str, pack_dir: &std::path::Path) -> Result<String, String> {
    let added = cys::pack::merge_desired_hooks(
        std::path::Path::new(settings_path),
        pack_dir,
        &cys::pack::AWAKENING_HOOKS,
    )?;
    if added.is_empty() {
        return Ok("hook already installed (skipped)".into());
    }
    Ok(format!(
        "hook registered in {settings_path}: {} (backup: .bak-cys)",
        added.join(", ")
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// cys hooks-prune (G3 축1 치유층) — 공용/개인 settings.json 에서 지정 팩을 가리키는
// 훅만 제거한다. 훅 명령 문자열(hook_command_for)에 팩 절대경로가 박혀 있으므로 **경로
// 접두가 곧 소유 ID** 다(태깅 제2 SOT 기각). teardown(cys-dept down)·수동 치유의 단일
// 진입점이며, doctor(dept-hook-residue) --fix 도 같은 제거 엔진을 소비한다(제거 엔진 단일).
// ─────────────────────────────────────────────────────────────────────────────

/// hooks-prune 게이트 거부 exit — claim-role 정당거부(7) 계열 고정(예약 {0,1,2,64} 회피 ·
/// EXIT_UNSAFE_CORE_REFUSED·EXIT_QUEUE_GATE_REFUSED 선례). 오류(IO·파싱 거부)는 exit 1.
const EXIT_HOOKS_PRUNE_GATE_REFUSED: i32 = 7;

/// 게이트 순수부(진리표 테스트용) — base 팩(비 `pack-dept-*`) 대상은 `--allow-base` 없이는
/// 거부한다(부서 전용 기본 · base 훅 오제거 fail-closed).
fn hooks_prune_gate_refused(pack: &std::path::Path, allow_base: bool) -> bool {
    cys::pack::dept_scope_of(pack).is_none() && !allow_base
}

/// `<settings>.cys-lock` 파일락 획득(G16 3-writer 직렬화)[MAJOR 명기].
///
/// python preflight 는 settings.json RMW 를 파일별 락 `<settings>.cys-lock`(javis_lock.FileLock ·
/// unix=flock/win=msvcrt)으로 직렬화한다(javis_preflight.py G16 계약). Rust 신규 작성자
/// (hooks-prune·doctor --fix 의 잔존 제거)가 락 없이 RMW 하면 C28 재등록과 교차해 lost-update
/// (한쪽 쓰기 증발)가 난다 — 같은 락 파일로 직렬화한다.
///  · unix: flock(LOCK_EX) **블로킹**(보유 창 = 파일 1개 RMW, 수 ms) · 락 파일 열기 실패 = None
///    (직렬화만 포기하고 치유는 진행 — write_atomic 이 파손은 이미 차단, 락 실패가 치유를 막으면
///    잔존 훅이 영구화된다).
///  · windows: **미획득(None) — 감수 범위 명기**: python 쪽 백엔드가 msvcrt 바이트락이라 flock 과
///    상호 배제가 성립하지 않고(이종 락), Windows 부서 churn 표면은 현 릴리스에 없다. 파손은
///    원자 교체가 차단하며 최악은 RMW lost-update(다음 preflight C28/부트 시드가 재수렴). 승격 시
///    LockFileEx 동형 배선이 조건이다.
fn acquire_settings_lock(settings: &std::path::Path) -> Option<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let lock_path = std::path::PathBuf::from(format!("{}.cys-lock", settings.display()));
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&lock_path)
            .ok()?;
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return None;
        }
        Some(f)
    }
    #[cfg(not(unix))]
    {
        let _ = settings;
        None
    }
}

/// hooks-prune 의 대상 settings 목록 — 공용 격리 config(팩 부모/claude · 결함2 의 오염 표면) +
/// 개인 프로필(~/.claude*) 전부. 부서 자신의 acctdir 는 대상이 아니다(그곳의 훅은 그 부서의
/// 정당한 시드 — teardown 후엔 아무도 그 dir 로 claude 를 띄우지 않으므로 무해 잔존).
fn hooks_prune_targets(pack: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut targets: Vec<std::path::PathBuf> = Vec::new();
    if let Some(parent) = pack.parent() {
        targets.push(parent.join("claude").join("settings.json"));
    }
    for p in cys::pack::personal_profile_settings_paths() {
        if !targets.contains(&p) {
            targets.push(p);
        }
    }
    targets
}

/// 산 부서 훅 무조건 제거와 절대 불변 '산 훅 제거는 실측 조건부만'의 관계[리뷰 MINOR 명문화]:
/// 그 조건부 계약의 주어는 **자가치유 판단 주체**(doctor --fix — 시스템이 스스로 판단해 지우는
/// 경로 = `diag_dept_hook_residue` 의 acctdir 실측 조건부)다. hooks-prune 의 표면은
/// ①teardown 배선(cys-dept down/down-sock — 부서 소멸 중이라 잔존 훅이 곧 죽은 경로가 된다)
/// ②운영자 명시 호출(의도 선언 + --dry-run + per-run 백업 + 부서 전용 기본 게이트) 둘뿐이므로
/// 무조건 제거가 계약 위반이 아니다 — 산 부서를 보존하는 자가치유가 필요하면 doctor --fix 를 쓰라.
///
/// exit: 0=제거 완료·대상 없음 / 1=IO·파싱 거부(fail-closed) / 7=게이트 거부(base 팩 + --allow-base 부재).
fn run_hooks_prune(pack_dir_arg: &str, dry_run: bool, allow_base: bool) -> i32 {
    let pack = std::path::PathBuf::from(shellexpand_home(pack_dir_arg));
    let pack = if pack.is_absolute() {
        pack
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(pack)
    };
    if hooks_prune_gate_refused(&pack, allow_base) {
        eprintln!(
            "hooks-prune 게이트 거부: {} 는 부서 팩(pack-dept-*)이 아니다 — base 팩 훅 오제거를 \
             막는 기본 게이트(fail-closed). base 팩 훅을 정말 제거하려면 --allow-base 를 명시하라.",
            pack.display()
        );
        return EXIT_HOOKS_PRUNE_GATE_REFUSED;
    }
    let mut rc = 0;
    let mut any = false;
    for t in hooks_prune_targets(&pack) {
        let res = if dry_run {
            cys::factory_reset::hooks_pointing_into_pack(&t, &pack)
        } else {
            // 락은 파일별 · RMW 구간만 보유(스코프 drop 해제). 규약·감수 범위는 acquire 문서 참조.
            let _lock = acquire_settings_lock(&t);
            cys::factory_reset::strip_hooks_pointing_into_pack(&t, &pack, None)
        };
        match res {
            Ok(labels) if labels.is_empty() => {}
            Ok(labels) => {
                any = true;
                println!(
                    "{}: {}{}",
                    t.display(),
                    labels.join("·"),
                    if dry_run { " (dry-run — 무변경)" } else { " (백업: .bak-cys-dept)" }
                );
            }
            Err(e) => {
                eprintln!("error: {}: {e}", t.display());
                rc = 1;
            }
        }
    }
    if !any && rc == 0 {
        println!("제거 대상 없음 — {} 를 가리키는 훅 0", pack.display());
    }
    rc
}

/// 훅 명령 문자열에서 부서 팩 루트(`<base>/pack-dept-<name>`)를 추출한다(순수 —
/// dept-hook-residue 탐지). 경로경계 앵커: base 접두 + `/pack-dept-` + 비구분자 이름.
/// Windows 한계(감수 범위 · 리뷰 MINOR): 비교는 대소문자 민감 — NTFS 무구분 경로로 케이스만 다른
/// 잔존 훅은 미탐지(fail-safe: 오삭제 없음). Windows 부서 churn 표면이 생기는 릴리스에서 케이스
/// 폴딩 승격(`acquire_settings_lock` 의 Windows 감수 범위와 같은 트랙 · command_points_into_pack_root 동일).
fn dept_pack_of_command(command: &str, cys_base: &std::path::Path) -> Option<std::path::PathBuf> {
    let base = cys_base.to_string_lossy().replace('\\', "/");
    let cmd = command.replace('\\', "/");
    let needle = format!("{}/pack-dept-", base.trim_end_matches('/'));
    let at = cmd.find(&needle)?;
    let rest = &cmd[at + needle.len()..];
    let end = rest
        .find(|c: char| c == '/' || c == '"' || c == '\'' || c.is_whitespace())
        .unwrap_or(rest.len());
    let name = &rest[..end];
    if name.is_empty() {
        return None;
    }
    Some(cys_base.join(format!("pack-dept-{name}")))
}

/// 부서 팩 agents.json 이 시드한 계정 config dir(CLAUDE_CONFIG_DIR) — cys-dept `pack_seeded_acct`
/// 와 동일 규약(env 맵 우선 · 레거시 cmd 인라인 리터럴 폴백). None = 계정격리 미사용/판독 불가.
fn dept_seeded_acct_dir(dept_pack: &std::path::Path) -> Option<std::path::PathBuf> {
    let raw = std::fs::read_to_string(dept_pack.join("agents.json")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let claude = v.get("claude")?;
    if let Some(d) = claude
        .get("env")
        .and_then(|e| e.get("CLAUDE_CONFIG_DIR"))
        .and_then(|d| d.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(std::path::PathBuf::from(d));
    }
    let cmd = claude.get("cmd").and_then(|c| c.as_str())?;
    let i = cmd.find("CLAUDE_CONFIG_DIR=\"")? + "CLAUDE_CONFIG_DIR=\"".len();
    let rest = &cmd[i..];
    let d = &rest[..rest.find('"')?];
    if d.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(d))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// cys doctor (§3.4) — 시스템 자기진단·수리. 진단은 읽기전용, --fix는 안전 항목만
// (stale lock·고아 소켓·staging 잔재 제거 + hook 재등록). ★사용자 데이터·pack 본체·
// channels.db는 절대 삭제하지 않는다(비가역 삭제 경계).
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Clone, Copy)]
enum DiagStatus {
    Ok,
    Warn,
    Fail,
    /// **판정 불가** — 대상 부재·플랫폼 미해당·검사 도구 부재로 *검사를 수행하지 못한* 상태.
    /// ★거짓 FAIL·거짓 OK 동시 금지의 자리다: "검사하지 못했다"를 "이상 없다"(Ok)로 적으면
    /// 관측 부재가 통과로 둔갑하고, Fail 로 적으면 정상 기계에 거짓 경보가 뜬다.
    /// 종료코드는 Fail 수만 보므로 Skip 은 exit 0 을 바꾸지 않는다.
    Skip,
}

impl DiagStatus {
    fn as_str(&self) -> &'static str {
        match self {
            DiagStatus::Ok => "OK",
            DiagStatus::Warn => "WARN",
            DiagStatus::Fail => "FAIL",
            DiagStatus::Skip => "SKIP",
        }
    }
}

struct DiagItem {
    name: &'static str,
    status: DiagStatus,
    detail: String,
    /// 권고(진단 전용) 또는 --fix 시 실제 수행한 조치.
    action: String,
}

/// 진단 컨텍스트 — 실 경로(run_doctor) 또는 임시 경로(테스트)를 주입한다.
struct DoctorCtx {
    pack_dir: std::path::PathBuf,
    /// pack_dir 부모(~/.cys) — apply lock·.pack-staging 잔재 루트.
    state_base: std::path::PathBuf,
    socket_path: std::path::PathBuf,
    /// channels.db 위치(= state_dir(socket)). unix는 소켓 부모 디렉토리.
    daemon_state_dir: std::path::PathBuf,
    settings_paths: Vec<String>,
    binary_version: String,
    /// 자기 앱 번들 루트(`…/cys.app`). 번들 밖 실행(cargo run·비번들 설치)이면 None =
    /// 코드서명 봉인 검사 **판정 불가**(Skip). 테스트는 여기에 임시 번들을 주입한다.
    app_bundle: Option<std::path::PathBuf>,
}

/// settings.json 루트에 우리 SessionStart hook 명령이 등록돼 있는가.
/// ★W3(A9): 각성 집합 전체 판정은 `cys::pack::hook_registered_in`(매니페스트 소비)이 담당한다 —
/// 이 함수는 SessionStart 단일 판정의 기존 형태를 보존하는 얇은 사본이므로 신규 소비처를 만들지 마라.
#[allow(dead_code)]
fn doctor_hook_present(root: &Value, hook_cmd: &str) -> bool {
    root.get("hooks")
        .and_then(|h| h.get("SessionStart"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().any(|m| {
                m.get("hooks")
                    .and_then(|v| v.as_array())
                    .map(|hs| {
                        hs.iter()
                            .any(|h| h.get("command").and_then(|c| c.as_str()) == Some(hook_cmd))
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn diag_pack_version(ctx: &DoctorCtx) -> DiagItem {
    let vf = ctx.pack_dir.join(".pack-version");
    match std::fs::read_to_string(&vf) {
        Err(_) => DiagItem {
            name: "pack-version",
            status: DiagStatus::Warn,
            detail: "팩 미설치(.pack-version 없음)".into(),
            action: "cys init-pack 실행".into(),
        },
        Ok(s) => {
            let disk = s.trim().to_string();
            if disk == ctx.binary_version {
                return DiagItem {
                    name: "pack-version",
                    status: DiagStatus::Ok,
                    detail: format!("팩 {disk} = 바이너리 {}", ctx.binary_version),
                    action: String::new(),
                };
            }
            let note = match (
                cys::pack::parse_semver(&disk),
                cys::pack::parse_semver(&ctx.binary_version),
            ) {
                (Some(d), Some(b)) if d < b => "팩이 바이너리보다 구버전 — cys init-pack 권장",
                (Some(d), Some(b)) if d > b => "팩이 바이너리보다 신버전(바이너리 구버전) — 업데이트 권장",
                _ => "버전 파싱 불가 — 수동 확인",
            };
            DiagItem {
                name: "pack-version",
                status: DiagStatus::Warn,
                detail: format!("팩 {disk} ≠ 바이너리 {}", ctx.binary_version),
                action: note.into(),
            }
        }
    }
}

fn diag_pack_state(ctx: &DoctorCtx) -> DiagItem {
    match cys::pack::read_pack_state(&ctx.pack_dir) {
        cys::pack::PackStateRead::Absent => DiagItem {
            name: "pack-state",
            status: DiagStatus::Ok,
            detail: "채널 상태 미기록(free 기본)".into(),
            action: String::new(),
        },
        cys::pack::PackStateRead::Valid(st) => DiagItem {
            name: "pack-state",
            status: DiagStatus::Ok,
            detail: format!(
                "channel={} base={} pro_rev={}",
                st.channel, st.base_version, st.pro_revision
            ),
            action: String::new(),
        },
        cys::pack::PackStateRead::Corrupt(e) => DiagItem {
            name: "pack-state",
            status: DiagStatus::Fail,
            detail: format!(".pack-state.json 손상: {e}"),
            action: "cys pack-repair-channel (doctor는 상태파일을 자동 수정하지 않음)".into(),
        },
    }
}

fn diag_install_manifest(ctx: &DoctorCtx) -> DiagItem {
    let mf = ctx.pack_dir.join(".install-manifest.json");
    if !mf.exists() {
        let installed = ctx.pack_dir.join(".pack-version").exists();
        return if installed {
            DiagItem {
                name: "install-manifest",
                status: DiagStatus::Warn,
                detail: "설치 매니페스트 없음(구설치본) — 자동갱신·prune이 보존측으로만 동작".into(),
                action: "cys init-pack --force 로 매니페스트 재생성 권장".into(),
            }
        } else {
            DiagItem {
                name: "install-manifest",
                status: DiagStatus::Ok,
                detail: "팩 미설치(매니페스트 해당 없음)".into(),
                action: String::new(),
            }
        };
    }
    match std::fs::read_to_string(&mf)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    {
        Some(v) => {
            let n = v.as_object().map(|o| o.len()).unwrap_or(0);
            DiagItem {
                name: "install-manifest",
                status: DiagStatus::Ok,
                detail: format!("설치 매니페스트 {n}개 항목·정상 파싱"),
                action: String::new(),
            }
        }
        None => DiagItem {
            name: "install-manifest",
            status: DiagStatus::Fail,
            detail: "설치 매니페스트 파싱 실패(손상)".into(),
            action: "cys init-pack --force 로 재생성".into(),
        },
    }
}

fn diag_hook(ctx: &DoctorCtx, fix: bool) -> DiagItem {
    // ★G3 축1(리뷰 BLOCK-1 봉인 — 3층째 dept 게이트): 부서 팩(pack-dept-*) 스코프의 대조 표면은
    //   개인 프로필(~/.claude*)이 아니라 그 부서의 실소비 acctdir(CYS_ACCOUNT_DIR)다. 이 게이트가
    //   없으면 부서 컨텍스트(cys-dept <name> -- cys doctor · 부서 데몬이 띄운 pane 셸 전부)에서
    //   (a) '개인 프로필에 부서 훅 없음'을 미등록으로 오보하고(신계약상 없는 게 정상),
    //   (b) --fix 가 부서 훅을 ~/.claude* 전부에 재기록해 이 릴리스가 봉인한 결함2를 doctor 가
    //   재생산한다 — 같은 run 바로 뒤의 dept-hook-residue 가 그것을 '산 부서 오염'으로 재탐지하는
    //   자기모순(한 doctor 실행이 쓰고 지운다). 데몬 경로(merge_awakening_hooks_into_personal_profiles
    //   base-전용 게이트)·init-pack CLI 게이트와 동형이다.
    if cys::pack::dept_scope_of(&ctx.pack_dir).is_some() {
        return diag_hook_dept(ctx, fix, std::env::var("CYS_ACCOUNT_DIR").ok().as_deref());
    }
    // ★W3(A9): 진단 대상 = **소망 훅 집합 전체**(SessionStart + UserPromptSubmit). 종전엔 SessionStart
    //   하나만 봐서, 각성 훅(role-bootstrap)이 빠진 기계를 doctor 가 "OK"로 보고했다(보고≠실측).
    let missing_in = |path: &str| -> Vec<&'static str> {
        let root = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .unwrap_or_else(|| json!({}));
        cys::pack::AWAKENING_HOOKS
            .iter()
            .filter(|h| {
                !cys::pack::hook_registered_in(
                    &root,
                    h.event,
                    &cys::pack::hook_command_for(&ctx.pack_dir, h.script),
                )
            })
            .map(|h| h.script)
            .collect()
    };
    if ctx.settings_paths.iter().any(|p| missing_in(p).is_empty()) {
        return DiagItem {
            name: "hook",
            status: DiagStatus::Ok,
            detail: "각성 hook 집합 등록됨(SessionStart+UserPromptSubmit)".into(),
            action: String::new(),
        };
    }
    let missing: Vec<String> = ctx
        .settings_paths
        .iter()
        .map(|p| format!("{p}: {}", missing_in(p).join("+")))
        .collect();
    if fix {
        let mut done = 0usize;
        let mut errs: Vec<String> = Vec::new();
        for p in &ctx.settings_paths {
            if let Some(parent) = std::path::Path::new(p).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match install_claude_hook(p, &ctx.pack_dir) {
                Ok(_) => done += 1,
                Err(e) => errs.push(format!("{p}: {e}")),
            }
        }
        let status = if errs.is_empty() && done > 0 {
            DiagStatus::Ok
        } else if done > 0 {
            DiagStatus::Warn
        } else {
            DiagStatus::Fail
        };
        return DiagItem {
            name: "hook",
            status,
            detail: format!("각성 hook 미등록({}) — 재등록 시도", missing.join(" | ")),
            action: format!(
                "등록 {done}건{}",
                if errs.is_empty() {
                    String::new()
                } else {
                    format!(", 실패: {}", errs.join("; "))
                }
            ),
        };
    }
    DiagItem {
        name: "hook",
        status: DiagStatus::Warn,
        detail: format!(
            "각성 hook 미등록({}개 settings 확인) — {}",
            ctx.settings_paths.len(),
            missing.join(" | ")
        ),
        action: "cys doctor --fix 또는 cys init-pack 로 등록".into(),
    }
}

/// hook 진단의 **부서 스코프 arm**(acct env 주입형 — 전 OS 단위 테스트 가능 · 비어있지 않음
/// 필터는 `pack::config_dir_for` 와 동일 규약). 대조·--fix 기록 표면은 **acctdir 하나**다 —
/// 개인 프로필(~/.claude*)은 읽지도 쓰지도 않는다(공용 프로필 무변조 기본 계약).
fn diag_hook_dept(ctx: &DoctorCtx, fix: bool, acct_env: Option<&str>) -> DiagItem {
    let Some(acct) = acct_env.filter(|s| !s.is_empty()) else {
        // 시드 생략 상태(setup_isolated_config_dir 의 loud WARN 과 같은 셀) — 시드 표적이 없으므로
        // --fix 도 아무 것도 쓰지 않는다(fail-closed · 개인 프로필/공용 claude 폴백 금지).
        return DiagItem {
            name: "hook",
            status: DiagStatus::Warn,
            detail: format!(
                "부서 팩({}) 스코프인데 CYS_ACCOUNT_DIR 미설정 — 시드 표적 없음(시드 생략 상태·\
                 이상 기동 신호). 개인 프로필(~/.claude*)은 신계약상 무접촉이라 대조하지 않는다",
                ctx.pack_dir.display()
            ),
            action: "cys-dept <name> launch/rotate 로 계정 dir 주입 후 재진단 — 이 상태의 --fix 는 \
                     아무 것도 쓰지 않는다(fail-closed)"
                .into(),
        };
    };
    let settings = std::path::Path::new(acct).join("settings.json");
    let missing = cys::pack::verify_desired_hooks_registered(
        &settings,
        &ctx.pack_dir,
        &cys::pack::AWAKENING_HOOKS,
    );
    if missing.is_empty() {
        return DiagItem {
            name: "hook",
            status: DiagStatus::Ok,
            detail: format!(
                "부서 acctdir({acct}) 각성 hook 집합 등록됨(SessionStart+UserPromptSubmit)"
            ),
            action: String::new(),
        };
    }
    if fix {
        if let Some(parent) = settings.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // G16 3-writer 직렬화 — preflight C28·hooks-prune 과 같은 락 파일(<settings>.cys-lock).
        let _lock = acquire_settings_lock(&settings);
        return match install_claude_hook(&settings.to_string_lossy(), &ctx.pack_dir) {
            Ok(_) => DiagItem {
                name: "hook",
                status: DiagStatus::Ok,
                detail: format!("부서 acctdir 각성 hook 미등록({}) — 재등록", missing.join("+")),
                action: format!("등록: {}", settings.display()),
            },
            Err(e) => DiagItem {
                name: "hook",
                status: DiagStatus::Fail,
                detail: format!(
                    "부서 acctdir 각성 hook 재등록 실패: {}: {e}",
                    settings.display()
                ),
                action: "부서 재기동(cys-dept launch/rotate)이 부트 자동설치로 재시드한다".into(),
            },
        };
    }
    DiagItem {
        name: "hook",
        status: DiagStatus::Warn,
        detail: format!(
            "부서 acctdir({}) 각성 hook 미등록 — {}",
            settings.display(),
            missing.join("+")
        ),
        action: "cys doctor --fix(부서 acctdir 에만 기록) 또는 부서 재기동(cys-dept launch/rotate)"
            .into(),
    }
}

/// dept-hook-residue(G3 축1 마이그레이션 진단) — 공용 격리 config(~/.cys/claude)·개인 프로필
/// (~/.claude*) settings 에 **부서 팩(pack-dept-*) 경로 훅**이 남아 있는가.
///
///  · 죽은 경로(팩 dir 부재) = **FAIL** — 그 훅은 매 Claude 세션 "No such file" 실패 벡터다.
///    --fix: 무조건 제거.
///  · 산 경로(부서 실재 = 공용 오염) = **WARN** — 신계약(부서 훅은 자기 acctdir 시드)상 오염이나,
///    --fix 는 **그 부서 acctdir 에 각성 훅 시드가 실측 확인된 경우에만** 제거한다
///    (`verify_desired_hooks_registered` 빈 벡터 — 실측 없는 제거는 부서 각성 공백 창을 연다:
///    절대 불변 앵커 ③④ '조용한 광역 회귀 금지'). 미확인이면 보존 + 부서 rotate 안내.
///  · 제거 엔진은 hooks-prune 와 **동일 함수**(`strip_hooks_pointing_into_pack`) — 제거 로직
///    이원화 금지(제거 엔진 단일 · dept_scope_of 술어 통일표 참조).
///  · 판정 불능(파싱 실패)은 통과가 아니다 — WARN 이상으로 가시화한다.
fn diag_dept_hook_residue(ctx: &DoctorCtx, fix: bool) -> DiagItem {
    let mut targets: Vec<std::path::PathBuf> =
        vec![ctx.state_base.join("claude").join("settings.json")];
    for p in &ctx.settings_paths {
        let pb = std::path::PathBuf::from(p);
        if !targets.contains(&pb) {
            targets.push(pb);
        }
    }
    let mut unreadable: Vec<String> = Vec::new();
    // (대상 settings, 부서 팩, 훅 수) — 판정·조치의 단위.
    let mut residues: Vec<(std::path::PathBuf, std::path::PathBuf, usize)> = Vec::new();
    for t in &targets {
        let raw = match std::fs::read_to_string(t) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                unreadable.push(format!("{}: {e}", t.display()));
                continue;
            }
        };
        let root: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                unreadable.push(format!("{}: parse error: {e}", t.display()));
                continue;
            }
        };
        let Some(hooks) = root.get("hooks").and_then(|h| h.as_object()) else {
            continue;
        };
        for h in hooks
            .values()
            .filter_map(|v| v.as_array())
            .flatten()
            .filter_map(|e| e.get("hooks").and_then(|v| v.as_array()))
            .flatten()
        {
            let Some(cmd) = h.get("command").and_then(|c| c.as_str()) else {
                continue;
            };
            let Some(pack) = dept_pack_of_command(cmd, &ctx.state_base) else {
                continue;
            };
            match residues.iter_mut().find(|(rt, rp, _)| rt == t && rp == &pack) {
                Some((_, _, n)) => *n += 1,
                None => residues.push((t.clone(), pack, 1)),
            }
        }
    }
    if residues.is_empty() && unreadable.is_empty() {
        return DiagItem {
            name: "dept-hook-residue",
            status: DiagStatus::Ok,
            detail: format!("공용·개인 settings {}개에 부서 훅 잔존 0", targets.len()),
            action: String::new(),
        };
    }
    let dead_cnt = residues.iter().filter(|(_, p, _)| !p.is_dir()).count();
    if !fix {
        let listing: Vec<String> = residues
            .iter()
            .map(|(t, p, n)| {
                format!(
                    "{}←{}({})×{n}",
                    t.display(),
                    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                    if p.is_dir() { "산 부서 오염" } else { "죽은 경로" }
                )
            })
            .chain(unreadable.iter().map(|u| format!("판정 불능 {u}")))
            .collect();
        return DiagItem {
            name: "dept-hook-residue",
            status: if dead_cnt > 0 { DiagStatus::Fail } else { DiagStatus::Warn },
            detail: format!("부서 훅 잔존 탐지: {}", listing.join(" | ")),
            action: "cys doctor --fix(죽은 경로 무조건 · 산 부서는 acctdir 시드 실측 확인 시만) \
                     또는 cys hooks-prune --pack-dir <부서 팩>"
                .into(),
        };
    }
    let mut removed: Vec<String> = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    let mut errs: Vec<String> = unreadable.clone();
    for (t, pack, _) in &residues {
        let live = pack.is_dir();
        if live {
            // 산 부서: acctdir 시드 실측(있다고 주장이 아니라 디스크 실측 — CS-3) 후에만 제거.
            let seeded_ok = dept_seeded_acct_dir(pack)
                .map(|a| {
                    cys::pack::verify_desired_hooks_registered(
                        &a.join("settings.json"),
                        pack,
                        &cys::pack::AWAKENING_HOOKS,
                    )
                    .is_empty()
                })
                .unwrap_or(false);
            if !seeded_ok {
                kept.push(format!(
                    "{}←{}(acctdir 시드 미확인 — 보존)",
                    t.display(),
                    pack.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                ));
                continue;
            }
        }
        let _lock = acquire_settings_lock(t);
        match cys::factory_reset::strip_hooks_pointing_into_pack(t, pack, None) {
            Ok(labels) => removed.push(format!("{}: {}", t.display(), labels.join("·"))),
            Err(e) => errs.push(format!("{}: {e}", t.display())),
        }
    }
    let status = if !errs.is_empty() {
        DiagStatus::Fail
    } else if !kept.is_empty() {
        DiagStatus::Warn
    } else {
        DiagStatus::Ok
    };
    DiagItem {
        name: "dept-hook-residue",
        status,
        detail: format!(
            "제거 {}건{}{}",
            removed.len(),
            if kept.is_empty() { String::new() } else { format!(" · 보존 {}", kept.join(", ")) },
            if errs.is_empty() { String::new() } else { format!(" · 실패 {}", errs.join("; ")) }
        ),
        action: if kept.is_empty() {
            "제거 완료(백업: .bak-cys-dept) — ★claude 재시작 후 적용".into()
        } else {
            "보존된 산 부서 훅은 부서 rotate(acctdir 재시드) 후 doctor --fix 재실행".into()
        },
    }
}

/// dept-awakening-seed(G3 축1 확정 결정 3종 세트의 doctor anomaly 항목 · 리뷰 BLOCK-2) —
/// 등록된 부서 팩(pack-dept-*) 각각의 **acctdir 각성 훅 시드를 실측**한다. 시드 생략 loud WARN
/// (pack.rs setup_isolated_config_dir)의 '진단: cys doctor' 포인터가 실제로 보여주는 항목.
///
///  · dept-hook-residue(공용/개인 settings 의 '잔존 훅' 탐지)와 **별개 축**: 잔존이 0인 신규/
///    레거시 부서가 CYS_ACCOUNT_DIR 없이 무각성 부팅한 상태('계속 실수·실패 보고' 셀 — 치명위험
///    앵커 ③ 인접)는 잔존 탐지로는 영원히 보이지 않는다 — 여기서만 잡힌다.
///  · 읽기 전용(--fix 없음): 시드는 부서 부트 자동설치(cys-dept launch/rotate →
///    setup_isolated_config_dir) 소관 — base 레인 doctor 가 타 레인 acctdir 에 쓰기 시작하면 새
///    교차-레인 쓰기 표면이 열린다. 부서 레인 안에서의 시드 수리는 `cys-dept <name> -- cys doctor
///    --fix`(= hook 진단 부서 arm) 소관.
///  · 판정 소스 = agents.json 시드값(`dept_seeded_acct_dir` — cys-dept 3순위 유도의 영속 정본)
///    이며, residue --fix 의 조건부 제거와 **같은 술어**(`verify_desired_hooks_registered`)를
///    쓴다(두 항목의 보고가 갈리지 않는다). 부서 판정은 `dept_scope_of` 단일 술어.
fn diag_dept_awakening_seed(ctx: &DoctorCtx) -> DiagItem {
    let mut packs: Vec<std::path::PathBuf> = std::fs::read_dir(&ctx.state_base)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && cys::pack::dept_scope_of(p).is_some())
        .collect();
    packs.sort();
    if packs.is_empty() {
        return DiagItem {
            name: "dept-awakening-seed",
            status: DiagStatus::Ok,
            detail: "부서 팩 0 — 해당 없음".into(),
            action: String::new(),
        };
    }
    let mut anomalies: Vec<String> = Vec::new();
    for pack in &packs {
        let name = pack
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        match dept_seeded_acct_dir(pack) {
            None => anomalies.push(format!(
                "{name}: 계정 dir 미시드(agents.json CLAUDE_CONFIG_DIR 부재·판독 불가) — \
                 CYS_ACCOUNT_DIR 미주입 부팅은 시드 생략=무각성"
            )),
            Some(acct) => {
                let missing = cys::pack::verify_desired_hooks_registered(
                    &acct.join("settings.json"),
                    pack,
                    &cys::pack::AWAKENING_HOOKS,
                );
                if !missing.is_empty() {
                    anomalies.push(format!(
                        "{name}: acctdir({}) 각성 훅 미시드 — {}",
                        acct.display(),
                        missing.join("+")
                    ));
                }
            }
        }
    }
    if anomalies.is_empty() {
        return DiagItem {
            name: "dept-awakening-seed",
            status: DiagStatus::Ok,
            detail: format!("부서 팩 {}개 acctdir 각성 훅 실측 — anomaly 0", packs.len()),
            action: String::new(),
        };
    }
    DiagItem {
        name: "dept-awakening-seed",
        status: DiagStatus::Warn,
        detail: format!("부서 각성 시드 anomaly: {}", anomalies.join(" | ")),
        action: "해당 부서 재기동(cys-dept <name> launch/rotate)이 acctdir 에 재시드한다 · \
                 부서 레인 수리: cys-dept <name> -- cys doctor --fix"
            .into(),
    }
}

/// ★★M5(2026-08-24 자기성찰 3회전) — 각성 훅 **설치 표적 ≠ 실소비 SOT** doctor 항목.
///
/// `pack::config_dir()`(설치 표적 — base 레인에서 `pack.parent()/claude`)와
/// `resolve_claude_config_dir()`(에이전트가 실제로 읽는 `${CYS_ACCOUNT_DIR:-$HOME/.cys/claude}`)를
/// 대조한다. 두 값은 팩이 `~/.cys/pack` 일 때만 **우연히** 일치한다.
///
/// 어긋나면 각성 훅이 아무도 읽지 않는 폴더에 설치되고, 노드는 떠도 `/clear` 후 지침 재주입·
/// 마스터 선언 부트 발화가 영구히 죽는다(팀 미기동). 회전2 격리 주행에서 처방된 `cys init-pack`·
/// `javis_preflight.py --fix` 를 **완주시켰는데도 같은 경고가 재현**된 원인이 이것이다.
///
/// **읽기 전용(--fix 없음)**: 표적 통일은 부서 레인·기존 설치본의 훅 잔존까지 건드리는 반경이라
/// 태그 전 자동 수리 대상이 아니다. 여기서는 어긋남을 **보이게** 만들고 처방을 정직하게 적는다.
/// (`dept-awakening-seed` 와 별개 축: 그쪽은 **부서 팩**의 acctdir 시드 실측이고, 이쪽은
///  **이 레인 자신**의 설치 표적과 실소비의 어긋남이다 — 부서 팩이 0개인 base 기계에서
///  그 항목은 항상 Ok 라 이 결함을 영원히 못 본다.)
fn diag_config_dir_target(ctx: &DoctorCtx) -> DiagItem {
    let install = cys::pack::config_dir_for(
        cys::env_compat(cys::pack::ENV_CONFIG_DIR).as_deref(),
        cys::pack::dept_scope_of(&ctx.pack_dir).as_deref(),
        std::env::var("CYS_ACCOUNT_DIR").ok().as_deref(),
        &ctx.pack_dir,
    );
    let consumed = cys::resolve_claude_config_dir();
    let Some(install) = install else {
        return DiagItem {
            name: "config-dir-target",
            status: DiagStatus::Skip,
            detail: format!(
                "부서 팩({}) + CYS_ACCOUNT_DIR 미설정 = 설치 표적 없음 — 대조 불가(별도 항목 \
                 dept-awakening-seed 가 받는다)",
                ctx.pack_dir.display()
            ),
            action: "부서 재기동(cys-dept <name> launch/rotate)이 계정 dir 를 주입한다".into(),
        };
    };
    match cys::pack::config_target_mismatch(Some(&install), std::path::Path::new(&consumed)) {
        None => DiagItem {
            name: "config-dir-target",
            status: DiagStatus::Ok,
            detail: format!("각성 훅 설치 표적 = 실소비 dir ({consumed})"),
            action: String::new(),
        },
        Some((install, consumed)) => DiagItem {
            name: "config-dir-target",
            status: DiagStatus::Fail,
            detail: format!(
                "각성 훅이 아무도 읽지 않는 폴더에 설치된다 — 설치 표적 {} ≠ 노드 실소비 {} \
                 (실소비 SOT = ${{CYS_ACCOUNT_DIR:-$HOME/.cys/claude}} · 설치 표적은 팩 위치 파생). \
                 귀결: 노드는 떠도 /clear 후 지침 재주입(SessionStart)·마스터 선언 부트 발화\
                 (UserPromptSubmit)가 발동하지 않는다(팀 미기동).",
                install.display(),
                consumed.display()
            ),
            action: "★`cys init-pack` 과 `javis_preflight.py --fix` 로는 해소되지 않는다(두 명령은 \
                     설치 표적에 쓴다). CYS_ACCOUNT_DIR 을 설치 표적과 같은 값으로 맞추거나 팩을 \
                     ~/.cys/pack 으로 되돌린 뒤 그 두 명령 중 하나를 실행하라 — 자동 수리 대상 아님."
                .into(),
        },
    }
}

#[cfg(unix)]
fn doctor_socket_connectable(p: &std::path::Path) -> bool {
    std::os::unix::net::UnixStream::connect(p).is_ok()
}

// ─────────────────────── WS-7: doctor 소켓·락 진단 (flock 스팬 · fail-closed) ───────────────────────
//
// 두 진단(diag_orphan_socket → diag_stale_lock)은 run_doctor_diagnostics에서 **연속 실행**되고 둘 다
// startup flock을 만진다. 부팅 중인 데몬의 acquire_startup_lock이 그 순간 실패하면 데드맨이
// `dead-holder-reclaim-failed`라는 **오사유로 exit(1)** 시키므로, 여기서는 보유 구간을 최소화하고
// (블로킹 connect는 스팬 밖) 데몬 쪽은 지수 백오프 재시도로 흡수한다(cysd/main.rs acquire_startup_lock).

/// doctor가 flock을 쥔 채 머무는 인위적 시간 — **테스트 전용 노브**(기본 0). 통합 테스트가 벽시계
/// 경합에 기대지 않고 "doctor가 락을 쥔 순간 부팅 데몬이 재시도로 이긴다"를 결정론으로 재현한다.
fn doctor_lock_hold() -> std::time::Duration {
    std::env::var("CYS_DOCTOR_LOCK_HOLD_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or_default()
}

/// 락 파일의 홀더 pid — cysd `deadman::read_holder_pid`와 **동일 규약**(빈 파일·0·파싱 실패 = None).
/// None은 구형 락파일을 뜻하며 데드맨의 `FailClosed`와 같은 보수 해석(어떤 제거도 금지)을 받는다.
#[cfg(unix)]
fn doctor_read_holder_pid(lock: &std::path::Path) -> Option<u32> {
    let s = std::fs::read_to_string(lock).ok()?;
    match s.trim().parse::<u32>().ok()? {
        0 => None,
        pid => Some(pid),
    }
}

#[cfg(unix)]
fn doctor_pid_alive(pid: u32) -> bool {
    pid != 0 && unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// pid의 프로세스명이 정확히 `cysd`인가 — **sysinfo 1회 스냅샷**(단일 pid 대상).
/// `ps` fork는 고부하에서 50~150ms가 걸려 락 보유 상한을 깬다(cysd/deadman.rs와 동일 교체).
#[cfg(unix)]
fn doctor_pid_is_cysd(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
        true,
    );
    sys.process(sysinfo::Pid::from_u32(pid))
        .map(|p| {
            p.name()
                .to_string_lossy()
                .rsplit('/')
                .next()
                .map(|b| b == "cysd")
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// 고아 소켓 제거 판정(순수 함수 — 진리표 테스트 가능). 삭제는 "살아있는 cysd 홀더가 없다"의
/// **3중 부정**이 전부 성립할 때만: ①flock 홀더 부재(=락 획득 성공 또는 락파일 ENOENT)
/// ②기록된 홀더 pid가 사망 ③(pid 재사용 방어) 그 pid가 cysd가 아님. 하나라도 어긋나면 보류한다.
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum OrphanVerdict {
    /// 삭제 가능 — 살아있는 홀더 없음이 3중으로 확인됨.
    Removable,
    /// flock 획득 실패 = 데몬이 **부팅 중이거나 보유 중** — 판정 보류·삭제 금지(fail-closed).
    /// 이 분기를 미정의로 두면 산 소켓을 지우는 영구 장애 경로가 재현된다.
    HeldByDaemon,
    /// 구형 락파일(holder pid 미기재) — 데드맨 `FailClosed`와 동일한 보수 해석으로 삭제 금지.
    UnknownHolder,
    /// 기록된 홀더 pid가 살아있는 cysd — 삭제 금지.
    LiveHolder(u32),
}

#[cfg(unix)]
fn judge_orphan_socket(
    lock_exists: bool,
    flock_acquired: bool,
    holder_pid: Option<u32>,
    pid_alive: bool,
    pid_is_cysd: bool,
) -> OrphanVerdict {
    if !lock_exists {
        // 락 파일 ENOENT = 홀더 없음으로 진행. (미정의로 두면 --fix가 영구 무력해진다.)
        return OrphanVerdict::Removable;
    }
    if !flock_acquired {
        return OrphanVerdict::HeldByDaemon;
    }
    match holder_pid {
        None => OrphanVerdict::UnknownHolder,
        Some(pid) if pid_alive && pid_is_cysd => OrphanVerdict::LiveHolder(pid),
        Some(pid) if pid_alive => OrphanVerdict::LiveHolder(pid), // 정체 불명 생존 pid = 보수적 보류
        Some(_) => OrphanVerdict::Removable,
    }
}

#[cfg(unix)]
fn diag_orphan_socket(ctx: &DoctorCtx, fix: bool) -> DiagItem {
    use std::os::unix::io::AsRawFd;
    let sp = &ctx.socket_path;
    if !sp.exists() {
        return DiagItem {
            name: "socket",
            status: DiagStatus::Ok,
            detail: "소켓 파일 없음(데몬 미가동)".into(),
            action: String::new(),
        };
    }
    // ★블로킹 connect(타임아웃 없음)는 반드시 **락 스팬 밖**에서 — 안에서 하면 보유 상한을 깬다.
    if doctor_socket_connectable(sp) {
        return DiagItem {
            name: "socket",
            status: DiagStatus::Ok,
            detail: "데몬 소켓 연결 가능(가동 중)".into(),
            action: String::new(),
        };
    }
    // 존재하나 연결 불가 = 고아 **후보**. 여기부터 flock 스팬 — check→unlink 전 구간을 보유해
    // "판정 후 데몬이 부팅해 bind" TOCTOU를 봉합한다(락 핸들 drop = flock 해제이므로 조기 drop 금지).
    let lock = sp.with_extension("lock");
    let lock_exists = lock.exists();
    let mut _guard: Option<std::fs::File> = None;
    let mut flock_acquired = true;
    if lock_exists {
        match std::fs::OpenOptions::new().read(true).open(&lock) {
            Ok(f) => {
                flock_acquired =
                    unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
                if flock_acquired {
                    _guard = Some(f);
                }
            }
            Err(e) => {
                return DiagItem {
                    name: "socket",
                    status: DiagStatus::Warn,
                    detail: format!("고아 후보 소켓 — 시작 락 열기 실패로 판정 보류: {e}"),
                    action: "수동 확인(삭제 금지)".into(),
                }
            }
        }
    }
    let holder_pid = if lock_exists {
        doctor_read_holder_pid(&lock)
    } else {
        None
    };
    let (alive, is_cysd) = match holder_pid {
        Some(p) => (doctor_pid_alive(p), doctor_pid_is_cysd(p)),
        None => (false, false),
    };
    let verdict = judge_orphan_socket(lock_exists, flock_acquired, holder_pid, alive, is_cysd);
    std::thread::sleep(doctor_lock_hold()); // 테스트 노브(기본 0) — 부팅측 재시도 흡수 검증용.

    let item = match verdict {
        OrphanVerdict::HeldByDaemon => DiagItem {
            name: "socket",
            status: DiagStatus::Warn,
            detail: "고아 후보 소켓 — 시작 락을 누군가 보유 중(데몬 부팅/보유) → 판정 보류".into(),
            action: "삭제 금지(잠시 후 재실행)".into(),
        },
        OrphanVerdict::UnknownHolder => DiagItem {
            name: "socket",
            status: DiagStatus::Warn,
            detail: "고아 후보 소켓 — 락파일에 홀더 pid 미기재(구형) → 보수적 보류".into(),
            action: "삭제 금지(수동 확인)".into(),
        },
        OrphanVerdict::LiveHolder(pid) => DiagItem {
            name: "socket",
            status: DiagStatus::Warn,
            detail: format!("고아 후보 소켓 — 홀더 pid {pid} 생존 → 보류"),
            action: "삭제 금지(수동 확인)".into(),
        },
        OrphanVerdict::Removable if fix => match std::fs::remove_file(sp) {
            Ok(()) => DiagItem {
                name: "socket",
                status: DiagStatus::Ok,
                detail: "고아 소켓 제거(홀더 부재 3중 확인)".into(),
                action: "삭제함".into(),
            },
            Err(e) => DiagItem {
                name: "socket",
                status: DiagStatus::Warn,
                detail: format!("고아 소켓 제거 실패: {e}"),
                action: "수동 삭제 필요".into(),
            },
        },
        OrphanVerdict::Removable => DiagItem {
            name: "socket",
            status: DiagStatus::Warn,
            detail: "고아 소켓(리스너 없음·홀더 부재)".into(),
            action: "cys doctor --fix 로 제거".into(),
        },
    };
    drop(_guard); // 여기서 flock 해제 — 판정~삭제 전 구간을 보유했다.
    item
}

#[cfg(not(unix))]
fn diag_orphan_socket(_ctx: &DoctorCtx, _fix: bool) -> DiagItem {
    DiagItem {
        name: "socket",
        status: DiagStatus::Ok,
        detail: "소켓 진단은 unix 전용(skip)".into(),
        action: String::new(),
    }
}

/// ★K4(CRITICAL): 이 진단은 **락 파일을 절대 unlink 하지 않는다**.
///
/// flock은 프로세스가 죽으면 커널이 자동 해제하므로 "잔여 락"이라는 개념 자체가 성립하지 않는다.
/// 반대로 락파일을 unlink 하면 상호배제가 **영구 무효화**된다: 부팅 데몬은 unlink된 inode에 락을
/// 잡고, 그 다음 데몬은 새로 생성된 별개 inode에 락을 잡아 둘 다 승자가 된다. 게다가 가시 락파일에
/// holder pid가 없어져 데드맨(cysd/deadman.rs)이 영구 무장해제된다.
/// → 기본은 **읽기 전용 보고**, `--fix`는 stale pid 문자열 truncate까지만(락 보유 중이므로 홀더
///   부재가 확정된 상태 — 파일 자체와 inode는 보존한다).
#[cfg(unix)]
fn diag_stale_lock(ctx: &DoctorCtx, fix: bool) -> DiagItem {
    use std::os::unix::io::AsRawFd;
    // 데몬 시작 락 = socket_path.with_extension("lock") (main.rs 부트락과 동일 규약·읽기 전용 참조).
    let lock = ctx.socket_path.with_extension("lock");
    if !lock.exists() {
        return DiagItem {
            name: "startup-lock",
            status: DiagStatus::Ok,
            detail: "시작 락 없음".into(),
            action: String::new(),
        };
    }
    let f = match std::fs::OpenOptions::new().read(true).write(true).open(&lock) {
        Ok(f) => f,
        Err(e) => {
            return DiagItem {
                name: "startup-lock",
                status: DiagStatus::Warn,
                detail: format!("시작 락 열기 실패(보수적 유지): {e}"),
                action: "수동 확인".into(),
            }
        }
    };
    // 비차단 획득 시도: 획득되면 아무도 안 쥔 상태, 실패면 데몬 보유(정상). fd를 쥔 채 판정·기록해
    // 진단↔기록 사이 데몬 재기동 레이스를 차단한다.
    let got = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    if !got {
        return DiagItem {
            name: "startup-lock",
            status: DiagStatus::Ok,
            detail: "시작 락 활성(데몬 보유)".into(),
            action: String::new(),
        };
    }
    std::thread::sleep(doctor_lock_hold()); // 테스트 노브(기본 0).
    let stale_pid = doctor_read_holder_pid(&lock);
    let item = match (stale_pid, fix) {
        (None, _) => DiagItem {
            name: "startup-lock",
            status: DiagStatus::Ok,
            detail: "시작 락 유휴(홀더 없음·기록된 pid 없음)".into(),
            action: String::new(),
        },
        (Some(pid), false) => DiagItem {
            name: "startup-lock",
            status: DiagStatus::Warn,
            detail: format!("시작 락 유휴이나 stale holder pid {pid} 기록 잔존"),
            action: "cys doctor --fix 로 pid 표기만 정리(락 파일은 보존)".into(),
        },
        (Some(pid), true) => match f.set_len(0) {
            Ok(()) => DiagItem {
                name: "startup-lock",
                status: DiagStatus::Ok,
                detail: format!("stale holder pid {pid} 표기 정리(락 파일·inode 보존)"),
                action: "pid 표기 truncate".into(),
            },
            Err(e) => DiagItem {
                name: "startup-lock",
                status: DiagStatus::Warn,
                detail: format!("stale pid 표기 정리 실패: {e}"),
                action: "수동 확인(락 파일 삭제 금지)".into(),
            },
        },
    };
    unsafe {
        libc::flock(f.as_raw_fd(), libc::LOCK_UN);
    }
    item
}

#[cfg(not(unix))]
fn diag_stale_lock(_ctx: &DoctorCtx, _fix: bool) -> DiagItem {
    DiagItem {
        name: "startup-lock",
        status: DiagStatus::Ok,
        detail: "락 진단은 unix 전용(skip)".into(),
        action: String::new(),
    }
}

/// L5 진행중 staging 보호 임계(초) — 이 시간 내 수정된 staging은 doctor --fix가 삭제하지 않는다.
/// 기본 60초·env override(테스트는 0으로 보호 해제). 0이면 항상 삭제(보호 off).
fn staging_protect_secs() -> u64 {
    std::env::var("CYS_DOCTOR_STAGING_MIN_IDLE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60)
}

/// staging 디렉토리(자신+직속 엔트리)의 최신 수정 후 경과 초(L5 진행중 보호용). 실패 시 None.
fn staging_idle_secs(path: &std::path::Path) -> Option<u64> {
    let mut newest = std::fs::metadata(path).ok()?.modified().ok()?;
    if let Ok(rd) = std::fs::read_dir(path) {
        for e in rd.flatten() {
            if let Ok(mt) = e.metadata().and_then(|m| m.modified()) {
                if mt > newest {
                    newest = mt;
                }
            }
        }
    }
    newest.elapsed().ok().map(|d| d.as_secs())
}

fn diag_staging_residue(ctx: &DoctorCtx, fix: bool) -> DiagItem {
    let mut residue: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&ctx.state_base) {
        for e in rd.flatten() {
            if let Some(name) = e.file_name().to_str() {
                // .pack-staging(pack-update)·.pack-staging-init-<pid>(init-pack) 잔재만.
                // pack.prev(1세대 롤백)는 이름이 다르므로 건드리지 않는다.
                if name.starts_with(".pack-staging") {
                    residue.push(e.path());
                }
            }
        }
    }
    if residue.is_empty() {
        return DiagItem {
            name: "staging-residue",
            status: DiagStatus::Ok,
            detail: "staging 잔재 없음".into(),
            action: String::new(),
        };
    }
    if fix {
        let mut removed = 0usize;
        let mut fail = 0usize;
        let mut skipped = 0usize;
        for p in &residue {
            // L5: 진행중(최근 N초 내 수정) staging은 삭제하지 않는다 — 무중단 배포/init 도중
            // 스테이징을 파괴해 배포를 깨는 것을 방지(mtime 미상=보수적으로 삭제 진행).
            let protect = staging_protect_secs();
            if protect > 0 && staging_idle_secs(p).map(|s| s < protect).unwrap_or(false) {
                skipped += 1;
                continue;
            }
            if std::fs::remove_dir_all(p).is_ok() {
                removed += 1;
            } else {
                fail += 1;
            }
        }
        DiagItem {
            name: "staging-residue",
            status: if fail == 0 && skipped == 0 {
                DiagStatus::Ok
            } else {
                DiagStatus::Warn
            },
            detail: format!("staging 잔재 {}건", residue.len()),
            action: format!(
                "{removed}건 정리{}{}",
                if skipped > 0 {
                    format!(", {skipped}건 진행중 보호")
                } else {
                    String::new()
                },
                if fail > 0 {
                    format!(", {fail}건 실패")
                } else {
                    String::new()
                }
            ),
        }
    } else {
        DiagItem {
            name: "staging-residue",
            status: DiagStatus::Warn,
            detail: format!("staging 잔재 {}건", residue.len()),
            action: "cys doctor --fix 로 정리".into(),
        }
    }
}

fn diag_channels_db(ctx: &DoctorCtx) -> DiagItem {
    let db = ctx.daemon_state_dir.join("channels.db");
    if !db.exists() {
        return DiagItem {
            name: "channels-db",
            status: DiagStatus::Ok,
            detail: "채널 DB 없음(온디맨드 생성)".into(),
            action: String::new(),
        };
    }
    match rusqlite::Connection::open(&db) {
        Ok(conn) => {
            // 유효 SQLite 파일인지 먼저 확인 — garbage 파일은 sqlite_master 접근에서 NotADatabase.
            if conn
                .query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
                .is_err()
            {
                return DiagItem {
                    name: "channels-db",
                    status: DiagStatus::Fail,
                    detail: "채널 DB가 유효한 SQLite가 아님(손상 가능)".into(),
                    action: "수동 점검(doctor는 DB를 삭제하지 않음)".into(),
                };
            }
            let sv: rusqlite::Result<String> = conn.query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |r| r.get(0),
            );
            match sv {
                // pid 생존 프로브는 전 OS 실측(cysd state::pid_alive — windows 는 OpenProcess+
                // WaitForSingleObject)이라 status alive·자가치유(죽은 브리지 재스폰)가 OS 무관
                // 동일 계약이다 — windows 별도 경고 없음.
                Ok(v) => DiagItem {
                    name: "channels-db",
                    status: DiagStatus::Ok,
                    detail: format!("채널 DB 정상·schema_version={v}"),
                    action: String::new(),
                },
                Err(_) => DiagItem {
                    name: "channels-db",
                    status: DiagStatus::Warn,
                    detail: "채널 DB 열림·schema_version 없음(구 스키마?)".into(),
                    action: "데몬 기동 시 마이그레이션 확인".into(),
                },
            }
        }
        Err(e) => DiagItem {
            name: "channels-db",
            status: DiagStatus::Fail,
            detail: format!("채널 DB 열기 실패(손상 가능): {e}"),
            action: "수동 점검(doctor는 DB를 삭제하지 않음)".into(),
        },
    }
}

fn diag_legacy_config(_ctx: &DoctorCtx) -> DiagItem {
    // 이 시스템의 config는 env 기반(온디스크 canonical config 파일 없음). 레거시 env 키 사용을
    // 진단한다(런타임은 canonical CYS_* 우선). 백업·재작성은 대상 파일이 없어 해당 없음(진단만).
    let legacy_keys = [
        "JAVIS_PACK_DIR",
        "AITERM_JARVIS_DIR",
        "AITERM_PACK_DIR",
        "JAVIS_SOCKET",
        "AITERM_SOCKET",
    ];
    let set: Vec<&str> = legacy_keys
        .iter()
        .copied()
        .filter(|k| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false))
        .collect();
    if set.is_empty() {
        DiagItem {
            name: "legacy-config",
            status: DiagStatus::Ok,
            detail: "레거시 env 미사용(canonical CYS_*)".into(),
            action: String::new(),
        }
    } else {
        DiagItem {
            name: "legacy-config",
            status: DiagStatus::Warn,
            detail: format!("레거시 env 사용: {}", set.join(", ")),
            action: "CYS_* 키로 이관 권장(런타임은 canonical 우선)".into(),
        }
    }
}

// ───────────────────── M3: 앱 번들 코드서명 봉인 자가진단 (app-seal) ─────────────────────
//
// ★실사고 2026-08-01(근본원인 확정): 번들 안 Python 런타임이 **실행 중**
//   `Contents/Resources/runtime/python/lib/python3.12/**/__pycache__/*.pyc` 를 번들 *안에*
//   생성한다. 그 순간 codesign 봉인(sealed resources)이 깨진다 —
//   "a sealed resource is missing or invalid / file added: …/_compression.cpython-312.pyc".
//   로컬에서는 이미 실행 중인 앱이라 아무 증상이 없다(무증상 보균). 그러나 이 상태의 번들을
//   사용자가 **브라우저로 받아** 설치하면 quarantine 이 붙고, 첫 실행 시 Gatekeeper 가 번들
//   **전체를 재검증**해 "손상되었기 때문에 열 수 없습니다"로 차단한다. 공증·staple 은 정상인데도.
//   릴리스 검증이 curl 사본(quarantine 없음)만 봐서 이 경로를 한 번도 재현하지 못했다.
//
// 그래서 이 진단의 임무는 "고치는 것"이 아니라 **정직하게 알리는 것**이다: 이 기계의 설치본이
// 이미 봉인을 깼는지, 깼다면 어떤 파일 때문인지, 그리고 어떻게 복구하는지.
// ★doctor --fix 는 이 항목을 절대 자동 수정하지 않는다 — 번들 안 파일을 지우는 "부분 수리"는
//   ⓐ App Management(TCC) 보호에 막히고 ⓑ 지운 파일이 원래 봉인에 있던 것이면 added 가
//   missing 으로 바뀔 뿐 봉인은 여전히 깨진 채다. 유일한 복구는 **번들 통째 교체**다.

/// 실행 파일 경로에서 자기 앱 번들 루트(`…/*.app`)를 찾는다. macOS 번들 레이아웃
/// `X.app/Contents/MacOS/<exe>` 를 조상 방향으로 거슬러 올라가되, `Contents/Info.plist`
/// 존재로 **진짜 번들임을 확증**한다(이름만 `.app` 인 디렉토리에 속지 않는다).
/// 번들 밖 실행(cargo run·비번들 설치)이면 None → 호출부가 Skip 으로 강등한다.
/// ★심링크: `current_exe()` 는 이미 realpath 라 `/usr/local/bin/cys → 번들 안 실체`로 불러도
///   번들이 정상 탐지된다(심링크 경로를 그대로 쓰면 탐지 실패했을 자리).
fn detect_app_bundle(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    for anc in exe.ancestors() {
        let looks_app = anc
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".app"))
            .unwrap_or(false);
        if looks_app && anc.join("Contents").join("Info.plist").is_file() {
            return Some(anc.to_path_buf());
        }
    }
    None
}

/// codesign 진단 출력에서 봉인 파손의 **원인 파일**을 갈래별로 분류한다(순수 함수 = 테스트 대상).
/// 반환 `(added, modified, missing, other)` — other 는 세 갈래에 안 잡힌 진단 문장(요약줄 포함).
/// ★`--verbose` 필수: 무-verbose 출력은 "a sealed resource is missing or invalid" 요약 한 줄뿐이라
///   *어떤 파일 때문인지*를 사용자에게 말할 수 없다(실측 확인).
/// ★lib 로 승격됨(`cys::app_bundle::parse_seal_failure`) — doctor 와 기동 자가진단(SEAL-DIAG)이
/// **같은 판정 어휘**를 쓰게 하려는 것이 이 위임의 전부다. 사본을 둘로 두면 한쪽만 고쳐졌을 때
/// 같은 codesign 출력을 놓고 두 진단이 다른 말을 한다(=규약 산재). 아래 회귀 핀은 그대로 둔다 —
/// 위임이 끊기면 그 핀이 먼저 깨진다.
fn parse_codesign_seal_failure(out: &str) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    cys::app_bundle::parse_seal_failure(out)
}

/// 번들 루트 접두를 떼어 사람이 읽을 수 있게 줄인다(로그 폭·개인 경로 노출 축소).
/// ★lib 로 승격됨(`cys::app_bundle::seal_relative`) — 여기 있던 사본은 codesign 이 **realpath 로**
/// 보고한다는 사실을 몰라서, 심링크를 거친 설치 경로에서는 접두를 못 떼고 전체 절대경로를 그대로
/// 노출했다(SEAL-DIAG e2e 핀이 실 codesign 출력으로 잡은 결함). 판정 어휘와 마찬가지로 표시 규약도
/// 하나만 둔다 — doctor 와 기동 자가진단이 같은 문면을 쓴다.
fn seal_rel(bundle: &std::path::Path, p: &str) -> String {
    cys::app_bundle::seal_relative(bundle, p)
}

/// 봉인 파손 시 사용자에게 주는 **유일하게 통하는 복구 절차**.
/// /Applications 안에서 파일을 지우는 부분 수정은 App Management(TCC)에 막히므로,
/// 임시 폴더에 새 번들을 스테이징한 뒤 `mv` 로 통째 교체해야 한다.
const APP_SEAL_RECOVERY: &str = "복구는 번들 통째 재설치뿐 — ①cys 종료 ②새 DMG의 cys.app 을 임시 폴더에 \
스테이징(`ditto --rsrc --extattr --acl <dmg>/cys.app /tmp/cys-stage/cys.app`) ③`mv /Applications/cys.app \
~/.Trash/cys.app.broken` ④`mv /tmp/cys-stage/cys.app /Applications/`. \
★/Applications 안 번들의 파일을 지우는 '부분 수정'은 App Management 보호에 막히고, 막히지 않아도 \
봉인은 복구되지 않는다(added 가 missing 으로 바뀔 뿐). ★재설치 전까지 이 사본을 다른 맥으로 \
전달하지 말 것 — quarantine 이 붙은 첫 실행에서 '손상되었기 때문에 열 수 없습니다'로 차단된다";

fn diag_app_seal(ctx: &DoctorCtx) -> DiagItem {
    let name = "app-seal";
    let skip = |detail: String| DiagItem {
        name,
        status: DiagStatus::Skip,
        detail,
        action: String::new(),
    };
    if !cfg!(target_os = "macos") {
        return skip("macOS 아님 — 코드서명 봉인 검사 미해당".into());
    }
    let Some(bundle) = ctx.app_bundle.as_ref() else {
        return skip("앱 번들 밖 실행(개발 빌드·비번들 설치) — 검사 대상 없음".into());
    };
    if !bundle.exists() {
        return skip(format!("앱 번들 경로 소멸({}) — 판정 불가", bundle.display()));
    }
    // codesign 은 stock macOS 의 /usr/bin 상주 도구다. PATH 하이재킹을 피해 절대경로로 부르고,
    // 부재(비정상 OS·축소 이미지)는 FAIL 이 아니라 판정 불가다.
    let tool = std::path::Path::new("/usr/bin/codesign");
    if !tool.exists() {
        return skip("/usr/bin/codesign 부재 — 판정 불가".into());
    }
    // `--verify --strict` = Gatekeeper 가 보는 최상위 봉인 판정(+ 주 실행파일). `--deep` 은 쓰지
    // 않는다: 이번 파손은 최상위 sealed resource 이고, --deep 은 중첩 서명까지 훑어 느리다.
    // 읽기 전용·로컬·유계 연산이라 별도 타임아웃을 두지 않는다(실측: 실 번들 0.47s).
    let out = match std::process::Command::new(tool)
        .args(["--verify", "--strict", "--verbose"])
        .arg(bundle)
        .output()
    {
        Ok(o) => o,
        Err(e) => return skip(format!("codesign 실행 실패({e}) — 판정 불가")),
    };
    if out.status.success() {
        return DiagItem {
            name,
            status: DiagStatus::Ok,
            detail: format!("코드서명 봉인 무결 — {}", bundle.display()),
            action: String::new(),
        };
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let (added, modified, missing, other) = parse_codesign_seal_failure(&text);
    let broken = !added.is_empty() || !modified.is_empty() || !missing.is_empty();
    if !broken {
        // codesign 이 실패했지만 봉인 파손 문장이 아니다(미서명 dev 번들·해석 불가 오류 등).
        // 손상을 확증하지 못했으므로 FAIL 로 겁주지 않고, 관측 사실만 WARN 으로 남긴다.
        let tail: String = other.join(" | ").chars().take(300).collect();
        return DiagItem {
            name,
            status: DiagStatus::Warn,
            detail: format!(
                "codesign 검증 실패(exit {}) — 봉인 파손 파일은 특정되지 않음: {}",
                out.status.code().unwrap_or(-1),
                if tail.is_empty() { "(출력 없음)".into() } else { tail }
            ),
            action: "미서명 개발 빌드면 정상. 배포본이면 서명·공증 상태를 확인하라(codesign -dvvv)".into(),
        };
    }
    let sample: Vec<String> = added
        .iter()
        .chain(modified.iter())
        .chain(missing.iter())
        .take(3)
        .map(|p| seal_rel(bundle, p))
        .collect();
    let all: Vec<&String> = added.iter().chain(modified.iter()).chain(missing.iter()).collect();
    // 자기유발 파손의 지문: 원인 파일이 전부 __pycache__ 면 번들 안 Python 런타임이 범인이다.
    let pycache_note = if all.iter().all(|p| p.contains("__pycache__")) {
        " ★원인 파일이 전부 __pycache__ — 번들 안 Python 런타임이 실행 중 스스로 생성해 봉인을 깼다(자기유발)."
    } else {
        ""
    };
    DiagItem {
        name,
        status: DiagStatus::Fail,
        detail: format!(
            "코드서명 봉인 파손 — {} · 추가 {}건·수정 {}건·누락 {}건 (예: {}){} \
             이 번들을 브라우저로 배포하면 받는 쪽 첫 실행에서 Gatekeeper 가 '손상되었기 때문에 \
             열 수 없습니다'로 차단한다(공증·staple 은 정상이어도).",
            bundle.display(),
            added.len(),
            modified.len(),
            missing.len(),
            sample.join(", "),
            pycache_note
        ),
        action: APP_SEAL_RECOVERY.into(),
    }
}

fn run_doctor_diagnostics(ctx: &DoctorCtx, fix: bool) -> Vec<DiagItem> {
    vec![
        diag_pack_version(ctx),
        diag_pack_state(ctx),
        diag_install_manifest(ctx),
        diag_hook(ctx, fix),
        diag_dept_hook_residue(ctx, fix),
        diag_dept_awakening_seed(ctx),
        // ★(M5) 이 레인 자신의 '설치 표적 ≠ 실소비 SOT' — 위 두 항목이 못 보는 축.
        diag_config_dir_target(ctx),
        diag_orphan_socket(ctx, fix),
        diag_stale_lock(ctx, fix),
        diag_staging_residue(ctx, fix),
        diag_channels_db(ctx),
        diag_legacy_config(ctx),
        // M3: 자기 앱 번들 코드서명 봉인(설치본이 스스로 봉인을 깼는지) — 읽기 전용, --fix 무관.
        diag_app_seal(ctx),
    ]
}

/// 사람용 바이트 표기(완전 초기화 프리뷰 전용 — 근사치로 충분).
fn fmt_bytes(b: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = b as f64;
    let mut u = 0usize;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{b}B")
    } else {
        format!("{v:.1}{}", UNITS[u])
    }
}

/// 완전 초기화 확인 문구 — GUI(resetconfirm.ts)와 동일 문자열 계약.
const FACTORY_RESET_PHRASE: &str = "완전 초기화";

fn factory_reset_plan_json(plan: &cys::factory_reset::ResetPlan) -> Value {
    json!({
        "stamp": plan.stamp,
        "trash_dir": plan.trash_dir.to_string_lossy(),
        "total_bytes": plan.quarantine_total_bytes(),
        "quarantine": plan.quarantine.iter().map(|i| json!({
            "path": i.path.to_string_lossy(), "label": i.label, "size_bytes": i.size_bytes,
        })).collect::<Vec<_>>(),
        "keep": plan.keep.iter().map(|i| json!({
            "path": i.path.to_string_lossy(), "label": i.label,
        })).collect::<Vec<_>>(),
        "strip_settings": plan.strip_settings.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        "strip_skill_dirs": plan.strip_skill_dirs.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        "temp_sweep_count": plan.temp_sweep.len(),
        "report_only": plan.report_only,
        "purge_license": plan.purge_license,
        "purge_local": plan.purge_local,
        "purge_round": plan.purge_round,
        "trash_root_ready": plan.trash_root_ready.is_ok(),
        "trash_root_error": plan.trash_root_ready.as_ref().err(),
        "interrupted_prior": plan.interrupted_prior.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
    })
}

/// 완전 초기화(팩토리 리셋) 실행기 — 코어는 cys::factory_reset(계약: DESIGN-factory-reset.md).
/// exit: 0=성공(--plan 포함) · 1=부분 실패/확인 불일치/정지 실패 · 2=가드 거부.
/// 되돌리기 — 격리 폴더의 복구 지도로 원위치 복원(P0: "복구 가능" 고지의 이행).
fn run_factory_reset_undo(trash_dir: &str, plan_only: bool, yes: bool, json_out: bool) -> i32 {
    let dir = std::path::PathBuf::from(shellexpand_home(trash_dir));
    let plan = match cys::factory_reset::read_undo_plan(&dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    if json_out {
        println!("{}", json!({
            "mode": if plan_only { "undo-plan" } else { "undo" },
            "trash_dir": plan.trash_dir.to_string_lossy(),
            "source": plan.source,
            "restorable": plan.entries.len(),
            "blocked": plan.blocked.iter().map(|(_, to)| to.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        }));
    } else {
        println!("복구 계획 — {} (지도: {})", plan.trash_dir.display(), plan.source);
        for (from, to) in &plan.entries {
            println!("  복구  {} → {}", from.display(), to.display());
        }
        for (_, to) in &plan.blocked {
            println!("  건너뜀 {} — 원위치에 이미 항목이 있다(덮어쓰지 않는다)", to.display());
        }
    }
    if plan_only {
        return 0;
    }
    if plan.entries.is_empty() {
        println!("복구할 항목이 없다.");
        return 0;
    }
    if !yes {
        print!("복구를 실행하려면 y 를 입력: ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if !matches!(line.trim(), "y" | "Y" | "yes") {
            eprintln!("중단(아무것도 변경되지 않음)");
            return 1;
        }
    }
    let mut progress = |_p: &str, d: &str| {
        if !json_out {
            println!("[undo] {d}");
        }
    };
    let (restored, failed) = cys::factory_reset::execute_undo(&plan, &mut progress);
    if json_out {
        println!("{}", json!({
            "mode": "undo", "restored": restored,
            "failed": failed.iter().map(|(p, e)| json!({"path": p.to_string_lossy(), "error": e})).collect::<Vec<_>>(),
        }));
    } else {
        println!("\n복구 완료 — {restored}건 원위치");
        for (p, e) in &failed {
            eprintln!("  실패  {}: {e}", p.display());
        }
        println!("데몬·팩을 다시 세우려면 앱을 실행하거나 `cys init-pack && cys daemon install`");
    }
    if failed.is_empty() { 0 } else { 1 }
}

/// `~` 시작 경로를 홈으로 확장(셸을 거치지 않은 인자 대비).
fn shellexpand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest).to_string_lossy().into_owned();
        }
    }
    p.to_string()
}

fn run_factory_reset(
    plan_only: bool,
    yes: bool,
    json_out: bool,
    purge_license: bool,
    purge_local: bool,
    purge_round: bool,
    verbose: bool,
) -> i32 {
    // 자기 살해 방지: cys surface(데몬 PTY) 안에서 실행하면 데몬 kill 이 자기 세션을 끊는다.
    if cys::env_compat(ENV_SURFACE_ID).filter(|v| !v.is_empty()).is_some() {
        eprintln!(
            "cys surface 안에서는 완전 초기화를 실행할 수 없다 — 데몬이 죽는 순간 이 세션도 \
             끊긴다. 외부 터미널(맥 기본 터미널 등)이나 GUI '완전 초기화' 버튼에서 실행하라."
        );
        return 2;
    }
    let Some(roots) = cys::factory_reset::ResetRoots::live() else {
        eprintln!("홈 디렉토리를 해석할 수 없다 — 완전 초기화 불가");
        return 2;
    };
    // ★A3b(성찰 확정): GUI 앱이 떠 있으면 그 앱이 리셋 도중 데몬을 되살린다(부트 재시도 루프·
    // 재시작 버튼·drain 사이드카). 프리뷰(--plan)는 읽기 전용이라 허용하고, 실집행만 거부한다.
    if !plan_only && cys::factory_reset::gui_app_running() {
        eprintln!(
            "지금 켜져 있는 cys 앱의 상단바 '완전 초기화' 버튼을 쓰는 것이 가장 쉽다.\n\
             터미널에서 실행하려면 앱을 먼저 종료하라(맥: ⌘Q 또는 메뉴 → 종료 / \
             윈도: 창 닫기 후 작업 표시줄 아이콘 우클릭 → 종료).\n\
             앱이 살아 있으면 초기화 도중 데몬을 되살려 격리와 경합한다."
        );
        return 2;
    }
    let opts = cys::factory_reset::ResetOptions { purge_license, purge_local, purge_round };
    let plan = cys::factory_reset::build_plan(&roots, &opts);

    if json_out && plan_only {
        println!("{}", json!({"mode": "plan", "plan": factory_reset_plan_json(&plan)}));
        return 0;
    }
    if !json_out {
        println!("완전 초기화 계획 — 격리 {}건({}) → {}", plan.quarantine.len(),
            fmt_bytes(plan.quarantine_total_bytes()), plan.trash_dir.display());
        // ★P0-2: 사용자 폴더 안에서 사라지는 것은 **맨 앞에서 따로** 보여준다(묻히면 안 된다).
        let outside: Vec<_> = plan.quarantine.iter().filter(|i| i.outside_state).collect();
        if !outside.is_empty() {
            println!("\n  ⚠ 내 폴더 안에서 사라지는 항목 — 승인 전에 반드시 확인하세요:");
            for i in &outside {
                println!("     {}  ({})", i.path.display(), fmt_bytes(i.size_bytes));
            }
            println!();
        }
        // ★P2-1: 34줄 평면 나열은 가장 아픈 항목(대화기억 1.5GB·프로젝트 작업기억)을 묻는다.
        // 라벨별 소계 → 큰 것부터 상위 N → 나머지는 접는다(`--verbose` 로 전량).
        {
            let mut by_label: Vec<(String, usize, u64)> = Vec::new();
            for i in &plan.quarantine {
                match by_label.iter_mut().find(|(l, _, _)| *l == i.label) {
                    Some(e) => {
                        e.1 += 1;
                        e.2 += i.size_bytes;
                    }
                    None => by_label.push((i.label.clone(), 1, i.size_bytes)),
                }
            }
            by_label.sort_by(|a, b| b.2.cmp(&a.2));
            println!("\n  분류별 소계:");
            for (label, n, bytes) in &by_label {
                println!("     {label}: {n}건 · {}", fmt_bytes(*bytes));
            }
            let mut items: Vec<&cys::factory_reset::PlanItem> = plan.quarantine.iter().collect();
            items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
            let show = if verbose { items.len() } else { items.len().min(8) };
            println!("\n  큰 항목부터:");
            for i in items.iter().take(show) {
                println!("     {}  ({}, {})", i.path.display(), i.label, fmt_bytes(i.size_bytes));
            }
            if show < items.len() {
                println!("     …외 {}건 (전량 보기: --verbose)", items.len() - show);
            }
            println!();
        }
        for i in &plan.keep {
            println!("  보존  {}  ({})", i.path.display(), i.label);
        }
        for s in &plan.strip_settings {
            println!("  해제  {}  (cys 훅·statusLine 제거)", s.display());
        }
        for s in &plan.strip_skill_dirs {
            println!("  해제  {}  (pack 스킬 심링크 제거)", s.display());
        }
        if !plan.temp_sweep.is_empty() {
            println!("  소거  임시 캐시 {}건 ($TMPDIR)", plan.temp_sweep.len());
        }
        for r in &plan.report_only {
            println!("  안내  {r}");
        }
        for d in &plan.interrupted_prior {
            println!("  ⚠ 이전 초기화가 중단된 흔적: {} (복구 지도 journal.ndjson)", d.display());
        }
        println!(
            "  복구  cys factory-reset --undo {}  (또는 그 폴더의 manifest.json / journal.ndjson 을 역방향 mv)",
            plan.trash_dir.display()
        );
        println!("        격리본은 약 14일 뒤 정리 작업에서 소거될 수 있습니다(직접 지우려면 rm -rf).");
    }
    // ★P0-6: 격리 목적지가 못 쓰는 상태면 **데몬을 건드리기 전에** 거부한다.
    if let Err(e) = &plan.trash_root_ready {
        eprintln!("\n격리 폴더를 쓸 수 없어 초기화를 시작하지 않는다: {e}");
        return 2;
    }
    if plan_only {
        // ★P0-2/P2: 프리뷰만 본 사용자가 놓치던 정보(재로그인·비가역·다음 행동)를 마무리에 싣는다.
        if !json_out {
            println!("\n⚠ 실행하면: 열려 있던 세션이 저장 신호 없이 즉시 종료되고, 에이전트 계정 \
로그인(~/.cys/claude*)이 격리되어 재로그인이 필요합니다.");
            println!("쓰기 0 — 아무것도 변경되지 않았습니다.");
            println!("실제로 실행하려면: cys factory-reset   (cys 앱은 먼저 종료)");
        }
        return 0;
    }

    if json_out && !yes {
        // 계약 정합: `--json` 은 기계 소비용인데 확인 프롬프트가 stdout 을 오염시키고 stdin 을
        // 블록한다. 조용히 섞지 말고 명시적으로 거부한다(무음 오염 금지).
        eprintln!("--json 실집행은 --yes 와 함께 써야 한다(확인 프롬프트가 JSON 출력을 오염시킨다)");
        return 2;
    }
    if !yes {
        println!(
            "\n⚠ 지금 열려 있는 세션(마스터·워커·부서)이 **저장 신호 없이 즉시 종료**된다 — \
중요한 작업은 먼저 마무리하라.\n\
             모든 부서·대화기억·작업기억이 격리되고 설치 초기 상태로 돌아간다.\n\
             에이전트 계정 로그인(~/.cys/claude*)도 격리되어 재로그인이 필요하다."
        );
        // ★P2-1: 따옴표 동반 복사·NFD 자모·NBSP/전각/ZWSP 는 육안상 같은 입력이므로 같게 받고
        // (normalize_confirm_phrase), 틀리면 **사유를 말하고 최대 3회** 다시 받는다.
        // 종전엔 한 번 어긋나면 즉시 exit 1 이라, 프롬프트가 유도한 따옴표 복사로도 튕겼다.
        use std::io::Write;
        let want = cys::factory_reset::normalize_confirm_phrase(FACTORY_RESET_PHRASE);
        let mut ok = false;
        for attempt in 1..=3 {
            print!("실행하려면 {FACTORY_RESET_PHRASE} 를 그대로 입력({attempt}/3): ");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_err() || line.is_empty() {
                break; // EOF(파이프·Ctrl-D) — 대화가 불가능하므로 중단.
            }
            let got = cys::factory_reset::normalize_confirm_phrase(&line);
            if got == want {
                ok = true;
                break;
            }
            if got.is_empty() {
                eprintln!("  입력이 비어 있다.");
            } else {
                eprintln!("  입력 \"{got}\" 는 \"{FACTORY_RESET_PHRASE}\" 와 다르다(띄어쓰기까지 그대로).");
            }
        }
        if !ok {
            eprintln!("문구 불일치 — 중단(아무것도 변경되지 않음)");
            return 1;
        }
    }

    let mut progress = |phase: &str, detail: &str| {
        if !json_out {
            println!("[{phase}] {detail}");
        }
    };
    // ★P0-1: 여기서 센티널을 무장한다(RAII — 조기 return·패닉에도 Drop 이 해제).
    let _sentinel = cys::factory_reset::ResetSentinel::arm();
    if let Err(e) = cys::factory_reset::stop_daemons_and_unregister(&plan, &mut progress) {
        eprintln!("정지 실패: {e}");
        // ★P0-6: 이미 일어난 비가역 부수효과를 숨기지 않는다("실패=원래대로"라는 오해 차단).
        eprintln!("{}", cys::factory_reset::stop_side_effects_note());
        return 1;
    }
    match cys::factory_reset::execute_quarantine(
        &plan,
        &roots,
        &cys::factory_reset::live_pid_is_cysd,
        &cys::factory_reset::live_any_cysd_running,
        &mut progress,
    ) {
        Ok(rep) => {
            if json_out {
                println!("{}", json!({
                    "mode": "reset",
                    "ok": rep.ok(),
                    "trash_dir": rep.trash_dir.to_string_lossy(),
                    "moved": rep.moved.len(),
                    "failed": rep.failed.iter().map(|(p, e)| json!({
                        "path": p.to_string_lossy(), "error": e,
                    })).collect::<Vec<_>>(),
                    "kept": rep.kept.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
                    "stripped": rep.stripped,
                    "temp_swept": rep.temp_swept,
                    "deferred": rep.deferred.iter().map(|(p, e)| json!({
                        "path": p.to_string_lossy(), "error": e,
                    })).collect::<Vec<_>>(),
                    "revived_warning": rep.revived_warning,
                    "skipped_absent": rep.skipped_absent,
                    "manifest_written": rep.manifest_written,
                    "interrupted_prior": rep.interrupted_prior.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
                }));
            } else {
                // ★P0-4: 예고 건수와 완료 건수의 차이를 **분해해서** 보여준다(설명 없는 불일치 금지).
                println!(
                    "\n완전 초기화 {} — 이동 {}건 · 이미 없음 {}건 · 이연 {}건 · 실패 {}건 → {}",
                    if rep.ok() { "완료" } else { "부분 완료" },
                    rep.moved.len(),
                    rep.skipped_absent,
                    rep.deferred.len(),
                    rep.failed.len(),
                    rep.trash_dir.display()
                );
                if !rep.manifest_written {
                    eprintln!("  ⚠ 복구 지도(manifest.json)를 쓰지 못했다 — journal.ndjson 이 유일한 지도다");
                }
                for (p, e) in &rep.failed {
                    eprintln!("  실패  {}: {e}", p.display());
                }
                for s in &rep.stripped {
                    println!("  해제  {s}");
                }
                for (p, e) in &rep.deferred {
                    println!("  이연  {} ({e}) — 앱 종료 후 다시 실행하면 정리된다", p.display());
                }
                if let Some(w) = &rep.revived_warning {
                    eprintln!("  ⚠ {w}");
                }
                for d in &rep.interrupted_prior {
                    println!("  ⚠ 이전 중단 흔적: {} (복구 지도 journal.ndjson)", d.display());
                }
                println!(
                    "\n결과 요약 파일: {}/REPORT.txt (화면이 사라져도 여기 남는다)\n\
                     다음 앱 실행 시 설치 온보딩이 처음부터 시작된다.\n\
                     CLI 재구성: cys init-pack && cys daemon install\n\
                     되돌리기: cys factory-reset --undo {}",
                    rep.trash_dir.display(),
                    rep.trash_dir.display()
                );
            }
            if rep.ok() { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("완전 초기화 실패(격리 시작 전 중단): {e}");
            eprintln!("{}", cys::factory_reset::stop_side_effects_note());
            1
        }
    }
}

fn run_doctor(fix: bool, json_out: bool) -> i32 {
    let pack_dir = cys::pack::pack_dir();
    let state_base = pack_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let socket_path = cys::socket_path();
    let daemon_state_dir = socket_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let mut settings_paths = discover_claude_settings();
    if settings_paths.is_empty() {
        settings_paths = vec![dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".claude/settings.json")
            .to_string_lossy()
            .into_owned()];
    }
    let ctx = DoctorCtx {
        pack_dir,
        state_base,
        socket_path,
        daemon_state_dir,
        settings_paths,
        binary_version: env!("CARGO_PKG_VERSION").to_string(),
        // 번들 밖 실행이면 None → app-seal 은 Skip(판정 불가). 탐지 실패를 정상으로 적지 않는다.
        app_bundle: std::env::current_exe().ok().as_deref().and_then(detect_app_bundle),
    };
    let items = run_doctor_diagnostics(&ctx, fix);
    let fails = items.iter().filter(|i| i.status == DiagStatus::Fail).count();
    let warns = items.iter().filter(|i| i.status == DiagStatus::Warn).count();
    let skips = items.iter().filter(|i| i.status == DiagStatus::Skip).count();
    if json_out {
        let arr: Vec<Value> = items
            .iter()
            .map(|i| {
                json!({
                    "name": i.name,
                    "status": i.status.as_str(),
                    "detail": i.detail,
                    "action": i.action,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "fix": fix,
                "summary": {"ok": items.len() - fails - warns - skips, "warn": warns,
                            "fail": fails, "skip": skips},
                "items": arr
            }))
            .unwrap_or_default()
        );
    } else {
        println!(
            "cys doctor — 시스템 자기진단{}",
            if fix { " (--fix)" } else { "" }
        );
        for i in &items {
            println!("  [{:<4}] {:<16} {}", i.status.as_str(), i.name, i.detail);
            if !i.action.is_empty() {
                println!("           └ {}", i.action);
            }
        }
        println!(
            "요약: {} OK · {} WARN · {} FAIL · {} SKIP(판정 불가)",
            items.len() - fails - warns - skips,
            warns,
            fails,
            skips
        );
    }
    if fails > 0 {
        1
    } else {
        0
    }
}

/// 표준 노드 일괄 부트: 설치된 CLI만 자동 감지해 워커+리뷰어를 기동·지침 주입한다.
/// 마스터 부트 시퀀스 ④의 결정론적 구현 — 모델 재량("필요할 때 띄우자")에 맡기지 않는다.
/// '~/'-시작 경로를 홈으로 확장 (그 외는 그대로) — boot의 경로형 cmd 설치 판정용.
fn expand_tilde(p: &str) -> std::path::PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    std::path::PathBuf::from(p)
}

/// 절대지침 앵커4-1: 프로젝트 시작 시 CSO·worker·agy·codex 4개 노드를 의무 기동한다
/// (LLM orchestrating 상주 편성). grok은 설치돼 있으면 추가 리뷰어로 띄운다(미설치 skip).
/// 소켓별 boot 락 가드 — `Acquired`는 보유 토큰(`LockHold`)을 쥔 채 Drop에서 해제한다
/// (unix=flock fd 자동 해제 / non-unix=pidfile 삭제). 판정 불가·파일 열기 실패는
/// `LockHold::unserialized()`로 락 없이 진행(직렬화만 포기 — 중복은 데몬 특권 가드·live-slot이 흡수).
enum BootLock {
    Acquired(LockHold),
    Busy,
}

/// ─────────── ★BUDGET parity 상수 블록 (T-0147-7 W2 · B9·B17·P3-A-120S) ───────────
/// **이 블록의 값은 `cysjavis-pack/bin/javis_budget.py` 의 동명 leaf 와 기계 대조된다**
/// (`javis_budget.RUST_PARITY_CONSTS` 표 + 건강성 러너 H-TIME-1). Rust 는 python 을 import 할 수
/// 없으므로 파리티는 grep 기계 대조로만 보장된다 — 한쪽만 바꾸면 검체가 적색이 된다.
///
/// ★불변식(비평2 D-2): **내부 감액 금지**(이 값들은 냉시작 실측 하한이다) · 외부 상한은 python
/// 쪽에서 이 값들의 합+마진으로 파생 · 침묵 창은 하트비트로 상쇄.
/// ★카운트 회계 금지(B17): 아래 TICK 은 폴링 주기일 뿐 **시간 회계의 단위가 아니다** — 데드라인은
/// `Instant` 벽시계로만 판정한다(종전 `waited += 2` 산술은 실효 대기를 25%+α 어긋나게 했다).
const BUDGET_READINESS_FLOOR_SECS: u64 = 30;
const BUDGET_READINESS_MULT: u64 = 2;
const BUDGET_RESTORE_CAP_SECS: u64 = 20;
const BUDGET_TICK_MS: u64 = 2500;
const BUDGET_POST_MARKER_SETTLE_SECS: u64 = 2;
const BUDGET_ACK_WAIT_SECS: u64 = 8;
const BUDGET_TRUST_SETTLE_SECS: u64 = 2;
const BUDGET_HEARTBEAT_INTERVAL_SECS: u64 = 20;
/// non-unix pidfile 락의 스테일 회수 임계(초). 정상 부트 최악치보다 **넉넉히 크게** 잡아
/// 진행 중인 부트를 뺏지 않으면서, 크래시 잔재가 영구히 부트를 막는 것을 구조적으로 막는다.
/// (`allow(dead_code)`: 소비처가 `cfg(not(unix))` 라 unix 빌드에선 미사용 — 상수는 BUDGET
///  파리티 블록의 일원으로 **항상 소스에 존재**해야 한다(H-PRED-6 이 텍스트로 대조).)
#[allow(dead_code)]
const BUDGET_LOCK_STALE_SECS: u64 = 900;
/// G35: 폴더신뢰 자동확인 재전송 상한(멱등 래치 + 이 상한으로 매 tick 재전송을 절단).
const BUDGET_TRUST_MAX_SENDS: u32 = 2;

/// readiness 폴링 상한(벽시계) — python `javis_budget.launch_readiness_max_s` 와 동일 산식.
fn budget_readiness_max(inject_delay_secs: u64, restore: bool) -> std::time::Duration {
    let base = inject_delay_secs.max(BUDGET_READINESS_FLOOR_SECS) * BUDGET_READINESS_MULT;
    let secs = if restore {
        base.min(BUDGET_RESTORE_CAP_SECS)
    } else {
        base
    };
    std::time::Duration::from_secs(secs)
}

/// 현재 소켓(CYS_SOCKET 상속)의 boot 락 파일을 비차단 flock 획득 시도.
fn boot_lock_path() -> std::path::PathBuf {
    cys::socket_path()
        .parent()
        .map(|d| d.join("cys-boot.lock"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/cys-boot.lock"))
}

/// ★(W2 · G12) boot 락 **커버리지 확장**의 재진입 마커.
///
/// 종전 커버리지는 `cys boot` 본문 하나였다 — 그런데 팀 스폰은 세 경로로 일어난다:
///   ① `cys boot`(GUI 버튼·훅 ④·LLM 직접)  ② ④-b `boot-reviewers` → `javis_boot_node` →
///   **별도 프로세스 `cys launch-agent`**  ③ `cys restore`·`node-recover`.
/// ②·③이 락 **밖**이라 ①과 겹치면 같은 리뷰어를 두 번 스폰하는 창이 열려 있었다(G12).
/// 그래서 `launch-agent` 도 같은 소켓별 락에 참여시킨다. 두 겹의 재진입 방어:
///   · 프로세스 내부: `BOOT_LOCK_HELD`(run_boot 가 이미 쥔 채 in-process 로 호출한다 — 같은
///     프로세스에서 다른 fd 로 flock 을 재획득하면 자기 자신에게 막힌다).
///   · 자식 프로세스: `CYS_BOOT_LOCK_HELD=1` env 전파(javis_boot_node 가 띄우는 `cys launch-agent`
///     는 run_boot 의 자식이 아니지만, 미래에 그런 배선이 생겨도 자기 교착이 없다).
static BOOT_LOCK_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn boot_lock_already_held() -> bool {
    BOOT_LOCK_HELD.load(std::sync::atomic::Ordering::SeqCst)
        || std::env::var("CYS_BOOT_LOCK_HELD").map(|v| v == "1").unwrap_or(false)
}

/// launch-agent 경로의 락 참여 — **보류가 아니라 유계 대기**다.
/// Busy 에서 즉시 성공-skip 하면 호출자(boot_node)가 원하는 노드가 안 뜨고, 무한 대기하면 부트가
/// 멈춘다. 그래서 짧게 기다렸다가(선행 boot 가 이 role 을 세울 시간) 여전히 Busy 면 **경고 후 진행**
/// 한다 — 중복 스폰 창을 '무제한'에서 '대기 상한 이후의 꼬리'로 줄이는 것이 이 게이트의 목적이고,
/// 최종 중복 방어는 데몬의 특권 가드·live-slot 계약이다(가용성 우선).
fn acquire_launch_lock() -> Option<LockHold> {
    if boot_lock_already_held() {
        return None; // 이미 상위(run_boot)가 쥐고 있다 — 재획득은 자기 교착이다.
    }
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_millis(BUDGET_TICK_MS * 4);
    loop {
        match acquire_boot_lock() {
            // ★플래그를 여기서 세우지 않는다(의도): 세우면 이 함수가 반환한 guard 가 drop 된 뒤에도
            //   플래그가 참으로 남아, 같은 프로세스의 **다음** launch-agent 가 '이미 보유'로 오판해
            //   락을 아예 건너뛴다(`cys restore` 처럼 한 프로세스가 여러 role 을 순차 기동하는 경로에서
            //   2번째부터 커버리지가 조용히 사라진다). 플래그의 유일한 writer 는 run_boot 다 —
            //   그쪽은 함수 전체 수명 동안 guard 를 쥐고 있어 참/거짓이 실제 보유 상태와 일치한다.
            BootLock::Acquired(g) => return Some(g),
            BootLock::Busy => {
                if std::time::Instant::now() >= deadline {
                    eprintln!(
                        "[launch-agent] boot 락 대기 상한 초과 — 직렬화 없이 진행(중복은 데몬 특권 \
                         가드·live-slot 계약이 방어). 동시 부트가 진행 중일 수 있다."
                    );
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        }
    }
}

/// boot 락 보유 토큰. **unix**=flock 된 fd(닫히면 커널이 자동 해제) / **non-unix**=pidfile 경로
/// (Drop 에서 **파일을 삭제**해야 해제된다 — 파일시스템 락은 자동 해제가 없다).
///
/// ★왜 구조체인가(Windows 전용 치명 결함의 수리): 초안은 `Option<File>` 만 들고 있었다. Windows
/// 경로에서 그 File 은 pidfile 핸들이라, Drop 은 **핸들만 닫고 파일은 디스크에 남긴다**. 남은
/// pidfile 은 다음 부트에서 `create_new` 를 AlreadyExists 로 만들고, 그때 생존 판정이 실패하면
/// (tasklist 부재·제한 환경·pid 재사용) **모든 Windows 부트가 영구히 'Busy'** 가 된다 —
/// 조용하고 영구적인 온보딩 전멸(팀이 영영 안 뜸)이다. 해제를 RAII 로 못박는다.
struct LockHold {
    #[allow(dead_code)]
    file: Option<std::fs::File>,
    /// non-unix 전용: Drop 에서 삭제할 pidfile 경로(unix 는 항상 None).
    pidfile: Option<std::path::PathBuf>,
}

impl Drop for LockHold {
    fn drop(&mut self) {
        if let Some(p) = self.pidfile.take() {
            // 핸들을 먼저 닫는다(Windows 는 열린 파일 삭제가 실패할 수 있다).
            self.file = None;
            let _ = std::fs::remove_file(&p);
        }
    }
}

impl LockHold {
    fn unserialized() -> Self {
        LockHold { file: None, pidfile: None }
    }
}

fn acquire_boot_lock() -> BootLock {
    let lock_path = boot_lock_path();
    if let Some(parent) = lock_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    #[cfg(unix)]
    let f = match std::fs::OpenOptions::new().create(true).write(true).open(&lock_path) {
        Ok(f) => f,
        // 락 못 열면 직렬화 없이 진행(보수적 허용 — 종전 동작)
        Err(_) => return BootLock::Acquired(LockHold::unserialized()),
    };
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            BootLock::Acquired(LockHold { file: Some(f), pidfile: None })
        } else {
            BootLock::Busy
        }
    }
    // ★A8rs(T-0147-7 W2): non-unix 는 종전에 **무조건 `Acquired`** 였다 — 파일을 열기만 하고 어떤
    //   상호배제도 하지 않은 채 "락을 얻었다"고 보고했다. 그 거짓 보고가 Windows 에서 ①리뷰어
    //   중복 스폰 ②settings.json 3-writer 교차 파손을 무장했다(A8 재검증: 지배 실패 모드는 RMW
    //   유실이 아니라 '공유 .tmp 교차 파손 → 등록부 수리 거부'와 '중복 스폰'이었다).
    //   python 측 `javis_lock.py`(W1a 신설)와 **동형 규약**의 pidfile 락을 쓴다:
    //     · O_CREAT|O_EXCL 로 `<lock>.pid` 를 만든 쪽만 승자(파일시스템 원자성).
    //     · 보유자 pid 가 죽어 있으면 스테일로 회수(무한 거부 창 방지 — R1 과 동형).
    //     · 파일시스템이 EXCL 을 못 주는 예외 상황에서만 `Acquired(None)`(종전 동작)로 강등한다.
    #[cfg(not(unix))]
    {
        match win_pidfile_lock(&lock_path) {
            WinLock::Won(pidfile, handle) => BootLock::Acquired(LockHold {
                file: Some(handle),
                pidfile: Some(pidfile),
            }),
            WinLock::Busy => BootLock::Busy,
            // ★fail-OPEN: 판정할 수 없으면 **직렬화를 포기하고 진행**한다(= W2 이전 동작).
            //   Windows 에서 'Busy 로 굳는' 실패가 훨씬 위험하다 — 중복 스폰은 데몬 특권 가드·
            //   live-slot 계약이 뒤에서 막지만, 영구 Busy 는 **아무도 막지 못한다**(팀 0).
            WinLock::Unavailable => BootLock::Acquired(LockHold::unserialized()),
        }
    }
}

#[cfg(not(unix))]
enum WinLock {
    /// (삭제할 pidfile 경로, 열린 핸들)
    Won(std::path::PathBuf, std::fs::File),
    Busy,
    Unavailable,
}

/// A8rs: pidfile 기반 크로스플랫폼 락(non-unix 경로 전용). javis_lock.py 의 pidfile 백엔드와 동형.
///
/// ★★Windows 전용 안전 설계(오너 지시: "윈도우는 뜻하지 않은 에러가 자주 난다"):
///   파일시스템 락은 **자동 해제가 없다** — 크래시·강제종료·핸들 누수는 pidfile 을 남기고, 남은
///   pidfile 은 이후 전 부트를 막을 수 있다. 그래서 회수 근거를 **2중**으로 둔다:
///     ⓐ 보유자 pid 사망(tasklist)  — 1차, 정확
///     ⓑ **pidfile 나이 > 스테일 임계** — 2차 backstop. tasklist 가 없거나(제한 환경·PATH 문제)
///        실패하거나, pid 가 **재사용**돼 남의 살아있는 프로세스로 오인돼도 시간이 지나면 반드시
///        회수된다. 이 backstop 이 '영구 Busy' 를 **구조적으로 불가능**하게 만든다.
///   그리고 어느 쪽도 판정 못 하면 `Unavailable`(직렬화 포기·진행)로 강등한다 — 가용성 우선.
#[cfg(not(unix))]
fn win_pidfile_lock(lock_path: &std::path::Path) -> WinLock {
    use std::io::Write;
    let pidfile = lock_path.with_extension("pid");
    for attempt in 0..2 {
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&pidfile)
        {
            Ok(mut f) => {
                let _ = write!(f, "{}", std::process::id());
                let _ = f.flush();
                return WinLock::Won(pidfile, f);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if attempt == 0 && pidfile_reclaimable(&pidfile) {
                    // 스테일 회수 — 보유자 사망 또는 나이 초과. 무한 거부 창을 남기지 않는다.
                    if std::fs::remove_file(&pidfile).is_err() {
                        // 삭제 자체가 막히면(권한·핸들 점유) 판정 불가 → 직렬화 포기(가용성 우선).
                        return WinLock::Unavailable;
                    }
                    continue;
                }
                // 회수 불가 = 살아있는 동시 부트로 본다(정상 Busy — 그 boot 가 팀을 세운다).
                return WinLock::Busy;
            }
            // create_new 자체가 다른 이유로 실패(경로·권한) → 직렬화 포기(종전 동작으로 강등).
            Err(_) => return WinLock::Unavailable,
        }
    }
    WinLock::Busy
}

/// pidfile 을 회수해도 되는가 — ⓐ보유자 사망 ∨ ⓑ나이 초과(backstop).
#[cfg(not(unix))]
fn pidfile_reclaimable(pidfile: &std::path::Path) -> bool {
    // ⓑ 나이 backstop 을 **먼저** 본다: 외부 프로세스 조회에 의존하지 않는 유일한 근거다.
    // (판정 순서가 계약이다 — 조회 도구 실패가 회수 판정을 지배하면 영구 Busy 가 된다.)
    if let Ok(md) = std::fs::metadata(pidfile) {
        if let Ok(age) = md.modified().and_then(|t| {
            std::time::SystemTime::now()
                .duration_since(t)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
        }) {
            if age.as_secs() > BUDGET_LOCK_STALE_SECS {
                eprintln!(
                    "[boot-lock] pidfile 나이 {}s > 스테일 임계 {}s — 회수(영구 Busy 방지)",
                    age.as_secs(),
                    BUDGET_LOCK_STALE_SECS
                );
                return true;
            }
        }
    }
    let Ok(txt) = std::fs::read_to_string(pidfile) else {
        return true; // 읽을 수 없는 락 파일 = 신뢰 불가 → 스테일 취급
    };
    let Ok(pid) = txt.trim().parse::<u32>() else {
        return true; // 내용이 pid 가 아니다(파손) → 스테일 취급
    };
    if pid == std::process::id() {
        return true; // 우리가 남긴 잔재(핸들 누수) → 회수
    }
    pidfile_holder_dead(pid)
}

/// 보유 pid 의 사망 확인(외부 프로세스 조회 — Windows tasklist). 조회 실패는
/// '살아있음'(보수 방향) — 영구 Busy 는 pidfile_reclaimable 의 나이 backstop 이 상한을 보장한다.
/// javis_lock.py 의 pid 사망 검사와 동형 규약(H-CONC-2 스테일 회수 계약의 Rust 측 절반).
#[cfg(not(unix))]
fn pidfile_holder_dead(pid: u32) -> bool {
    match std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
    {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            !s.contains(&pid.to_string())
        }
        Err(_) => false,
    }
}

/// ★(W2 · B1) PLAN 정책 열 — `(role, agent, mandatory)`.
/// **python 정본 `javis_orchestra.BOOT_PLAN` 과 기계 대조된다**(H-PRED-7·H-EXIT-4):
///   mandatory=true  ↔ `FAIL_FATAL`   (cso·worker — 조직 최소 실행 단위. 실패=부트 실패)
///   mandatory=false ↔ `FAIL_DEGRADE` (리뷰어 — 경고 강등 후 ④-b·⑤ 계속. 대체 폴백으로 보완)
/// 종전엔 이 판정이 편성 테이블 **밖**(호출부 산문)에 있어, 리뷰어 1종 고장이 팀 전체 부트 실패로
/// 번지는 영구 데드엔드였다(B1). 정책을 편성과 같은 행에 둔다 — 소비자는 산문 대신 이 열을 읽는다.
const BOOT_PLAN: &[(&str, &str, bool)] = &[
    ("cso", "claude", true),
    ("worker", "claude", true),
    ("reviewer-gemini", "gemini", false),
    ("reviewer-codex", "codex", false),
    ("reviewer-grok", "grok", false),
];

/// ★(W2 · A1 클래스 · B3) `cys boot` 의 **스킵 술어** 3등급 — surface.list 한 행 → 판정.
/// `node_liveness`(python 정본 `javis_boot_node.node_liveness`) 의 Rust 미러다.
/// 종전 술어는 `!exited` 단독이었다: role 을 쥔 채 에이전트가 죽은 좌석(=빈 좌석)을 '가동 중'으로
/// 보고 건너뛰어, 부트가 영원히 팀을 못 세우는 라이브락을 먹였다(B3 — 데몬 주석이 예언한 재발).
///
/// ★(U-10) **제4 등급 `GatePending` 추가** — "프로세스는 살아 있으나 첫기동 관문(테마 →
/// 로그인방식 → OAuth → 폴더신뢰 → 면책 → 새기능안내)에 갇혀 **입력을 받을 수 없는** 좌석".
/// 종전엔 이 사실을 담을 자리가 없어 관문에 갇힌 좌석이 `agent_alive == true` 하나로
/// `AlivePresumed` 가 되고, `run_boot` 이 그것을 **"이미 가동 중 — 건너뜀"** 으로 접었다.
/// readiness 실패를 close 대신 **보류**로 바꾸는 U-11 을 그 위에 올리면 **관문에 갇힌 팀
/// 전체가 "정상 가동 중" 으로 집계**된다 — 지금보다 나빠진다.
/// 등급 서열은 `AlivePresumed` **아래**다(살아 있지만 쓸 수 없으므로 '충족' 이 아니다).
/// ★이 단위(U-10)에는 **생산자가 없다** — 데몬 wire 값이 항상 null 이라 이 변형은 도달
/// 불가이고, 거동은 오늘과 같다. 생산은 U-11/U-13 이 한다(자리를 먼저 만드는 이유는 위 사유).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeatLiveness {
    AwakeConfirmed,
    AlivePresumed,
    /// 살아 있으나 관문에 갇힘 — **충족 아님**(재스폰도 아님: pane 과 프로세스는 살아 있다).
    GatePending,
    Unknown,
    Absent,
}

fn seat_liveness(s: &Value) -> (SeatLiveness, &'static str) {
    if s["exited"].as_bool().unwrap_or(false) {
        return (SeatLiveness::Absent, "exited");
    }
    // ① awakened_at 래치 — 데몬 SOT·영속·단방향. 존재=각성 확정.
    //    ★(W-B1 래치 부정) 단, 래치는 영속·단방향이라 한 번 각성한 좌석이 그 안의 에이전트가
    //    죽은 뒤에도 영원히 AwakeConfirmed 로 읽히면 — boot 스킵("이미 가동 중") → 죽은 좌석
    //    위에서 거짓 성공이 된다(감사 blocker). 그래서 `seat_death_confirmed`(3중 AND:
    //    seat=="empty" ∧ agent_alive==Some(false) ∧ 좌석 나이>readiness 예산)가 죽음을
    //    **확정**한 좌석에서만 래치를 불신임하고 Absent 로 낸다 — 같은 계약의 재사용이다
    //    (새 술어 발명 금지: null(관측 미도달·Windows master meta 부재)·unknown·기동 중
    //    좌석은 그 게이트가 전부 Err 로 거부해 래치가 유지된다 = 보류 우선·치명위험 ④ 불변).
    //    python 미러 `javis_boot_node.node_liveness`/`latch_death_confirmed` 와 같은 사실·
    //    같은 등급을 내야 한다(tests/test_seat_latch_negation.py 파리티 검체가 기계 대조).
    //    이후 run_boot 흐름과의 정합: Absent + 좌석 존재 → 아래 회수 체인이 **같은**
    //    seat_death_confirmed 를 다시 물어 Ok 일 때만 node-recover(비파괴) → reclaim 순으로
    //    되살린다 — 래치를 부정한 근거와 침습 복구를 인가한 근거가 한 술어라 어긋날 수 없다.
    if s["awakened_at"].as_f64().unwrap_or(0.0) > 0.0 {
        if seat_death_confirmed(s).is_ok() {
            return (
                SeatLiveness::Absent,
                "awakened_at 래치 부정(죽음 3중 확정: seat=empty ∧ agent_alive=false ∧ 나이>readiness 예산)",
            );
        }
        return (SeatLiveness::AwakeConfirmed, "awakened_at 래치");
    }
    // ②(U-10) 관문 보류 — 프로세스는 살아 있으나 첫기동 관문에 갇혀 **입력 불가**.
    //    ★순서가 계약이다: `agent_alive` 분기 **앞**에 있어야 한다. 관문에 갇힌 에이전트도
    //      프로세스는 살아 있으므로, 뒤에 두면 이 분기는 영원히 도달 불가(죽은 코드)이고
    //      보류 좌석이 다시 `AlivePresumed` → `already_alive` 로 접힌다(이 단위의 존재 이유 소멸).
    //    ★래치(①) **뒤**인 이유: `awakened_at` 래치의 단방향 계약(존재=각성 확정)은 이 단위에서
    //      건드리지 않는다(금지 방향 ⑦). 한 번 각성한 좌석이 뒤에 관문에 갇히는 경로
    //      (node-recover 가 같은 pane 에 새 에이전트를 띄우는 창)는 이 순서에서 여전히
    //      AwakeConfirmed 로 읽힌다 — **오늘과 같은 결과**이고, 새 사망 경로가 아니다.
    //      그 잔여를 닫으려면 래치를 부정해야 하는데, 래치 부정의 유일 계약은
    //      `seat_death_confirmed` 3중 AND 이고 이 단위는 그 파괴 경로를 **동결**한다.
    //    ★`seat_death_confirmed` 를 **묻지 않는다**(60s 나이 게이트 미적용): 관문 보류는
    //      '죽음' 이 아니라 '살아 있는데 못 쓴다' 다. 나이 게이트를 태우면 갓 만든 보류 좌석이
    //      등급을 못 받고 종전 경로로 새어 다시 already_alive 가 된다.
    //    ★파괴는 여기서 일어나지 않는다: 아래 run_boot 은 이 등급에서 스폰도 회수도 하지
    //      않는다(관측·보고만). 오살이 오탐보다 훨씬 비싸다는 이 저장소의 규약 그대로다.
    if cys::gate_pending_from_wire(&s[cys::GATE_PENDING_KEY]) {
        return (
            SeatLiveness::GatePending,
            "첫기동 관문 보류(gate_pending — 프로세스 생존·입력 불가)",
        );
    }
    // ③ agent_alive — 프로세스 생존. **각성은 아니다**(B6) 그러나 재스폰 금지 대상이다.
    if s["agent_alive"].as_bool().unwrap_or(false) {
        return (SeatLiveness::AlivePresumed, "agent_alive(각성 미확인)");
    }
    // ④ 좌석(커널 사실). ★필드 부재(구 데몬)와 "unknown"(프로브 실패)을 **융합하지 않는다**:
    //    부재는 '이 차원 무신호' → 아래로 흘러 absent(구 동작), unknown 은 이원 규칙 대상.
    match s["seat"].as_str() {
        Some("occupied") => (SeatLiveness::AlivePresumed, "좌석 점유(자손 프로세스)"),
        Some("unknown") => (SeatLiveness::Unknown, "좌석 판정 불가(프로브 실패)"),
        _ => (SeatLiveness::Absent, "좌석 비었음/무신호"),
    }
}

/// ★★(W2 안전 게이트 · 치명위험 ④ 차단) 이 좌석의 **죽음이 확정**됐는가 —
/// 파괴적/침습적 복구(node-recover 의 pane 주입, reclaim 의 kill)를 허용할 유일한 조건.
///
/// **왜 별도 술어인가(내가 만든 결함의 수리)**: `seat_liveness` 의 `Absent` 는 세 가지 다른 사실이
/// 모여 있다 — ⓐ좌석이 명시적으로 비었다(`seat=="empty"`) ⓑ좌석 판정이 불가해 시한부로 결손 취급
/// 했다(Unknown 해소) ⓒ구 데몬이라 좌석 차원 자체가 없다. ⓑ·ⓒ에 침습적 복구를 걸면
/// **살아있는 팀을 파괴**한다:
///   · 냉시작 데몬은 watchdog 첫 틱(5s) 전까지 전 좌석이 Unknown 이고, GUI 는 앱 시작 즉시
///     `spawn_orchestra_boot` 를 쏜다 → 건강한 전 pane 이 '결손'으로 보인다.
///   · `run_node_recover` 는 `agent_alive == Some(true)` 만 거부한다 — watchdog 이 아직 자손을
///     관측하지 못한 **정상 기동 중** 노드는 `Some(false)` 라 통과해, 돌고 있는 claude 입력창에
///     `C-u` + 기동 커맨드를 밀어 넣는다(화면 파괴·중복 기동).
///   · 그 뒤 reclaim 은 kill 이다. 세 좌석에 연쇄하면 '모든 pane 사망(글자 0)'이다.
///
/// 그래서 **3중 AND** 로만 확정한다(보류 우선 원칙 — 산 노드를 죽이는 손해가 비가역):
///   1. `seat == "empty"`  : 데몬 커널 사실이 **명시적으로** 빈 좌석이라고 말한다(Unknown·부재 아님).
///   2. `agent_alive == Some(false)`: launch-agent 로 등록된 노드인데(meta 존재) 그 에이전트가 죽었다.
///      `null`(meta 없음 = 수동 셸·역할만 쥔 pane)은 **대상 아님** — 사용자 셸을 죽이지 않는다.
///      (그 케이스의 정답은 파괴가 아니라 `takeover_empty_seat` 비파괴 승계다.)
///   3. 좌석 나이 > readiness 최대 예산: 방금 만들어진 pane 은 **기동 중일 수 있다**(create → send →
///      set_meta → watchdog 관측 사이의 창). 동시 `cys restore`/다른 boot 와의 레이스를 결정론으로 끊는다.
fn seat_death_confirmed(s: &Value) -> Result<(), String> {
    match s["seat"].as_str() {
        Some("empty") => {}
        other => {
            return Err(format!(
                "좌석 사실이 'empty' 가 아니다(seat={other:?}) — 판정불가·구데몬 무신호에 침습적 복구 금지"
            ))
        }
    }
    if s["agent_alive"].as_bool() != Some(false) {
        return Err(format!(
            "agent_alive={} — meta 없는 pane(수동 셸)이거나 생존 신호가 있다. 파괴 대상 아님",
            s["agent_alive"]
        ));
    }
    // epoch: now_epoch()은 cysd 전용이라 cys.rs 는 chrono 를 쓴다(cys.rs:2882 선례).
    let now = chrono::Local::now().timestamp() as f64;
    let created = s["created_at"].as_f64().unwrap_or(0.0);
    if created <= 0.0 {
        return Err("created_at 미상 — 좌석 나이를 못 재므로 파괴 금지(보류 우선)".into());
    }
    let age = now - created;
    let floor = budget_readiness_max(0, false).as_secs() as f64;
    if !(age > floor) {
        return Err(format!(
            "좌석 나이 {age:.0}s ≤ readiness 예산 {floor:.0}s — 기동 중일 수 있다(레이스 방지)"
        ));
    }
    Ok(())
}

/// ★(W-B1) 래치 부정 파리티 검체 — python `javis_boot_node.latch_death_confirmed`/`node_liveness`
/// 의 래치 배터리(tests/test_seat_latch_negation.py)와 **같은 4상 표**를 Rust 정본에서 실행한다.
/// 두 언어가 같은 좌석을 반대로 판정하면(한쪽 생존·한쪽 사망) A1·B3 클래스 재발이다 —
/// 표의 케이스 태그(CASE-ALL/CASE-A/CASE-B/CASE-C)는 python 검체와 1:1 이고, python 쪽
/// 텍스트 핀이 이 모듈의 존재·태그를 기계 대조한다(짝 소실 = 파리티 검체 붕괴로 검출).
/// ★경계(age == floor) 케이스는 python 에만 있다: Rust `seat_death_confirmed` 는 벽시계를
/// 내부에서 읽어 정확 경계 픽스처가 본질적으로 flaky 다(python 은 now 주입 가능) —
/// 경계 규약(엄격 초과)은 python 검체가, 여유 마진 케이스(young)는 양쪽이 잰다.
#[cfg(test)]
mod seat_latch_negation_tests {
    use super::*;

    /// 픽스처 — 죽음 3중 확정(전부 참) 좌석. 개별 케이스는 여기서 한 항씩만 부정한다
    /// (한 번에 한 항: 실패 시 어느 항의 회귀인지 즉시 귀속되게).
    fn dead_seat() -> Value {
        let now = chrono::Local::now().timestamp() as f64;
        serde_json::json!({
            "role": "cso", "exited": false, "awakened_at": 1000.0,
            "seat": "empty", "agent_alive": false,
            "created_at": now - 3600.0,   // readiness 예산(60s) 대비 60배 마진 — 벽시계 틱 무관
        })
    }

    #[test]
    fn case_all_true_latch_negated_to_absent() {
        // CASE-ALL: 3중 AND 전부 참 → 래치 부정 → Absent(회수 체인 인계 — 거짓 already_alive 소멸).
        let s = dead_seat();
        assert!(seat_death_confirmed(&s).is_ok(), "3중 확정 좌석이 죽음 미확정으로 접힘");
        let (grade, why) = seat_liveness(&s);
        assert_eq!(grade, SeatLiveness::Absent, "래치가 죽음 확정 좌석을 계속 각성확정으로 유지: {why}");
        assert!(why.contains("래치 부정"), "부정 사유가 판정 이유에 남지 않음: {why}");
    }

    #[test]
    fn case_a_seat_not_empty_holds_latch() {
        // CASE-A: ⓐ 부정 — seat="unknown"(프로브 실패)·필드 부재(구 데몬)는 절대 트리거 금지.
        for a in [serde_json::json!("unknown"), serde_json::json!("occupied"), Value::Null] {
            let mut s = dead_seat();
            if a.is_null() {
                s.as_object_mut().unwrap().remove("seat");
            } else {
                s["seat"] = a;
            }
            assert!(seat_death_confirmed(&s).is_err(), "seat!=empty 인데 죽음 확정");
            assert_eq!(seat_liveness(&s).0, SeatLiveness::AwakeConfirmed,
                       "seat!=empty 에서 래치가 무효화됨(콜드스타트 전 팀 결손 오판 경로)");
        }
    }

    #[test]
    fn case_b_agent_alive_not_explicit_false_holds_latch() {
        // CASE-B: ⓑ 부정 — **null(관측 미도달·meta 부재)이 핵심**이다: claim-role 관측 등록이
        // #[cfg(unix)] 라 Windows master 좌석은 agent_alive 가 영구 null — 여기서 래치가 무효화되면
        // 매 check 결손 오판 → node-recover 가 살아있는 master 입력창에 주입 → 치명 앵커 ④.
        for b in [Value::Null, serde_json::json!(true)] {
            let mut s = dead_seat();
            if b.is_null() {
                s["agent_alive"] = Value::Null; // JSON null = 3상의 '관측 미도달'
            } else {
                s["agent_alive"] = b;
            }
            assert!(seat_death_confirmed(&s).is_err(), "agent_alive!=false 인데 죽음 확정(Windows master 오살 경로)");
            assert_eq!(seat_liveness(&s).0, SeatLiveness::AwakeConfirmed,
                       "agent_alive 명시적 false 아님(null/true)에서 래치가 무효화됨");
        }
    }

    #[test]
    fn case_c_young_or_unknown_age_holds_latch() {
        // CASE-C: ⓒ 부정 — 갓 만든 좌석(기동 중 레이스)·created_at 미상은 래치 유지(보류 우선).
        let now = chrono::Local::now().timestamp() as f64;
        let mut young = dead_seat();
        young["created_at"] = serde_json::json!(now - 1.0); // 예산 60s 대비 59s 마진 — 틱 무관
        assert!(seat_death_confirmed(&young).is_err(), "기동 중 좌석(나이<예산)인데 죽음 확정(레이스)");
        assert_eq!(seat_liveness(&young).0, SeatLiveness::AwakeConfirmed,
                   "기동 중 좌석에서 래치가 무효화됨");
        let mut unknown_age = dead_seat();
        unknown_age.as_object_mut().unwrap().remove("created_at");
        assert!(seat_death_confirmed(&unknown_age).is_err(), "created_at 미상인데 죽음 확정(보류 우선 위반)");
        assert_eq!(seat_liveness(&unknown_age).0, SeatLiveness::AwakeConfirmed,
                   "나이 측정 불가 좌석에서 래치가 무효화됨");
    }

    #[test]
    fn latch_only_seat_stays_awake_and_no_latch_flow_unchanged() {
        // 무회귀 핀 2종: ①래치 단독(seat·agent_alive·created_at 무신호 — H-PRED-3 ⓐ 픽스처)은
        // 여전히 각성 확정. ②래치 없는 죽은 좌석의 기존 결론(Absent)은 부정 로직과 무관하게 불변.
        let latched_only = serde_json::json!({"role": "cso", "exited": false, "awakened_at": 1.0});
        assert_eq!(seat_liveness(&latched_only).0, SeatLiveness::AwakeConfirmed,
                   "래치 단독 좌석이 각성확정을 잃음(legacy 계약 회귀)");
        let mut no_latch = dead_seat();
        no_latch.as_object_mut().unwrap().remove("awakened_at");
        assert_eq!(seat_liveness(&no_latch).0, SeatLiveness::Absent,
                   "래치 없는 빈 좌석의 기존 Absent 결론이 변형됨");
    }

    /// ★(U-10) 좌석 제4 등급 `gate_pending` 배터리 — python `javis_boot_node.node_liveness`
    /// 미러와 **같은 4상 표**를 Rust 정본에서 실행한다(케이스 태그가 python 검체와 1:1).
    ///
    /// 무엇을 봉인하는가: ①관문에 갇힌 좌석이 `AlivePresumed` 로 접혀 `already_alive` 가 되지
    /// 않는다 ②`null`·키 부재·비 object 는 **무신호**로 접혀 종전 판정 그대로다(구 데몬 혼재
    /// 안전) ③이 등급이 **파괴 경로를 새로 열지 않는다**(`seat_death_confirmed` 3중 AND 무접촉).
    #[test]
    fn gate_case_all_gated_seat_is_not_already_alive() {
        // CASE-GATE-ALL: 살아 있는 관문 좌석(agent_alive=true ∧ seat=occupied) + gate_pending object.
        //   종전 술어라면 ②agent_alive 분기에서 AlivePresumed → run_boot 이 already_alive 로 접었다.
        let s = serde_json::json!({
            "role": "cso", "exited": false, "agent_alive": true, "seat": "occupied",
            "gate_pending": {"gate": "disclaimer", "since": 1.0},
        });
        let (grade, why) = seat_liveness(&s);
        assert_eq!(grade, SeatLiveness::GatePending,
                   "관문 보류 좌석이 제4 등급을 받지 못함(허위 already_alive 경로): {why}");
        assert!(!matches!(grade, SeatLiveness::AwakeConfirmed | SeatLiveness::AlivePresumed),
                "관문 보류가 run_boot 의 already_alive 집합에 들어감(이 단위의 존재 이유 소멸)");
    }

    #[test]
    fn gate_case_a_null_or_missing_is_no_signal_not_negation() {
        // CASE-GATE-A: null·키 부재 = '축 미도입/무신호'. **NOT-gated 가 아니라 항 생략**이다 —
        //   래치의 '부재≠부정' 규약과 동형. 구 데몬(키 없음) + 신 CLI 혼재의 안전 근거.
        let mut base = serde_json::json!({
            "role": "cso", "exited": false, "agent_alive": true, "seat": "occupied",
        });
        assert_eq!(seat_liveness(&base).0, SeatLiveness::AlivePresumed,
                   "구 데몬(키 부재)에서 종전 등급이 바뀌었다(혼재 안전 붕괴)");
        base["gate_pending"] = Value::Null;
        assert_eq!(seat_liveness(&base).0, SeatLiveness::AlivePresumed,
                   "null 이 종전 등급을 바꿨다(항 생략 규약 위반)");
    }

    #[test]
    fn gate_case_b_non_object_folds_to_prior_grade() {
        // CASE-GATE-B: 비 object non-null(스큐·손상)은 무신호로 접는다(fail-open → 종전 동작).
        //   'gated' 로 접으면 판정불가가 미충족을 만들어 부트 재시도 라이브락(A1)이 된다.
        for bad in [serde_json::json!(true), serde_json::json!("gated"), serde_json::json!(1),
                    serde_json::json!([])] {
            let s = serde_json::json!({
                "role": "cso", "exited": false, "agent_alive": true, "seat": "occupied",
                "gate_pending": bad,
            });
            assert_eq!(seat_liveness(&s).0, SeatLiveness::AlivePresumed,
                       "손상 gate_pending 값이 등급을 움직였다(fail-open 방향 위반): {s}");
        }
    }

    #[test]
    fn gate_case_c_destruction_path_is_frozen() {
        // CASE-GATE-C: 파괴 경로 **동결** 단언 — 이 단위는 `seat_death_confirmed` 3중 AND 를
        //   건드리지 않는다. 살아 있는 관문 좌석은 그 게이트를 통과할 수 없고(agent_alive=true·
        //   seat=occupied), 반대로 여기에 새 hold 항을 더하면 stale 보류가 reclaim 을 영구
        //   마비시킨다(A1 역방향). 그래서 '무접촉'이 정답이고, 그 사실을 기계로 고정한다.
        let gated = serde_json::json!({
            "role": "cso", "exited": false, "agent_alive": true, "seat": "occupied",
            "created_at": 1.0,
            "gate_pending": {"gate": "trust", "since": 1.0},
        });
        assert!(seat_death_confirmed(&gated).is_err(),
                "살아 있는 관문 좌석이 죽음 확정으로 읽힘(치명위험 ④ — 오살 경로 신설)");
        // 래치 보유 좌석은 관문 신호가 있어도 여전히 각성 확정이다 — 래치 단방향 계약 무접촉
        // (금지 방향 ⑦). 이 잔여(각성 이력 좌석의 후발 관문)는 U-11 이 다룰 사안이고,
        // **오늘과 같은 결과**이므로 새 사망 경로가 아니다.
        let mut latched = gated.clone();
        latched["awakened_at"] = serde_json::json!(1000.0);
        assert_eq!(seat_liveness(&latched).0, SeatLiveness::AwakeConfirmed,
                   "래치 단방향 계약이 관문 신호로 뒤집혔다(금지 방향 ⑦ 위반)");
    }

    #[test]
    fn readiness_budget_parity_pin() {
        // ⓒ항 임계의 언어 간 파리티 핀 — python `javis_budget.launch_readiness_max_s()` 기본값과
        // 같은 수(60s)여야 한다. 이 수가 갈리면 같은 좌석 나이를 한쪽은 '기동 중', 한쪽은
        // '죽음 확정'으로 읽는다(python 검체 test_seat_latch_negation.py 가 같은 60 을 단언).
        assert_eq!(budget_readiness_max(0, false).as_secs(), 60,
                   "readiness 예산 기본 산식(max(0,30)×2)이 움직임 — python leaf 와 동시 이동 필요");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // ★(U-11) readiness 판정 타입화 — **축④(readiness) 실측 배터리**
    //
    //   S-4(좌석 생존 판정 샷건 서저리)의 4벌 중 마지막 한 벌이 이 축이다. python 검체
    //   (H-SEAT-4AXIS)는 축①②③을 실행으로 재고, **이 축은 Rust 안에만 있으므로** 여기서
    //   진리표로 잰다. python 검체는 이 배터리의 실재와 배선 계약을 소스 핀으로 대조한다.
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn gate_verdict_truth_table_is_decided_by_the_kernel_fact() {
        // 커널 사실 하나가 '파괴 가능'과 '보류'를 가른다 — 화면 문자열은 근거가 아니다.
        let tail = "❯ 2. Yes, I accept";
        assert!(matches!(readiness_timeout_verdict(Some(true), "claude", 60, tail, None),
                         BootVerdict::GatePending { .. }),
                "생존이 관측된 좌석을 실패로 판정 — 오살 경로(치명위험 ④) 신설");
        assert!(matches!(readiness_timeout_verdict(Some(false), "claude", 60, tail, None),
                         BootVerdict::LaunchFailed { .. }),
                "커널이 부재를 확정했는데 보류로 접힘 — 진짜 실패 좌석이 역할을 쥔 채 쌓인다");
        // ★'부재 ≠ 부정': 필드 부재(구 데몬)·조회 실패는 **판정 불가**다. 사망으로 접으면
        //   그 자체가 새 파괴 경로다(래치·gate 축의 null 규약과 동형).
        assert!(matches!(readiness_timeout_verdict(None, "claude", 60, tail, None),
                         BootVerdict::GatePending { .. }),
                "판정 불가를 사망으로 접었다(부재≠부정 규약 위반)");
    }

    #[test]
    fn gate_verdict_carries_the_screen_tail_as_evidence_only() {
        // 화면 꼬리는 **진단 근거**로만 실린다 — 판정은 이미 커널 사실이 끝냈다.
        let tail = "line-a\nline-b";
        match readiness_timeout_verdict(Some(true), "claude", 60, tail, None) {
            BootVerdict::GatePending { gate, tail: t } => {
                assert_eq!(t, tail, "처방 문안이 근거 없이 나간다");
                // 어느 관문인지는 이 단위가 알지 못한다(관문 코퍼스는 뒤 단위 소유) —
                // 모르는 것을 아는 척하지 않는다.
                assert_eq!(gate, "unknown", "판정하지 않은 관문 이름을 단정했다");
            }
            other => panic!("생존 좌석이 보류로 읽히지 않음: {other:?}"),
        }
    }

    /// ★N2 — 관문 id 를 **알면서 버리지 않는다**(`gate=unknown` 자리표시자 회귀 차단).
    ///
    /// 【무엇이 틀렸었는가】 이 판정기는 U-11 시절 "어느 관문인지 이 단위는 모른다"는 전제로
    /// 리터럴 `"unknown"` 을 반환했다. U-12(코퍼스 해소)·U-13(관문 축 ready 판정)이 착지한 뒤
    /// 그 전제는 깨졌다 — **같은 실행의 폴링 루프가 바로 앞 줄에서 `id=theme` 를 찍고 있었다.**
    /// 그런데 판정기는 그 사실을 받지 않았고, 좌석 표식(`mark_gate_pending`)·사람 처방
    /// (`print_gate_pending_prescription`)·상위 관측이 전부 `unknown` 으로 굳었다. 그러면
    /// 관문별 `AbsenceCost`(theme=가역 vs 면책=비가역)를 **어디에서도 소비할 수 없다**.
    #[test]
    fn gate_verdict_carries_the_identified_gate_id_not_a_placeholder() {
        let tail = "❯ 2. Yes, I accept";
        // ⓐ 폴링이 관문을 식별했으면 그 id 가 그대로 실린다.
        match readiness_timeout_verdict(Some(true), "claude", 60, tail, Some("bypass-disclaimer")) {
            BootVerdict::GatePending { gate, .. } => assert_eq!(
                gate, "bypass-disclaimer",
                "알고 있던 관문 id 를 버리고 자리표시자를 실었다 — 좌석 표식·처방·상위 관측이 \
                 전부 unknown 으로 굳고 부재의 비용(AbsenceCost)을 소비할 수 없다"
            ),
            other => panic!("생존 좌석이 보류로 읽히지 않음: {other:?}"),
        }
        // ⓑ 식별이 **없었으면** 모르는 것을 아는 척하지 않는다(빈 문자열도 미식별이다).
        for none_ish in [None, Some("")] {
            match readiness_timeout_verdict(Some(true), "claude", 60, tail, none_ish) {
                BootVerdict::GatePending { gate, .. } => assert_eq!(
                    gate, GATE_ID_UNIDENTIFIED,
                    "판정하지 않은 관문 이름을 단정했다"
                ),
                other => panic!("생존 좌석이 보류로 읽히지 않음: {other:?}"),
            }
        }
        // ⓒ ★배선 핀 — 부트 폴링이 **실제로 관측한 id** 를 넘긴다. 순수 함수만 고치고 호출부가
        //    상수를 넘기면 ⓐ 는 아무것도 지키지 못한다(이 저장소가 반복해 맞은 형태).
        let src = include_str!("cys.rs");
        let prod = &src[..src.find("\n#[cfg(test)]\nmod tests {").expect("테스트 모듈 경계")];
        let ci = prod
            .find("readiness_timeout_verdict(surface_agent_alive(sid)")
            .expect("부트 폴링의 판정기 호출부가 사라졌다 — 배선 핀이 눈이 멀었다");
        // ★char 경계 안전 절단 — 이 파일의 주석은 한글이라 바이트 슬라이스는 패닉한다
        //   (계측기가 판정 대신 패닉으로 죽으면 그것도 초록이 아니라 고장이다).
        let call: String = prod[ci..].chars().take(400).collect();
        assert!(
            call.contains("gate_logged.as_deref()"),
            "폴링이 관측한 관문 id 를 판정기에 넘기지 않는다(U-11 자리표시자 회귀):\n{call}"
        );
    }

    #[test]
    fn gate_verdict_rollback_switch_demotes_at_exactly_one_point() {
        // 롤백 = 이 단위 착지 **이전과 완전히 같은 동작**(무조건 close). 강등은 순수 매핑 1개다.
        let gated = BootVerdict::GatePending { gate: "unknown".into(), tail: "t".into() };
        assert_eq!(boot_verdict_effective(gated.clone(), false), gated,
                   "기본값에서 보류가 강등됐다(신동작 소실)");
        match boot_verdict_effective(gated, true) {
            BootVerdict::LaunchFailed { evidence } => {
                assert!(evidence.contains(cys::ENV_GATE_PENDING_CLOSE),
                        "강등 사유에 롤백 env 이름이 없다 — 사용자가 왜 닫혔는지 알 수 없다");
            }
            other => panic!("롤백 스위치가 보류를 강등하지 못함: {other:?}"),
        }
        // 나머지 두 변형은 스위치와 무관하다(롤백이 성공·실패의 의미를 흔들면 안 된다).
        assert_eq!(boot_verdict_effective(BootVerdict::Ready, true), BootVerdict::Ready);
        let failed = BootVerdict::LaunchFailed { evidence: "e".into() };
        assert_eq!(boot_verdict_effective(failed.clone(), true), failed);
    }

    #[test]
    fn gate_verdict_exit_code_is_neither_success_nor_failure() {
        // 0 이면 소비부가 '노드를 세웠다'로 읽어 디렉티브·티켓을 태우고(그 주입 Return 이 실측상
        // 면책 창의 종료 버튼을 누른다), 1 이면 '깨졌다'로 읽어 살아 있는 좌석을 회수·파괴한다.
        assert_ne!(cys::EXIT_GATE_PENDING, 0, "보류가 성공으로 접혔다");
        assert_ne!(cys::EXIT_GATE_PENDING, 1, "보류가 실패로 접혔다");
        // ★핀 이사(M3 · 2026-08-24): 종전 이 자리에는 "`cys boot` 의 bare exit 계약(0/1/75)은
        //   **불변**이다 — 보류는 outcome 채널로만 흐른다" 고 적혀 있었다. 그 설계가 곧 결함이었다:
        //   의무 4좌석 전량이 관문 보류일 때 bare exit 이 **0** 이라 소비부가 '팀을 세웠다' 로
        //   읽었다(회전2 실주행). 보류는 이제 **outcome 채널 + 전용 종료코드 78** 둘 다로 흐른다.
        //   판정 조건은 약해지지 않았다 — 0 은 '보류도 없음' 에서만 나온다(아래 3항).
        assert_eq!(boot_exit_code(0, 0, false), 0, "보류·실패 0인데 성공이 아니다");
        assert_eq!(boot_exit_code(1, 0, false), 1);
        assert_eq!(boot_exit_code(0, 0, true), EXIT_BOOT_BUSY);
        // ★의무 관문 보류 = 전용 78(성공도 실패도 아니다).
        assert_eq!(
            boot_exit_code(0, 1, false),
            cys::EXIT_GATE_PENDING,
            "의무 역할이 관문에 갇혔는데 exit 0 — 소비부가 '팀을 세웠다'로 읽는다(M3)"
        );
        assert_ne!(boot_exit_code(0, 1, false), 0, "보류가 성공으로 접혔다");
        // 더 나쁜 상태(좌석 자체가 없음)가 이긴다 — 처방이 다르므로 뭉개면 안 된다.
        assert_eq!(boot_exit_code(1, 1, false), 1, "Fatal 이 보류에 가려졌다");
        assert_eq!(boot_exit_code(0, 3, true), EXIT_BOOT_BUSY, "busy 가 보류에 가려졌다");
    }

    /// ★★M1 검체(2026-08-24) — **미관측 상태에서 close 로 흐르지 않는다.**
    ///
    /// wire 행(`agent_alive`) → `surface_agent_alive_in` → `readiness_timeout_verdict` 까지
    /// **한 실행으로 관통**한다. 순수 술어만 따로 재면 "데몬이 실제로 null 을 내는가" 와
    /// "CLI 가 그 null 을 보류로 옮기는가" 사이의 이음매가 무검체로 남는다 — 이 저장소가
    /// 반복해 맞은 자리다.
    ///
    /// 【적색 증명(in-band)】 ⓐ′ 가 **종전 데몬 산출**(미관측 = `false`)을 같은 행에 실어 같은
    /// 사슬에 태운다. 그 행은 `LaunchFailed`(= close)로 흐른다 — 고친 것이 실재하는 파괴
    /// 경로였음을 같은 실행이 증명한다.
    #[test]
    fn unobserved_agent_never_flows_to_close() {
        const SID: u64 = 7;
        let row = |alive: Value| {
            vec![json!({"surface_id": SID, "agent": "claude", "agent_alive": alive})]
        };
        let verdict = |alive: Value| {
            readiness_timeout_verdict(
                surface_agent_alive_in(&row(alive), SID),
                "claude",
                90,
                "tail",
                None,
            )
        };

        // ⓐ ★핵심 — 미관측(null)은 **보류**다(좌석 보존 · close 0 · kill 0 · 주입 0).
        assert!(
            matches!(verdict(Value::Null), BootVerdict::GatePending { .. }),
            "미관측 좌석이 보류로 가지 않는다 — 살아 있는 좌석이 close 된다(재난 ④)"
        );
        // ⓐ′ 적색 증명 — 종전 데몬은 같은 상태를 `false` 로 냈고, 그 값은 close 로 흐른다.
        assert!(
            matches!(verdict(json!(false)), BootVerdict::LaunchFailed { .. }),
            "계측 무효: `false` 가 close 로 흐르지 않는다면 M1 은 결함이 아니다"
        );
        // 키가 아예 없는 행(구 데몬)도 같은 보류 — '부재 ≠ 부정'.
        assert!(matches!(
            readiness_timeout_verdict(
                surface_agent_alive_in(&[json!({"surface_id": SID})], SID),
                "claude",
                90,
                "tail",
                None
            ),
            BootVerdict::GatePending { .. }
        ));
        // 좌석 행 자체가 없어도(조회 실패) 보류다.
        assert!(matches!(
            readiness_timeout_verdict(surface_agent_alive_in(&[], SID), "claude", 90, "tail", None),
            BootVerdict::GatePending { .. }
        ));
        // ⓑ 관측 생존 — 종전대로 보류.
        assert!(matches!(verdict(json!(true)), BootVerdict::GatePending { .. }));

        // ⓒ ★파괴 문안이 **근거를 정직하게** 말한다(종전 문안 "데몬이 agent 프로세스 부재를
        //    관측했다" 는 이름 매칭 미성립에서도 나왔으므로 거짓이었다).
        let BootVerdict::LaunchFailed { evidence } = verdict(json!(false)) else {
            panic!("close 분기가 아니다");
        };
        assert!(
            !evidence.contains("데몬이 agent 프로세스 부재를 관측했다"),
            "거짓 근거 문안이 남아 있다: {evidence}"
        );
        assert!(
            evidence.contains("agent_seen") && evidence.contains("agent_exit_notified"),
            "close 근거가 관측 전이를 지목하지 않는다(사람이 조치를 고를 수 없다): {evidence}"
        );
        assert!(
            evidence.contains("미관측"),
            "미관측이 이 분기로 오지 않는다는 사실이 문안에 없다: {evidence}"
        );
    }

    /// ★★M2 검체(2026-08-24) — **관문 통과 → 재부트 → 주입**. 보류에서 빠져나오는 길이 있다.
    ///
    /// 【고치는 결함】 `run_boot` 의 `gate_pending` 분기는 출력 후 `continue` 뿐이었다.
    /// `clear_gate_pending` 도 디렉티브 주입도 `boot_agent_on_surface` 안에만 있었으므로,
    /// 사람이 처방대로 관문을 통과시키고 `cys boot` 을 다시 실행해도 그 좌석은 **다시 보류로
    /// 건너뛰어졌고** 절대지침은 한 번도 주입되지 않았다. 30분 뒤 TTL 이 표식을 접으면
    /// `orchestra check` 가 그 좌석을 충족으로 세어 **exit 0 = READY** 를 냈다(R1 의 타이머 재발).
    ///
    /// 여기서는 재관측 **판정**을 프로덕션 경로(`readiness::judge` → `gate_pending_recheck`)로
    /// 관통시켜 세 갈래를 전부 잰다. 배선(분기가 실제로 재관측·채택을 부르는가)은 아래
    /// `gate_pending_branch_has_an_escape_path_source_pin` 이 소스로 못 박는다.
    #[test]
    fn passed_gate_is_adopted_on_reboot_while_a_held_gate_stays_pending() {
        let gates = cys::first_run_gates::builtin();
        // 재관측의 관측 재료를 그대로 재현한다(델타 없음 · 시간 폴백 지남 — `gate_pending_reobserve`).
        let obs = |screen: &'static str, alive: Option<bool>| cys::readiness::Observed {
            site: cys::readiness::Site::Boot,
            agent_alive: alive,
            screen,
            delta: "",
            marker: Some("❯"),
            gates: &gates,
            tail_is_shell_prompt: Some(screen_tail_is_shell_prompt_on(screen, false)),
            bare_shell: Some(screen_is_bare_shell_on(screen, false)),
            time_fallback_reached: true,
            idle_quiet: None,
            legacy_v1: false,
        };
        let recheck =
            |screen: &'static str, alive: Option<bool>| gate_pending_recheck(cys::readiness::judge(&obs(screen, alive)));

        // ① 관문이 **아직 떠 있다**(실측 킬체인 화면 — 면책 창) — 보류 유지(주입 0 · 스폰 0 · 파괴 0).
        let held = cys::first_run_gates::fixtures::TRUST_ECHO_THEN_DISCLAIMER;
        assert!(
            cys::first_run_gates::identify(&gates, held).is_some(),
            "드릴 전제 붕괴: 이 화면이 관문으로 식별되지 않는다"
        );
        assert!(
            matches!(recheck(held, Some(true)), GateRecheck::StillHeld { .. }),
            "관문이 떠 있는데 채택으로 갔다 — 관문 창에 디렉티브가 주입된다(면책 창이면 좌석 사망)"
        );

        // ② 사람이 **통과시켰다** — 살아있는 TUI 가 입력 프롬프트를 그리고 있다.
        //    ★이것이 M2 가 여는 경로다: 종전에는 이 화면에서도 `continue` 뿐이었다.
        let passed = cys::first_run_gates::fixtures::LIVE_TUI_AT_PROMPT;
        assert!(
            matches!(recheck(passed, Some(true)), GateRecheck::Adopt(_)),
            "관문을 통과한 좌석이 채택되지 않는다 — 절대지침이 영영 주입되지 않는다(M2 의 결함)"
        );

        // ③ 커널 생존이 **판정 불가**(M1 의 그 상태)여도 파괴로 승격하지 않는다 —
        //    이 자리의 산출은 Adopt 아니면 NoEvidence 이고, 어느 쪽도 close·kill 이 아니다.
        assert!(
            !matches!(recheck(passed, None), GateRecheck::StillHeld { .. }),
            "관문 부재 화면에서 관문 보류가 나왔다(판정 이원화)"
        );
        // ④ 맨 셸(에이전트가 죽고 셸만 남음) — 채택하지 않는다(죽은 셸 주입 차단).
        assert_eq!(
            recheck("user@mac cys-terminal-rel %", Some(true)),
            GateRecheck::NoEvidence,
            "맨 셸에 디렉티브를 주입한다"
        );
    }

    /// ★★M2 배선 핀 — 보류 분기가 **탈출 경로를 실제로 부른다.**
    /// 판정만 고치고 `run_boot` 이 종전처럼 `continue` 만 하면 이 단위는 통째로 무력화된다.
    #[test]
    fn gate_pending_branch_has_an_escape_path_source_pin() {
        let src = include_str!("cys.rs");
        // ★이 파일은 프로덕션과 테스트 모듈이 **번갈아** 배치돼 있어 단일 경계로는 자를 수 없다.
        //   그래서 **열 0 의 `fn` 정의부**를 앵커로 함수 슬라이스를 직접 잡는다(테스트 안의
        //   같은 이름 언급은 들여쓰기돼 있어 걸리지 않는다).
        let fn_body = |name: &str| -> &str {
            let head = format!("\nfn {name}(");
            let i = src
                .find(&head)
                .unwrap_or_else(|| panic!("{name} 이 사라졌다(열 0 정의부 부재)"));
            let rest = &src[i + 1..];
            let end = rest
                .find("\n}\n")
                .map(|e| e + 2)
                .expect("함수 끝(열 0 닫는 중괄호)을 찾지 못했다");
            &rest[..end]
        };
        let body = fn_body("run_boot");
        for anchor in [
            // 스폰 0 의 재관측을 부른다.
            "gate_pending_reobserve(sid, agent)",
            // Ready 면 표식 해제 + 디렉티브 주입(판정 이후 절반 재사용).
            "gate_pending_adopt(sid, role, agent)",
        ] {
            assert!(
                body.contains(anchor),
                "관문 보류 분기의 탈출 경로가 끊겼다 — 앵커 부재: {anchor}"
            );
        }
        // 채택 경로가 **주입 절반**을 경유한다(사본으로 갈라지지 않는다).
        assert!(
            fn_body("gate_pending_adopt").contains("inject_directive_after_ready("),
            "채택이 주입 절반을 경유하지 않는다 — 주입 경로가 둘로 갈라졌다"
        );
        // ★재관측은 **스폰 0** 이다 — 기동 send 를 부르면 살아있는 입력창이 파괴된다(재난 ④).
        let reobserve = fn_body("gate_pending_reobserve");
        assert!(
            reobserve.contains("surface.read_text") || reobserve.contains("gate_guard_screen("),
            "재관측이 화면을 읽지 않는다 — 잴 것이 없다"
        );
        for forbidden in ["surface.send_text", "surface.send_key", "surface.create"] {
            assert!(
                !reobserve.contains(forbidden),
                "재관측이 좌석에 쓴다({forbidden}) — 관측만 해야 하는 경로다"
            );
        }
        // ★낡은 주석 정정 확인(M2): "이 단위에는 생산자가 없어 실제로는 나오지 않는다" 는
        //   U-11 이 생산자를 만든 뒤로 거짓이고, 그 문장이 이 분기가 재방문되지 않은 증거였다.
        assert!(
            !body.contains("이 단위에는 생산자가 없어 실제로는 나오지 않는다"),
            "낡은 주석이 남아 있다 — 다음 감사자가 이 분기를 '도달 불가' 로 읽고 건너뛴다"
        );
    }

    /// ★★M3 검체(2026-08-24) — **버킷 합 = roles 길이.**
    ///
    /// 【고치는 결함】 종전 summary 는 버킷을 손으로 세었고 typed outcome 7종 중
    /// `gate_pending`·`skipped_unconfirmed` **둘에 버킷이 없었다**. 회전2 실주행에서 로스터 5개
    /// 중 4개가 gate_pending 인데 **버킷 합은 1** 이었고, 그 차이는 어떤 채널에도 나타나지 않았다.
    #[test]
    fn boot_summary_buckets_cover_every_typed_outcome() {
        // ① 프로덕션 소스에서 **실제로 발행되는 outcome 값 전량**을 뽑는다(목록을 손으로
        //    복사하면 다음 outcome 이 추가될 때 이 핀이 조용히 낡는다 — 그것이 이 결함의 형태다).
        let src = include_str!("cys.rs");
        // 열 0 의 run_boot 정의부부터 열 0 의 닫는 중괄호까지 = 발행부 전량.
        // ★앵커는 **런타임 조립**한다 — 이 파일 안에 프로덕션 앵커 문면이 그대로 있으면
        //   소스 스캔 검체(여기와 팩 헬스 러너 양쪽)가 **테스트 코드에 먼저 앵커링**된다.
        let head = format!("\nfn {}(", "run_boot");
        let i = src.find(&head).expect("run_boot 정의부가 사라졌다") + 1;
        let rest = &src[i..];
        let body = &rest[..rest.find("\n}\n").expect("run_boot 끝을 찾지 못했다") + 2];
        let mut outcomes: Vec<String> = Vec::new();
        let needle = "\"outcome\": \"";
        let mut at = 0usize;
        while let Some(k) = body[at..].find(needle) {
            let s = at + k + needle.len();
            let e = s + body[s..].find('"').expect("outcome 리터럴이 닫히지 않았다");
            outcomes.push(body[s..e].to_string());
            at = e;
        }
        outcomes.sort();
        outcomes.dedup();
        assert!(
            outcomes.len() >= 7,
            "발행 outcome 을 {}종밖에 못 찾았다 — 추출이 깨졌다: {outcomes:?}",
            outcomes.len()
        );

        // ② 모든 outcome 이 **자기 버킷**을 갖는다(`unbucketed` 로 새면 그것도 결함이다).
        //    `busy` 만 예외다 — 그것은 **락 경합 조기 종료**(무스폰)의 값이라 roles 루프를 한 번도
        //    돌지 않는다. 그래도 summary 키는 실재해야 소비부가 두 경로를 같은 스키마로 읽는다.
        assert!(
            outcomes.iter().any(|o| o == "busy"),
            "busy 조기 종료 경로가 사라졌다 — 이 예외 처리의 전제가 무너졌다"
        );
        assert_eq!(
            boot_summary_buckets(&[])["busy"],
            json!(0),
            "정상 종료 summary 에 busy 키가 없다(스키마 이원화)"
        );
        for o in outcomes.iter().filter(|o| *o != "busy") {
            let roles = vec![json!({"role": "cso", "outcome": o, "mandatory": true})];
            let sum = boot_summary_buckets(&roles);
            assert_eq!(
                sum.get(o.as_str()).and_then(|v| v.as_u64()),
                Some(1),
                "typed outcome `{o}` 에 대응 버킷이 없다 — 그 좌석이 집계에서 사라진다"
            );
            assert_eq!(
                sum["unbucketed"], json!(0),
                "typed outcome `{o}` 가 unbucketed 로 샜다"
            );
        }

        // ③ ★핵심 계약 — **버킷 합 = roles 길이**. 회전2 실주행 로스터로 잰다.
        let live_run: Vec<Value> = vec![
            json!({"role": "cso", "outcome": "gate_pending", "mandatory": true}),
            json!({"role": "worker", "outcome": "gate_pending", "mandatory": true}),
            json!({"role": "reviewer-gemini", "outcome": "gate_pending", "mandatory": true}),
            json!({"role": "reviewer-codex", "outcome": "gate_pending", "mandatory": true}),
            json!({"role": "reviewer-grok", "outcome": "launched", "mandatory": false}),
        ];
        let sum = boot_summary_buckets(&live_run);
        let bucket_sum: u64 = BOOT_SUMMARY_BUCKETS
            .iter()
            .map(|k| sum[*k].as_u64().unwrap_or(0))
            .sum();
        assert_eq!(
            bucket_sum,
            live_run.len() as u64,
            "버킷 합 {bucket_sum} ≠ 로스터 {} — 좌석이 집계에서 사라졌다(실주행: 합 1 vs 로스터 5)",
            live_run.len()
        );
        // 적색 증명(in-band): 종전 버킷 집합(gate_pending·skipped_unconfirmed 없음)으로 세면
        // 같은 로스터의 합이 **1** 이다 — 이 검체가 고치는 결함이 실재한다.
        let legacy_buckets = ["launched", "already_alive", "recovered", "missing", "failed"];
        let legacy_sum: u64 = legacy_buckets
            .iter()
            .map(|k| sum[*k].as_u64().unwrap_or(0))
            .sum();
        assert_eq!(
            legacy_sum, 1,
            "계측 무효: 종전 버킷 집합이 이 로스터에서 합 1 을 내지 않았다면 M3 은 결함이 아니다"
        );
        assert_eq!(sum["gate_pending"], json!(4), "관문 보류가 보고되지 않는다");
        assert_eq!(sum["fatal_gate_pending"], json!(4), "의무 관문 보류 교차 집계가 틀렸다");
        assert_eq!(sum["fatal_failed"], json!(0));

        // ④ 미지 outcome 도 **합을 유지한다**(조용히 사라지지 않는다 — 다음 outcome 방어).
        let unknown = vec![
            json!({"role": "x", "outcome": "some_future_outcome", "mandatory": true}),
            json!({"role": "y", "outcome": "launched", "mandatory": true}),
        ];
        let s2 = boot_summary_buckets(&unknown);
        let sum2: u64 = BOOT_SUMMARY_BUCKETS
            .iter()
            .map(|k| s2[*k].as_u64().unwrap_or(0))
            .sum();
        assert_eq!(sum2, 2, "미지 outcome 이 집계에서 증발했다");
        assert_eq!(s2["unbucketed"], json!(1));

        // ⑤ 빈 로스터 — 전 버킷이 0 이고 키는 전량 존재한다(키 부재 ≠ 0 이원화 차단).
        let s3 = boot_summary_buckets(&[]);
        for k in BOOT_SUMMARY_BUCKETS {
            assert_eq!(s3[*k], json!(0), "빈 로스터에서 버킷 키 {k} 가 사라졌다");
        }
    }
}

/// role → 그 role 을 쥔 비종료 surface 행(없으면 None). worker 는 접두 수용(데몬 dedup: worker-N).
fn find_seat_row<'a>(surfaces: &'a [Value], role: &str) -> Option<&'a Value> {
    surfaces.iter().find(|s| {
        let r = s["role"].as_str().unwrap_or("");
        !s["exited"].as_bool().unwrap_or(true)
            && (r == role || (role == "worker" && r.starts_with("worker")))
    })
}

fn fetch_surfaces() -> Vec<Value> {
    request("surface.list", json!({}))
        .ok()
        .and_then(|r| r["surfaces"].as_array().cloned())
        .unwrap_or_default()
}

/// 플랫폼별 설치 힌트(G29·B8) — 의무 CLI 미설치는 exit 0 성공이 아니라 typed `missing` outcome 이다.
fn install_hint(agent: &str) -> &'static str {
    match agent {
        "claude" => {
            if cfg!(windows) {
                "PowerShell: `irm https://claude.ai/install.ps1 | iex` 후 자비스 재시작"
            } else {
                "`curl -fsSL https://claude.ai/install.sh | bash` 후 새 탭"
            }
        }
        "codex" => "`npm i -g @openai/codex` (선택 리뷰어)",
        "gemini" => "Antigravity CLI `agy` 설치 후 agents.json 의 cmd 경로 확인 (선택 리뷰어)",
        "grok" => "grok CLI 설치 (선택 리뷰어 — 미설치면 건너뜀이 정상)",
        _ => "해당 CLI 설치 후 agents.json 의 cmd 를 확인 (선택 노드)",
    }
}

/// B8: agents.json 의 cmd 가 Windows 실설치 경로와 어긋날 때의 안내 — 후보 전탐색까지 빈손일 때만.
#[cfg(windows)]
const WINDOWS_AGENT_PATH_HINT: &str = "agents.json의 cmd 경로를 실제 설치 경로로 수정하세요 \
(agy: npm i -g @google/antigravity 후 where agy / codex: npm i -g @openai/codex 후 where codex)";

/// 어댑터 설치 감지 결과 — `cys agent-detect`·`run_boot` 이 공유하는 **단일 오라클**의 산출물(CS-1③).
struct AgentDetection {
    /// 지금 이 어댑터를 기동할 수 있는가 = 바이너리 실재 **+ 실행권**.
    installed: bool,
    /// agents.json cmd 에서 env-prefix 를 건너뛴 바이너리 토큰(extract_bin 단일 진실).
    bin: String,
    /// 해소된 실경로(경로형=틸드 확장 후 / PATH형=which·where 첫 줄). 미해소 시 None.
    resolved: Option<std::path::PathBuf>,
    /// 사람용 판정 근거 한 줄 (python detect_reviewer 의 reason 과 동형 문면).
    reason: String,
    /// 미설치 안내 — 기본은 install_hint, Windows 후보 전탐색 실패 시 경로수정 힌트로 대체.
    hint: String,
}

/// 파일이 실재하고 **실행 가능**한가. 종전 `exists()` 판정의 강화 — python 쪽
/// `detect_reviewer`(`os.access(binp, os.X_OK)`)와 오라클 문면을 일치시킨다(재감사 §3 CS-1③).
/// unix 는 실유효 권한(access(2) X_OK)으로 판정하고, Windows 는 실행권 개념이 없어 실재로만 본다.
/// ★무회귀: 실행권 없는 파일은 셸 경유 기동도 EACCES 로 실패하므로, 종전 exists()=true 판정은
///   '기동 가능'의 오탐이었다 — 판정이 좁아지는 방향뿐이고 기동 가능한 대상을 잃지 않는다.
/// ★cfg 를 **함수 두 벌**로 가른다(블록 `#[cfg]` 를 tail expression 으로 쓰면 속성 붙은 블록이
///   statement 로 파싱돼 `if` 가 tail 이 되고 E0317 로 **Windows 빌드만** 깨진다 — 실측 확인).
#[cfg(unix)]
fn is_executable_file(p: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    if !p.is_file() {
        return false;
    }
    match std::ffi::CString::new(p.as_os_str().as_bytes()) {
        Ok(c) => unsafe { libc::access(c.as_ptr(), libc::X_OK) == 0 },
        Err(_) => false,
    }
}

/// Windows 는 실행권(X_OK) 개념이 없다 — 실재로만 판정한다(대칭 구현).
#[cfg(not(unix))]
fn is_executable_file(p: &std::path::Path) -> bool {
    p.is_file()
}

/// PATH 해석 → (발견?, 해소경로). **발견 판정의 권위는 종전과 동일하게 exit status** 이고
/// 경로는 best-effort 다(stdout 파싱 실패가 '미설치' 로 뒤집히지 않게 — 무회귀).
fn which_in_path(bin: &str) -> (bool, Option<std::path::PathBuf>) {
    #[cfg(windows)]
    let prog = "where";
    #[cfg(not(windows))]
    let prog = "which";
    match std::process::Command::new(prog).arg(bin).output() {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            let first = s
                .lines()
                .map(|l| l.trim())
                .find(|l| !l.is_empty())
                .map(std::path::PathBuf::from);
            (true, first)
        }
        _ => (false, None),
    }
}

/// B8: Windows 후보 순회 — 선언 경로·PATH 가 빈손일 때 확장자 변형(.cmd/.exe/.bat/.ps1)과
/// npm 전역 설치 표준 위치(`%APPDATA%\npm`·`%LOCALAPPDATA%\npm`)를 훑는다.
/// (npm -g 는 Windows 에서 `<prefix>\<name>.cmd` 셸 래퍼를 깔고, prefix 기본값이 `%APPDATA%\npm`
///  이다. 일부 설정·설치기는 `%LOCALAPPDATA%` 하위를 prefix 로 쓰므로 함께 본다.)
/// 선언이 unix 경로형(`~/.local/bin/agy`)이어도 Windows 에선 그 경로가 없으므로 **바이너리 이름만**
/// 취해 순회한다 — "OS 별 후보 목록" 미해소로 Windows 부트가 전멸하던 구멍을 메운다.
#[cfg(windows)]
fn windows_agent_candidates(bin: &str) -> Option<std::path::PathBuf> {
    let raw = std::path::Path::new(bin)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| bin.to_string());
    let stem = raw
        .trim_end_matches(".exe")
        .trim_end_matches(".cmd")
        .trim_end_matches(".bat")
        .trim_end_matches(".ps1")
        .to_string();
    if stem.is_empty() {
        return None;
    }
    // ① 확장자 변형을 PATH 로 재해석(`where agy` 는 실패해도 `where agy.cmd` 는 잡히는 경로).
    for ext in ["cmd", "exe", "bat", "ps1"] {
        let cand = format!("{stem}.{ext}");
        let (found, path) = which_in_path(&cand);
        if found {
            return Some(path.unwrap_or_else(|| std::path::PathBuf::from(cand)));
        }
    }
    // ② npm 전역 prefix 후보 직접 탐색(PATH 미갱신 셸 — Windows 설치 직후 흔한 상태).
    for var in ["APPDATA", "LOCALAPPDATA"] {
        let Some(base) = std::env::var_os(var) else {
            continue;
        };
        let npm = std::path::PathBuf::from(base).join("npm");
        for name in [
            format!("{stem}.cmd"),
            format!("{stem}.exe"),
            format!("{stem}.bat"),
            format!("{stem}.ps1"),
            stem.clone(),
        ] {
            let cand = npm.join(&name);
            if is_executable_file(&cand) {
                return Some(cand);
            }
        }
    }
    None
}

/// B8 후보 순회는 **Windows 한정** — 다른 OS 빌드에서는 항등(cfg 분기를 함수 두 벌로 갈라
/// `mut` 미사용 경고 없이 컴파일된다).
#[cfg(not(windows))]
fn apply_windows_agent_fallback(d: AgentDetection) -> AgentDetection {
    d
}

#[cfg(windows)]
fn apply_windows_agent_fallback(mut d: AgentDetection) -> AgentDetection {
    if d.installed {
        return d;
    }
    match windows_agent_candidates(&d.bin) {
        Some(p) => {
            d.reason = format!("{} → Windows 후보 발견 {}", d.reason, p.display());
            d.resolved = Some(p);
            d.installed = true;
        }
        None => {
            d.reason = format!(
                "{} → Windows 후보(.cmd/.exe/.bat/.ps1 · %APPDATA%\\npm · %LOCALAPPDATA%\\npm) 전부 미발견",
                d.reason
            );
            d.hint = WINDOWS_AGENT_PATH_HINT.to_string();
        }
    }
    d
}

/// 어댑터 설치 감지의 **단일 오라클**(재감사 §3 CS-1③ · B12) — run_boot·`cys agent-detect`,
/// 그리고 그 JSON 을 소비하는 python `detect_reviewer` 가 **같은 판정**을 쓴다(종전엔 Rust 인라인
/// 판정과 python 자체 판정이 별개라 실행권 체크 유무가 어긋났다 = 두 오라클 불일치).
/// ①extract_bin 으로 env-prefix 건너뛴 바이너리 토큰 ②경로형(`~`·경로구분자 포함)은 틸드 확장 후
/// 실재+실행권 ③그 외는 which/where(exit status 권위) ④Windows 는 실패 시 B8 후보 순회.
fn detect_agent_binary(agent: &str, agents: &Value) -> AgentDetection {
    let bin = agents
        .get(agent)
        .and_then(|a| a["cmd"].as_str())
        // env-prefix를 건너뛰고 실제 바이너리 토큰을 찾는다 (extract_bin 단일 진실) — claude
        // cmd가 `CLAUDE_CONFIG_DIR="..." claude ...`처럼 env 대입으로 시작해 첫 토큰을 바이너리로
        // 오판('미설치')하던 회귀를 차단한다 (gemini/codex는 바이너리로 시작해 영향 없음).
        .map(|c| extract_bin(c, agent).to_string())
        .unwrap_or_else(|| agent.to_string());
    // 경로형 cmd('~/'·'/' 포함 — 예: agy 절대경로)는 which/where가 틸드를 확장하지
    // 않아 '미설치'로 오판한다 → 파일 실재+실행권으로 판정 (실행은 셸 -lc 경유라 틸드 확장됨).
    // '\\' 도 경로형으로 본다(Windows 선언 경로 — unix 어댑터엔 등장하지 않아 영향 0).
    let path_form = bin.starts_with('~') || bin.contains('/') || bin.contains('\\');
    let (installed, resolved, reason) = if path_form {
        let p = expand_tilde(&bin);
        if is_executable_file(&p) {
            let r = format!("실행가능 {}", p.display());
            (true, Some(p), r)
        } else {
            (false, None, format!("바이너리 부재/실행불가 {}", p.display()))
        }
    } else {
        let (found, path) = which_in_path(&bin);
        if found {
            let shown = path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| bin.clone());
            (true, path, format!("PATH 발견 {shown}"))
        } else {
            (false, None, format!("PATH 미발견 {bin}"))
        }
    };
    apply_windows_agent_fallback(AgentDetection {
        installed,
        bin,
        resolved,
        reason,
        hint: install_hint(agent).to_string(),
    })
}

/// `cys agent-detect [--json]` — 어댑터별 설치 감지를 **한 곳에서** 판정해 내보낸다.
/// 데몬 무의존(순수 파일시스템·PATH 조회)이라 부트 전에도 호출할 수 있다.
/// exit: 0=판정 산출 / **3=판정 불가**(어댑터 정의를 못 읽음). 3 과 '전부 미설치'(0 +
/// installed:false)는 **다른 사실**이다 — 소비부(python detect_reviewer)가 3 을 보고 자체 감지로
/// 폴백해야 하고, 0 을 보면 그 판정을 신뢰해야 한다(cys_status 의 exit 2 vs 1 구분과 동형 규약).
/// ★임베드 폴백을 **의도적으로 하지 않는다**: load_agent_spec 은 agents.json **파일 자체**가 없으면
///   `init-pack` 을 요구하며 실패한다(키 단위 폴백만 있다). 여기서 내장본으로 "설치됨"을 답하면
///   기동이 불가능한 상태를 '가용'으로 보고하는 오라클 거짓말이 된다.
fn run_agent_detect(as_json: bool) -> i32 {
    let p = cys::pack::pack_dir().join("agents.json");
    let agents: Value = match std::fs::read_to_string(&p)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "[agent-detect] 판정 불가 — {} 읽기/파싱 실패({e}). 복구: `cys init-pack`",
                p.display()
            );
            if as_json {
                println!(
                    "{}",
                    json!({"agents": {}, "error": format!("agents.json unreadable: {e}")})
                );
            }
            return 3;
        }
    };
    let keys: Vec<String> = agents
        .as_object()
        .map(|o| {
            o.keys()
                .filter(|k| !k.starts_with('_')) // '_doc'·'_roles'·'_schema' 등 메타 키 제외
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let mut out = serde_json::Map::new();
    for k in &keys {
        let d = detect_agent_binary(k, &agents);
        out.insert(
            k.clone(),
            json!({
                "installed": d.installed,
                "bin": d.bin,
                "resolved": d.resolved.map(|p| p.display().to_string()),
                "reason": d.reason,
                "hint": d.hint,
            }),
        );
    }
    if as_json {
        println!("{}", json!({"agents": out}));
    } else {
        for k in &keys {
            let e = &out[k];
            println!(
                "{:<8} {} {}",
                k,
                if e["installed"].as_bool() == Some(true) {
                    "installed"
                } else {
                    "missing  "
                },
                e["reason"].as_str().unwrap_or("")
            );
        }
    }
    0
}

fn run_boot(cwd: Option<String>, as_json: bool) -> i32 {
    // ★이중 boot 직렬화(오너 2026-07-15 적대검증 D-7 + 아키텍트 성찰): 마스터 팀 스폰 트리거가
    // 겹칠 수 있다(고전 경로=UserPromptSubmit 훅이 javis_bootstrap.py ④ boot 발화 · 버튼 경로=GUI
    // spawn_orchestra_boot · 마스터 LLM이 스스로 boot). 두 boot가 겹치면 각자 "역할 미가동" 스냅샷을
    // 보고 리뷰어(데몬 특권 가드 없음)를 중복 스폰할 수 있다. 소켓별 boot 락을 비차단 획득.
    // (claim-role의 boot 부작용은 아키텍트 성찰로 제거 — 레지스트리 op가 프로세스 스폰하는 결합 차단.)
    //
    // ★(W2 · G11) **busy 를 스폰 성공으로 오인하던 경로 차단**. 종전엔 락 경합에서 산문 한 줄만 찍고
    // exit 0 을 냈다 — 소비부(javis_bootstrap ④)는 그 0을 '팀을 세웠다'로 읽고 **CEO 티켓을 소각**해
    // 무스폰 티켓 소각(1회성 티켓 ⟺ 실스폰 불변식 파괴)이 났다. 이제 `--json` 의 outcome=`busy` 로
    // 타입 구분하고, 소비부는 실스폰 확인 후에만 티켓을 태운다.
    // ★bare exit 의미는 구계약 유지(0) — 전환은 W4 GUI --json 소비와 원자(금지 방향 ⑧).
    let _boot_lock = match acquire_boot_lock() {
        BootLock::Acquired(g) => {
            // ★(W2 · G12) 락 보유를 프로세스 내부·자식 프로세스에 알린다 — 아래에서 in-process 로
            // 호출하는 run_launch_agent 가 같은 락을 재획득해 자기 자신에게 막히는 것을 막는다.
            BOOT_LOCK_HELD.store(true, std::sync::atomic::Ordering::SeqCst);
            std::env::set_var("CYS_BOOT_LOCK_HELD", "1");
            g
        }
        BootLock::Busy => {
            println!(
                "cys boot — 다른 boot 진행 중(락 보유) — 중복 스폰 방지로 skip (그 boot가 팀을 세움) \
                 · exit {EXIT_BOOT_BUSY}(busy=무스폰)"
            );
            if as_json {
                let roles: Vec<Value> = BOOT_PLAN
                    .iter()
                    .map(|(role, agent, mandatory)| {
                        json!({"role": role, "agent": agent, "outcome": "busy",
                               "mandatory": mandatory, "reason": "boot 락 보유자 존재(중복 스폰 방지)"})
                    })
                    .collect();
                println!(
                    "{}",
                    json!({"roles": roles,
                           "summary": {"launched": 0, "already_alive": 0, "busy": BOOT_PLAN.len(),
                                       "missing": 0, "failed": 0, "recovered": 0,
                                       "fatal_failed": 0, "lock": "busy"}})
                );
            }
            // ★(W4) busy 는 0 이 아니다 — 무스폰이므로 소비부가 '팀을 세웠다'로 읽으면 안 된다.
            return boot_exit_code(0, 0, true);
        }
    };
    let agents: Value = std::fs::read_to_string(cys::pack::pack_dir().join("agents.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    // ★(W2 · G12) run_boot 은 **iteration 마다 role 생존을 재조회**한다(루프 안 `fetch_surfaces`).
    // 종전엔 루프 진입 전에 한 번 스냅샷을 떠서, 앞 role 의 기동이 만든 상태 변화(dedup·좌석 승계·
    // 중간에 붙은 다른 부트의 산물)를 못 봤다 — 락 커버리지 밖 변화에 stale 판정으로 중복 스폰했다.
    // ★★M3(2026-08-24 자기성찰 3회전) — **버킷이 outcome 을 전부 덮지 않았다.**
    //   종전 `--json` summary 는 7버킷인데 typed outcome 은 7종이고 그중 `gate_pending` ·
    //   `skipped_unconfirmed` 두 종에 **대응 버킷이 없었다**(`fatal_failed` 는 버킷이 아니라
    //   교차 집계다). 회전2 실주행에서 로스터 5개 중 4개가 gate_pending 인데 **버킷 합은 1** 이
    //   나왔고(합 ≠ 로스터 길이), 사람과 master 가 읽는 첫 줄은 "의무 실패 0" 이었다.
    //
    //   ★수리는 버킷을 **더하는 것으로 끝내지 않는다** — 집계를 `outcomes` 에서 **파생**시킨다.
    //   종전엔 분기마다 `launched += 1` 류를 손으로 늘렸고, 그래서 W4 가 이미 한 번
    //   "이 분기만 `fatal_failed` 를 빼먹어" fail-open 이 났다(그 사고가 이 함수 주석에 그대로
    //   적혀 있다). 손 계상이 남아 있는 한 같은 결함이 **다음 분기에서 또** 난다. 이제 계상 지점은
    //   `outcomes.push` 하나이고, 버킷 합 = roles 길이는 [`boot_summary_buckets`] 의 **구조적
    //   불변식**이다(미지 outcome 도 `unbucketed` 로 새어 나와 합을 유지한다 — 조용히 사라지지
    //   않는다). 검체 `boot_summary_buckets_cover_every_typed_outcome` 이 그것을 박제한다.
    let mut outcomes: Vec<Value> = Vec::new();
    let started = std::time::Instant::now();
    let mut last_hb = std::time::Instant::now();
    println!("cys boot — LLM orchestrating 편성 점검 (CSO·worker·agy·codex 4종 의무 + grok 선택)");
    // ★★(M5) 각성 훅 **설치 표적 ≠ 실소비 SOT** 를 기동 시 한 번 소리 내어 알린다.
    //   기본 경로(팩=`~/.cys/pack`)에서는 두 값이 우연히 일치하므로 이 줄은 나오지 않는다 —
    //   나온다면 그 기계에서는 각성 훅이 **아무도 읽지 않는 폴더**에 설치된다는 뜻이고,
    //   노드는 떠도 `/clear` 후 재주입·마스터 선언 부트 발화가 영구히 죽는다(팀 미기동).
    warn_if_config_target_mismatch();
    for (role, agent, mandatory) in BOOT_PLAN {
        // 침묵 창 상쇄(B9 방향 ③) — 진행 하트비트는 stderr(stdout 기계 계약 무오염).
        if last_hb.elapsed().as_secs() >= BUDGET_HEARTBEAT_INTERVAL_SECS {
            eprintln!(
                "[boot] 진행 중 {}s 경과 — 다음 대상 role={role}",
                started.elapsed().as_secs()
            );
            last_hb = std::time::Instant::now();
        }
        // ★(W4 · CS-1③) 인라인 감지 폐기 → **공유 오라클**(detect_agent_binary) 소비.
        //   같은 판정을 `cys agent-detect --json` 이 내보내고 python detect_reviewer 가 그걸 쓴다
        //   (종전엔 Rust=exists() / python=X_OK 로 두 오라클이 어긋났다). Windows 후보 순회(B8)도
        //   오라클 안에 있으므로 부트가 자동으로 함께 받는다.
        let det = detect_agent_binary(agent, &agents);
        let bin = det.bin;
        if !det.installed {
            // ★(W2 · G29·B8) 미설치는 **typed `missing`** 이다 — 종전엔 산문 skip + exit 0 이라
            // 소비부(bootstrap exit 4 계약)와 불일치했고, 의무 CLI 미설치가 '성공'으로 보고됐다.
            let hint = det.hint;
            println!("· {agent}: CLI '{bin}' 미설치 — 건너뜀 ({}) (설치: {hint})", det.reason);
            // ★fatal_failed 는 **--json 전용 요약 필드**다 — bare exit 을 움직이지 않는다(금지 방향 ⑧).
            //   의무 CLI 미설치가 '성공'이 아니라는 사실(G29)은 typed outcome=missing+mandatory 로
            //   전달되고, 그것을 exit 4 로 승격하는 판정은 **소비부(javis_bootstrap)** 가 한다.
            //   ★(M3) 계상은 아래 `outcomes.push` 하나에서 파생된다(손 계상 제거).
            outcomes.push(json!({"role": role, "agent": agent, "outcome": "missing",
                                 "mandatory": mandatory, "bin": bin, "install_hint": hint}));
            continue;
        }
        // ── ★(W2 · B3) 스킵 술어: `!exited` 단독 → `!exited ∧ (awake ∨ presumed ∨ unknown-해소)` ──
        let surfaces = fetch_surfaces();
        // 소유 클론 — 아래 Unknown 시한부 해소가 목록을 재조회하므로 참조를 들고 가지 않는다.
        let mut seat: Option<Value> = find_seat_row(&surfaces, role).cloned();
        let (mut grade, mut why) = match seat.as_ref() {
            Some(row) => seat_liveness(row),
            None => (SeatLiveness::Absent, "좌석 없음"),
        };
        // ★Unknown 이원 규칙(비평2 B-2)의 **스폰 경로 절반** — 시한부 해소.
        //   파괴 경로(kill·reclaim)의 Unknown 은 무조건 hold 지만, 스폰 경로에서 영구 hold 하면
        //   GUI 콜드스타트(앱 시작 즉시 spawn_orchestra_boot)에서 좌석 캐시가 아직 안 채워진 창에
        //   술어가 구 `!exited` 로 퇴화해 B3 를 보존한다. seat_cache 의 유일 writer 는 watchdog 5초
        //   틱이므로 **1주기 대기 후 재조회 1회**, 그래도 불명이면 결손 취급해 스폰한다(가용성 우선).
        //   중복 스폰은 boot 락이 방어한다 — 그래서 이 fail-open 이 안전하다.
        if grade == SeatLiveness::Unknown {
            eprintln!("[boot] role={role}: 좌석 판정 불가 — 워치독 1주기(5s) 대기 후 재조회 1회");
            std::thread::sleep(std::time::Duration::from_secs(5));
            let refreshed = fetch_surfaces();
            seat = find_seat_row(&refreshed, role).cloned();
            let (g2, w2) = match seat.as_ref() {
                Some(row) => seat_liveness(row),
                None => (SeatLiveness::Absent, "좌석 없음(재조회)"),
            };
            (grade, why) = if g2 == SeatLiveness::Unknown {
                (SeatLiveness::Absent, "잔존 불명 → 결손 취급·스폰(중복은 boot 락이 방어)")
            } else {
                (g2, w2)
            };
        }
        if matches!(grade, SeatLiveness::AwakeConfirmed | SeatLiveness::AlivePresumed) {
            let label = if grade == SeatLiveness::AwakeConfirmed {
                "각성 확정"
            } else {
                "생존추정"
            };
            println!("· {agent}: 역할 '{role}' 이미 가동 중({label}: {why}) — 건너뜀");
            outcomes.push(json!({"role": role, "agent": agent, "outcome": "already_alive",
                                 "mandatory": mandatory, "liveness": label, "reason": why}));
            continue;
        }
        // ── ★(U-10) 관문 보류 좌석: **관측·보고만**. 스폰 0 · 회수 0 · 파괴 0 ──
        //   프로세스도 pane 도 살아 있다 — 사람이 관문을 통과시키면 그대로 쓸 수 있는 좌석이다.
        //   ⓐ`already_alive` 로 접지 않는다(그것이 이 등급의 존재 이유 — 관문에 갇힌 팀이
        //     "정상 가동 중" 으로 집계되는 것을 막는다).
        //   ⓑ아래 죽음확정 체인(node-recover 주입 · reclaim kill)에도 **넣지 않는다**. 살아 있는
        //     입력창에 기동 커맨드를 밀어 넣으면 화면 파괴·중복 기동이고, 연쇄하면 치명위험 ④
        //     (전 pane 사망)다. 보류 우선 — 비가역 손해를 피한다.
        //   ⓒ새 스폰도 하지 않는다. 좌석(role)이 살아 있으므로 새 surface 는 `claim_denied`·
        //     litter 만 남긴다.
        //   ★outcome 은 신규 값 `gate_pending` 이다. 소비 계약상 **Fatal 이 아니다**
        //     (`javis_bootstrap._boot_fatal_verdict` 의 fatal 집합 = {failed, missing} · GUI
        //     경고 필터도 같은 두 값) — 오늘의 `skipped_unconfirmed` 와 같은 무해 등급이면서
        //     사유가 정확하다.
        //   ★(M2 정정 · 2026-08-24) 종전 이 자리에는 "이 단위에는 생산자가 없어 실제로는 나오지
        //     않는다" 고 적혀 있었다. **U-11 이 생산자를 만든 뒤로 거짓이다** — 그리고 이 분기가
        //     그 뒤 한 번도 재방문되지 않았다는 증거가 그 문장이었다(회전2 실주행에서 로스터
        //     5개 중 4개가 이 분기로 흘렀다). 지금은 실제 생산자가 셋이다:
        //     `settle_gate_pending`(readiness 타임아웃·주입 직전·주입 도중) → 데몬 표식 →
        //     `seat_liveness` 의 이 등급.
        //   ★그리고 이 분기는 이제 **탈출 경로**를 갖는다(M2): 스폰 0 의 비파괴 재관측으로
        //     관문 통과를 확인하면 표식을 해제하고 디렉티브를 주입해 좌석을 그대로 채택한다.
        if grade == SeatLiveness::GatePending {
            let sref = seat
                .as_ref()
                .and_then(|r| r["surface_ref"].as_str())
                .unwrap_or("")
                .to_string();
            let sid = seat.as_ref().and_then(|r| r["surface_id"].as_u64());
            // ── ★(M2) 비파괴 재관측: `read_text` 1회 + `readiness::judge` 1회. 스폰 0 ──
            let recheck = match sid {
                Some(sid) => gate_pending_reobserve(sid, agent),
                // surface_id 를 못 읽으면 재관측 대상이 없다 — 종전대로 보류.
                None => GateRecheck::NoEvidence,
            };
            if let (GateRecheck::Adopt(evidence), Some(sid)) = (&recheck, sid) {
                println!(
                    "· {agent}: 역할 '{role}' 관문 통과 확인({}) — 좌석 재사용 · 디렉티브 주입(스폰 0)",
                    evidence.label()
                );
                match gate_pending_adopt(sid, role, agent) {
                    Ok(BootVerdict::Ready) => {
                        outcomes.push(json!({"role": role, "agent": agent, "outcome": "recovered",
                                             "mandatory": mandatory, "surface_ref": sref,
                                             "reason": format!("관문 보류 재관측 → 통과 확인({}) · 표식 해제 + 디렉티브 주입(스폰 0)",
                                                               evidence.label())}));
                        continue;
                    }
                    // 재관측과 주입 사이에 **다음 관문**이 떴다(실측: 폴더신뢰 통과 → 면책 창).
                    // 귀결은 종전과 같은 보류다 — 표식은 주입 가드가 다시 찍었다.
                    Ok(other) => {
                        eprintln!(
                            "[boot] role={role} 재관측 채택 중 관문 재발 — 보류 유지({other:?})"
                        );
                    }
                    Err(e) => {
                        eprintln!("[boot] role={role} 재관측 채택 실패 — 보류 유지(파괴 0): {e}");
                    }
                }
            }
            // 재관측 결과를 사람이 읽는 줄과 typed outcome **양쪽**에 싣는다 — 보류가 왜
            // 계속되는지(관문 상주인가 · 증거 부재인가)를 모르면 사람이 조치를 고를 수 없다.
            let recheck_note: String = match &recheck {
                GateRecheck::Adopt(_) => "재관측=통과했으나 채택 중 관문 재발/주입 실패".into(),
                GateRecheck::StillHeld { gate_id, title } => {
                    format!("재관측=관문 상주({title} · id={gate_id})")
                }
                GateRecheck::NoEvidence => "재관측=증거 없음(화면 미관측·맨 셸 의심)".into(),
            };
            println!(
                "· {agent}: 역할 '{role}' 첫기동 관문 보류({why} · {recheck_note}) — 사람 1회 조치 필요\n                   확인: `cys read-screen --surface {sref}` (스폰·회수·파괴 모두 하지 않음)"
            );
            outcomes.push(json!({"role": role, "agent": agent, "outcome": "gate_pending",
                                 "mandatory": mandatory, "surface_ref": sref,
                                 "liveness": "gate_pending", "reason": why,
                                 "recheck": recheck_note,
                                 "hint": "첫기동 관문(테마·로그인·OAuth·폴더신뢰·면책·새기능안내) 통과 후 재부트 — 좌석과 프로세스는 살아 있다(재부트가 스폰 없이 이 좌석을 채택한다)"}));
            continue;
        }
        // ── ★죽음 **확정** 좌석: node-recover(비파괴) 우선 → reclaim 에스컬레이션 자동 체인 ──
        //   좌석이 남아 있는데 에이전트만 죽은 경우(B3 의 그 상태), 새 surface 를 만들면 특권 역할은
        //   claim_denied 로 막히고 리뷰어는 litter 를 남긴다. 기존 pane 위에서 되살리는 것이
        //   **비파괴적이고 정확한** 처방이다. 실패하면 reclaim(파괴)으로 한 단계 올린다.
        //
        // ★★안전 게이트(치명위험 ④ 차단 · `seat_death_confirmed` 주석 전문 참조): 이 체인은
        //   **죽음이 확정된 좌석에만** 닿는다. Absent 는 '명시적 빈 좌석'·'판정불가 시한부 해소'·
        //   '구 데몬 무신호'가 섞인 등급이고, 뒤 두 경우에 침습적 복구를 걸면 냉시작 데몬의 건강한
        //   전 팀을 파괴한다(watchdog 첫 틱 전 = 전 좌석 Unknown, GUI 는 그 순간 boot 를 쏜다).
        //   확정 실패 시엔 **좌석을 건드리지도, 새로 스폰하지도 않는다** — 살아있을 수 있는 좌석
        //   위에 중복 스폰하면 claim_denied·litter·이중 에이전트가 된다. 보류 우선(비가역 회피).
        if let Some(row) = seat.as_ref() {
            let sref = row["surface_ref"].as_str().unwrap_or("").to_string();
            match seat_death_confirmed(row) {
                Err(hold) => {
                    println!(
                        "· {agent}: 역할 '{role}' 좌석 존재·생존 신호 없음({why})이나 **죽음 미확정** \
                         — 침습적 복구·스폰 모두 보류(보류 우선): {hold}"
                    );
                    eprintln!(
                        "[boot] role={role} 보류 — 수동 확인: `cys read-screen --surface {sref}` / \
                         회수가 필요하면 `javis_boot_node.py --reclaim --role {role}`(hold-first 판정 내장)"
                    );
                    outcomes.push(json!({"role": role, "agent": agent,
                                         "outcome": "skipped_unconfirmed", "mandatory": mandatory,
                                         "surface_ref": sref, "reason": hold,
                                         "hint": "죽음 미확정 좌석 — 파괴·중복 스폰 금지(수동 확인)"}));
                    continue;
                }
                Ok(()) => {
                    println!("· {agent}: 역할 '{role}' 좌석 죽음 **확정**({why}) — node-recover 시도(비파괴)");
                    let rc = run_node_recover(Some(sref.clone()), Some((*role).to_string()));
                    if rc == 0 {
                        outcomes.push(json!({"role": role, "agent": agent, "outcome": "recovered",
                                             "mandatory": mandatory, "surface_ref": sref,
                                             "reason": format!("node-recover(비파괴): {why}")}));
                        continue;
                    }
                    // ★★(U-11) 치명 분기: 여기서 빠져나가지 않으면 **살아 있는 에이전트를 죽인다**.
                    //   node-recover 는 같은 pane 에 에이전트를 다시 띄운다 — 그 새 에이전트가
                    //   첫기동 관문에 갇히면 프로세스는 살아 있는데 준비만 미확정이다. 종전 계약
                    //   (0 아니면 전부 실패)에서는 그 상태가 곧바로 `escalate_reclaim`(kill)으로
                    //   내려갔다. 전용 종료코드로 그 체인을 끊는다: 스폰 0 · 회수 0 · 파괴 0.
                    //   (reclaim 헬퍼의 hold-first 판정이 2선 방어로 남아 있지만, 파괴 경로를
                    //    **부르지 않는 것**이 1선이다 — 보류 우선.)
                    if rc == cys::EXIT_GATE_PENDING {
                        println!(
                            "· {agent}: 역할 '{role}' 재기동 후 첫기동 관문 보류 — 회수·파괴 모두 하지 않음(사람 1회 조치 필요)"
                        );
                        outcomes.push(json!({"role": role, "agent": agent,
                                             "outcome": "gate_pending", "mandatory": mandatory,
                                             "surface_ref": sref, "liveness": "gate_pending",
                                             "reason": "node-recover 후 readiness 미확정 · 프로세스 생존 관측",
                                             "hint": "첫기동 관문(테마·로그인·OAuth·폴더신뢰·면책·새기능안내) 통과 후 재부트 — 좌석과 프로세스는 살아 있다"}));
                        continue;
                    }
                    println!("· {agent}: node-recover 실패 — reclaim 에스컬레이션(파괴·hold-first 판정 내장)");
                    escalate_reclaim(role);
                    let after = fetch_surfaces();
                    if find_seat_row(&after, role).is_some() {
                        // reclaim 이 좌석을 못 비웠다(hold 판정 포함) — 새 스폰은 claim_denied/litter 뿐이다.
                        println!("· {agent}: reclaim 후에도 좌석 잔존 — 스폰 보류(수동 점검 필요)");
                        // ★(W4 → M3) 종전 이 분기는 `fatal_failed` 를 **빼먹어** fail-open 이
                        //   났다(그 사고가 이 수리의 동기다). 지금은 계상 지점이 아래
                        //   `outcomes.push` 하나이고 Fatal 판정은 그 typed 값에서 파생되므로,
                        //   '분기가 계상을 빼먹는' 결함이 원리상 재발하지 않는다.
                        outcomes.push(json!({"role": role, "agent": agent, "outcome": "failed",
                                             "mandatory": mandatory,
                                             "reason": "죽음 확정 좌석을 node-recover·reclaim 으로도 해소 못 함",
                                             "install_hint": "javis_boot_node.py --reclaim --role 로 수동 회수 후 재부트"}));
                        continue;
                    }
                }
            }
        }
        println!("· {agent}: 기동 시작 (role={role})…");
        let launch_rc = run_launch_agent(role, agent, cwd.clone());
        if launch_rc == cys::EXIT_GATE_PENDING {
            // ★(U-11) pane 은 떴고 프로세스도 살아 있다 — 실패로 계상하지 않는다.
            //   U-10 이 **좌석 경로**의 같은 상태에 이미 준 판정과 같은 값을 쓴다(같은 사실 =
            //   같은 outcome). Fatal 집합 {failed, missing} 밖이라 `cys boot` 의 exit 계약
            //   (0/1/75)은 불변이고, 최종 게이트는 `orchestra check` 다 — U-10 이 그 축에서
            //   gate_pending 을 **미충족**으로 못박았으므로 이 상태로 READY 가 선언되지 않는다.
            println!("· {agent}: 첫기동 관문 보류 — pane 보존(닫지 않음) · 사람 1회 조치 필요");
            outcomes.push(json!({"role": role, "agent": agent, "outcome": "gate_pending",
                                 "mandatory": mandatory, "liveness": "gate_pending",
                                 "reason": "launch 후 readiness 미확정 · 프로세스 생존 관측",
                                 "hint": "첫기동 관문(테마·로그인·OAuth·폴더신뢰·면책·새기능안내) 통과 후 재부트 — 좌석과 프로세스는 살아 있다"}));
            continue;
        }
        if launch_rc == 0 {
            outcomes.push(json!({"role": role, "agent": agent, "outcome": "launched",
                                 "mandatory": mandatory}));
        } else {
            println!("· {agent}: 기동 실패 — 나머지 노드는 계속 진행");
            outcomes.push(json!({"role": role, "agent": agent, "outcome": "failed",
                                 "mandatory": mandatory,
                                 "install_hint": install_hint(agent)}));
        }
    }
    // ★(M3) 집계는 **전부 typed outcome 에서 파생**한다(손 계상 0 — 위 선언부 주석 참조).
    let summary = boot_summary_buckets(&outcomes);
    let b = |k: &str| summary.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    let (fatal_failed, fatal_gate_pending) = (b("fatal_failed"), b("fatal_gate_pending"));
    // ★(M3) 사람이 읽는 **첫 줄**에 관문 보류를 싣는다. 종전 첫 줄은 "의무 실패 0" 이었고,
    //   의무 4좌석이 전부 관문에 갇혀 팀이 서지 않은 상태에서도 그 줄만 읽으면 정상으로 보였다.
    //   `run_boot` 주석 자신이 busy 에 대해 "0 을 주면 소비부가 팀을 세웠다로 읽는다"고 경고해
    //   놓고 같은 구멍을 남겼다 — M1·M2 가 고쳐지기 전까지 이것이 유일한 조기 경보다.
    println!(
        "boot 완료: 신규 기동 {} · 회수복구 {} · 이미가동 {} · ★관문 보류 {}(의무 {}) · \
         죽음미확정 보류 {} · 미설치 {} · 실패 {} (의무 실패 {}) · 현황은 `cys list`로 확인",
        b("launched"),
        b("recovered"),
        b("already_alive"),
        b("gate_pending"),
        fatal_gate_pending,
        b("skipped_unconfirmed"),
        b("missing"),
        b("failed"),
        fatal_failed,
    );
    if as_json {
        println!(
            "{}",
            json!({"roles": outcomes, "summary": summary})
        );
    }
    // ★★(W4) bare exit **의미 전환** — GUI `--json` 소비와 **동일 커밋**(하드 제약 6-⑧).
    //   구계약: `launch 실패>0 → 1 / 그 밖 0`. busy 도 0, 리뷰어 1종 실패도 1 이었다 —
    //   즉 "재시도하면 되는 상황"과 "팀이 없는 상황"과 "선택 노드만 빠진 상황"이 한 값에 뭉개져
    //   있었다(RC2 의미 융합). 이제 세 의미를 분리한다:
    //     · busy(다른 boot 가 락 보유)   → EXIT_BOOT_BUSY(75 = EX_TEMPFAIL) — **무스폰**이다.
    //       0 을 주면 소비부가 '팀을 세웠다'로 읽어 CEO 티켓을 소각한다(G11 의 그 사고).
    //     · Fatal 실패(mandatory 역할의 failed·missing) → 1 — 팀의 최소 실행 단위가 없다.
    //     · Degrade-only(선택·리뷰어만 실패/미설치)  → 0 — 대체 폴백·익명 peer-review 로 보완
    //       가능하며, ⑤check 가 최종 게이트다(B1 데드엔드 재발 금지).
    //   exit 은 `--json` 의 typed 판정과 **같은 사실**을 낸다: exit 1 ⟺ mandatory 중 failed|missing
    //   존재(= `javis_bootstrap._boot_fatal_verdict` 의 판정). 두 채널이 갈리면 fail-open 이 생긴다
    //   (Rust 테스트 `boot_exit_matches_json_fatal_verdict` 가 이 동등성을 박제).
    boot_exit_code(fatal_failed as usize, fatal_gate_pending as usize, false)
}

/// `cys boot` bare exit 판정의 **순수 함수**(W4 · ★M3 에서 78 추가) — 0/1/75/78 네 의미의 단일
/// 소유자. run_boot 의 두 종료 지점(busy skip · 정상 종료)이 모두 이것을 통과하므로, 의미가
/// 코드 두 곳에 흩어지지 않는다. 회귀 테스트(`boot_exit_matches_json_fatal_verdict`)가 --json 의
/// Fatal 판정(mandatory && outcome ∈ {failed, missing})과 **같은 사실**을 내는지 박제한다.
///
/// ## ★M3(2026-08-24) — 의무 0/4 인데 exit 0 · "실패 0" 이 나갔다
///
/// `fatal_failed` 는 outcome ∈ {failed, missing} 에서만 증가하므로, 의무 역할 전량이
/// **관문 보류**로 남아도 이 함수는 0 을 냈다. 회전2 실주행이 정확히 그 상태였다 — 로스터
/// 5개 중 4개가 gate_pending 인데 첫 줄은 "의무 실패 0", bare exit 는 0. 소비부는 그것을
/// '팀을 세웠다' 로 읽는다(busy 에 대해 이 함수 주석이 이미 경고한 그 오독을 같은 함수가
/// 다른 축에서 반복했다).
///
/// **전용 종료코드 78**([`cys::EXIT_GATE_PENDING`] · launch-agent 가 이미 쓰는 값 재사용)로
/// 세 번째 의미를 분리한다: *팀이 살아 있으나 사람 1회 조치 전까지는 일할 수 없다.*
/// 성공(0)도 실패(1)도 아니고, 재시도로 풀리는 busy(75)도 아니다.
///
/// **우선순위**: busy(75) > Fatal 실패(1) > 의무 관문 보류(78) > 0.
/// Fatal 이 78 보다 앞인 이유 — '좌석이 아예 없다' 는 '좌석은 있는데 갇혔다' 보다 나쁜 상태이고,
/// 처방도 다르다(재설치·재기동 vs 사람 1회 통과). 둘 다 있으면 더 나쁜 쪽을 보고한다.
/// `cys boot --json` 의 **버킷 집계**(순수 · ★M3) — typed outcome 하나가 유일한 계상 지점이다.
///
/// ## 계약(이 함수의 존재 이유)
///
/// **버킷 합 = `roles` 길이.** 종전 summary 는 버킷을 손으로 세었고, typed outcome 7종 중
/// `gate_pending` · `skipped_unconfirmed` **둘에 버킷이 없어** 합이 로스터보다 작았다. 회전2
/// 실주행에서 로스터 5개 중 4개가 `gate_pending` 인데 **버킷 합은 1** 이었고, 그 차이는 어떤
/// 채널에도 나타나지 않았다 — 사람과 master 가 읽는 첫 줄은 "의무 실패 0" 이었다.
///
/// 여기서 버킷을 **더하기만** 하면 다음 outcome 이 생길 때 같은 결함이 재발한다. 그래서
///   ① 집계를 `outcomes` 에서 파생시키고(분기가 계상을 빼먹을 자리가 없다 — W4 가 실제로 한 번
///      맞은 결함이다),
///   ② 미지 outcome 은 조용히 사라지지 않고 **`unbucketed` 로 새어 나온다**(합 보존).
/// 그래서 "합 ≠ roles 길이" 는 원리상 불가능하고, 검체
/// `boot_summary_buckets_cover_every_typed_outcome` 이 그것을 전수로 박제한다.
///
/// ## 버킷이 아닌 필드
///
/// `busy`(무스폰 조기 종료 · 이 경로에서는 항상 0) · `lock` · `fatal_failed` ·
/// `fatal_gate_pending` 은 **교차 집계·메타**라 합에 들어가지 않는다. 합 계산에서 제외되는
/// 키 집합의 여집합은 [`BOOT_SUMMARY_BUCKETS`] 하나가 소유한다(검체가 같은 상수를 쓴다 — 사본 금지).
fn boot_summary_buckets(roles: &[Value]) -> serde_json::Map<String, Value> {
    let mut counts: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    let (mut fatal_failed, mut fatal_gate_pending) = (0u64, 0u64);
    for r in roles {
        let mandatory = r["mandatory"].as_bool().unwrap_or(false);
        let outcome = r["outcome"].as_str().unwrap_or("");
        // typed outcome → 버킷 키. 미지 값은 `unbucketed` 로 흘려 **합을 보존**한다.
        let key = match outcome {
            "launched" => "launched",
            "already_alive" => "already_alive",
            "recovered" => "recovered",
            "gate_pending" => "gate_pending",
            "skipped_unconfirmed" => "skipped_unconfirmed",
            "missing" => "missing",
            "failed" => "failed",
            _ => "unbucketed",
        };
        *counts.entry(key).or_insert(0) += 1;
        // 교차 집계 — python 소비부 `_boot_fatal_verdict` 와 문자 그대로 같은 술어.
        if mandatory {
            match outcome {
                "failed" | "missing" => fatal_failed += 1,
                "gate_pending" => fatal_gate_pending += 1,
                _ => {}
            }
        }
    }
    let mut out = serde_json::Map::new();
    // 버킷은 **항상 전량 노출**한다(0 이어도 키를 지우지 않는다 — 소비부가 키 부재와 0 을
    // 구분하려 들면 그 순간 판정이 이원화된다).
    for k in BOOT_SUMMARY_BUCKETS {
        out.insert((*k).into(), json!(counts.get(*k).copied().unwrap_or(0)));
    }
    for (k, v) in [
        ("busy", 0),
        ("fatal_failed", fatal_failed),
        ("fatal_gate_pending", fatal_gate_pending),
    ] {
        out.insert(k.into(), json!(v));
    }
    out.insert("lock".into(), json!("acquired"));
    out
}

/// summary 의 **버킷 키 전량**(합 = roles 길이의 대상). 이 목록 하나가 정본이며 프로덕션
/// 직렬화와 검체가 **같은 상수**를 소비한다 — 목록을 사본으로 두면 한쪽만 늘어나 합이 갈린다.
/// 여기 없는 키(`busy`·`lock`·`fatal_*`)는 버킷이 아니라 교차 집계·메타다.
const BOOT_SUMMARY_BUCKETS: &[&str] = &[
    "launched",
    "already_alive",
    "recovered",
    "gate_pending",
    "skipped_unconfirmed",
    "missing",
    "failed",
    "unbucketed",
];

fn boot_exit_code(fatal_failed: usize, fatal_gate_pending: usize, busy: bool) -> i32 {
    if busy {
        EXIT_BOOT_BUSY
    } else if fatal_failed > 0 {
        1
    } else if fatal_gate_pending > 0 {
        cys::EXIT_GATE_PENDING
    } else {
        0
    }
}


/// 죽음 확정 좌석의 reclaim 에스컬레이션 — 팩 헬퍼(`javis_boot_node.py --reclaim`)에 위임한다.
/// ★왜 위임인가: reclaim 은 kill 을 포함한 **파괴 행위**이고, 그 안전 판정(`_reclaim_verdict` 의
/// hold-status/hold-alive/hold-pid 4분기 + kill 직전 pid 재확인)은 이미 감사·검체로 결박돼 있다.
/// Rust 에 재구현하면 판정 이원화(RC1)를 새로 만든다 — 같은 술어를 두 번 쓰지 않는다.
/// 선례: `run_skillscan_gate`(cys.rs)가 팩 python 헬퍼를 호출하는 형태와 동일.
fn escalate_reclaim(role: &str) {
    let helper = cys::pack::pack_dir().join("bin/javis_boot_node.py");
    if !helper.exists() {
        eprintln!("[boot] reclaim 헬퍼 부재({}) — 에스컬레이션 생략", helper.display());
        return;
    }
    // ★SEAL-1: PATH 선두가 동봉 runtime 이면 이 `python3` 는 앱 번들 안의 인터프리터다 —
    // 팩토리가 PYTHONDONTWRITEBYTECODE 를 얹어 `.pyc` 번들 오염(코드서명 봉인 파손)을 막는다.
    match cys::python_command("python3")
        .arg(&helper)
        .args(["--reclaim", "--role", role])
        .output()
    {
        Ok(o) => {
            let out = String::from_utf8_lossy(&o.stdout);
            eprintln!(
                "[boot] reclaim(role={role}) rc={} — {}",
                o.status.code().unwrap_or(-1),
                out.trim()
            );
        }
        // ★의도적으로 인터프리터 후보를 넓히지 않는다(Windows 보수 판정):
        //   Windows 는 `python3` 가 보통 없고(팩이 embeddable python 을 동봉 · 훅은 CYS_PY 로 해소),
        //   여기서 후보를 넓히면 **파괴 경로(kill)가 Windows 에서 더 쉽게 발화**한다. 오너 지시대로
        //   Windows 는 예기치 않은 실패가 잦은 플랫폼이므로, 파괴는 '실행되지 않는 쪽'이 안전하다.
        //   대신 무엇이 일어나지 않았고 무엇을 실행해야 하는지 **시끄럽게** 남긴다(무음 no-op 금지).
        Err(e) => eprintln!(
            "[boot] reclaim **미실행**(python3 해소 실패: {e}) — 좌석은 그대로 보존된다(안전측). \
             회수가 필요하면 팩 인터프리터로 직접: \
             \"$CYS_PY\" \"{}\" --reclaim --role {role}  (Windows 는 CYS_PY=동봉 python)",
            helper.display()
        ),
    }
}

/// 임베드(vendor) agents.json 파싱 — 컴파일타임 내장 사본이 **코드 기본값** 계층의 원천이다.
fn embedded_agents_json() -> Option<Value> {
    cys::pack::PACK_ALL
        .iter()
        .find(|(r, _)| *r == "agents.json")
        .and_then(|(_, c)| serde_json::from_str(c).ok())
}

/// ★(W4 · 재감사 §3 CS-1③ / 비평2 C-1) **필드 단위 계층 — 마커·trust 패턴 한정**.
/// load_agent_spec 의 폴백은 종전 **agent 전체(whole-object)** 단위였다: 유저 agents.json 에 그
/// 키가 있으면 부분적이어도 통째로 이기므로, 예전에 커스터마이즈한 `claude` 항목은 vendor 가 새로
/// 출하한 `ready_marker`·`approval_patterns` 를 **영영 못 받는다**(동결 = readiness 시간폴백 퇴화,
/// 폴더신뢰 자동확인 불발). 그래서 **판정 술어가 소비하는 키만** 코드 기본값(vendor 임베드)
/// + user override 계층으로 만든다. 전면 스키마 마이그레이션은 하지 않는다(의도적 보류).
/// ★(U-12 · agents.json `_schema` 3) 계층 대상이 3키가 됐다 — `first_run_gates` 는 **신규 키**라
/// 기존 디스크 파일에 부재하고, 그래서 이 계층이 **유일하게 실효하는 배달 경로**다(K-1).
/// 규칙: 키가 **아예 없을 때만** 메모리상 반환값에 임베드 값을 채운다(디스크 파일 무접촉 — 사용자
/// 소유 파일을 코드가 고쳐 쓰지 않는다 ★W-B). 명시적 `null` 은 "의도적으로 없음" 선언으로 보고
/// 채우지 않는다(사용자 주권 보존). 어댑터 값이 객체가 아니면(손상 커스텀) 아무것도 하지 않는다.
fn fill_missing_fields(resolved: &mut Value, embedded: Option<&Value>) {
    // ★(U-12 · K-1) `first_run_gates` 추가 — 이 키는 **기존 설치 기계의 디스크 파일에 없다**.
    //   그래서 계층이 채우고, 그 결과 첫기동 관문 정책이 **결함이 있는 바로 그 기계들에도
    //   도달한다**(값 수정 경로로는 영원히 도달하지 못한다 — 아래 무접촉 규칙 때문이다).
    const LAYERED_KEYS: [&str; 3] = [
        "ready_marker",
        "approval_patterns",
        cys::first_run_gates::ADAPTER_KEY,
    ];
    // 보강 사실을 사람에게 알릴 키. `first_run_gates` 는 제외한다 —
    //   ① 이 키는 **모든 기존 기계에서 매번** 결손이라 매 launch 마다 같은 줄이 나간다(순수 소음).
    //   ② 안내 문안이 권하는 `pack-merge` 가 이 키에서는 **해로운 조치**다: 디스크로 병합되는
    //      순간 사용자 소유가 되어 이후 벤더 갱신(관문 증식·버전 핀)이 도달하지 않는다.
    const NOTIFY_KEYS: [&str; 2] = ["ready_marker", "approval_patterns"];
    let Some(emb) = embedded else { return };
    if !resolved.is_object() {
        return;
    }
    let mut filled: Vec<&str> = Vec::new();
    for k in LAYERED_KEYS {
        if resolved.get(k).is_some() {
            continue; // 디스크 선언(null 포함)은 손대지 않는다
        }
        if let Some(v) = emb.get(k) {
            resolved[k] = v.clone();
            if NOTIFY_KEYS.contains(&k) {
                filled.push(k);
            }
        }
    }
    if !filled.is_empty() {
        eprintln!(
            "[agents] 내 agents.json 에 없는 [{}] 을 **내장 vendor 값으로 보강**했다 \
             (필드 계층 · 디스크 파일 무변경). 영속 편입: cys pack-merge --file agents.json",
            filled.join(", ")
        );
    }
}

/// ★(W4 · B19) 폴더신뢰 프롬프트 패턴을 **어댑터 선언에서** 읽는다 — 종전 하드코딩
/// (`trustthisfolder`/`Doyoutrust`)은 claude 문면 변경·타 CLI 도입 때마다 코드 수정을 강요했다.
/// 소스 = agents.json `approval_patterns` 중 `name=="trust-prompt"` 항목의 `pattern`.
/// ★범위: **trust-prompt 항목만** 여기서 소비한다. 그 외 패턴(tool-permission 등)은 데몬 승인
/// 격상 스캔용이고 자동 응답 대상이 아니다(agents.json `_doc` 계약 — 사람 판단 보존).
/// 컴파일 실패는 None(=내장 needle 폴백) — 사용자가 정규식을 깨뜨려도 부트가 멈추지 않는다.
fn trust_prompt_regex(spec: &Value) -> Option<regex::Regex> {
    spec["approval_patterns"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|p| p["name"].as_str() == Some("trust-prompt"))
                .and_then(|p| p["pattern"].as_str())
        })
        .and_then(|pat| match regex::Regex::new(pat) {
            Ok(re) => Some(re),
            Err(e) => {
                eprintln!(
                    "[launch-agent] agents.json trust-prompt pattern 컴파일 실패({e}) \
                     — 내장 needle 로 폴백"
                );
                None
            }
        })
}

/// 폴더신뢰 프롬프트가 **신규 출현분**에 있는가.
/// ★매칭 대상 결정(정규식 vs delta_flat): 선언 `pattern` 은 **정규식**이라 공백·`(a|b)`·`?` 를
///   쓰므로, 공백을 전부 제거한 `delta_flat` 에는 원리상 맞지 않는다(기존 하드코딩이 공백 없는
///   `trustthisfolder` 형태였던 것도 매칭 대상이 flat 텍스트였기 때문이다). 그래서 정규식은
///   **공백을 1칸으로 정규화한 원문 델타**에 돌린다 — 정규식의 공백 의미를 지키면서 TUI 폭에 따라
///   프롬프트가 접히는 줄바꿈·들여쓰기도 흡수한다(원문 그대로 돌리면 줄바꿈 한 번에 매칭이 깨져
///   자동확인이 불발하고, 그 대가는 '노드 0 + 고아 좌석'이다).
/// ★(U-15) 폴백 축이 **하드코딩 needle → 코퍼스(U-12 정본) 소비**로 이사했다.
///   종전 폴백은 `delta_flat.contains("trustthisfolder")` 였고, 그것이 킬체인의 실제 방아쇠다 —
///   폴더신뢰를 통과하면 화면에 확인 에코 `Yes, I trust this folder ✔` 가 남는데 그 문자열이
///   같은 needle 에 **재매칭**된다(공백 제거형이라 더 잘 걸린다). 그 순간 화면은 이미 면책 창
///   (기본 포커스 `No, exit`)이라 2발째 Return 이 좌석을 rc 1 로 죽였다.
///   정본 코퍼스의 folder-trust needle 은 **질문형 문면만** 담고, 확인 에코·버튼 라벨은
///   `confirm_echo` 로 분리돼 "needle 이 어떤 에코에도 포함되지 않는다"는 불변식 검체가
///   그 형태를 구조적으로 금지한다. 그래서 사본을 새로 만들지 않고 그 정본을 읽는다(S-1 차단).
///   ★구 하드코딩 needle 은 **삭제가 아니라 롤백 분기로 격하**된다(`CYS_TRUST_RETURN_V1=1`):
///     실측상 선언 패턴(`Do you trust the files in this folder`)은 claude 2.1.236~241 어디에도
///     없으므로, 감지 폭이 예상 밖으로 좁아졌을 때 되돌릴 손잡이를 남겨 둔다. 되돌려도 킬체인은
///     열리지 않는다 — 전송은 1발 래치와 화면 재확인(U-14 축)이 따로 막는다.
fn trust_prompt_hit(
    re: Option<&regex::Regex>,
    gates: &[cys::first_run_gates::Gate],
    delta_text: &str,
    delta_flat: &str,
    legacy_v1: bool,
) -> bool {
    if let Some(re) = re {
        let norm: String = delta_text.split_whitespace().collect::<Vec<_>>().join(" ");
        if re.is_match(&norm) {
            return true;
        }
    }
    if cys::inject_guard::folder_trust_needle_hit(gates, delta_text) {
        return true;
    }
    legacy_v1 && (delta_flat.contains("trustthisfolder") || delta_flat.contains("Doyoutrust"))
}

/// agents.json에서 어댑터 스펙 로드
fn load_agent_spec(agent: &str) -> Result<Value, String> {
    let agents_path = cys::pack::pack_dir().join("agents.json");
    // agents.json 은 user 소유(★W-B) — 손상돼도 치유가 자동 복구하지 않으므로 부재/파싱 실패를
    // 구분해 복구 경로를 정확히 안내한다(부재→init-pack 시드 / 손상→take-new 또는 삭제 후 재시드).
    let raw = std::fs::read_to_string(&agents_path).map_err(|_| {
        format!(
            "agents.json not found at {} — run `cys init-pack` first",
            agents_path.display()
        )
    })?;
    let agents: Value = serde_json::from_str(&raw).map_err(|e| {
        format!(
            "agents.json 파싱 실패({e}) — 사용자 수정 중 손상된 듯. 복구: 파일을 백업·삭제 후 \
             `cys init-pack`(vendor 본 재시드) → 백업에서 커스텀 어댑터만 되살리기",
        )
    })?;
    // ★임베드는 두 경로가 **공통으로** 참조한다 — 디스크 히트에도 필드 계층(fill_missing_fields)이
    //   걸리므로 whole-object 폴백보다 먼저 해소해 둔다.
    let embedded_agents: Option<Value> = embedded_agents_json();
    if let Some(spec) = agents.get(agent) {
        let mut spec = spec.clone();
        fill_missing_fields(&mut spec, embedded_agents.as_ref().and_then(|v| v.get(agent)));
        return Ok(spec);
    }
    // ★W-B 보완(성찰 2 적대검증 산물): user 승격의 대가 = 동결 — 사용자가 agents.json 을 수정해
    // 두면 vendor 가 **새 어댑터**를 출하해도 .new 병치만 되고 디스크 본엔 영영 안 들어와
    // "신규 CLI 지원했는데 안 됨"이 된다(schedule.json 은 데몬의 ensure_builtin_jobs 가 같은
    // 문제를 코드로 메우지만 agents.json 엔 그 보완이 없었다). 디스크에 없는 키만 **임베드
    // 어댑터로 폴백**해 '사용자 수정 보존'과 'vendor 신기능 즉시 사용'의 합집합을 만든다.
    // (덮어쓰기 0 — 디스크 정의가 있으면 항상 디스크가 이긴다.)
    if let Some(spec) = embedded_agents.as_ref().and_then(|v| v.get(agent)) {
        eprintln!(
            "[agents] '{agent}' 은 내 agents.json 에 없어 **내장 정의로 폴백**했다 \
             (vendor 신규 어댑터 — 내 수정본은 그대로 보존됨). 편입하려면: cys pack-merge --file agents.json"
        );
        let mut spec = spec.clone();
        // 대칭 유지(경로별 특례 금지) — 같은 소스라 실제로 채울 것은 없다(no-op).
        fill_missing_fields(&mut spec, embedded_agents.as_ref().and_then(|v| v.get(agent)));
        return Ok(spec);
    }
    Err(format!("unknown agent '{agent}' (agents.json에 정의 필요)"))
}

/// 역할 디렉티브 + soul.md + 장기메모리 색인 + 스킬 색인 조립 (launch/reinject/cycle 공용)
fn compose_directive(role: &str) -> Result<String, String> {
    let dir = cys::pack::pack_dir();
    // 표준 4역할 외(임시 역할 — fresh heartbeat의 scan-bot 등)는 WORKER 지침으로 폴백
    let directive_path = cys::pack::role_directive_path(role).unwrap_or_else(|| {
        eprintln!("[directive] non-standard role '{role}' — falling back to WORKER_DIRECTIVE");
        dir.join("directives/WORKER_DIRECTIVE.md")
    });
    let mut directive = std::fs::read_to_string(&directive_path)
        .map_err(|e| format!("cannot read {}: {e}", directive_path.display()))?;
    // RSI 학습 directive(5번째)는 master·worker 양쪽에 추가 주입한다(cso·reviewer 제외 — RSI
    // 학습 루프 주체는 master·worker). 기존 역할 directive 흐름은 보존하고 뒤에 이어붙인다.
    if role == "master" || role.starts_with("worker") {
        let rsi_path = dir.join("directives/RSI_LEARNING_DIRECTIVE.md");
        // ★fail-closed(codex REVISE): 5번째 절대지침 누락을 침묵 통과시키지 않는다 — 다른 directive
        // 읽기와 동일하게 실패 시 Err. 침묵 스킵은 학습 봉쇄 지침 없는 master·worker 각성을 부른다.
        let rsi = std::fs::read_to_string(&rsi_path)
            .map_err(|e| format!("cannot read {}: {e}", rsi_path.display()))?;
        directive.push_str("\n\n■ RSI_LEARNING_DIRECTIVE.md (5번째 절대지침 — 학습 루프)\n");
        directive.push_str(&rsi);
    }
    let soul_path = dir.join("soul.md");
    if let Ok(soul) = std::fs::read_to_string(&soul_path) {
        directive.push_str("\n\n■ soul.md (운영 헌장)\n");
        directive.push_str(&soul);
    }
    // 장기메모리 색인 동봉 — 본문(1파일 1사실)은 필요 시 해당 파일을 읽어 점진 로드.
    // 헤더에 절대경로를 박는다: 노드가 본문 읽기·증류 쓰기 위치를 추론하지 않게(결정론).
    let memory_path = dir.join("memory/MEMORY.md");
    if let Ok(memory) = std::fs::read_to_string(&memory_path) {
        directive.push_str(&format!(
            "\n\n■ 장기메모리 색인 ({} — 노드 공유 의미 기억 · 증류는 bin/javis_memory.py add)\n",
            memory_path.display()
        ));
        directive.push_str(&memory);
    }
    // 스킬 색인(표지) 동봉 — 본문은 필요 시 `cys skill show <name>`으로 점진 로드.
    // ① 오버레이: ~/.cys/local/skills 가 동명 팩 스킬을 shadowing(업데이트 불가침 사용자 커스텀).
    let scan_skills = |root: &std::path::Path| -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                if let Ok(content) = std::fs::read_to_string(entry.path().join("SKILL.md")) {
                    let (mut name, mut desc) = (String::new(), String::new());
                    for line in content.lines().take(10) {
                        if let Some(v) = line.strip_prefix("name:") {
                            name = v.trim().to_string();
                        } else if let Some(v) = line.strip_prefix("description:") {
                            desc = v.trim().to_string();
                        }
                    }
                    if !name.is_empty() {
                        out.push((name, desc));
                    }
                }
            }
        }
        out
    };
    let mut skill_index: std::collections::BTreeMap<String, (String, bool)> = Default::default();
    for (name, desc) in scan_skills(&dir.join("skills")) {
        skill_index.insert(name, (desc, false));
    }
    for (name, desc) in scan_skills(&cys::pack::local_dir().join("skills")) {
        skill_index.insert(name, (desc, true)); // 동명 → local 이 이긴다(shadowing)
    }
    if !skill_index.is_empty() {
        directive.push_str("\n\n■ 보유 스킬 색인 (본문: `cys skill show <name>`)\n");
        for (name, (desc, local)) in &skill_index {
            directive.push_str(&format!(
                "- {name}: {desc}{}\n",
                if *local { " [local 오버레이]" } else { "" }
            ));
        }
    }
    // ① 사용자 로컬 디렉티브 오버레이(~/.cys/local/directives/<ROLE>_DIRECTIVE.local.md) —
    // 업데이트·치유가 절대 건드리지 않는 사용자 영역. 안전핵 키워드 줄은 strip(오버라이드 동일
    // 필터·⑥ 경계). 아래 render_block 의 SAFETY_CORE_REASSERT last-word 가 항상 뒤따르게 한다.
    let mut local_appended = false;
    if let Some(stem) = directive_path.file_name().and_then(|n| n.to_str()) {
        let local_name = format!("{}.local.md", stem.trim_end_matches(".md"));
        let local_path = cys::pack::local_dir().join("directives").join(&local_name);
        if let Ok(raw) = std::fs::read_to_string(&local_path) {
            let (clean, warnings) = cys::overrides::sanitize_local_directive(&raw);
            for w in &warnings {
                eprintln!("[directive] ⚠ {w}");
            }
            if !clean.trim().is_empty() {
                directive.push_str(&format!(
                    "\n\n■ 사용자 로컬 지침 ({} — 오버레이 · 업데이트 불가침 · 안전핵 뒤집기 불가)\n",
                    local_path.display()
                ));
                directive.push_str(clean.trim_end());
                directive.push('\n');
                local_appended = true;
            }
        }
    }
    // 사용자 오버라이드(취향·운영 노브) — 스킬 색인 뒤. PACK 밖 파일이라 install 불가침·
    // 정식 directive 무동결. render_block이 SAFETY_CORE_REASSERT를 항상 최후에 둬(last-word)
    // 사용자 텍스트가 안전핵을 못 뒤집는다. 파일 부재 시 빈 문자열(회귀 0).
    let expert = std::env::var("CYS_OVERRIDE_EXPERT").map(|v| v == "1").unwrap_or(false);
    let ov = cys::overrides::load_overrides(role, expert);
    let ov_block = cys::overrides::render_block(&ov);
    if ov_block.is_empty() && local_appended {
        // 오버라이드 파일이 없어도 로컬 지침이 붙었으면 안전핵 재선언이 최후(last-word)여야 한다(⑥).
        directive.push_str(cys::overrides::SAFETY_CORE_REASSERT);
    } else {
        directive.push_str(&ov_block);
    }
    // T4-3 ②: 런타임 카탈로그 플레이스holder 치환 — 정적 본문에 `$action_catalog`가 있으면
    // 실제 레지스트리(edit_kinds::EditKind)에서 파생한 카탈로그로 교체(하드코딩 미주입 = Max
    // 토큰효율 + 반드리프트). 플레이스홀더 부재 시 무변(회귀 0). 단건 상세는 on-demand
    // (`editor.action_info` RPC) — 전체 산문 미주입.
    let directive = cys::action_catalog::substitute_catalog(&directive);
    Ok(directive)
}

/// 꼬리 줄이 **Windows 셸 프롬프트의 형태**인가 — `>` 종결자 전용 보강 판정.
///
/// 왜 `t.ends_with('>')` 하나로 끝내지 않는가: `>` 는 Unix 종결자 4종(`% $ # ❯`)과 달리
/// 일반 본문에 흔하다(다이어그램 `-->`·화살표 함수 `=>`·태그 `<div>`·autolink `<https://…>`
/// ·리다이렉션 `2>&1 >`). 무조건 종결자로 넣으면 **오탐이 늘고**, 오탐이 늘면 ready 선언이
/// 줄어 `Err` → 롤백 close 가 늘어난다 = 건강한 pane 이 닫히는 방향. 그래서 `>` 는
/// **프롬프트 형태(PS 접두 또는 드라이브 경로)와 AND** 로만 참이 된다.
/// 형태 근거는 실측 캡처다: `PS C:\Users\x>`(사용자명 발행 제네릭화 — 재는 축은 이름이 아니라 `PS `·드라이브·`>` 형태다 · docs/plans/2026-07-29-win-two-defects-plan.md:319-323)
/// · `PS C:\WINDOWS\system32\WindowsPowerShell\v1.0>`(Parallels VM master pane 실화면).
///
/// ★이 술어 자신에는 `cfg(windows)` 를 두지 않는다 — 순수 형태 판정이라 진리표를 전 OS 에서
/// 돌릴 수 있어야 한다. **OS 게이트는 호출부**(`screen_tail_is_shell_prompt_on`)에 있다:
/// mac/Linux pane 의 워커가 Windows 안내문(`PS C:\Users\x>`)을 마지막 비공백 줄로 출력하면
/// 건강한 pane 이 '죽은 셸' 로 오판돼 롤백 close 대상이 되기 때문이다(U-9 × 호출부 상호작용).
fn windows_shell_prompt_shape(t: &str) -> bool {
    let Some(body) = t.strip_suffix('>') else {
        return false;
    };
    // ① PowerShell: `PS <위치>>` — 파일시스템/레지스트리/Env 공급자 공통, 축약 `PS>` 포함.
    //    연속행 프롬프트 `PS C:\x>>` 도 body 가 `PS …>` 라 여기서 참이 된다(프롬프트 맞다).
    if body == "PS" || body.starts_with("PS ") {
        return true;
    }
    // ② cmd.exe: `<드라이브문자>:\<경로>>` — `C:\>` · `C:\Users\x>` · `D:\work\cys>`.
    //    콜론+역슬래시까지 요구해 "3 > 2 이므로 a > b>" 류 산문을 배제한다.
    let mut it = body.chars();
    matches!(
        (it.next(), it.next(), it.next()),
        (Some(d), Some(':'), Some('\\')) if d.is_ascii_alphabetic()
    )
}

/// 화면 마지막 비공백 줄이 셸 프롬프트로 끝나는지 판정 — marker 없는 에이전트의 시간 폴백
/// 직전 검사다. TUI가 떴다면 끝줄이 셸 프롬프트일 수 없다; 셸 프롬프트가 남아 있으면
/// 에이전트가 조용히 즉시 종료(에러 문구 없이)한 것이므로 주입하면 zsh로 들어간다.
///
/// 실행 플랫폼은 `cfg!(windows)` 로 정해지며, 실동작은 전부 `_on` 에 있다(진리표가 전 OS 에서
/// 두 축을 모두 돌릴 수 있게 — 테스트 가능성 손실 0).
fn screen_tail_is_shell_prompt(text: &str) -> bool {
    screen_tail_is_shell_prompt_on(text, cfg!(windows))
}

/// `screen_tail_is_shell_prompt` 의 순수 본체 — `windows` 는 "이 pane 이 Windows 셸 위에서
/// 도는가"다.
///
/// ★왜 Windows 축을 OS 로 가두는가(P1-5): `windows_shell_prompt_shape` 는 화면 **텍스트**만
/// 본다. mac/Linux pane 에서 워커가 Windows 안내문(`PS C:\Users\x>` · `C:\Users\x>`)을 출력하고
/// 그게 마지막 비공백 줄이 되면, 살아있는 TUI 가 없는 것도 아닌데 '죽은 셸' 로 판정돼
/// **건강한 pane 이 close 대상**이 된다. 형태 요건이 Windows 고유라 해도 그건 *프롬프트의*
/// 고유성이지 *화면 텍스트의* 고유성이 아니다 — 유닉스 pane 은 남의 프롬프트 문자열을
/// 얼마든지 그릴 수 있다. 그래서 판정은 "형태 ∧ 실제 Windows" 로만 참이 된다.
/// Unix 종결자 4종(`% $ # ❯`)은 게이트하지 않는다: Windows 콘솔에도 git-bash·WSL 프롬프트가
/// 뜰 수 있어 가두면 그쪽에서 오부정(죽은 셸에 주입)이 생긴다 — 위험 방향이 반대다.
fn screen_tail_is_shell_prompt_on(text: &str, windows: bool) -> bool {
    let Some(last) = text.lines().rev().find(|l| !l.trim().is_empty()) else {
        return false; // 화면 비어 있음 — 판단 보류(시간 폴백 유지)
    };
    let t = last.trim_end();
    // zsh "...%" / bash·sh "...$" / root "#" / powerlevel10k·starship "❯" —
    // 끝문자 기준(프롬프트 커스텀의 공통 꼬리). 오탐 효과는 '대기 후 명시 Err'(안전측).
    // Windows(PowerShell `PS C:\…>` · cmd `C:\>`)는 `>` 로 끝나므로 형태 요건과 함께 추가한다
    // (T-D4 / F4-cys-boot-launch-06 — 이게 빠져 Windows 에서 marker 없는 어댑터의 시간 폴백이
    // 죽은 PowerShell 을 ready 로 선언하고 54KB 디렉티브를 셸 명령으로 제출했다).
    // 판정 축은 그대로다 — '마지막 비공백 줄의 끝문자' 규칙을 유지한 채 종결자만 늘렸다.
    t.ends_with('%')
        || t.ends_with('$')
        || t.ends_with('#')
        || t.ends_with('❯')
        || (windows && windows_shell_prompt_shape(t))
}

/// TUI 렌더 증거 문면 — **살아 있는 대화형 위젯**이 화면에 있음을 뜻하는 실측 문자열.
///
/// 배너·인사말은 넣지 않는다: 그것들은 에이전트가 한 번 출력하고 **죽은 뒤에도** 화면에
/// 남으므로 "지금 렌더 중"의 증거가 못 된다(그 방향의 오판은 죽은 셸에 주입이다).
/// 여기 있는 것은 **입력을 기다리는 동안에만** 그려진다.
///   · `for shortcuts` — 프롬프트 대기 상태의 힌트 줄(`? for shortcuts`).
///
/// ## ★M6 수리(2026-08-24 자기성찰 3회전) — 관문 위젯 서명 둘을 **뺐다**
///
/// 종전 코퍼스는 `["Enter to confirm", "Esc to cancel", "for shortcuts"]` 였고 **앞 둘은
/// 관문 코퍼스 6종 중 3종(폴더신뢰 · 면책 · 신기능 안내)의 위젯 서명 그 자체**다
/// (`first_run_gates` 의 `widget` 필드와 문자 그대로 같다).
///
/// 그래서 **코퍼스에 없는 새 관문**이 뜨면 그 위젯 푸터가 여기서 '렌더 증거' 로 세어져
/// `bare_shell=false` → 안전 밸브(`positive_evidence` 의 **첫 항** · 조기 return)가 열리고
/// `Verdict::Ready` 가 나온다 — **모르는 관문일수록 디렉티브가 더 잘 나간다**는 역방향
/// 성질이었다(회전2 실주행: 코퍼스 미매칭 화면에 "안전 밸브 — 주입 안전" 이 뜨고 디렉티브
/// 전량이 주입되는 것이 실제로 관측됐다).
///
/// ★움직이는 방향은 **오탐(보류) 쪽 하나뿐**이다: 축이 좁아지면 `screen_is_bare_shell_on` 은
///   참이 더 자주 되고 밸브는 **더 자주 닫힌다**. 밸브가 닫혀도 마커 델타·화면 폴백 경로는
///   그대로 남고, 최악의 귀결은 `GatePending`(좌석 보존)이지 close 가 아니다.
///
/// ★관문 문면을 판정에 쓰는 자리는 이제 **관문 축 하나**(`readiness::gate_on_screen` ·
///   `first_run_gates` 코퍼스)다 — 같은 문자열이 '관문이다' 와 '살아있다' 를 동시에 뜻하던
///   모순을 없앤 것이 이 수리의 본체다.
const TUI_RENDER_MARKS: &[&str] = &["for shortcuts"];

/// 프레임 **연속 길이** 하한(문자). 이 이상 이어져야 '프레임 자'로 인정한다.
///
/// 【왜 개수가 아니라 연속 길이인가 — P4-2 · 2026-08-24 이종 리뷰어】 종전 판별은
/// `text.chars().any(|c| 0x2500..=0x259F)` = **화면 어디든 박스 문자 1개**였다. 그런데 박스
/// 문자는 TUI 만 그리는 것이 아니다:
///   · powerlevel10k 2줄 프롬프트 — `╭─ ~/dev` / `╰─❯`
///   · starship·oh-my-posh 의 장식 프롬프트
///   · `tree` 출력의 `├──` `└──` · `git log --graph` 의 괘선
/// 그래서 p10k/starship 사용자의 기계에서는 **맨 셸이 영영 맨 셸로 판정되지 않았고**, 밸브의
/// AND 항이 공허해져 밸브가 `agent_alive` 단독으로 퇴화했다 — P1-1 이 막으려던 바로 그 상태다
/// (부모 커밋 `3014101` 대비 회귀).
///
/// 【판별의 방향】 실측상 TUI 프레임은 **pane 폭을 가로지르는 자**다(`╭──────╮` · 면책 창
/// 구분선 `─────…`). 반대로 프롬프트 장식·트리 괘선은 1~3 글자다. 그래서 축을 '존재' 에서
/// **'연속 길이'** 로 바꾼다. 값 8 은 두 부류가 실제로 갈리는 자리이고(장식 최대 3 ≪ 8 ≤ 자),
/// 검체 `bare_shell_predicate_separates_a_live_tui_from_a_dead_shell` ⓓ 가 양쪽을 다 건다.
///
/// ★방향 확인: 이 축이 좁아지면 `screen_is_bare_shell_on` 은 **참이 더 자주** 되고 밸브는
///   **더 자주 닫힌다**. 즉 판정이 느슨해지는 방향이 아니라 조여지는 방향이다(주입 억제).
const TUI_FRAME_RUN_MIN: usize = 8;

/// 한 줄 안에 프레임 문자가 [`TUI_FRAME_RUN_MIN`] 이상 **연달아** 있는가.
///
/// 줄 단위로 보는 이유: 프레임 자는 한 줄 안에서 이어진다. 화면 전량을 이어 붙여 세면
/// 서로 다른 줄의 장식 문자가 합쳐져 없는 자를 만들어낸다.
fn screen_has_frame_rule(text: &str) -> bool {
    text.lines().any(|line| {
        let mut run = 0usize;
        for c in line.chars() {
            // Box Drawing `U+2500..=U+257F` · Block Elements `U+2580..=U+259F`.
            if matches!(c as u32, 0x2500..=0x259F) {
                run += 1;
                if run >= TUI_FRAME_RUN_MIN {
                    return true;
                }
            } else {
                run = 0;
            }
        }
        false
    })
}

/// 화면에 **TUI 가 렌더 중이라는 증거**가 있는가 — `screen_is_bare_shell_on` 의 AND 항.
///
/// 두 축을 본다.
///   ① **프레임 자**([`screen_has_frame_rule`] — 연속 길이 하한이 있다).
///      실측 근거: `╭──────╮` 형태의 위젯 테두리 · 면책 창 구분선 `─────…`(PROBE_RESULTS.md
///      측정 2). 맨 셸의 프롬프트 장식은 이 길이에 닿지 않는다.
///   ② 위 [`TUI_RENDER_MARKS`] 의 대화형 위젯 문면.
///
/// ★단서 하나가 화면에 **남아 있다**는 것과 TUI 가 **지금 그리고 있다**는 것은 다른 사실이다.
///   이 함수가 낼 수 있는 최선은 후자의 근사이고, 근사의 오차는 언제나 '주입하지 않는 쪽'
///   으로 접어야 한다 — 그래서 축은 넓히는 것이 아니라 **좁히는** 방향으로만 고친다.
fn screen_has_tui_render_evidence(text: &str) -> bool {
    if screen_has_frame_rule(text) {
        return true;
    }
    let flat = cys::first_run_gates::flatten(text);
    TUI_RENDER_MARKS
        .iter()
        .any(|m| flat.contains(&cys::first_run_gates::flatten(m)))
}

/// ★(P3-0) **안전 밸브 전용** 술어 — 이 화면이 맨 셸인가.
///
/// 【왜 `screen_tail_is_shell_prompt` 를 그대로 쓰면 안 되는가】 그 함수는 이름과 달리 셸
/// 프롬프트 탐지기가 아니라 **마지막 비공백 줄의 끝문자가 `%` `$` `#` `❯`(+Windows 형태)인지**
/// 보는 검사다. 그런데 `❯` 는 **살아있는 Claude Code TUI 의 입력 프롬프트 그 자체**이므로
/// 건강한 pane 의 꼬리가 일상적으로 `❯` 다. 밸브의 AND 항으로 쓰면 **건강한 pane 에서 밸브가
/// 상시 차단**되고, 밸브의 존재 이유(델타 가정이 깨져도 살아있는 pane 이 전부 닫히지 않게
/// 하는 영구 오부정 차단)가 통째로 사문화된다.
///
/// 【축의 비용 부호가 반대다】 ready 판정(마커·시간 폴백)은 **정밀도**가 필요하고 — 오탐의
/// 대가가 '관문에 주입' 이다 — 그래서 거기서는 넓은 꼬리 술어가 옳다. 밸브는 **재현율**이
/// 필요하고 — 오탐의 대가가 '건강 pane 미기동' 이다 — 그래서 "맨 셸인가" 의 **높은 정밀도**
/// 판별이 필요하다. 두 축을 한 술어로 겸하면 한쪽이 반드시 틀린 폭을 갖는다.
///
/// 【판별자】 `꼬리가 셸 프롬프트 ∧ (꼬리에 사망 문면 ∨ ¬화면에 TUI 렌더 증거)`.
/// 이 술어는 꼬리 술어보다 **참이 덜 되므로** 밸브 재현율은 오직 올라가고, 오살 방향으로는
/// 열리지 않는다(맨 셸이면 여전히 참).
///
/// 【★남는 미탐의 실제 귀결 — 2026-08-24 정정(P4-1b)】 이 자리에는 종전에 이렇게 적혀 있었다:
///   "남는 미탐은 '프레임을 그린 뒤 즉사' 뿐인데, 그 화면은 마커 축이 따로 막고 최악이어도
///    귀결은 U-11 의 보류(좌석 보존)다."
/// **거짓이다.** `readiness::positive_evidence` 의 사다리에서 밸브는 **첫 항**이고
/// `if o.agent_alive == Some(true) && bare_shell_ok { return Some(Evidence::Valve); }` 로
/// **조기 반환**한다 — 밸브가 열리는 순간 마커 축은 한 줄도 평가되지 않으므로 "마커 축이
/// 따로 막는다" 는 성립할 수 없다. 이 미탐의 실제 귀결은 보류가 아니라 **주입**이다:
/// 프레임을 그린 뒤 즉사한 셸에 디렉티브가 들어간다. 축소하지 않고 그대로 적는다.
/// (`readiness.rs` 쪽 사본은 이미 정정됐고 여기 사본만 남아 있었다 — 사본이 낡는다는 것의
///  교과서적 예다. 그래서 이 문장은 기계 검체
///  `readiness::tests::valve_short_circuits_the_ladder_so_the_marker_axis_is_never_consulted`
///  가 박제하고, 이 doc 은 그 검체를 가리킨다.)
///
/// 【그래서 무엇을 했는가 — P4-2】 미탐을 줄이는 방향으로 두 축을 조였다.
///   ① 프레임 축을 '박스 문자 1개' 에서 **연속 길이**([`TUI_FRAME_RUN_MIN`])로 좁혔다 —
///      p10k/starship 프롬프트 장식·`tree` 괘선이 밸브를 무장해제하던 표면을 없앴다.
///   ② **사망 문면**([`screen_shows_launch_failure`])을 OR 로 더했다 — 렌더 증거가 남아 있어도
///      꼬리에 기동 실패 문면이 보이면 그 화면은 맨 셸이다(다중 방어 · 둘 중 하나가 미래에
///      다시 넓어져도 나머지가 선다).
fn screen_is_bare_shell(text: &str) -> bool {
    screen_is_bare_shell_on(text, cfg!(windows))
}

/// 사망 문면을 찾을 **꼬리 창**(비공백 줄 수). 화면 전량을 보면 50줄 전 잔상 하나가 살아있는
/// TUI 를 맨 셸로 만든다 — 그 방향의 오판은 안전측(주입 안 함)이지만, 근거 없이 넓히지는 않는다.
const BARE_SHELL_DEATH_TAIL_LINES: usize = 5;

/// 위의 순수 본체 — `windows` 는 "이 pane 이 Windows 셸 위에서 도는가"(형제 술어와 같은 계약).
fn screen_is_bare_shell_on(text: &str, windows: bool) -> bool {
    if !screen_tail_is_shell_prompt_on(text, windows) {
        return false;
    }
    // ★사망 문면(다중 방어 ②) — 꼬리에 `command not found` 류가 보이면 렌더 잔상과 무관하게
    //   맨 셸이다. 판정 방향은 '주입 억제' 라 넓혀도 오살 위험이 없다.
    let tail = screen_tail_lines(text, BARE_SHELL_DEATH_TAIL_LINES);
    if screen_shows_launch_failure(&cys::first_run_gates::flatten(&tail)) {
        return true;
    }
    !screen_has_tui_render_evidence(text)
}

// ★(U-13 · 핀 이사) readiness **안전 밸브**의 판정부는 `src/readiness.rs` 로 옮겼다.
//
//   종전 이 자리에는 `readiness_safety_valve(_on)` 두 함수가 있었고 폴링 루프가 그것을 직접
//   불렀다. 그런데 ready 를 선언하는 자리가 그 밸브 말고도 셋(마커 델타·화면 폴백·시간 폴백)
//   더 있었고, 게다가 두 번째 소비처 `adapter_ready` 는 아무 가드도 없이 마커만 봤다 —
//   그래서 "마커 축만 고치면 판정이 하나도 안 바뀌는" 상태였다. 이 단위는 그 넷을 순수 술어
//   하나(`cys::readiness::judge`)로 합쳤다. 밸브는 **삭제되지 않았다**: 사다리의 첫 항으로
//   그대로 있고(`Evidence::Valve`), P1-1 이 붙인 AND(`alive ∧ ¬화면꼬리가_셸프롬프트`)도
//   근거 전문과 함께 그 자리로 옮겨 갔다.
//
//   여기(cys.rs)에 남는 것은 **관측**뿐이다: `agent_alive` 조회 · `screen_tail_is_shell_prompt`
//   호출 · 그 둘을 `Observed` 에 실어 판정에 넘기는 배선 한 줄. 판정 조건은 하나도 완화되지
//   않았고, 관문 문면 AND 항 하나가 **추가**됐다.
//   회귀 핀: `safety_valve_does_not_fire_when_only_a_wrapper_outlives_the_agent`(진리표는 이제
//   판정부를 관통한다) · `readiness_judgment_is_wired_at_the_call_site_source_pin`(배선) ·
//   python 검체 H-SAFE-2 ①(밸브 근거의 화면 무의존 · 마커보다 선행).

/// 기동 화면(공백 제거 평탄화 문자열)에 "명령을 못 찾았다"는 셸 오류가 떴는지 판정.
/// readiness 폴링이 죽은 셸에 지침을 주입하는 것을 막는 사망 감지의 핵심 술어다.
/// Unix sh/zsh/bash뿐 아니라 Windows PowerShell·cmd.exe의 표현까지 덮어
/// 크로스플랫폼으로 동일하게 기동 실패를 잡는다(`hook_command` OS 대칭화와 짝).
fn screen_shows_launch_failure(flat: &str) -> bool {
    // Unix: sh/zsh/bash "command not found" / 직접 실행 시 "No such file or directory" / "not found in PATH"
    flat.contains("commandnotfound")
        || flat.contains("notfoundinPATH")
        || flat.contains("Nosuchfileordirectory")
        // Windows PowerShell: "... is not recognized as the name of a cmdlet, function, ..."
        || flat.contains("isnotrecognizedasthenameofacmdlet")
        // Windows cmd.exe: "... is not recognized as an internal or external command, ..."
        || flat.contains("isnotrecognizedasaninternalorexternalcommand")
}

/// 살아있는 surface 위에서: 에이전트 기동 → 준비 폴링 → 지침 주입 → 메타 등록.
/// RC-3(B′): agents.json env 값의 셸 확장을 Rust에서 해소한다(Windows용 — unix는 셸이 직접 전개).
/// 지원 패턴: `${VAR:-default}`(현 agents.json 패턴)·`$HOME`·선두 `~`. HOME은 Windows에서
/// dirs::home_dir()(USERPROFILE 기반)로 해소 — env::var("HOME")이 Windows 미설정인 함정 회피(RC-7 동형).
fn resolve_env_value(v: &str) -> String {
    fn home() -> String {
        dirs::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
    let mut s = v.to_string();
    // ${VAR:-default} 한 겹 해소 (default 내부의 $HOME도 재귀 전개)
    if let (Some(a), Some(b)) = (s.find("${"), s.find('}')) {
        if a < b {
            let inner = &s[a + 2..b];
            let resolved = if let Some((name, default)) = inner.split_once(":-") {
                std::env::var(name)
                    .ok()
                    .filter(|x| !x.is_empty())
                    .unwrap_or_else(|| resolve_env_value(default))
            } else {
                std::env::var(inner).unwrap_or_default()
            };
            s.replace_range(a..=b, &resolved);
        }
    }
    s = s.replace("$HOME", &home());
    if let Some(rest) = s.strip_prefix("~/") {
        s = format!("{}/{}", home(), rest);
    }
    s
}

/// spec["env"] 맵 → 정렬된 (key, value) 벡터(결정론). 없으면 빈 벡터(레거시 cmd·env 없는 에이전트).
fn agent_env_pairs(spec: &Value) -> Vec<(String, String)> {
    spec.get("env")
        .and_then(|e| e.as_object())
        .map(|m| {
            let mut v: Vec<(String, String)> = m
                .iter()
                .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
                .collect();
            v.sort();
            v
        })
        .unwrap_or_default()
}

/// RC-3(B′): OS-aware 기동 렌더 — (pane에 send할 문자열, surface.create가 주입할 env).
/// unix: `KEY="val" ... cmd` 인라인 재조립(셸이 ${:-}·$HOME 전개 — **기존 단일문자열과 byte-identical**),
///       env 주입 없음(셸 전개가 진실원). → mac 무회귀(master D5 조건).
/// windows: 순수 cmd만 send(powershell이 POSIX env-assign 미해석 회귀 차단) + 해소된 env를 주입 맵으로 반환
///          (surface.create → builder.env). CLAUDE_CONFIG_DIR 등이 pane env에 직접 실린다.
fn render_launch(cmd: &str, env: &[(String, String)]) -> (String, Vec<(String, String)>) {
    if cfg!(windows) {
        let inject = env
            .iter()
            .map(|(k, v)| (k.clone(), resolve_env_value(v)))
            .collect();
        (cmd.to_string(), inject)
    } else {
        let mut s = String::new();
        for (k, v) in env {
            s.push_str(&format!("{k}=\"{v}\" "));
        }
        s.push_str(cmd);
        (s, Vec::new())
    }
}

/// (W1-5) resume 인자 해소 + claude 사전검증 게이트. `{session_id}` 정확 핀은 실제
/// `<config_dir>/projects/<munge cwd>/<id>.jsonl`이 실재할 때만 부착하고, 미실재면 None을 반환해
/// resume 자체를 생략한다(--continue 대체 금지 — 다른 대화 오염 방지). session_id 부재는 fallback,
/// placeholder 없는 arg·타 agent(codex 등)는 무변경. 파일시스템만 접근하는 순수 함수라 단위 테스트 가능.
fn resolve_resume_suffix(
    agent: &str,
    arg: &str,
    session_id: Option<&str>,
    config_dir: Option<&str>,
    cwd: Option<&str>,
    fallback: &str,
) -> Option<String> {
    if !arg.contains("{session_id}") {
        return Some(arg.to_string());
    }
    let Some(id) = session_id else {
        return Some(fallback.to_string());
    };
    if agent != "claude" {
        // 타 agent는 세션 파일 레이아웃을 검증할 수 없다 → 기존 정책 그대로(핀 부착).
        return Some(arg.replace("{session_id}", id));
    }
    let cfg = config_dir
        .map(String::from)
        .unwrap_or_else(cys::resolve_claude_config_dir);
    let comp = cys::claude_project_component(cwd.unwrap_or(""));
    let jsonl = format!("{cfg}/projects/{comp}/{id}.jsonl");
    if std::path::Path::new(&jsonl).exists() {
        Some(arg.replace("{session_id}", id))
    } else {
        eprintln!(
            "[launch-agent] resume 생략: 세션 파일 미실재 ({jsonl}) — 새 세션으로 기동(다른 대화 오염 방지)"
        );
        None
    }
}

/// (W1-4) restore 시 agents.json env 템플릿(`${CYS_ACCOUNT_DIR:-...}`) 대신 topology에 기록된 원
/// config_dir을 launch 문자열에 리터럴 인라인 오버라이드한다 — 데몬 env가 바뀌어도 원 계정 dir로 정확히
/// 재개. 신규 기동(restore=false)·config_dir 부재는 무변경(mac 무회귀·byte-identical 유지). spec env에
/// CLAUDE_CONFIG_DIR 키가 있을 때만 치환하므로 codex 등 타 agent엔 무영향(claude 한정).
fn apply_config_dir_override(
    env_pairs: &mut [(String, String)],
    restore: bool,
    config_dir: Option<&str>,
) {
    if !restore {
        return;
    }
    let Some(cfg) = config_dir else {
        return;
    };
    for (k, v) in env_pairs.iter_mut() {
        if k == "CLAUDE_CONFIG_DIR" {
            *v = cfg.to_string();
        }
    }
}

/// ★(W4 · D5 관측) launch-agent ready 판정 직후의 alternate-screen 통지 판정 — 순수 함수.
///
/// 입력 `alt_screen` = 데몬 surface.list 의 동명 필드(`as_bool()` — **필드 부재(구 데몬)는
/// None = 판정 불가**로 통지 자체를 생략한다. FAIL 격상 금지 — 스큐 규칙, 스펙 §D5).
/// 반환 = Some((stderr 1줄, directive.verify reason 에 부기할지)) / None = 발화 없음.
///  · mac ∧ claude ∧ true → **WARN**(D5 env 방어층이 우회된 fullscreen — 휠이 앱으로 들어가
///    프롬프트 히스토리 오염 경로가 열려 있다) + reason 부기 true.
///  · win ∧ claude ∧ true → 힌트 1줄(경보 아님·차단 없음 — win fullscreen 은 문제2 동형
///    발현이라 비지원 선언이고, 사용자가 원인을 자가진단할 단서만 남긴다) + reason 부기 false.
///    ★정정(2026-08-17 실측): 이 분기를 'claude 쪽 설정을 건드린 사용자만 밟는다'고 적었던 종전
///    문안은 틀렸다 — Claude Code 2.1.233 의 fullscreen 판정 함수 `ra()` 에 순수 Windows→inline
///    분기가 없고 Windows 관련 분기는 `Windows ∧ SSH` 하나뿐이며, settings 의 `tui` 키가 없으면
///    최종 판정은 서버측 기능 게이트가 한다. 즉 fullscreen 여부는 OS 가 아니라 계정·롤아웃이
///    결정하므로 이 분기는 아무 설정도 만진 적 없는 win 사용자에게도 걸린다(그래서 힌트는
///    유지하되 등급은 그대로 — 등급 상향은 win 에 차단 수단이 없는 상태에서 소음만 늘린다).
///    ★등급이 mac(WARN)과 다른 이유가 강등 후 **하나 더 늘었다**: mac 은 D5 env 를 기본 주입하므로
///    fullscreen 목격 = '기본 방어가 우회됨' 이라는 이상 신호지만, Windows 는 D5 가 **옵트인**
///    (기본 미주입 — lib.rs `d5_gate_for_os` doc)이라 fullscreen 은 이상이 아니라 **기본 상태**다.
///    기본 상태를 WARN 으로 찍으면 그것은 경보가 아니라 소음이다.
///    ★문안 함정(고치지 마라 — 일부러 이렇게 적었다): 이 힌트를 실제로 찍는 곳은
///    `boot_agent_on_surface` 안이고, 그 함수는 **기존 pane 재기동(node-recover)에도** 쓰인다.
///    Windows 의 그 경로는 `render_launch` 의 env 를 폐기하므로(`let (send, _send_env) = …`),
///    "agents.json env 를 확인하라"를 **확실한 해결책으로 안내하면 거짓말이 된다**. 그래서 문안은
///    ①본체 방어가 GUI 휠 가드임을 먼저 밝히고 ②env 경로는 '새 surface 를 만드는 launch-agent
///    기동에서만 실린다'는 조건을 붙여 말하며 ③**Windows 의 D5 는 옵트인이라 스위치를 먼저 켜야
///    한다**는 조건을 함께 말한다(③ 이 빠지면 "새 pane 을 띄우면 된다"가 거짓 안내가 된다 —
///    기본값에서는 새 pane 을 띄워도 그 env 가 주입되지 않는다). 진리표 핀은 `hint` 토큰과 부기
///    false 만 보므로 문안 개정은 자유롭다 — 다만 이 세 조건절을 지우지 마라.
/// OS 를 인자로 받아 어느 호스트에서든 양 분기를 테스트한다(lib compose_pane_path 관례).
fn alt_screen_notice(
    alt_screen: Option<bool>,
    agent: &str,
    is_macos: bool,
    is_windows: bool,
) -> Option<(String, bool)> {
    if agent != "claude" || alt_screen != Some(true) {
        return None; // None(구 데몬)=판정 불가 생략 · false=정상(inline) — 무발화.
    }
    if is_macos {
        return Some((
            "[launch-agent] WARN: claude 가 alternate screen(fullscreen)으로 떴다 — D5 기본 env \
             (CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1)가 settings tui 키 등으로 우회된 상태. \
             fullscreen 휠은 프롬프트 히스토리 오염 경로다: 해당 계정 settings 의 tui 키 제거 \
             (D1(d) 정규화·.bak 관례) 또는 agents.json env 확인."
                .to_string(),
            true,
        ));
    }
    if is_windows {
        return Some((
            "[launch-agent] hint: claude 가 alternate screen(fullscreen)으로 떴습니다 — Windows \
             fullscreen 은 비지원(릴리스 노트 '알려진 제한'). fullscreen 여부는 OS 가 아니라 \
             계정·롤아웃이 결정하므로 설정을 만진 적이 없어도 이렇게 뜹니다: settings 의 tui 키 \
             제거는 판정을 서버측 기능 게이트에 넘길 뿐 inline 을 보장하지 않습니다. 휠→방향키 \
             합성 오염은 GUI 의 Windows 휠 가드가 막습니다 — 끄려면 PowerShell 에서 \
             `New-Item -ItemType File -Force $HOME\\.cys\\win-wheel-guard-off` 를 실행한 뒤 \
             **새 pane 을 여세요** (`touch` 는 PowerShell·cmd 에 없는 명령입니다. 되돌리기 \
             취소는 Remove-Item). \
             env(CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN)로 inline 을 강제하려면 **두 가지가 함께** \
             필요합니다 — ①Windows 는 이 env 주입이 **기본 off(옵트인)** 입니다(실기 검증 전이라 \
             그렇습니다): `New-Item -ItemType File -Force $HOME\\.cys\\win-no-alt-screen` \
             (되돌리기 Remove-Item · env CYS_WIN_NO_ALT_SCREEN=1 도 동등하나 GUI 재시작 필요). \
             ②그 뒤 이 pane 이 아니라 **새 pane 을 launch-agent 로 기동**하세요 — Windows 에서 \
             env 는 새 surface 를 만들 때만 실립니다(기존 pane 재기동 경로는 env 를 싣지 \
             못합니다). agents.json 에 '0' 이 적혀 있으면 옵트인해도 주입하지 않습니다."
                .to_string(),
            false,
        ));
    }
    None
}

// ═══════════════════════════════════════════════════════════════════════════
// ★(U-11) readiness 결과의 **타입화** — 실패 귀결을 호출부마다 다시 정한다
// ═══════════════════════════════════════════════════════════════════════════
//
// **무엇이 문제였나**: `boot_agent_on_surface` 는 세 개의 전혀 다른 사실을 `Err(String)` 하나로
// 뭉갰다 — ⓐ에이전트가 **안 떴다** ⓑ에이전트는 **떴는데** 준비를 확정하지 못했다 ⓒ데몬 왕복
// 자체가 실패했다. 그리고 호출부 세 곳은 그 1비트를 각자 다르게 해석했다:
//   · `launch-agent` → `surface.close(cause="reap")`  = 좌석 파괴
//   · `node-recover` → rc 1 → `run_boot` 이 `escalate_reclaim` = **kill**
//   · `restore` in-seat → fresh 폴백 = 좌석 +1
// ∴ **관문에 갇힌 채 살아 있는 에이전트가 어느 문으로 들어오느냐에 따라 닫히거나 죽거나
//   중복 좌석이 됐다.** 판정이 하나인데 귀결이 셋이면 그 셋 중 최소 둘은 틀린다.
//
// **왜 이것이 다음 단위(ready 술어 엄격화)보다 먼저인가** — 치명위험 ④(전 pane 사망):
// ready 판정을 좁히는 순간 벤더 문면이 한 글자만 바뀌어도 **전 좌석이 미충족**이 된다.
// 그때 미충족의 기본 귀결이 여전히 close 라면 그 한 글자가 팀 전체를 닫는다(글자 0 화면).
// 그래서 **실패 경로를 먼저 안전하게** 만든다: 미충족의 기본 귀결을 '파괴' 에서 '보류' 로 옮긴다.
//
// **오늘 무엇이 실제로 달라지는가(정직한 범위)**: 현 ready 술어에서는 안전 밸브
// (`agent_alive` ∧ 꼬리≠셸프롬프트)가 관문 화면을 **ready 로 통과시키므로**, 타임아웃까지
// 내려오는 좌석은 대개 '아무것도 안 뜬' 경우다. 즉 이 단위의 즉시 효과는
// **"프로세스 생존이 관측된 좌석은 더 이상 닫지 않는다"** 한 줄이고(밸브의 '영구 오부정'
// 가정이 깨지는 날 팀 전체를 살리는 것이 그 한 줄이다), 관문 판정 자체는 다음 단위가 붙인다.
// 이 단위는 그 판정이 착지할 **귀결 배선**을 먼저 놓는 것이다.

/// readiness 판정의 **타입드 결과**. `Err(String)`(=데몬 왕복 실패 등 절차 미완)과 함께
/// `Result<BootVerdict, String>` 로 반환된다 — `Err` 의 귀결은 **종전 그대로**(close)다.
///
/// ★세 변형의 경계는 **커널 사실**이지 화면 문자열이 아니다(H-SAFE-2 밸브와 같은 규율):
/// 화면으로 파괴를 결정하면 렌더 방식·벤더 문면 한 번의 변화가 곧 좌석 파괴가 된다.
#[derive(Debug, Clone, PartialEq)]
enum BootVerdict {
    /// 준비 확정 — 종전 성공 경로와 완전히 동일하다.
    Ready,
    /// **살아 있으나 준비 미확정** — 프로세스는 관측되는데 입력 가능 상태를 확정하지 못했다.
    /// 실측상 이 상태의 대표 원인이 첫기동 관문(테마 → 로그인방식 → OAuth → 폴더신뢰 →
    /// 면책 → 새기능안내)이라 U-10 의 좌석 제4 등급 이름을 그대로 쓴다.
    /// **귀결: close 0 · kill 0 · 주입 0 · 좌석 보존 + 표식 + 처방 문안.**
    GatePending { gate: String, tail: String },
    /// 진짜 기동 실패 — 화면이 기동 실패를 **확증**했거나, 커널이 "그 프로세스는 없다"고 했다.
    /// **귀결: 종전 그대로**(launch=close · node-recover=rc1 → reclaim · restore=fresh 폴백).
    LaunchFailed { evidence: String },
}

/// 관문을 **식별하지 못했을 때**만 쓰는 라벨(N2 · 2026-08-24). 자리표시자는 한 곳에서만
/// 정의한다 — 리터럴이 여기저기 박히면 "알면서 버린 자리" 와 "정말 모르는 자리" 가 밖에서
/// 구별되지 않는다. 그 구별이 사라진 것이 이 파일의 `gate=unknown` 결함 본체였다.
const GATE_ID_UNIDENTIFIED: &str = "unknown";

/// readiness **타임아웃**의 분류 — 순수 함수(진리표 테스트 대상 · 화면 문자열 비의존).
///
/// | `alive`(데몬 관측) | 판정 | 근거 |
/// |---|---|---|
/// | `Some(true)` | `GatePending` | 커널 프로세스 표에 에이전트가 **있다**. 살아 있는 것을 닫는 것이 이 저장소가 가장 비싸게 치른 실수다("오살이 오탐보다 훨씬 위험하다"). |
/// | `Some(false)` | `LaunchFailed` | 사망감지가 **'보였다가 사라짐' 전이를 확정**했다(`agent_seen` → `agent_exit_notified`) + 준비 예산을 다 썼다. 종전 귀결 유지. |
/// | `None` | `GatePending` | 필드 부재(구 데몬)·조회 실패·**한 번도 관측되지 않음** = 판정 불가. '부재 ≠ 부정' 규약(래치·gate 축과 동형) — 판정 불가를 사망으로 접으면 그것이 새 파괴 경로다. |
///
/// ## ★★M1 수리(2026-08-24 자기성찰 3회전) — `None` 가지가 **처음 도달 가능**해졌다
///
/// 이 표는 U-11 착지 때부터 이렇게 적혀 있었지만 **launch-agent 좌석에서 `None` 은 나올 수
/// 없었다**: 데몬의 `agent_alive` 산출이 `agent_meta.map(|_| agent_seen && !exit_notified)` 라
/// meta 가 등록된 좌석은 항상 `Some(bool)` 이었고, `agent_seen` 은 watchdog 의 자손 argv 매칭이
/// 성공해야만 켜진다. 그래서 **미관측이 `Some(false)` 로 나갔고**, 이 표가 그것을 "커널이 부재를
/// 확정" 으로 규정해 `LaunchFailed → close` 로 흘렸다 — argv 미가독 환경(Windows·EDR·래퍼 기동·
/// 벤더 실행 형태 변경)에서 U-10~U-15 의 좌석 보존 배선 **전체가 이 술어 하나로 무력화**됐다
/// (회전2 격리 실주행 1차: 의무 4좌석 전량 close 재현).
///
/// ★수리는 **판정 잣대가 아니라 입력의 정직성**을 고쳤다(선택지 ⓑ). 데몬이
/// [`cysd::handlers::agent_alive_tri`] 로 3상을 그대로 내보내므로 미관측은 `None` 이고, 이 표의
/// `None → GatePending` 가지가 그 좌석을 받는다. 잣대(`manual_reap_denial` 과의 통일 = 선택지 ⓐ)
/// 를 고르지 않은 이유는 그 3중 OR 의 재료(`live_owned`·`live_descendants`)가 **CLI 에 없는
/// 데몬 내부 사실**이라 새 wire 필드 둘과 전 프로세스 표 refresh 를 CLI 경로에 들여야 하기
/// 때문이다 — 파괴 판정에 관측을 더 얹는 대신, **거짓 관측을 없애는** 쪽이 반경도 작고
/// 소비부(python 미러·UI)가 이미 3상을 전제하고 있어 계약 변경도 아니다.
///
/// ★`Some(true)` 인데 화면 꼬리가 셸 프롬프트인 경우(= 래퍼만 살아있는 사망 은폐 의심)도
///   `GatePending` 이다. 그 의심은 **증거이지 확증이 아니고**, 오판의 대가가 비대칭이다:
///   보류의 최악은 '역할을 쥔 pane 이 남는다'(사람이 닫으면 끝) · 파괴의 최악은 '살아 있는
///   노드를 죽인다'(되돌릴 수 없다). 그 의심 자체는 `tail` 에 실려 처방 문안에 그대로 나간다.
fn readiness_timeout_verdict(
    alive: Option<bool>,
    agent: &str,
    max_wait_secs: u64,
    tail: &str,
    observed_gate: Option<&str>,
) -> BootVerdict {
    if alive == Some(false) {
        return BootVerdict::LaunchFailed {
            // ★M1: 문안이 **근거를 그대로** 말한다. 종전 문안("데몬이 agent 프로세스 부재를
            //   관측했다(기동 실패 확정)")은 거짓이었다 — 그 값은 '부재 관측' 이 아니라
            //   '이름/argv 매칭 미성립' 에서도 나왔고, 지금은 3값화로 **관측된 사망 확정**
            //   (보였다가 사라짐 전이)에서만 나온다. 두 사실은 사람이 취할 조치가 다르다.
            evidence: format!(
                "agent '{agent}' readiness not confirmed in {max_wait_secs}s — directive injection \
                 aborted (셸 오주입 차단). 데몬 사망감지가 이 좌석의 에이전트를 **한 번 관측한 뒤 \
                 사라짐**을 확정했다(agent_seen → agent_exit_notified 전이 = 기동 실패 확정) — \
                 실패 surface는 정리된다.\n\
                 ※ 이름·argv 매칭이 한 번도 성립하지 않은 상태(미관측)는 여기로 오지 않는다 — \
                 그것은 `agent_alive=null`(판정 불가)이고 귀결은 **보류**(좌석 보존)다.\n\
                 마지막 화면 꼬리:\n{tail}\n\
                 → agents.json의 cmd를 점검하고 `cys launch-agent --role <role> --agent {agent}`로 \
                 재시도하라"
            ),
        };
    }
    BootVerdict::GatePending {
        // ★(N2) 관문 id 는 이 단위가 **판정하지 않고 전달받는다**. 판정은 여전히 뒤 단위
        //   (`readiness::judge` → 관문 코퍼스)의 소유이고, 이 자리가 하는 일은 같은 실행이
        //   이미 관측한 사실을 버리지 않는 것뿐이다. 종전엔 리터럴 자리표시자를 실었고,
        //   그래서 좌석 표식·사람 처방·상위 관측이 전부 미식별로 굳었다.
        //   관측이 없었을 때만(폴링이 관문을 한 번도 못 봤다) 자리표시자다.
        gate: observed_gate
            .filter(|g| !g.is_empty())
            .unwrap_or(GATE_ID_UNIDENTIFIED)
            .to_string(),
        tail: tail.to_string(),
    }
}

/// 보류 귀결의 **롤백 단일 지점** — `CYS_GATE_PENDING_CLOSE=1` 이면 `GatePending` 을
/// `LaunchFailed` 로 강등해 **이 단위 착지 이전과 완전히 같은 동작**(무조건 close)으로 돌린다.
///
/// ★강등을 판정 반환 지점 **한 곳**에서만 하는 이유: 호출부 3곳이 각자 env 를 보면 롤백이
/// 3지점이 되고, 한 곳이라도 빠뜨리면 "되돌렸다"가 거짓말이 된다. 순수 함수라 진리표로 박제한다.
fn boot_verdict_effective(v: BootVerdict, close_override: bool) -> BootVerdict {
    match v {
        BootVerdict::GatePending { gate, tail } if close_override => BootVerdict::LaunchFailed {
            evidence: format!(
                "readiness 미확정(gate={gate}) — {} 로 종전 동작(즉시 close)으로 강등됨. \
                 마지막 화면 꼬리:\n{tail}",
                cys::ENV_GATE_PENDING_CLOSE
            ),
        },
        other => other,
    }
}

/// 데몬이 관측한 `agent_alive`(커널 프로세스 표 사실). 필드 부재·조회 실패는 `None`(판정 불가).
fn surface_agent_alive(sid: u64) -> Option<bool> {
    surface_agent_alive_in(&fetch_surfaces(), sid)
}

/// 위의 **순수 절반**(M1) — wire 행 목록에서 좌석의 3상 커널 사실을 뽑는다.
///
/// ★분리한 이유: 파괴 판정(`readiness_timeout_verdict`)의 입력이 **wire 의 `null` 을 실제로
///   `None` 으로 옮기는가**가 M1 의 핵심인데, 그 사슬이 `fetch_surfaces()`(데몬 왕복) 안에
///   숨어 있으면 기계로 박을 수 없다. 이제 검체가 wire 행 → 판정까지를 한 줄로 관통한다.
///   `as_bool()` 은 `null`·필드 부재·비-bool 을 모두 `None` 으로 접는다(= 판정 불가).
fn surface_agent_alive_in(surfaces: &[Value], sid: u64) -> Option<bool> {
    surfaces
        .iter()
        .find(|s| s["surface_id"].as_u64() == Some(sid))
        .and_then(|s| s["agent_alive"].as_bool())
}

/// 좌석 제4 등급 표식 **기록**(U-10 이 만든 자리의 유일한 생산자 · best-effort).
///
/// 실패해도 부트를 막지 않는다(구 데몬은 `method_not_found`) — 표식이 없으면 좌석은 종전
/// 등급으로 읽힐 뿐이고, 그것이 이 축의 fail-open 방향("오늘보다 나빠지지 않는다")이다.
fn mark_gate_pending(sid: u64, gate: &str, tail: &str) {
    // 근거 발췌는 topology 에도 실린다 — 화면 전문을 넣으면 스냅샷이 부풀고 사람이 못 읽는다.
    let evidence: String = tail.chars().take(400).collect();
    if let Err(e) = request(
        "surface.gate_pending",
        json!({"surface_id": sid, "gate": gate, "evidence": evidence}),
    ) {
        eprintln!(
            "[launch-agent] 관문 보류 상태 기록 실패(구 데몬?): {e} — 좌석은 그대로 보존된다"
        );
    }
}

/// 보류 확정의 **단일 경로** — 롤백 강등 적용 + 표식 기록을 한 곳에 모은다.
///
/// ★(U-14) 이 헬퍼가 생긴 이유: 보류가 나는 자리가 셋이 됐다(readiness 타임아웃 · 주입 직전
///   관문 감지 · 주입 도중 가드 발화). 각 자리가 `boot_verdict_effective` + `mark_gate_pending`
///   를 스스로 부르면 **롤백 킬스위치 판독이 3지점**이 되고, 한 곳만 빠져도 "되돌렸다"가
///   거짓말이 된다(U-11 이 세운 계약 · H-SEAT-4AXIS ⑦ 이 기계 집행). 그래서 강등은 여전히
///   판정 반환 지점 **한 곳**이고, env 판독은 부트 1회다(호출부가 값을 넘긴다).
fn settle_gate_pending(sid: u64, gate: &str, tail: String, close_override: bool) -> BootVerdict {
    let verdict = boot_verdict_effective(
        BootVerdict::GatePending {
            gate: gate.to_string(),
            tail,
        },
        close_override,
    );
    if let BootVerdict::GatePending { gate, tail } = &verdict {
        // 좌석 등급을 기록한다(U-10 이 만든 자리의 유일한 생산자). 이것이 없으면 보류 좌석이
        // `agent_alive` 하나로 `AlivePresumed` → **"이미 가동 중"** 으로 접혀, 관문에 갇힌
        // 팀 전체가 '정상 가동 중' 으로 집계된다 — 지금보다 나빠진다.
        mark_gate_pending(sid, gate, tail);
    }
    verdict
}

/// 표식 **해제** — readiness 재확정의 유일한 능동 경로(best-effort).
///
/// ★왜 여기서 지워야 하는가: 보류 좌석은 `cys boot` 이 **관측만 하고 건너뛰므로**, 사람이
/// 관문을 통과시킨 뒤 그 좌석에 다시 붙는 경로(node-recover·restore in-seat·같은 좌석 재기동)가
/// 표식을 지우지 않으면 좌석이 영구 미충족으로 남는다. 마지막 안전망은 데몬의 TTL 만료다.
fn clear_gate_pending(sid: u64) {
    let _ = request(
        "surface.gate_pending",
        json!({"surface_id": sid, "clear": true}),
    );
}

/// 롤백 킬스위치의 **유일한 env 판독 지점**(프로세스 수명 동안 1회).
///
/// 【이 함수가 존재하는 이유 — 판정 완화가 아니라 판독 단일화】
/// 이 축의 계약은 U-11 이 세우고 H-SEAT-4AXIS ⑦ 이 기계로 집행한다: **롤백 축은 1지점에서만
/// 판독한다.** 소비처가 각자 lib 의 합류 함수를 직접 부르면 그 순간 env 판독이 여러 지점이
/// 되고, ⓐ 두 판독 사이에 env 가 바뀌거나 ⓑ 개정에서 한 곳이 누락되면 **반쪽 롤백**이
/// 된다 — pane 은 보류인데 좌석은 `already_alive` 로 읽히는 상태(관측 없는 보류 = 허위 READY).
/// 이 저장소는 BLOCK-4 로 그 값을 이미 치렀다(문서화된 스위치 하나로 전 pane 사망).
///
/// 【무엇을 바꾸지 않는가】 **판정 기준에는 손대지 않는다.** 합류 규칙(마스터 `CYS_BOOT_GATES=0`
/// ∨ 강등 `CYS_GATE_PENDING_CLOSE=1` ∨ 축 `CYS_GATE_PENDING=0`)은 여전히
/// [`cys::gate_pending_close_override`] 하나가 소유하고, 여기서는 **몇 번 읽는가**만 1회로
/// 고정한다. 부트 폴링이 `gate_corpus`·`readiness_v1`·`trust_v1` 을 루프 밖에서 1회만 해소하는
/// 것과 같은 규율이다 — 판정 재료가 관측 도중에 바뀌면 그 자체가 관측의 일관성 상실이다.
///
/// 【소비처】 ⓐ `boot_agent_on_surface` 의 readiness 폴링(보류 확정 3자리에 값을 넘긴다)
///           ⓑ `gate_pending_adopt` 의 재관측 채택(주입 절반에 값을 넘긴다).
///           새 소비처가 생기면 **이 함수를 부르고 env 를 직접 읽지 않는다.**
// ★`|| f()` 를 `f` 로 줄이지 않는다(clippy `redundant_closure` 제안 거절): H-SEAT-4AXIS ⑦ 은
//   판독 지점을 **호출 문자열의 개수**로 센다. 괄호를 지우면 그 검체가 판독 0지점으로 읽고
//   (= 계약의 기계 집행이 조용히 죽고) 사람만 "1지점"을 통과했다고 믿게 된다.
#[allow(clippy::redundant_closure)]
fn gate_close_override_once() -> bool {
    static CELL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CELL.get_or_init(|| cys::gate_pending_close_override())
}

/// 보류 처방 문안(stderr 전용 — stdout 은 surface ref 계약이 소유한다).
///
/// 실측(2026-08-23 · macOS Claude Code 2.1.241 · 격리 프로필 PTY 캡처)으로 확정된 사실만 적는다:
/// 관문 순서와 **면책 창의 기본 포커스가 `No, exit`** 라는 것. 이 두 줄이 없으면 사용자는
/// pane 을 보고 Return 을 눌러 스스로 노드를 종료시킨다(rc 1) — 처방이 곧 킬 스텝이 된다.
fn print_gate_pending_prescription(sid: u64, role: &str, agent: &str, gate: &str, tail: &str) {
    eprintln!(
        "[launch-agent] ★관문 보류(gate={gate}) — {} 을(를) **닫지 않았다**. \
         데몬이 '{agent}' 프로세스 생존을 관측했다(role={role}).\n\
         사람이 1회 조치하면 이 좌석을 그대로 쓴다:\n\
         \x20 1) `cys read-screen --surface {}` 로 화면을 확인하라 — 첫기동 관문 순서는 \
         테마 → 로그인방식 → OAuth → 폴더신뢰 → 면책 → 새기능안내다.\n\
         \x20 2) ★면책(Bypass) 창의 기본 포커스는 `No, exit` 다 — 그대로 Return 하면 노드가 \
         종료된다. 아래 방향키 1회 뒤 Return(또는 숫자 `2`)으로 통과하라.\n\
         \x20 3) 통과한 뒤 `cys boot` 을 다시 실행하면 이 좌석이 그대로 쓰인다 — 재부트가 \
         **스폰 없이**(read_text 1회 + 판정 1회) 관문 통과를 확인하고 이 좌석에 절대지침을 \
         주입한다(M2 재관측 경로). **새 pane 을 만들지 마라**(역할은 이미 이 좌석이 쥐고 있어 \
         새 기동은 claim_denied 다).\n\
         \x20 · 종전처럼 즉시 닫으려면: {}=1 (한 지점 롤백)\n\
         마지막 화면 꼬리:\n{tail}",
        surface_ref(sid),
        surface_ref(sid),
        cys::ENV_GATE_PENDING_CLOSE
    );
}

/// launch-agent(새 surface)와 node-recover(기존 surface 재기동)가 공유한다.
fn boot_agent_on_surface(
    sid: u64,
    role: &str,
    agent: &str,
    spec: &Value,
    resume: bool,
    session_id: Option<&str>,
    restore: bool,
    // (W1) 이 pane의 cwd(resume 사전검증 munge용)와 데몬이 기록·반환한 권위 config_dir.
    // config_dir=None이면 게이트가 cys::resolve_claude_config_dir()로 best-effort 해소한다.
    cwd: Option<&str>,
    config_dir: Option<&str>,
) -> Result<BootVerdict, String> {
    let mut cmd = spec["cmd"].as_str().ok_or("agent cmd missing")?.to_string();
    if resume {
        if let Some(arg) = spec["resume_arg"].as_str() {
            // T2-6 resume 어댑터: 대화 기억 복원 플래그 (예: claude --continue).
            let fallback = spec["resume_arg_fallback"].as_str().unwrap_or("--continue");
            if let Some(resolved) =
                resolve_resume_suffix(agent, arg, session_id, config_dir, cwd, fallback)
            {
                cmd.push(' ');
                cmd.push_str(&resolved);
            }
        }
    }
    let delay = spec["inject_delay_secs"].as_u64().unwrap_or(12);
    // resume 복원 노드엔 전문 디렉티브를 재주입하지 않는다 — 직전 컨텍스트(.jsonl resume)에 이미
    // WORKER/REVIEWER_DIRECTIVE가 들어 있어, 전문 재주입은 토큰 2배·중복 지침 혼선 + 거대 주입으로
    // resume 직후 컨텍스트 임계(clear)를 유발한다(적대검증 serious). resume 시엔 짧은 복귀 가드만.
    let directive = if resume {
        format!(
            "[RESUME] 직전 작업 컨텍스트가 복원됐다(역할={role}). 절대지침은 이미 보유 중이니 \
             재숙지만 하고, _round/SESSION_STATE.md와 자기 TODO를 읽어 상태를 정합한 뒤 이어서 작업하라."
        )
    } else {
        compose_directive(role)?
    };

    // 1) 에이전트 기동 (authoritative: launch-agent의 모든 시스템 주입은 타이핑 가드 면제)
    // RC-3(B′): OS-aware 렌더 — unix는 `KEY="val" cmd` 인라인(기존 byte-identical·셸 전개),
    // windows는 순수 cmd(env는 surface.create가 pane env로 주입). send_env는 여기선 미사용
    // (주입은 run_launch_agent_opts의 surface.create에서 이미 수행) — send 문자열만 취한다.
    let mut env_pairs = agent_env_pairs(spec);
    apply_config_dir_override(&mut env_pairs, restore, config_dir);
    // ★D5(v4 · W4): claude 에 fullscreen(alternate screen) 차단 기본값을 주입 — spec env 에
    // 키가 **부재할 때만**(사용자 "0" 불가침 — fullscreen 되살리기 · append+sort 금지 함정은
    // lib 헬퍼 주석 참조).
    // 게이트 = `d5_gate_for_os(OS, d5_win_opt_in())` ∧ extract_bin(cmd)=='claude'
    // (`fn extract_bin` 헬퍼 재사용 — 어댑터 키 개명 내성).
    // ★게이트 매핑은 **mac = 무조건 · Windows = 옵트인(`~/.cys/win-no-alt-screen` ·
    // `CYS_WIN_NO_ALT_SCREEN=1`)했을 때만 · 그 외 = 미주입**이다. Windows 가 기본이 아닌 이유
    // (앵커 ④ 전 pane 사망 위험 · 실기 스모크 B-5 미수행)와 기본 on 승격 절차는 lib.rs 의
    // `d5_gate_for_os` doc 이 정본이다.
    //
    // ★★그리고 **이 호출은 Windows 에서 pane 에 도달하지 않는다**(옵트인해도 그렇다).
    // 바로 아래 `let (send, _send_env) = render_launch(...)` 가 증거다: unix 는 env 를 `KEY="val" cmd`
    // 인라인으로 send 문자열에 실어 셸이 전개하지만, Windows 는 순수 cmd 만 보내고 env 는
    // `_send_env` 로 **폐기**된다(이 경로엔 surface.create 가 없다 — 이미 존재하는 pane 에 붙는다).
    // 저장소 자신이 같은 구멍을 restore 의 계정격리 가드(E8) 주석에 문서화하고 있다
    // (grep `★계정격리 가드(E8)` — Windows 는 순수 cmd 라 CLAUDE_CONFIG_DIR 이 실리지 않아
    // 빈 좌석 재연결이 fail-closed 한다). ∴ Windows 에서 이 벨트가 실제로
    // 닿는 경로는 run_launch_agent_opts 의 surface.create env 맵 하나뿐이고, Windows 휠 오염의
    // **본체 방어는 UI 가드**(ui/src/wheelgate.ts 의 shouldSuppressWheelWin)다. 여기 호출이
    // 있다는 이유로 "Windows 는 env 로 막힌다"고 판단하지 마라 — 규약 단일화(사본 금지)를 위해
    // 두 소비처가 모두 lib 헬퍼를 경유할 뿐이다.
    cys::inject_claude_alt_screen_default(&mut env_pairs, extract_bin(&cmd, agent));
    let (send, _send_env) = render_launch(&cmd, &env_pairs);
    // ★(W2 · B4) **기동 send 직전 line_count 스냅샷** — readiness 판정의 시간 귀속 기준선.
    //
    // 종전 판정은 화면 **전체**를 봤다. claude 의 ready_marker 는 `❯`(agents.json)이고 그건
    // powerlevel10k·starship 프롬프트 문자와 같다 — node-recover·좌석 재사용 경로에서 화면에 남은
    // **잔존 ❯** 가 마커로 매칭돼 "준비됨"을 조기 선언하고, 디렉티브가 맨 셸(zsh)로 들어갔다(B4).
    // 여기서 뜬 커서 이후의 **신규 출현분**만 매칭 재료로 삼는다(surface.read_text 의 since_line 델타).
    //
    // ★개수 비교 구현 금지(브리프 명문): "마커 개수가 늘었나"로 구현하면 TUI 재드로우가 개수를
    //   흔들어 **영구 오부정**이 되고, T-0147-4 이후 롤백 close 가 실제로 성공하므로 그 오부정은
    //   **건강한 surface 를 실제로 닫는다**. 그래서 개수가 아니라 '커서 이후 출현'으로 판정한다.
    let since_line: u64 = fetch_surfaces()
        .iter()
        .find(|s| s["surface_id"].as_u64() == Some(sid))
        .and_then(|s| s["line_count"].as_u64())
        .unwrap_or(0);
    request(
        "surface.send_text",
        json!({"surface_id": sid, "text": send, "quiet": true, "authoritative": true}),
    )?;
    request(
        "surface.send_key",
        json!({"surface_id": sid, "key": "Return", "authoritative": true}),
    )?;
    // ★Phase 5 ①a: agent_meta를 기동 직후(readiness 폴링 前)에 등록한다. 등록이 폴링 뒤(step 5)에만
    // 있으면 readiness 미확인·restore 중 stall 시 meta=None으로 남아 → 사망감지 스킵(governance.rs)
    // → agent_seen 영원히 false → status 허위 DEAD → task-prompt 생존게이트가 '미기동' 오판(DRILL_LIVE_1).
    // 스폰 시점에 의도가 확정되므로 여기서 등록하는 것이 정직하다(§3-1 진단의 수리).
    let bin = extract_bin(&cmd, agent).to_string();
    request(
        "surface.set_meta",
        json!({"surface_id": sid, "agent": agent, "agent_bin": bin}),
    )?;
    eprintln!(
        "[launch-agent] {agent} starting… (polling readiness, max {}s)",
        delay.max(30) * 2
    );

    // 2) 준비 감지 폴링: 폴더 신뢰 프롬프트는 자동 확인, ready_marker가 보이면 주입 단계로
    let ready_marker = spec["ready_marker"].as_str().map(|s| s.to_string());
    // ★Phase 5 ①b: restore 모드에선 역할별 readiness 대기를 짧게 캡한다(타임아웃+continue). 한
    // 역할이 readiness에서 stall해도 run_restore가 실패로 처리해 다음 역할로 진행하게 해, 한 노드
    // stall이 로스터 전체를 멈추는 것을 막는다(DRILL_LIVE_1: worker spawn 후 중단처럼 보인 근원).
    // agent_meta는 위에서 이미 등록됐으므로(①a) 짧은 캡에도 사망감지·status는 정상 동작한다.
    // ★(W2 · B17/H-TIME-3) **카운트 회계 전폐** — `waited += 2` 산술을 지우고 `Instant` 벽시계
    //   데드라인만 쓴다. 종전 회계는 틱당 실비용(RPC 왕복 + 2.5s sleep + trust 분기의 추가 sleep,
    //   그중 trust 분기 sleep 은 아예 미집계)이 가정치 2s 와 어긋나 실효 대기가 25%+α 오차났다.
    //   상한은 BUDGET 파생(하드코딩 30/2/20 제거 — javis_budget 와 기계 대조).
    let max_wait = budget_readiness_max(delay, restore);
    let max_wait_secs = max_wait.as_secs();
    let deadline = std::time::Instant::now() + max_wait;
    let time_fallback_at = std::time::Instant::now() + std::time::Duration::from_secs(delay.max(1));
    let mut ready = false;
    let mut last_screen = String::new();
    // ★(U-13) 관문 코퍼스는 **루프 밖에서 1회** 해소한다 — 틱마다 env·JSON 을 다시 읽으면
    //   판정 재료가 폴링 도중에 바뀔 수 있고(관측의 일관성 상실), 비용도 틱 수만큼 곱해진다.
    //   문면의 진실원천은 `src/first_run_gates.rs`(U-12) 하나이고 여기서는 읽기만 한다.
    let gate_corpus = resolve_gate_corpus(agent);
    // 롤백 스위치도 1회만 읽는다(env 1지점 규약 — 판정 중 값이 바뀌지 않는다).
    let readiness_v1 = cys::readiness::legacy_v1();
    // ★(U-11 계약 · U-14 에서 지역 변수로 승격 · M2 에서 접근자로 승격) 보류 강등 스위치의
    //   **판독은 1지점**이다. 보류가 나는 자리가 셋으로 늘었는데(타임아웃 · 주입 직전 · 주입
    //   도중) 각 자리가 env 를 따로 읽으면 롤백이 3지점이 되고, 한 곳만 빠져도 "되돌렸다"가
    //   거짓말이 된다. env 판독의 소유자는 이제 `gate_close_override_once` 하나다 — 이 폴링과
    //   `gate_pending_adopt` 가 **같은 값**을 본다(값이 갈리면 그것이 반쪽 롤백의 씨앗이다).
    let gate_close_override = gate_close_override_once();
    if readiness_v1 {
        eprintln!(
            "[launch-agent] ⚠ {}=1 또는 상위 마스터 스위치({}=0)/보류 강등 — 관문 문면 AND 항을 \
             끈 종전 판정으로 되돌렸다(관문 화면을 ready 로 오탐할 수 있다)",
            cys::readiness::ENV_V1,
            cys::ENV_BOOT_GATES
        );
    }
    // 관문 보류 진단은 **1회만** 낸다(틱마다 같은 줄을 찍으면 진짜 신호가 묻힌다).
    let mut gate_logged: Option<String> = None;
    let mut valve_held_logged = false;
    // ★(W2 · G35) 폴더신뢰 자동확인의 **멱등 래치 + 소멸 확인 + ready 봉쇄 해제**.
    //   종전 코드는 매 tick 화면을 매칭해 Return 을 **재전송**했고(래치 0·상한 0), 그 분기가
    //   `continue` 로 끝나 **ready 검사 자체를 봉쇄**했다(준비 감지 구조 차단 — 레포 티켓 T-D2a).
    //   실측 2회에서 기계 Return 1발이 claude 신뢰창을 종료시킨 적이 있어, 반복 전송은
    //   '노드 0 + 고아 좌석'으로 번진다. 신뢰 분기는 readiness 검사를 막지 않는다.
    // ★(U-15) **전송은 1발이다.** 2026-08-23 실측으로 재전송 기구가 킬체인의 방아쇠임이 확정됐다 —
    //   확인 에코 `Yes, I trust this folder ✔` 가 구 하드코딩 needle 에 재매칭되고, 그때 화면은
    //   이미 면책 창(기본 포커스 `No, exit`)이라 2발째 Return 이 좌석을 rc 1 로 죽인다.
    //   상한 상수를 2→1 로 내리는 대신 **조건 자체**를 줄였다(정책 판정은
    //   `cys::inject_guard::trust_send` 진리표가 소유). 상수·`persisted`·커서는 지우지 않고
    //   롤백 스위치(`CYS_TRUST_RETURN_V1=1`) 분기의 실사용 입력으로 남긴다 — 그 사유는
    //   `trust_send` 의 doc('죽은 코드를 남길지 지울지' 명시 결정)에 적혀 있다.
    let mut trust_sends: u32 = 0;
    let mut trust_seen_at: Option<u64> = None; // 프롬프트를 관측한 시점의 델타 커서
    // 롤백 스위치는 루프 밖에서 1회만 읽는다(env 1지점 규약 — 판정 중 값이 바뀌지 않는다).
    let trust_v1 = cys::inject_guard::trust_v1();
    if trust_v1 {
        eprintln!(
            "[launch-agent] ⚠ {}=1 — 폴더신뢰 Return 을 종전 정책(하드코딩 needle 감지 + 재전송 \
             상한 {BUDGET_TRUST_MAX_SENDS}발)으로 되돌렸다",
            cys::inject_guard::ENV_TRUST_V1
        );
    }
    // ★(W4 · B19) 폴더신뢰 프롬프트 패턴을 **어댑터 선언에서** 읽는다(하드코딩 제거).
    //   상세 근거는 trust_prompt_regex·trust_prompt_hit 정의부 주석.
    let trust_re: Option<regex::Regex> = trust_prompt_regex(&spec);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(BUDGET_TICK_MS));
        // 화면(vt100 그리드) — 사람이 보는 현재 상태. 잔존 프롬프트도 여기 남는다.
        let screen = request("surface.read_text", json!({"surface_id": sid}))?;
        let text = screen["text"].as_str().unwrap_or("");
        last_screen = text.to_string();
        // 델타(커서 이후 신규 출현분) — **시간 귀속이 있는** 유일한 재료(B4).
        let delta = request(
            "surface.read_text",
            json!({"surface_id": sid, "since_line": since_line}),
        )?;
        let delta_text = delta["text"].as_str().unwrap_or("").to_string();
        let delta_cursor = delta["next_cursor"].as_u64().unwrap_or(since_line);
        let delta_flat: String = delta_text.chars().filter(|c| !c.is_whitespace()).collect();
        // ① 기동 실패 — **신규 출현분에서만** 판정한다(잔존 에러 텍스트로 새 기동을 죽이지 않는다).
        if screen_shows_launch_failure(&delta_flat) {
            // ★(U-11) 화면이 기동 실패를 **확증**한 유일한 지점 — 종전 귀결(close)을 그대로
            //   유지한다. 보류로 흐르면 안 된다: 여기서 보류하면 진짜 실패 좌석이 역할을 쥔 채
            //   쌓이고, 그 다음 기동이 전부 claim_denied 가 된다(2026-08-16 실사고 계열).
            return Ok(BootVerdict::LaunchFailed {
                evidence: format!(
                    "agent '{agent}' failed to start (command error in new output) — check cmd in agents.json"
                ),
            });
        }
        // ② 폴더신뢰 프롬프트 — 멱등 래치·화면 재확인·1발. `continue` 하지 않는다(ready 검사 계속).
        //
        // ★(U-15) 감지는 **누적 델타**에서 하지만 전송 판정은 **지금 화면**을 한 번 더 본다.
        //   신뢰 창을 통과한 뒤에도 델타에는 그 질문이 그대로 남아 있고(since_line 이후 전량),
        //   그때 화면은 이미 면책 창이다 — 종전 코드가 2발째를 그 화면에 쏜 경로가 정확히 이것이다.
        //   `decide_allowing(..., Some(GATE_FOLDER_TRUST))` 의 구멍은 **폴더신뢰 하나**뿐이라
        //   자동확인 기능은 살고 킬 스텝만 닫힌다.
        if trust_prompt_hit(
            trust_re.as_ref(),
            &gate_corpus.gates,
            &delta_text,
            &delta_flat,
            trust_v1,
        ) {
            let other_gate = cys::inject_guard::decide_allowing(
                &cys::inject_guard::Observed {
                    screen: text,
                    gates: &gate_corpus.gates,
                    awakened: Some(false), // 부트 창은 상수다(구 데몬에서 꺼지면 안 된다)
                    guard_off: cys::inject_guard::guard_off(),
                },
                Some(cys::inject_guard::GATE_FOLDER_TRUST),
            )
            .blocks();
            let first = trust_sends == 0;
            let persisted = trust_seen_at.map(|c| delta_cursor > c).unwrap_or(false);
            let send = cys::inject_guard::trust_send(&cys::inject_guard::TrustObserved {
                hit: true,
                first,
                persisted,
                sends: trust_sends,
                max_sends: BUDGET_TRUST_MAX_SENDS,
                other_gate,
                legacy_v1: trust_v1,
            });
            if send {
                eprintln!(
                    "[launch-agent] folder-trust prompt {} → confirm ({}발째 · 상한 {})",
                    if first { "detected(new output)" } else { "persisted" },
                    trust_sends + 1,
                    if trust_v1 { BUDGET_TRUST_MAX_SENDS } else { 1 }
                );
                request(
                    "surface.send_key",
                    json!({"surface_id": sid, "key": "Return", "authoritative": true}),
                )?;
                trust_sends += 1;
                trust_seen_at = Some(delta_cursor);
                std::thread::sleep(std::time::Duration::from_secs(BUDGET_TRUST_SETTLE_SECS));
            } else if other_gate && !trust_v1 && trust_sends > 0 {
                // 실측 킬체인의 그 순간 — 여기서 보냈다면 면책 창의 `No, exit` 를 눌렀다.
                eprintln!(
                    "[launch-agent] folder-trust 잔상 재매칭 — 화면은 이미 다른 관문이라 Return 을 \
                     보내지 않는다(킬 스텝 차단)"
                );
            }
            // 1발 이후에는 더 보내지 않는다 — 반복 Return 이 신뢰창·면책창을 누르는 실측 경로 차단.
        }
        // ★★안전 밸브(치명위험 ④ 차단 — 영구 오부정 불가능성 보장):
        //   `agent_alive` 는 **데몬이 커널 프로세스 표에서 관측한 사실**이다(이 surface 의 자손에
        //   agent 바이너리가 살아있음). 화면 텍스트와 달리 잔존 화면으로 위조될 수 없고, 렌더 방식
        //   (개행 없는 in-place 그리기)에도 영향받지 않는다.
        //   ★왜 필수인가: 델타 매칭은 claude 의 `❯` 가 **scrollback(개행 완성 라인)에 실린다**는
        //   가정에 서 있다. 그 가정이 어떤 버전·터미널 폭에서 깨지면 readiness 가 **영구 오부정**이
        //   되고, T-0147-4 이후 롤백 close 가 실제로 성공하므로 **건강한 pane 이 전부 닫힌다**
        //   (= '모든 pane 사망·글자 0'). 그래서 화면과 무관한 양성 증거를 하나 둔다.
        //   ★B4 를 되돌리지 않는다: 기동에 **실패한** 에이전트는 자손 프로세스가 없어 agent_alive 가
        //   참이 되지 않는다 → 잔존 ❯ 만으로는 절대 ready 가 되지 않는다(오탐 방향 무변).
        //   agent_meta 는 기동 send 직후 등록되므로(①a) watchdog 첫 틱(≤5s) 뒤부터 참이 될 수 있다.
        //   ★P1-1(치명·U-5 × U-9): `alive` **단독**으로는 부족하다. argv 승격 이후 넓은 생존 매처가
        //   래퍼(`cmd.exe /c …\claude-2.cmd`·`sh -c 'claude …'`)까지 생존 증거로 세므로, 에이전트가
        //   즉사하고 래퍼만 남은 틱에 밸브가 잘못 발화해 디렉티브를 맨 셸에 제출한다. 그래서
        //   판정부(`cys::readiness`)의 밸브 항이 U-9 화면 꼬리 술어와 AND 를 건다(근거 전문은
        //   그 판정부 주석에 그대로 옮겨져 있다).
        //   ★(U-13) 밸브의 **판정**은 `cys::readiness::positive_evidence` 로 이사했고 여기에는
        //   **관측만** 남는다. 밸브는 삭제되지 않았다 — 관문 AND 항이 붙었을 뿐이고, 그 AND 항은
        //   밸브 블록 **밖**(판정부)에서 계산되므로 밸브의 근거는 여전히 화면 무의존이다.
        let alive = fetch_surfaces()
            .into_iter()
            .find(|s| s["surface_id"].as_u64() == Some(sid))
            .map(|s| s["agent_alive"].as_bool().unwrap_or(false))
            .unwrap_or(false);
        // ③ ready 판정 — **단일 진입점**. 관측을 한 구조체로 모아 순수 술어 하나에 넘긴다.
        //
        // ★정정(2026-08-23 실측 · 종전 서술 "신규 출현분에 마커 = 잔존 ❯ 오탐이 원리상 불가능한
        //   유일한 판정" 은 반증됐다): 델타 우선이 배제하는 것은 **잔존** ❯ 뿐이다. 기동 직후에
        //   *새로* 그려지는 ❯ 는 그대로 델타에 실린다.
        //   측정(macOS · Claude Code 2.1.241 · 격리 CLAUDE_CONFIG_DIR PTY 캡처 · Windows 는
        //   Parallels 실기 화면): 첫기동 관문 화면 **6종 전부**가 선택지 커서로 `❯` 를 그린다 —
        //   테마 / 로그인 방식 / OAuth 코드 / 폴더 신뢰 / Bypass 면책 / 신기능 안내.
        //   그래서 `.claude.json` 이 없는 프로필에서는 마커 축도 밸브 축도 **관문 화면을 ready 로
        //   선언**하고, 디렉티브가 선택기에 붙여넣어진다(그 붙여넣기의 Return 이 면책 창의
        //   `No, exit` 를 눌러 좌석을 죽인다 — 2026-08-23 실측 킬체인).
        //   델타 우선 규칙은 그대로 유지한다(잔존 오탐 차단분은 유효하고, 개수 비교로 되돌리면
        //   영구 오부정 회귀다). 관문 배제는 그 축이 아니라 **관문 문면 AND 항**이 한다.
        let obs = cys::readiness::Observed {
            site: cys::readiness::Site::Boot,
            agent_alive: Some(alive),
            screen: text,
            delta: &delta_text,
            marker: ready_marker.as_deref(),
            gates: &gate_corpus.gates,
            tail_is_shell_prompt: Some(screen_tail_is_shell_prompt(text)),
            // ★(P3-0) 밸브 전용 축 — 꼬리 술어와 **다른 술어**다. 살아있는 TUI 의 입력
            //   프롬프트가 곧 `❯` 라서 꼬리 술어를 밸브에 쓰면 건강 pane 이 상시 차단된다.
            bare_shell: Some(screen_is_bare_shell(text)),
            time_fallback_reached: std::time::Instant::now() >= time_fallback_at,
            idle_quiet: None,
            legacy_v1: readiness_v1,
        };
        match cys::readiness::judge(&obs) {
            cys::readiness::Verdict::Ready { evidence } => {
                eprintln!("[launch-agent] ready({}) — 주입 안전", evidence.label());
                ready = true;
                break;
            }
            // 관문이 떠 있다 = **준비가 아니다**. 여기서 하는 일은 관측·보고뿐이고 키는 하나도
            // 보내지 않는다(관문 창의 Return 이 곧 킬 스텝인 화면이 실재한다 — 면책 창).
            // 계속 폴링하는 이유: 사람이 그 사이에 통과시키면 같은 좌석이 그대로 ready 가 된다.
            cys::readiness::Verdict::GateHeld { gate_id, title, human_only, vetoed } => {
                if gate_logged.as_deref() != Some(gate_id.as_str()) {
                    eprintln!(
                        "[launch-agent] 관문 보류: {title}(id={gate_id}{}) — 주입 0 · 키 전송 0{}",
                        if human_only { " · 사람 1회 필요" } else { "" },
                        match vetoed {
                            Some(e) => format!(" · 종전 판정이라면 ready 였다({})", e.label()),
                            None => String::new(),
                        }
                    );
                    gate_logged = Some(gate_id);
                }
            }
            // 증거 없음 — 계속 관측한다. `alive` 인데 여기로 왔다면 화면 꼬리가 셸 프롬프트라는
            // 뜻이다(밸브의 두 번째 근거가 닫혔다 = 래퍼만 살아있는 사망 은폐 의심 · P1-1).
            cys::readiness::Verdict::NotYet => {
                if alive && !valve_held_logged {
                    eprintln!(
                        "[launch-agent] 안전 밸브 보류: agent 생존은 관측되나 화면이 **맨 셸**이다 \
                         (꼬리가 셸 프롬프트이고 TUI 렌더 증거 0) — 래퍼만 살아있는 사망 은폐 \
                         의심(주입 금지)"
                    );
                    valve_held_logged = true;
                }
            }
        }
        // ── readiness 판정 배선 끝 ──
    }
    if !ready {
        // 준비 미확인 주입 금지: 에이전트가 안 떠 있으면 디렉티브가 맨 셸(zsh)로 들어가
        // 첫 단어가 명령으로 실행된다("zsh: command not found: 는" — 2026-06-12 실측).
        // 주의: launch 경로 호출자가 실패 surface를 정리(close)하므로, 진단 증거(화면 꼬리)는
        // 여기서 에러 본문에 동봉한다 — "read-screen으로 확인하라"는 안내는 close 후 거짓이 된다.
        // (U-14) 꼬리 계산은 공용 헬퍼 1개로 — 보류 처방·주입 가드 에러가 같은 형태를 보고한다.
        let tail = screen_tail_lines(&last_screen, 5);
        // ★(U-11) 여기가 이 단위의 **핵심 분기점**이다. 종전엔 무조건 `Err` 였고 호출부 셋이
        //   그것을 close·kill·좌석증식으로 각자 번역했다. 이제 커널 사실(`agent_alive`)로
        //   '없다(파괴 가능)' 와 '있는데 준비 미확정(보류)' 을 가른다 — 판정식 전문은
        //   `readiness_timeout_verdict` doc 표 참조.
        let verdict = boot_verdict_effective(
            readiness_timeout_verdict(
                surface_agent_alive(sid),
                agent,
                max_wait_secs,
                &tail,
                // ★(N2) 폴링 루프가 관측한 관문 id. 바로 위 루프가 `id=…` 로 stderr 에 찍은
                //   그 값이며, 여기서 버리면 좌석 표식·처방이 전부 미식별로 굳는다.
                gate_logged.as_deref(),
            ),
            gate_close_override,
        );
        if let BootVerdict::GatePending { gate, .. } = &verdict {
            // 좌석 등급을 기록한다(U-10 이 만든 자리의 유일한 생산자). 이것이 없으면 보류 좌석이
            // `agent_alive` 하나로 `AlivePresumed` → **"이미 가동 중"** 으로 접혀, 관문에 갇힌
            // 팀 전체가 '정상 가동 중' 으로 집계된다 — 지금보다 나빠진다.
            mark_gate_pending(sid, gate, &tail);
        }
        return Ok(verdict);
    }
    // ★(M2 · 2026-08-24) 판정 이후 절반은 **재사용 가능한 단위**로 분리했다 —
    //   `cys boot` 의 관문 보류 재관측 경로(`gate_pending_reobserve`)가 **스폰 0** 으로
    //   같은 절반(표식 해제 + 주입 + ack 검증)을 그대로 태운다. 사본을 만들면 그 순간
    //   '주입 경로가 둘' 이 되고, 한쪽만 고쳐지는 것이 이 저장소가 반복해 맞은 형태다.
    inject_directive_after_ready(
        sid,
        agent,
        &directive,
        &gate_corpus.gates,
        gate_close_override,
        since_line,
    )
}

/// `boot_agent_on_surface` 의 **판정 이후 절반** — ready 가 확정된 좌석에 표식을 해제하고
/// 디렉티브를 주입한 뒤 ack 를 검증한다. **스폰·기동 send 를 하지 않는다**(이 함수에 들어올
/// 때 그 좌석의 에이전트는 이미 떠 있고 입력을 받을 수 있다).
///
/// ★(M2) 분리한 이유: `cys boot` 의 관문 보류 좌석은 **다시 기동할 수 없다**(살아 있는 입력창에
///   기동 커맨드를 밀어 넣으면 화면 파괴·중복 기동이고, 연쇄하면 치명위험 ④). 그런데 사람이
///   관문을 통과시킨 뒤 그 좌석을 **쓰려면** 표식 해제와 디렉티브 주입이 필요하다. 그 둘은
///   종전에 `boot_agent_on_surface` 안에만 있었고, run_boot 의 보류 분기는 `continue` 뿐이라
///   **도달할 방법이 없었다** — 처방 문안은 '통과 후 cys boot 을 다시 실행하면 이 좌석이
///   그대로 쓰인다' 고 약속하는데 실제로는 매번 보류로 건너뛰어졌다(M2 의 결함 본체).
///
/// 반환 계약은 호출부와 같다: `Ready` = 주입 완료 · `GatePending` = 주입 직전/도중 관문 감지로
/// 좌석 보존(close 0 · kill 0 · 주입 0) · `Err` = 주입 자체가 실패(파괴 근거 아님).
fn inject_directive_after_ready(
    sid: u64,
    agent: &str,
    directive: &str,
    gates: &[cys::first_run_gates::Gate],
    gate_close_override: bool,
    since_line: u64,
) -> Result<BootVerdict, String> {
    // ★(U-11) 준비 확정 = 보류 표식의 **해제** 지점. 보류 좌석은 `cys boot` 이 관측만 하고
    //   건너뛰므로(U-10), 사람이 관문을 통과시킨 뒤 이 좌석에 다시 붙는 경로(node-recover·
    //   restore in-seat·같은 좌석 재기동·★M2 재관측)가 표식을 지우지 않으면 좌석이 영구
    //   미충족으로 남는다.
    clear_gate_pending(sid);
    // marker 감지 직후 TUI 입력 활성화까지 약간의 여유
    std::thread::sleep(std::time::Duration::from_secs(BUDGET_POST_MARKER_SETTLE_SECS));

    // ★(W4 · D5 관측) ready 판정 직후 alternate-screen 확인 — 판정·문안은 alt_screen_notice
    // (순수 fn) 참조. 필드 부재(구 데몬)면 as_bool()=None → 판정 불가·통지 생략(FAIL 금지).
    let alt_verify_tag: Option<&'static str> = {
        let alt = fetch_surfaces()
            .iter()
            .find(|s| s["surface_id"].as_u64() == Some(sid))
            .and_then(|s| s["alt_screen"].as_bool());
        match alt_screen_notice(alt, agent, cfg!(target_os = "macos"), cfg!(windows)) {
            Some((line, attach_reason)) => {
                eprintln!("{line}");
                attach_reason.then_some(" · alt_screen=true(ready 직후 관측 — D5 env 우회 fullscreen)")
            }
            None => None,
        }
    };

    // 3) 지침 주입 — bracketed paste로 감싸 단일 입력으로 전달
    let inject_cursor: u64 = fetch_surfaces()
        .iter()
        .find(|s| s["surface_id"].as_u64() == Some(sid))
        .and_then(|s| s["line_count"].as_u64())
        .unwrap_or(since_line);
    // ★(U-14) 주입 직전 **typed** 관문 가드. ready 판정(U-13)과 이 지점 사이에는 settle sleep 이
    //   있고, 그 사이에 다음 관문이 뜰 수 있다(실측: 폴더신뢰 통과 → 면책 창).
    //   여기서 걸리면 귀결은 **보류**다 — close 도 kill 도 아니고 좌석을 그대로 둔다.
    //   ★부트 창은 상수로 연다(`gate_guard_decide_in_boot`): 구 데몬에서 `awakened_at` 키가
    //     없다는 이유로 가드가 가장 필요한 자리에서 꺼지면 안 된다.
    if let cys::inject_guard::Decision::Hold(hit) =
        gate_guard_decide_in_boot(sid, &gates)
    {
        // 진단 문안 전용 — 판정이 아니라 에러 본문이라 관측 실패의 빈 문자열이 정확하다.
        let tail = screen_tail_lines(&gate_guard_screen(sid).unwrap_or_default(), 5);
        eprintln!(
            "[launch-agent] ★주입 직전 관문 감지({} · {}) — 디렉티브 {} 바이트 **미주입** · \
             Return 0발 · 좌석 보존",
            hit.id,
            hit.title,
            directive.len()
        );
        return Ok(settle_gate_pending(sid, &hit.id, tail, gate_close_override));
    }
    // ★가드에 걸린 실패는 `Err` 로 올라오지만 **파괴 근거가 아니다**(머리표가 그 계약이다).
    //   `?` 로 흘리면 호출부 3곳이 그것을 close·kill·좌석증식으로 번역한다 — 정확히 U-11 이
    //   막으려던 경로다. 위 typed 사전 판정과 붙여넣기 사이의 경합(가드 ②가 뒤늦게 발화)도
    //   같은 보류로 접는다.
    if let Err(e) = inject_text(sid, &directive) {
        if !cys::inject_guard::is_hold_error(&e) {
            return Err(e);
        }
        eprintln!("[launch-agent] {e}");
        // 진단 문안 전용 — 판정이 아니라 에러 본문이라 관측 실패의 빈 문자열이 정확하다.
        let tail = screen_tail_lines(&gate_guard_screen(sid).unwrap_or_default(), 5);
        // 사전 판정을 통과한 뒤 뜬 관문이므로 id 를 특정하지 않는다 — 화면 꼬리가 근거다
        // (`readiness_timeout_verdict` 의 `"unknown"` 과 같은 규약).
        return Ok(settle_gate_pending(
            sid,
            GATE_ID_UNIDENTIFIED,
            tail,
            gate_close_override,
        ));
    }

    // ── 4) 주입 확인 — ★(W2 · B14/CS-3⑤) **신호의 질을 화면 문자열 → ack 계약으로 교체** ──
    //
    // 종전 검증은 "화면에 '절대지침' 문자열이 보이나"였고, 실패는 stderr 경고 1줄로 삼켜졌다
    // (관측 채널 부재 — RC3). 화면 문자열은 ①TUI 스크롤·박스 렌더에 따라 안 보일 수 있고
    // ②잔존 화면(직전 세션 잔재)으로 **거짓 통과**할 수도 있어 신호 자체가 약했다.
    //
    // 새 신호 = **데몬 SOT 의 `awakened_at` 래치**(주입 후 첫 set-status). 화면과 달리 위조·부패가
    // 없고, 부트 성공의 계약(javis_boot_node docstring)과 문자 그대로 같은 사실이다.
    //
    // ★치명 격상 금지(금지 방향 ③ · 비평2 B14): 미확인을 실패로 올리면 '위경고 모드'가 된다 —
    //   노드가 디렉티브를 **읽는 중**일 수도 있고(LLM 왕복은 수십 초), ack 창을 길게 잡으면 부트
    //   전체가 그만큼 늘어난다. 그래서 판정을 **상태로 남기고 부트는 계속한다**(directive.verify).
    // ★멱등 1회 재주입은 여기가 아니라 `javis_boot_node` VERIFY 가 담당한다(B11 3분기 + 짧은
    //   각성문). 여기서 **전문 디렉티브를 재주입하면** 토큰 2배·중복 지침 혼선 + resume 직후
    //   컨텍스트 임계라는, 이 함수가 이미 주석으로 경고하는 그 사고를 만든다.
    let ack_deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(BUDGET_ACK_WAIT_SECS);
    let mut verified = false;
    let mut verify_reason = String::new();
    while std::time::Instant::now() < ack_deadline {
        std::thread::sleep(std::time::Duration::from_millis(BUDGET_TICK_MS));
        let row = fetch_surfaces()
            .into_iter()
            .find(|s| s["surface_id"].as_u64() == Some(sid));
        if let Some(row) = row {
            if row["awakened_at"].as_f64().unwrap_or(0.0) > 0.0 {
                verified = true;
                verify_reason = "awakened_at 래치(set-status ack)".into();
                break;
            }
        }
    }
    if !verified {
        // 보조 증거(주 신호가 아니다): 주입 커서 **이후 신규 출현분**에 지침 머리말이 있나.
        // 잔존 화면 오통과를 막기 위해 델타에서만 본다(B4 와 같은 규율).
        let delta = request(
            "surface.read_text",
            json!({"surface_id": sid, "since_line": inject_cursor}),
        )
        .ok();
        let flat: String = delta
            .as_ref()
            .and_then(|d| d["text"].as_str())
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let echoed = flat.contains("ABSOLUTEDIRECTIVE") || flat.contains("절대지침");
        verify_reason = format!(
            "ack 미확인({BUDGET_ACK_WAIT_SECS}s 창) · 화면 에코(신규 출현분)={}",
            if echoed { "관측" } else { "미관측" }
        );
        eprintln!(
            "[launch-agent] directive 주입 검증 미확인 — {verify_reason}. \
             부트는 계속한다(치명 아님). 상태를 directive_verified=false 로 기록: \
             `cys status --json` 또는 `cys read-screen --surface {}` 로 확인, \
             필요하면 javis_boot_node.py --role <role> --agent <agent> 로 재각성하라",
            surface_ref(sid)
        );
    } else {
        eprintln!(
            "[launch-agent] directive 주입 검증 확정 — {verify_reason} ({} bytes)",
            directive.len()
        );
    }
    // ★(W4 · D5) alt-screen WARN 이 있었으면 verify reason 에 부기한다 — 대시보드·진단이
    // '주입은 됐는데 fullscreen 상태였다'는 맥락을 상태로 읽을 수 있게(stderr 휘발 보완).
    if let Some(tag) = alt_verify_tag {
        verify_reason.push_str(tag);
    }
    // 상태화(경고 삼킴 제거) — 실패해도 부트를 막지 않는다(best-effort · 구 데몬은 미지 메서드 에러).
    if let Err(e) = request(
        "directive.verify",
        json!({"surface_id": sid, "verified": verified, "reason": verify_reason}),
    ) {
        eprintln!("[launch-agent] directive_verified 상태 기록 실패(구 데몬?): {e}");
    }

    // 5) T2-5 에이전트 메타 등록은 ★Phase 5 ①a로 기동 직후(위)로 이동했다 — readiness 폴링/주입
    // 성공에 의존하지 않게. 여기서 재등록하면 set_meta가 agent_seen을 false로 리셋해, 이미 사망감지가
    // 관측한(agent_seen=true) 노드를 일시 허위 DEAD로 되돌리므로 재호출하지 않는다.
    Ok(BootVerdict::Ready)
}

// ═══════════════════════════════════════════════════════════════════════════
// ★★M2(2026-08-24 자기성찰 3회전) — 관문 보류의 **탈출 경로**
//
// 【무엇이 틀렸었는가】 `run_boot` 의 `gate_pending` 분기는 출력 후 `continue` 뿐이라
// `boot_agent_on_surface` 를 부르지 않았다 — `clear_gate_pending` 도 디렉티브 주입도 **그 함수
// 안에만** 있었으므로, 사람이 처방대로 관문을 통과시키고 `cys boot` 을 다시 실행해도 그 좌석은
// **다시 보류로 건너뛰어졌다.** 표식의 유일한 생산자도 같은 함수라 `since` 가 영영 갱신되지
// 않았고, 30분 뒤 데몬 TTL 이 표식을 null 로 접으면 등급이 `alive_presumed` 로 떨어져
// `javis_orchestra.py check` 가 그것을 **충족으로 세어 exit 0 = READY** 를 냈다.
// 귀결: 절대지침이 **한 번도 주입되지 않은** 좌석이 역할을 영구 점유한 채 초록으로 집계된다
// (근본원인 R1 의 타이머 재발). 그리고 처방 문안 3항은 "통과한 뒤 `cys boot` 을 다시 실행하면
// 이 좌석이 그대로 쓰인다" 고 **약속**하고 있었다 — 듣지 않는 손잡이(BLOCK-2 계열).
//
// 【무엇을 하는가】 **스폰 0 · 회수 0 · 파괴 0** 의 재관측이다: `surface.read_text` 한 번과
// `readiness::judge` 한 번. Ready 면 표식을 해제하고 `inject_directive_after_ready`(판정 이후
// 절반)를 그대로 태운다. 기동 send 는 **한 글자도** 보내지 않는다.
// ═══════════════════════════════════════════════════════════════════════════

/// 보류 좌석 재관측의 **판정**(순수 · 진리표 대상). 입력은 `readiness::judge` 의 산출 하나다.
#[derive(Debug, Clone, PartialEq)]
enum GateRecheck {
    /// 관문이 지나갔고 입력활성 증거가 있다 — 표식 해제 + 디렉티브 주입(같은 좌석 재사용).
    Adopt(cys::readiness::Evidence),
    /// 아직 관문이 떠 있다 — 보류 유지. 사람이 1회 더 조치해야 한다.
    StillHeld { gate_id: String, title: String },
    /// 증거 없음(맨 셸 의심·화면 미관측) — 보류 유지. **파괴로 승격하지 않는다.**
    NoEvidence,
}

/// 재관측 판정 — `judge` 의 세 갈래를 그대로 옮긴다(새 규약을 만들지 않는다).
///
/// ★`NotYet` 을 파괴로 승격하지 않는 이유: 이 자리의 입력은 **이미 보류로 판정된 좌석**이고,
///   프로세스는 살아 있다. 증거를 못 찾은 것은 '없다'가 아니다 — U-11 이 세운 비대칭
///   (보류의 최악 = 사람이 닫으면 끝 / 파괴의 최악 = 되돌릴 수 없음)이 여기서도 그대로다.
fn gate_pending_recheck(v: cys::readiness::Verdict) -> GateRecheck {
    match v {
        cys::readiness::Verdict::Ready { evidence } => GateRecheck::Adopt(evidence),
        cys::readiness::Verdict::GateHeld { gate_id, title, .. } => {
            GateRecheck::StillHeld { gate_id, title }
        }
        cys::readiness::Verdict::NotYet => GateRecheck::NoEvidence,
    }
}

/// 보류 좌석 **비파괴 재관측**(스폰 0) — 관측을 모아 [`gate_pending_recheck`] 에 넘긴다.
///
/// 관측 재료는 `boot_agent_on_surface` 폴링과 **같은 축**이되 델타는 없다(빈 문자열):
/// 그 좌석의 기동 커서는 이전 부트 프로세스의 지역 변수였고 재현할 방법이 없다. 델타가 비면
/// 마커 델타 증거가 발화하지 않으므로 남는 통과 경로는 **밸브**(커널 생존 ∧ 화면이 맨 셸 아님)와
/// **화면 마커 + 시간 폴백**뿐이다 — 둘 다 `gate_on_screen` 의 AND 항 뒤에 있으므로, 관문이
/// 떠 있는 한 어느 쪽도 Ready 를 내지 못한다. 시간 폴백은 참으로 준다(이 좌석은 이미 준비
/// 예산을 한 번 다 쓴 좌석이라 '아직 이르다' 가 성립하지 않는다).
fn gate_pending_reobserve(sid: u64, agent: &str) -> GateRecheck {
    let Some(screen) = gate_guard_screen(sid) else {
        // 화면 관측 실패는 **판정 불가**다 — 보류 유지(fail-closed · P4-6 의 loud 규율).
        eprintln!(
            "[boot] 관문 보류 재관측: 화면을 읽지 못했다({}) — 보류 유지(스폰·회수·파괴 0)",
            surface_ref(sid)
        );
        return GateRecheck::NoEvidence;
    };
    let marker = load_agent_spec(agent)
        .ok()
        .and_then(|s| s["ready_marker"].as_str().map(|m| m.to_string()));
    let corpus = resolve_gate_corpus(agent);
    let obs = cys::readiness::Observed {
        site: cys::readiness::Site::Boot,
        agent_alive: surface_agent_alive(sid),
        screen: &screen,
        delta: "",
        marker: marker.as_deref(),
        gates: &corpus.gates,
        tail_is_shell_prompt: Some(screen_tail_is_shell_prompt(&screen)),
        bare_shell: Some(screen_is_bare_shell(&screen)),
        time_fallback_reached: true,
        idle_quiet: None,
        legacy_v1: cys::readiness::legacy_v1(),
    };
    gate_pending_recheck(cys::readiness::judge(&obs))
}

/// 재관측이 Ready 를 냈을 때의 **채택** — 표식 해제 + 디렉티브 주입 1회.
///
/// ★주입이 **1회**인 근거: `inject_directive_after_ready` 가 맨 앞에서 `clear_gate_pending` 을
///   부르고, 그 뒤 `directive.verify` 까지 한 번에 끝난다. 재관측은 좌석당 부트 1회만 돌고,
///   채택한 좌석은 다음 부트에서 `awakened_at` 래치로 `AwakeConfirmed` = `already_alive` 가 되어
///   이 분기에 다시 들어오지 않는다(중복 주입 없음).
fn gate_pending_adopt(sid: u64, role: &str, agent: &str) -> Result<BootVerdict, String> {
    let directive = compose_directive(role)?;
    let corpus = resolve_gate_corpus(agent);
    let since_line = fetch_surfaces()
        .iter()
        .find(|s| s["surface_id"].as_u64() == Some(sid))
        .and_then(|s| s["line_count"].as_u64())
        .unwrap_or(0);
    inject_directive_after_ready(
        sid,
        agent,
        &directive,
        &corpus.gates,
        // ★롤백 축은 **1지점 판독**이다(U-11 계약 · H-SEAT-4AXIS ⑦). 여기서 env 를 따로 읽으면
        //   같은 부트 안에서 `boot_agent_on_surface` 폴링의 판독과 값이 갈릴 수 있고, 그 갈림이
        //   곧 **반쪽 롤백**(pane 은 보류인데 좌석은 `already_alive`)이다 — BLOCK-4 와 같은
        //   클래스. 판정을 완화한 것이 아니라 **판독을 합친 것**이다(기준은 그대로).
        gate_close_override_once(),
        since_line,
    )
}

/// 에이전트 기동 + 역할 지침 자동 주입 (어댑터: agents.json).
/// 워커 todo 경로 결정론 산출: 자기 surface의 (데몬 권위) 역할 → `<pack>/round/<ROLE>_TODO.md`.
/// 역할은 데몬 roles 맵(dedup된 worker-N 포함)에서 읽으므로 LLM 치환·env 스냅샷에 의존하지 않는다.
/// 복수 워커는 각자 distinct 역할 → distinct 파일 → 충돌 0. 파일이 없으면 골격을 만들어 둔다.
/// 자기 surface의 cysd-권위 역할 한 단어를 stdout으로 출력 (PreToolUse capability-gate hook 전용).
/// CYS_SURFACE_ID(데몬이 PTY에 주입·상속)로 자기 surface를 surface.list에서 찾아 데몬 roles 맵의
/// role을 출력한다. 역할 미등록·env 부재·데몬 미응답이면 빈 줄 + exit 0(hook이 deny측 안전 판정).
/// ★role은 self-declared가 아니라 데몬 권위 — claim_role/launch-agent가 신원검증 후 등록한 값.
/// ★(W2 · A20/H-EXIT-3) `cys claim-role` **타입드 exit** — CLI 경계의 판정 타입 부재를 메운다.
///
/// 종전 계약은 성공 0 / 그 밖 전부 1 이었다. 그 1비트 붕괴 때문에 소비부(javis_bootstrap ③)는
/// **에러 문자열을 grep** 해 '정당거부'와 '세션 컨텍스트 오류'를 갈라야 했다(문자열 계약 = 드리프트
/// 시한폭탄). 이제 CLI 가 타입을 낸다:
///
/// | exit | 의미 | 소비 처방 |
/// |---|---|---|
/// | 0 | 등록 성공(멱등 재claim 포함) | 계속 |
/// | 7 | **정당거부** — 살아있는 보유자가 있다(이 surface 는 그 역할이 아니다) | 지휘 중단·인계. boot-last 오염 금지(ok:null) |
/// | 6 | **발신 신원 미확정** — 데몬은 응답했으나 발신 pane 을 붙이지 못했다(분리 실행·타 surface · seat 토큰 불일치/토큰-체인 모순 포함) | 세션 배선 점검. **부서 자동 생성 금지** |
/// | 3 | 미도달 — 데몬 미응답·소켓 부재(요청이 데몬에 닿지 못했다) | `cys ping`·데몬 기동 |
/// | 2 | 식별 불가 — surface 해석 실패·인자 오류(요청을 만들 수조차 없다) | 세션 배선(CYS_SURFACE_ID) 점검 |
///
/// ★rc=6 신설(2026-08-16 현장 결함): 데몬이 **신원 미해석**(claim_caller_unresolved)과 **소유
///   불일치**(claim_not_owner)를 "살아있는 보유자 있음"(claim_denied)과 같은 코드로 내던 것을
///   갈랐다. 종전 사슬은 훅이 세션 분리로 발화한 부트의 claim 이 조상 체인 단절로 거부되면 그것을
///   rc 7(정당거부)로 접었고, javis_bootstrap ③이 이를 "다른 master 가 산다"로 읽어 **부서를 자동
///   생성**했다(master 영구 미등록·dept 증식). 6 은 그 오역을 구조적으로 불가능하게 만든다 —
///   bootstrap 은 6 을 EXIT_SESSION_CONTEXT(세션 배선 오류)로 소비하고 위계 폴백에 진입하지 않는다.
///
/// ★W1b 의 bootstrap 소비 분기와 정합(H-EXIT-3 발효): bootstrap 은 exit 7 → EXIT_CLAIM_DENIED,
///   exit 3/2 → EXIT_SESSION_CONTEXT 로 매핑하며 **둘 다 boot-last 에 ok:null** 을 쓴다(CS-2⑩).
///   문자열 grep 은 구 바이너리 하위호환 폴백으로만 남는다.
///
/// ★(P1 · seat 토큰) rc 6 가족에 데몬 payload `reason` 2종이 추가됐다 — **rc 값·분기 술어
///   (에러코드 접두 `claim_caller_unresolved`/`claim_not_owner`)는 불변**이라 구 CLI·bootstrap
///   소비 사슬은 무개정 정합한다(신설 에러코드 금지 — 구 CLI else 분기가 미지 코드를 rc 3
///   '미도달'로 오진하는 스큐 함정 · R3-P1-3):
///   · `claim_caller_unresolved` + reason=`token_mismatch` — 실려 온 seat 토큰이 대상
///     surface 토큰과 다르고 **동세대**(env 오염·타 surface 토큰 복사 의심 — 의도된 소음).
///     전세대 토큰은 부재 취급(체인 폴백)이라 이 사유를 내지 않는다.
///   · `claim_not_owner` + reason=`token_chain_conflict` — 토큰은 유효하나 발신 조상 체인이
///     **다른** pane 으로 신선 재해석됐다(모순 거부권 — 타 pane 토큰 절취·env 복사 봉쇄).
///   토큰 부재는 종전 체인 경로 바이트 동일(fail-open 폴백)이므로 rc 6 의 종전 의미
///   ('체인 단절 ∧ 토큰 부재/불일치')가 그대로 성립한다.
fn run_claim_role(role: &str, surface: Option<String>, takeover_empty_seat: bool) -> i32 {
    let sid = match target_surface(&surface, &None) {
        Ok(sid) => sid,
        Err(e) => {
            eprintln!("[claim-role] 식별 불가: {e} (rc=2 — 세션 배선/인자를 점검하라)");
            return 2;
        }
    };
    let mut params =
        json!({"role": role, "surface_id": sid, "takeover_empty_seat": takeover_empty_seat});
    // ★(P1) seat 토큰 첨부 — 데몬이 pane PTY env(`CYS_SEAT_TOKEN`)로 배달한 발급 비밀을 그대로
    //   실어 나른다(CLI 는 값을 해석·검증하지 않는다 — 발급·대조·수명은 데몬 소유). additive
    //   형제 키라 구 데몬은 무시(wire.rs 계약 — 키별 수동 추출·deny_unknown_fields 없음)하고,
    //   env 부재 시 페이로드는 종전과 **바이트 동일**(수동 실행·구 데몬 스폰 pane·스큐 안전).
    //   롤백: `CYS_BOOT_GATES=0` 이면 토큰 키 자체를 생략한다 — 데몬 무개정으로도 완전 레거시가
    //   성립하는 CLI 측 우산(R3-P1-1 · 전용 노브 신설 금지). env_compat 미사용은 의도다 —
    //   레거시 접두(JAVIS_/AITERM_) 별칭이 없는 신설 키라 정본 키 하나만 판독한다.
    if !cys::gate_axes_forced_legacy() {
        if let Some(tok) =
            std::env::var(cys::ENV_SEAT_TOKEN).ok().filter(|t| !t.is_empty())
        {
            params["seat_token"] = json!(tok);
        }
    }
    match request("system.claim_role", params) {
        Ok(r) => {
            println!(
                "registered: {} → surface:{}",
                r["role"].as_str().unwrap_or("?"),
                sid
            );
            0
        }
        Err(e) => {
            eprintln!("[claim-role] 실패: {e}");
            // 데몬이 낸 에러 **코드**로 분기한다(request 가 "code: message" 로 합성한다).
            // claim_denied = 데몬의 정당거부 마커(특권 역할 live 보유자·live-slot 보호·타 surface claim).
            if e.starts_with("claim_denied") {
                eprintln!(
                    "[claim-role] 정당거부(rc=7): 살아있는 보유자가 그 역할을 쥐고 있다. \
                     이 surface 는 그 역할이 아니다 — 지휘를 중단하고 기존 보유자에게 인계하라."
                );
                // ★출구 안내(2026-08 위계 배선): 이 거부는 '유령 마스터 차단'이지 '조직 확장 금지'가
                // 아니다. 새 부서장을 세우려는 정당한 의도라면 갈 길이 따로 있음을 여기서 알려준다 —
                // 안내가 없으면 선언 pane 의 에이전트가 출구 없이 인계 산문만 반복한다(현장 결함 3호).
                if role == "master" {
                    eprintln!(
                        "[claim-role] 새 부서장을 세우려는 경우: GUI '부서 워크스페이스 추가' 또는 \
                         `cys-dept allocate` 로 독립 부서(전용 데몬·역할 공간)를 만들고 그 안에서 \
                         선언하라. 부서 자동 생성은 **오너가 직접 타이핑한** 마스터 선언(훅 발화 \
                         경로 · base 레인 unix)에서만 이어진다 — 직접 실행·기계 배달 선언은 폭주 \
                         봉인으로 비적용이다."
                    );
                }
                7
            } else if e.starts_with("claim_caller_unresolved") || e.starts_with("claim_not_owner") {
                // ★신원 실패는 조직 사실이 아니다(rc=6 · 2026-08-16) — 정당거부(7)와 융합 금지.
                eprintln!(
                    "[claim-role] 발신 신원 미확정(rc=6): 데몬은 응답했으나 이 프로세스를 발신 \
                     pane 에 붙이지 못했다(세션 분리·재부모화·pane 밖 실행·타 surface 지정). \
                     살아있는 보유자가 있다는 뜻이 **아니다** — 세션 배선을 점검하라."
                );
                6
            } else if e.starts_with("invalid_params") || e.starts_with("not_found") {
                eprintln!("[claim-role] 식별 불가(rc=2): 요청 인자·surface 해석 실패.");
                2
            } else {
                // connect 실패·wire 파손·타임아웃 — 요청이 데몬에 닿지 못했거나 판정을 못 받았다.
                eprintln!(
                    "[claim-role] 미도달(rc=3): 데몬 왕복 실패 — `cys ping` 으로 데몬을 확인하라. \
                     ('남이 master' 라는 뜻이 아니다)"
                );
                3
            }
        }
    }
}

fn run_surface_role() -> i32 {
    // ★(W2 · A5) **3상화** — 종전 이 함수는 세 개의 다른 사실을 하나로 뭉갰다:
    //   ⓐ 역할 있음        → stdout=role, exit 0
    //   ⓑ 역할 없음(미claim) → stdout=빈 줄, exit 0
    //   ⓒ **판정 불가**(데몬 미응답·소켓 hang·응답 파손) → 종전에도 stdout=빈 줄, exit 0
    // ⓑ와 ⓒ가 같은 출력을 냈다(rc0+빈출력 삼킴 — 재검증이 서술을 교정한 실채널). 소비 훅
    // (role-capability-gate·role-bootstrap)은 빈 줄을 '미claim'으로 읽어, **데몬이 죽은 상황을
    // '빈 좌석'으로 오독**하고 부트 발화를 통과시켰다. 이제 ⓒ는 **exit 2 + stderr 진단**이다.
    //
    // ★[U-6 로 개정된 서술] 종전 이 주석은 "request() 전역 데드라인은 바꾸지 않는다 — `feed push
    //   --wait` 가 오너 승인을 데몬 응답 보류로 구현하므로 전역 데드라인이 CEO 승격 동의 채널을
    //   끊는다"였다. 그 위험 자체는 참이었고, U-6 은 전역 상한을 넣되 **블로킹 메서드를 상한에서
    //   파생 처리**해 그 채널을 보존한다(`rpc_server_wait_secs`: `feed.push` wait=true ·
    //   `surface.wait_for` 는 선언 `timeout_secs` + 마진). 그래도 이 경로는 계속
    //   `request_on_timeout` 을 쓴다 — 훅이 사람의 프롬프트 앞에서 기다릴 수 있는 시간은
    //   전역 기본(40s)보다 훨씬 짧아야 하고(아래 10s), 그 짧음은 **이 경로 고유의 요건**이다.
    let Some(my_sid) = cys::env_compat(ENV_SURFACE_ID).and_then(|s| parse_surface_ref(&s)) else {
        // surface env 부재 = pane 밖 실행. 판정 불가가 아니라 '이 프로세스에 surface 가 없다'는
        // 확정 사실이므로 종전대로 빈 줄 + exit 0(훅이 deny 측 안전 판정을 하게 둔다).
        println!();
        return 0;
    };
    // hang 방어 데드라인: readiness 틱 1회분 × 4(≈10s) — RPC 왕복 하나에 넉넉하고, 훅이 사람의
    // 프롬프트 앞에서 무한 정지하지 않을 만큼 짧다. 값은 BUDGET 파생(하드코딩 금지).
    let timeout = std::time::Duration::from_millis(BUDGET_TICK_MS * 4);
    let socket = cys::socket_path();
    // ★가용성 보존 1점: 소켓 파일이 **아예 없으면** 데몬이 내려간 것이고, 그 경우의 정답은 종전처럼
    //   autostart 를 허용하는 `request()`(4s 유계)다. 소켓이 **있는데** 무응답이면 그것이 바로 A5 의
    //   hang 이므로 데드라인 경로만 쓴다 — request() 로 폴백하면 무한 대기가 되살아난다.
    let resp = if socket.exists() {
        request_on_timeout(&socket, "surface.list", json!({}), timeout)
    } else {
        request("surface.list", json!({}))
    };
    match resp {
        Ok(r) => {
            let Some(arr) = r["surfaces"].as_array() else {
                // 응답은 왔지만 계약 형상이 아니다 = 판정 불가(버전 스큐·응답 파손).
                eprintln!(
                    "[surface-role] 판정 불가: surface.list 응답에 surfaces 배열이 없다(버전 스큐/파손). \
                     '미claim' 과 구분되는 사실이므로 exit 2 로 낸다."
                );
                return 2;
            };
            match arr
                .iter()
                .find(|s| s["surface_id"].as_u64() == Some(my_sid))
            {
                // ⓐ/ⓑ: 내 surface 를 찾았다 — role 유무는 확정 사실(빈 문자열=미claim).
                Some(s) => {
                    println!("{}", s["role"].as_str().unwrap_or(""));
                    0
                }
                // 내 surface 가 목록에 없다 = 이 pane 은 데몬이 모르는 surface(재기동 후 stale env 등).
                // 미claim 과 구분되는 사실이지만 '역할 없음'은 참이므로 빈 줄 + exit 0 을 유지하고
                // 진단만 남긴다(훅의 deny-측 안전 판정 계약 보존 — 여기서 exit 2 를 내면 stale env
                // 하나가 능력 가드를 전면 차단한다).
                None => {
                    eprintln!(
                        "[surface-role] CYS_SURFACE_ID={my_sid} 가 데몬 목록에 없다(stale env?) — \
                         역할 없음으로 보고한다"
                    );
                    println!();
                    0
                }
            }
        }
        Err(e) => {
            // ⓒ 판정 불가 — 데몬 미응답·소켓 부재·hang(데드라인 초과). 빈 줄로 삼키지 않는다.
            eprintln!(
                "[surface-role] 판정 불가(데몬 왕복 실패: {e}) — '미claim'이 아니다. \
                 `cys ping` 으로 데몬을 확인하라. (rc=2)"
            );
            2
        }
    }
}

// ══════════════════ ★(U-22) `cys hook` — 훅 결정 프런트도어 ══════════════════
//
// 근본원인 R2: 부트 판정이 **30초짜리 단명 UserPromptSubmit 훅** 안에서 python 프로세스
// 7~14 개를 띄우며 일어났고, 그 과정의 모든 불확실성이 침묵으로 접혔다(rc0+빈출력 삼킴).
// 이 단위는 그 판정 중 **데몬이 이미 메모리에 들고 있는 사실**(좌석·역할)을 데몬으로 돌려보낸다.
//
// ★계약 3줄 요약(셸 `hooks/role-bootstrap.sh` 가 유일한 소비자):
//   ① stdout 에 아무것도 쓰지 않는다 — 훅의 stdout 계약(hookSpecificOutput JSON)은 셸이 소유한다.
//   ② **판정의 1차 근거는 stderr 판정 토큰**(`[cys-hook] hook-decide: <verdict>` 단독 줄)이고
//      exit code 는 보조 진단이다. rc 는 여기서 1차 근거가 될 수 없다 — 이 자리의 통과값이
//      **0**(= 셸에서 가장 흔한 사고값)이면 stub `cys`·래퍼·`--help` 처럼 아무 일도 안 하고
//      성공한 프로세스가 곧 '게이트 통과'가 된다. 실측(2026-08-24 · H-DETECT-7/8): rc 계약만
//      두었더니 목 `cys`(무조건 exit 0) 하나로 role 게이트 전체가 증발해 worker 좌석에서
//      마스터 부트가 오발화했다(A3=B7 재발). 토큰은 판정 본문이 실제로 완주했을 때만 인쇄되므로
//      구조적으로 면역이다 — 이 파일의 기계유래 게이트가 이미 같은 이유로 같은 규약을 쓴다
//      (2026-08-10 W-B: "rc 는 보조 로그로만 남긴다").
//      토큰 줄에는 자유 문구를 섞지 않는다(주입 표면 제거) — 상세는 항상 **다음 줄**이다.
//   ③ **fail-open**: 확신이 없으면 절대 suppress 를 내지 않는다. 이 명령이 새 차단자가 되는
//      경우는 데몬이 "이 좌석은 비-master 다" 라고 명시할 때 하나뿐이다.
//      (제품 제1 계약: 오살이 오탐보다 훨씬 위험하다 — 여기서 잘못 막으면 마스터 부트가 죽는다.)

/// 판정 토큰 줄의 접두 — 셸은 이 접두 + verdict 로만 판정을 읽는다(rc 는 보조).
/// 자유 문구를 섞지 않는 **단독 줄**이라 데몬 오류 문자열이 토큰을 위조할 수 없다.
const HOOK_VERDICT_PREFIX: &str = "[cys-hook] hook-decide: ";
/// 위임 통과 — 셸은 종전 role 게이트를 **건너뛰고** 파이프라인을 계속한다.
const HOOK_EXIT_PROCEED: i32 = 0;
/// 데몬 왕복 실패(구 데몬 `method_not_found` 포함) — 셸이 stderr 문자열로 구 경로를 분류한다.
/// 값 1 은 `javis_reap_exited._legacy_unavailable` ②(구 데몬 스큐)와 **같은 관례**다.
const HOOK_EXIT_DAEMON_ERR: i32 = 1;
/// 데몬 권위 판정: 이 좌석은 비-master 다 — 셸은 마스터 부트를 발화하지 않는다(A3 allowlist).
const HOOK_EXIT_SUPPRESS: i32 = 3;
/// 데몬이 응답했으나 판정 불가(좌석 미해석·미지 verdict·계약 버전 스큐) — 셸이 종전 게이트 수행.
const HOOK_EXIT_UNDECIDED: i32 = 4;
/// 위임이 성립하지 않는다(롤백 스위치 · 소켓 부재) — 셸이 종전 게이트 수행.
const HOOK_EXIT_LEGACY: i32 = 5;
/// `hook.decide` **페이로드** 계약 버전. 전송 프로토콜(`wire::PROTO_PV`)은 무접촉이다 —
/// 이 메서드의 응답 형상만 버전한다. cysd 측 상수와 **같은 값**이어야 하고 그 정합은
/// 검체 H-HOOK-DECIDE-2 가 3중(cys.rs · handlers.rs · role-bootstrap.sh)으로 기계 대조한다.
const HOOK_DECIDE_CONTRACT_V: u64 = 1;
/// 훅 이벤트 이름(와이어 값) — clap 서브커맨드 `user-prompt-submit` 과 같은 철자.
const HOOK_EVENT_USER_PROMPT_SUBMIT: &str = "user-prompt-submit";
/// ★**전용** 데드라인. `surface-role`(BUDGET_TICK_MS×4 ≈10s)보다 짧다 — 이 명령은 사람의
/// 프롬프트 **앞**에 서 있어서, 여기서 쓰는 1초가 그대로 입력 지연이다. 초과해도 손해는
/// "종전 게이트로 폴백" 하나뿐이므로 짧게 잡는 것이 안전 방향이다(하드코딩 금지 — BUDGET 파생).
const HOOK_DECIDE_DEADLINE_MS: u64 = BUDGET_TICK_MS;

fn run_hook(event: HookEvent) -> i32 {
    // ★자기 강제(설계 U-22): 이 프로세스는 **단명 훅의 자식**이다. 여기서 데몬을 낳으면
    //   R2 그 자체(훅 안에서 무거운 기동이 일어나고 실패가 침묵으로 접힘)가 된다.
    //   종전 경로(`cys surface-role` → `request()`)의 autostart 는 셸 폴백에 그대로 남아 있으므로
    //   가용성은 잃지 않는다 — **새 경로만** 봉인한다.
    std::env::set_var(cys::ENV_NO_AUTOSTART, cys::NO_AUTOSTART_ON);
    match event {
        HookEvent::UserPromptSubmit => run_hook_user_prompt_submit(),
    }
}

/// `cys hook user-prompt-submit` — role 게이트(A3 allowlist)의 데몬 권위 판정 1왕복.
///
/// ★stdin 을 읽지 않는다. 훅 본체는 이 호출 **뒤에** `INPUT=$(cat)` 으로 hook JSON 을 먹으므로,
/// 여기서 stdin 을 건드리면 프롬프트 판정이 무음 실패한다(셸 쪽도 `</dev/null` 로 이중 방어).
fn run_hook_user_prompt_submit() -> i32 {
    // ★롤백 1지점(설계 U-22 롤백 = "항상 즉시 반환 · 데몬 미조회"). 새 판정 축은 **태어날 때**
    //   마스터 스위치에 접는다 — 사고 순간에 사람이 노브를 조합할 수는 없다(BLOCK-3 이 그 값을
    //   치렀다). `CYS_BOOT_GATES=0` 하나로 이 위임 전체가 무효가 되고 셸은 종전 게이트를 그대로
    //   수행한다(= 기본값이 아니라 **종전 동작**으로의 완전 복귀).
    if cys::gate_axes_forced_legacy() {
        return hook_verdict(
            "legacy",
            HOOK_EXIT_LEGACY,
            &format!(
                "롤백 스위치({}=0) — 위임 무효, 셸 종전 게이트로 반환",
                cys::ENV_BOOT_GATES
            ),
        );
    }
    let socket = cys::socket_path();
    if !socket.exists() {
        // 소켓 부재 = 데몬 미기동. 여기서 autostart 를 부르지 않는 것이 이 단위의 요점이다
        // (위 NO_AUTOSTART 자기 강제와 같은 방향). 셸 폴백의 종전 경로가 필요하면 그쪽이 켠다.
        return hook_verdict(
            "legacy",
            HOOK_EXIT_LEGACY,
            &format!(
                "소켓 부재({}) — 데몬 미기동, 셸 종전 게이트로 반환(autostart 금지)",
                socket.display()
            ),
        );
    }
    // ★인가 계약: 요청은 `surface_id` 를 **신고할 수 없다**. 좌석은 데몬이 커널 peer pid 의
    //   조상 체인으로 도출한다(claim_role 과 같은 규약) — 자기신고 surface 는 위조 가능하다.
    //   그래서 이 페이로드에는 좌석 식별자가 없다. 있으면 데몬이 invalid_params 로 거절한다.
    //   ★(P1) carve-out — `seat_token` 은 이 금지의 예외다: 데몬이 스폰 시 그 pane 의 PTY env
    //   로만 배달한 **발급 비밀의 대조**라 자기신고가 아니다(위조 불가·검증 가능). 데몬은 좌석
    //   '해석'만 토큰 1차로 확정하고(체인 단절 rc6 계급 관통), 토큰-체인 모순은 undecided 로
    //   접는다 — 판정 규칙(진리표)은 무접촉. additive 형제 키라 구 데몬은 무시(스큐 안전)하고,
    //   env 부재 시 페이로드는 종전과 바이트 동일. `CYS_BOOT_GATES=0` 롤백은 이 함수 선두의
    //   `gate_axes_forced_legacy()` 마스터 스위치가 전체를 legacy 로 접어 이미 토큰 키 생략을
    //   포함한다(별도 분기 불요 — RPC 자체가 나가지 않는다).
    let mut hook_params = json!({
        "event": HOOK_EVENT_USER_PROMPT_SUBMIT,
        "contract_version": HOOK_DECIDE_CONTRACT_V,
    });
    if let Some(tok) = std::env::var(cys::ENV_SEAT_TOKEN).ok().filter(|t| !t.is_empty()) {
        hook_params["seat_token"] = json!(tok);
    }
    let resp = request_on_timeout(
        &socket,
        "hook.decide",
        hook_params,
        std::time::Duration::from_millis(HOOK_DECIDE_DEADLINE_MS),
    );
    let r = match resp {
        Ok(r) => r,
        Err(e) => {
            // 구 데몬은 여기서 `method_not_found` 를 실어 보낸다 — 셸의 `_cys_hook_legacy_unavailable`
            // 가 그 **명시 증거**만 정상 스큐로 분류하고, 그 밖의 rc=1 은 시끄럽게 남긴다
            // (`javis_reap_exited._legacy_unavailable` 3중 계약과 동형 · fail-closed 방향 보존).
            // ★데몬 문자열은 **상세 줄**에만 실린다 — 토큰 줄과 섞으면 오류 본문이 판정을 위조한다.
            return hook_verdict("error", HOOK_EXIT_DAEMON_ERR, &format!("왕복 실패: {e}"));
        }
    };
    // 계약 버전 스큐는 **판정 불가**다(신 데몬 + 구 CLI 방향). 모르는 형상의 verdict 를 믿고
    // suppress 를 내는 것이 이 축에서 가장 나쁜 오작동이므로 undecided 로 접는다.
    if r["contract_version"].as_u64() != Some(HOOK_DECIDE_CONTRACT_V) {
        return hook_verdict(
            "undecided",
            HOOK_EXIT_UNDECIDED,
            &format!(
                "페이로드 계약 버전 스큐(기대 {} · 수신 {}) — 판정 불가",
                HOOK_DECIDE_CONTRACT_V, r["contract_version"]
            ),
        );
    }
    let reason = r["reason"].as_str().unwrap_or("");
    let role = r["role"].as_str().unwrap_or("");
    match r["verdict"].as_str().unwrap_or("") {
        "proceed" => hook_verdict(
            "proceed",
            HOOK_EXIT_PROCEED,
            &format!("role={role:?} · {reason} — 데몬 권위 판정"),
        ),
        "suppress" => hook_verdict(
            "suppress",
            HOOK_EXIT_SUPPRESS,
            &format!("role={role:?} · {reason} — 비-master 좌석(A3 allowlist)"),
        ),
        "undecided" => hook_verdict(
            "undecided",
            HOOK_EXIT_UNDECIDED,
            &format!("{reason} — 셸 종전 게이트로 반환"),
        ),
        // 미지 verdict = 형상 스큐. '선언 아님'으로도 '차단'으로도 접지 않는다.
        other => hook_verdict(
            "undecided",
            HOOK_EXIT_UNDECIDED,
            &format!("미지 verdict({other:?}) — 판정 불가로 처리"),
        ),
    }
}

/// 판정 산출의 **단일 출구** — 토큰 줄(자유 문구 없음) + 상세 줄을 stderr 로 내고 rc 를 돌려준다.
///
/// ★출구를 하나로 묶는 이유: 토큰을 각 분기가 직접 인쇄하면 언젠가 한 분기가 토큰을 빠뜨리고
/// (= 셸이 legacy 로 폴백 · 조용한 기능 소실) 다른 분기가 토큰 줄에 자유 문구를 섞는다
/// (= 위조 표면 부활). 둘 다 이 저장소가 이미 치른 '사본이 낡는' 형태다.
fn hook_verdict(verdict: &str, code: i32, detail: &str) -> i32 {
    let (token_line, detail_line) = hook_verdict_lines(verdict, detail);
    eprintln!("{token_line}");
    eprintln!("{detail_line}");
    code
}

/// 위 산출의 **순수 절반** — (판정 토큰 줄, 상세 줄). 검체가 stderr 를 가로채지 않고 같은
/// 문자열을 얻게 하려고 나눈다(프로덕션 경로를 그대로 태우는 것이 목적 · 목 금지).
fn hook_verdict_lines(verdict: &str, detail: &str) -> (String, String) {
    (
        format!("{HOOK_VERDICT_PREFIX}{verdict}"),
        format!(
            "[cys-hook] hook-decide detail: {}",
            sanitize_hook_detail(detail)
        ),
    )
}

/// 상세 줄 앞의 **치환 표기** — 무해화가 실제로 일어났음을 사람이 볼 수 있게 남긴다
/// (조용히 지우면 다음 감사자가 "원래 그런 문자열" 이라고 오독한다).
const HOOK_DETAIL_REDACTED: &str = "⟪verdict-token⟫";
/// 상세 줄 길이 상한(문자). 데몬이 준 문자열이 화면을 덮어 토큰 줄을 스크롤 밖으로 밀어내는
/// 표면도 함께 없앤다. 스큐 판별 증거(`method_not_found`)는 오류 분기의 짧은 본문에 있어
/// 이 상한에 걸리지 않는다(검체 ④가 단언).
const HOOK_DETAIL_MAX_CHARS: usize = 1024;

/// ★진단 문자열 무해화 — **판정 토큰의 위조 표면 제거**(결함 1 · 2026-08-24 이종 리뷰어).
///
/// 【무엇이 틀렸었는가 — 리뷰어 격리 재현】 상세 줄은 데몬이 준 `role`·`reason`·오류 본문을
/// 그대로 인쇄했고, 셸은 stderr **전문**에 대해 substring `case` 를 돌렸다. 그래서 비-master
/// 좌석이 role 을 `[cys-hook] hook-decide: proceed` 로 claim 하면, `cys hook` 이 올바르게
/// suppress(rc 3)를 내는데도 **상세 줄이 판정을 뒤집었다** — 부트 체인 전체가 이 토큰 하나에
/// 달려 있으므로 귀결은 A3=B7(비-master 좌석의 마스터 부트 오발화) 재발이다.
///
/// 【왜 여기서 하는가】 `hook_verdict` 는 판정 산출의 **단일 출구**다. 각 분기에서 role 만
/// 이스케이프하면 다음에 늘어나는 필드(reason·contract 값·오류 본문)가 그물 밖에 남는다 —
/// 이 저장소에서 살아남는 결함은 전부 그런 이음매에 있다. 출구 하나를 잠근다.
///
/// 【무해화 규칙】 ⓐ 제어문자(개행·CR 포함)는 공백으로 접는다 — 상세는 **항상 한 줄**이다
/// (줄이 늘어나는 순간 '정확 일치' 판독도 위조 가능해진다). ⓑ 토큰 접두는 치환 표기로 바꾼다
/// — 판독이 substring 으로 되돌아가도 뒤집히지 않는다(다중 방어). ⓒ 길이를 자른다.
///
/// ★이 함수는 **판정을 바꾸지 않는다**. verdict·exit code 는 데몬 응답에서만 나오고,
///   여기서 손대는 것은 사람이 읽는 진단 문자열뿐이다.
fn sanitize_hook_detail(detail: &str) -> String {
    // ⓐ 제어문자 → 공백(개행·CR 포함). Debug 포매팅이 이미 대부분을 이스케이프하지만,
    //    그 사실에 기대면 포매팅을 바꾸는 다음 사람이 표면을 되살린다.
    let flat: String = detail
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    // ⓑ 토큰 접두 무해화. ★(P2) boot-intent 토큰 접두도 같은 출구에서 접는다 — 상세 줄은
    //    어느 판정 어휘의 토큰도 실을 수 없다(두 판독기 모두 줄 단위 정확 일치이지만, 판독이
    //    미래에 다시 넓어져도 산출 측 층이 선다 — 다중 방어).
    let safe = flat.replace(HOOK_VERDICT_PREFIX, HOOK_DETAIL_REDACTED)
        .replace(BOOT_INTENT_VERDICT_PREFIX, HOOK_DETAIL_REDACTED);
    // ⓒ 길이 상한(문자 경계 안전).
    if safe.chars().count() <= HOOK_DETAIL_MAX_CHARS {
        return safe;
    }
    let head: String = safe.chars().take(HOOK_DETAIL_MAX_CHARS).collect();
    format!("{head}…(truncated)")
}

// ══════════════════ ★(P2) `cys boot-intent` — 부트 인텐트 프런트도어 ══════════════════
//
// 훅의 직접 spawn(백그라운드 발화 → 재부모화로 조상 체인 단절)을 데몬 감독자 스폰으로
// 이관하는 입구다(R3-P2-1 ⓑ′). 이 명령은 **판정을 하지 않는다** — RPC `boot.enqueue` 가
// 좌석을 커널 도출하고 스풀에 원자 기록한 뒤 즉시 ack 하며(R3-RISK-2 · 부트 완료 대기 금지),
// 여기서는 그 결과를 토큰+exit 로 환원할 뿐이다.
//
// ★계약(훅 `role-bootstrap.sh` 가 유일한 소비자 — hook-decide 판독기와 동형 · R3-P2-3):
//   ① stdout 에 아무것도 쓰지 않는다(훅 stdout 계약은 셸 소유).
//   ② 판정의 1차 근거는 stderr 토큰(`[cys-hook] boot-intent: <verdict>` 단독 줄) · rc 는 보조.
//      rc 를 1차로 읽으면 stub `cys`(무조건 exit 0)가 '등록 성공'으로 읽혀 폴백 spawn 이
//      건너뛰어지고 부트가 무음 사망한다 — 이 저장소가 두 번 치른 rc0=통과 클래스 그 자체.
//   ③ exit 계약은 run_hook 동형: 0=enqueued(스풀 기록 확정) / 1=daemon-error(왕복 실패 —
//      구 데몬 `method_not_found` **원문 보존** · R3-P2-8) / 4=undecided(응답 형상 스큐) /
//      5=legacy(마스터 스위치·소켓 부재·supervisor_off — 셸 종전 spawn 폴백).
//   ④ fail-open: 이 명령의 산출 중 훅이 '스폰 생략'으로 읽는 값은 enqueued+rc0 **하나**다 —
//      그 외 전부는 종전 spawn 폴백이므로 이 위임은 새 차단자를 만들지 않는다(R3-P2-8).

/// boot-intent 판정 토큰 줄의 접두 — 셸은 이 접두 + verdict 의 **줄 단위 정확 일치**로만 읽는다.
const BOOT_INTENT_VERDICT_PREFIX: &str = "[cys-hook] boot-intent: ";
/// 전용 데드라인 — hook-decide 와 같은 근거(사람의 프롬프트 앞 · BUDGET 파생 · 하드코딩 금지).
/// 서버는 즉답 계약(스풀 기록=ack)이므로 이 상한은 wedge 방어일 뿐이다. 바깥에는 훅의
/// `cys_timeout_run 5s` 외곽 데드라인이 한 겹 더 있다(R3-RISK-2).
const BOOT_INTENT_DEADLINE_MS: u64 = BUDGET_TICK_MS;

/// `cys boot-intent` — 선행 claim 을 마친 훅이 부트 인텐트를 데몬 스풀에 등록하는 1왕복.
///
/// ★stdin 을 읽지 않는다(훅 stdin 은 프롬프트 판정 소유 — `cys hook` 과 같은 규율).
/// ★페이로드는 env 릴레이다: 훅이 spawn 직전에 export 한 `CYS_DECL_ORIGIN`(기계유래 게이트
///   통과 마커)·`CYS_CLAIM_RC`/`CYS_CLAIM_AT`(선행 claim 관측치)을 데이터로 싣는다.
///   `surface_id`·`lane` 은 싣지 않는다 — 자기신고는 데몬이 invalid_params 로 거절하는 인가
///   계약이고(hook.decide 동형), 좌석은 데몬이 caller_pid 조상 체인으로 도출한다.
fn run_boot_intent() -> i32 {
    // ★자기 강제(run_hook 동형): 이 프로세스는 단명 훅의 자식이다 — 데몬을 낳지 않는다.
    std::env::set_var(cys::ENV_NO_AUTOSTART, cys::NO_AUTOSTART_ON);
    // ★롤백 1지점: 마스터 스위치는 태어날 때 접는다(R3-P2-4 — CLI 측 자기 env 우산).
    if cys::gate_axes_forced_legacy() {
        return boot_intent_verdict(
            "legacy",
            HOOK_EXIT_LEGACY,
            &format!(
                "롤백 스위치({}=0) — 위임 무효, 셸 종전 spawn 폴백",
                cys::ENV_BOOT_GATES
            ),
        );
    }
    let socket = cys::socket_path();
    if !socket.exists() {
        return boot_intent_verdict(
            "legacy",
            HOOK_EXIT_LEGACY,
            &format!(
                "소켓 부재({}) — 데몬 미기동, 셸 종전 spawn 폴백(autostart 금지)",
                socket.display()
            ),
        );
    }
    let mut params = json!({ "reason": "role-bootstrap-hook" });
    if let Some(origin) = std::env::var("CYS_DECL_ORIGIN").ok().filter(|v| !v.is_empty()) {
        params["decl_origin"] = json!(origin);
    }
    if let Some(rc) = std::env::var("CYS_CLAIM_RC").ok().and_then(|v| v.parse::<i64>().ok()) {
        params["claim_rc"] = json!(rc);
    }
    if let Some(at) = std::env::var("CYS_CLAIM_AT")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|at| at.is_finite() && *at > 0.0)
    {
        params["claim_at"] = json!(at);
    }
    let resp = request_on_timeout(
        &socket,
        "boot.enqueue",
        params,
        std::time::Duration::from_millis(BOOT_INTENT_DEADLINE_MS),
    );
    match resp {
        Ok(r) if r["enqueued"].as_bool() == Some(true) => boot_intent_verdict(
            "enqueued",
            HOOK_EXIT_PROCEED,
            &format!(
                "intent={} surface={} — 스풀 기록 완료, 스폰은 데몬 감독자 소관",
                r["id"].as_str().unwrap_or("?"),
                r["surface_id"]
            ),
        ),
        // 응답은 왔지만 기록 확정 페이로드가 아니다(형상 스큐) — '기록됨'으로 읽는 것이 이
        // 축에서 가장 나쁜 오작동이므로 undecided 로 접는다(셸은 spawn 폴백).
        Ok(r) => boot_intent_verdict(
            "error",
            HOOK_EXIT_UNDECIDED,
            &format!("응답 형상 스큐(enqueued≠true): {r}"),
        ),
        // ★(R3-P2-4 blocker 소비면) 감독자 미기동 typed 오류 — '등록 성공·발화자 0' 무음
        //   후퇴를 legacy 로 환원해 셸이 종전 spawn 폴백을 타게 한다(인텐트는 기록되지 않았다).
        Err(e) if e.starts_with("supervisor_off") => {
            boot_intent_verdict("legacy", HOOK_EXIT_LEGACY, &format!("{e} — 셸 종전 spawn 폴백"))
        }
        // 구 데몬은 여기서 `method_not_found` 를 실어 보낸다 — 상세 줄에 **원문 보존**해야
        // 셸 `_cys_hook_legacy_unavailable` 가 정상 스큐(조용)와 진짜 결함(loud)을 가른다.
        Err(e) => boot_intent_verdict("error", HOOK_EXIT_DAEMON_ERR, &format!("왕복 실패: {e}")),
    }
}

/// boot-intent 판정 산출의 **단일 출구** — `hook_verdict` 와 같은 이유로 하나로 묶는다
/// (토큰 누락 = 조용한 기능 소실 · 토큰 줄 자유 문구 = 위조 표면).
fn boot_intent_verdict(verdict: &str, code: i32, detail: &str) -> i32 {
    let (token_line, detail_line) = boot_intent_verdict_lines(verdict, detail);
    eprintln!("{token_line}");
    eprintln!("{detail_line}");
    code
}

/// 위 산출의 순수 절반 — (토큰 줄, 상세 줄). 상세는 `sanitize_hook_detail` 재사용(제어문자
/// 접기·토큰 접두 치환·길이 상한 — 위조 차단 층의 단일 소유).
fn boot_intent_verdict_lines(verdict: &str, detail: &str) -> (String, String) {
    (
        format!("{BOOT_INTENT_VERDICT_PREFIX}{verdict}"),
        format!(
            "[cys-hook] boot-intent detail: {}",
            sanitize_hook_detail(detail)
        ),
    )
}

/// 선언 블록 v1 한 줄을 만들고 **파서 왕복 검증**까지 한다(설계 §4-1 · S17).
///
/// ★생성물을 파서에 먹여 `counted`가 나오는지 보는 것이 유일한 계약 준수 증명이다 —
/// 문자 클래스(G4)·필수 키(G5)를 여기서 다시 검사하면 검사식이 두 벌이 되고, 그중 하나는
/// 반드시 뒤처져 소비자와 갈린다(Python 스탬프 도구 `build_decl_line`과 **같은 패턴**).
///
/// ★실패는 **시끄럽다**. 접거나 그럴듯한 기본값으로 대체하지 않는다 — `scope`를 정규화로
/// 접고 `"pack"`으로 폴백하던 생산자가 있었고, 그 "그럴듯하지만 틀린 정체성"이 살아있는
/// 파일을 남의 레인(foreign-scope)으로 조용히 배제시켰다(S14와 같은 병).
fn build_todo_decl_line(owner: &str, scope: &str) -> Result<String, String> {
    let line = format!("<!-- javis:todo v1 owner={owner} scope={scope} status=active -->");
    let decl = cys::todo_decl::parse(&format!("{line}\n"))
        .map_err(|e| format!("선언 생성 실패({}: {e})", e.code))?;
    let verdict = cys::todo_decl::classify(Some(&decl), scope, &|_| true);
    if verdict != cys::todo_decl::Verdict::Counted {
        return Err(format!("선언 생성 실패(판정={verdict})"));
    }
    Ok(line)
}

/// 새 todo 파일의 초기 본문 — **선언이 첫 줄**이다(G1' 위치 계약에 여유롭게 들어간다).
fn new_todo_body(role: &str, decl_line: &str) -> String {
    format!("{decl_line}\n\n# {role} TODO — 영속 todo (절대지침 7)\n\n")
}

fn run_todo_path(role_opt: Option<String>, emit_decl: bool) -> i32 {
    // `--role` 지정 = 남의 역할 산출 = **파일 기록 금지**(설계 R7 신원 게이트 우회 방지).
    let foreign = role_opt.is_some();
    // `--role`은 **남의 역할 경로 산출 전용**이다(파일 생성·기록 없음 · 설계 R7). 자기 역할은
    // 데몬이 정본이므로 surface.list로 조회한다 — 손으로 지어 부르는 것을 막는 것이 이 명령의 존재 이유다.
    let role = match role_opt {
        Some(r) => r,
        None => {
            let Some(sref) = cys::env_compat(ENV_SURFACE_ID) else {
                eprintln!("CYS_SURFACE_ID 없음 — 데몬이 띄운 pane 안에서만 동작한다(다른 역할은 --role)");
                return 1;
            };
            let Some(my_sid) = parse_surface_ref(&sref) else {
                eprintln!("CYS_SURFACE_ID 파싱 실패: {sref}");
                return 1;
            };
            let role = match request("surface.list", json!({})) {
                Ok(r) => r["surfaces"].as_array().and_then(|arr| {
                    arr.iter()
                        .find(|s| s["surface_id"].as_u64() == Some(my_sid))
                        .and_then(|s| s["role"].as_str().map(|x| x.to_string()))
                }),
                Err(e) => {
                    eprintln!("surface.list 실패: {e}");
                    return 1;
                }
            };
            let Some(role) = role else {
                eprintln!("이 surface에 역할 미등록 — todo-path는 역할 노드(claim-role/launch-agent) 전용");
                return 1;
            };
            role
        }
    };

    // ★S19 — 팩 경로는 `cys::pack::pack_dir()` 단일 구현을 경유한다. 종전에는
    // `env_compat("CYS_PACK_DIR")`(= CYS_/JAVIS_/AITERM_**PACK**_DIR)만 봐서 레거시 키
    // `AITERM_JARVIS_DIR`를 인식하지 못했다 — 그 환경에서는 **생성 위치와 스캔 위치가 갈려
    // 파일이 보고기에 영영 보이지 않는다**(env 목록이 두 벌이면 언젠가 갈린다는 실증).
    let pack = cys::pack::pack_dir();
    let scope = cys::pack::scope_id();

    // 선언은 **경로보다 먼저** 만든다 — 만들 수 없으면 파일도 만들지 않는다(부분 성공 금지).
    let decl_line = match build_todo_decl_line(&role, &scope) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "{e}\n  role={role} scope={scope}\n  \
                 값 문자 클래스(G4 `[A-Za-z0-9._:-]+`)를 벗어난 이름은 선언이 될 수 없다. \
                 접어서 그럴듯한 값을 만들지 않는 것이 계약이다 — 틀린 정체성은 살아있는 \
                 파일을 남의 레인으로 조용히 배제시킨다."
            );
            return 1;
        }
    };
    if emit_decl {
        println!("{decl_line}");
        return 0;
    }

    let round = pack.join("round");
    let fname = format!("{}_TODO.md", role.to_uppercase().replace('-', "_"));
    let path = round.join(&fname);

    // `--role`(남의 역할)은 산출만 한다 — 디렉터리도 만들지 않는다.
    if foreign {
        println!("{}", path.display());
        return 0;
    }

    if let Err(e) = std::fs::create_dir_all(&round) {
        eprintln!("round 디렉터리 생성 실패: {e}");
        return 1;
    }
    if !path.exists() {
        if let Err(e) = std::fs::write(&path, new_todo_body(&role, &decl_line)) {
            eprintln!("todo 파일 생성 실패: {e}");
            return 1;
        }
        // ★자기 산출물 파서 왕복 검증(스탬프 도구 `verify_counted`와 같은 패턴). 의무화된
        // 생성기가 의무화된 규칙을 위반하면 `unclaimed_ratio`가 구조적으로 M3 목표(<10%) 아래로
        // 수렴하지 못한다 — 실제로 종전 생성기는 선언 없는 파일만 찍었다.
        match verify_todo_counted(&path, &scope) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("생성한 todo가 파서 검증을 통과하지 못했다: {e}\n  경로={}", path.display());
                return 1;
            }
        }
    } else if let Err(e) = verify_todo_counted(&path, &scope) {
        // 기존 파일은 **건드리지 않는다**(증거 보존 · 스탬프 도구 소관). 다만 조용히 넘기지도
        // 않는다 — 이 파일이 왜 집계에 안 잡히는지를 지금 말해 주는 편이 낫다(G9의 정신).
        eprintln!(
            "ℹ 기존 todo가 선언 판정을 통과하지 못한다: {e}\n  경로={}\n  \
             일괄 스탬프: python3 \"${{CYS_PACK_DIR:-$HOME/.cys/pack}}/bin/javis_todo_stamp.py\" --apply",
            path.display()
        );
    }
    println!("{}", path.display());
    0
}

/// 파일을 다시 읽어 선언이 `counted`인지 확인한다 — 읽기 경로는 계약 정본을 경유한다.
fn verify_todo_counted(path: &std::path::Path, scope: &str) -> Result<(), String> {
    let raw = std::fs::read(path).map_err(|e| format!("재검증 읽기 실패: {e}"))?;
    let head = cys::todo_decl::head_from_bytes(&raw);
    let decl = cys::todo_decl::parse(&head).map_err(|e| format!("{}: {e}", e.code))?;
    // scope 실재는 여기서 묻는 것이 아니다(디스크 상태는 소비자 관심사) — 내 선언이 유효하고
    // 내 scope로 집계되는가만 본다. 스탬프 도구 `verify_counted`와 같은 판단이다.
    let verdict = cys::todo_decl::classify(Some(&decl), scope, &|_| true);
    if verdict != cys::todo_decl::Verdict::Counted {
        return Err(format!("판정={verdict}"));
    }
    Ok(())
}

/// 루트 cwd("/"·"\\"·"C:\\" 류)를 home으로 교정 — 순수 함수(진리표 테스트 가능).
/// 근거: launchd/Finder 상속으로 루트에서 태어난 노드·roster 오염 실사고(2026-07-15).
fn sanitize_launch_cwd(cwd: String) -> String {
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || (trimmed.len() == 2 && trimmed.ends_with(':')) {
        return cys::home_dir().to_string_lossy().into_owned();
    }
    cwd
}

fn run_launch_agent(role: &str, agent: &str, cwd: Option<String>) -> i32 {
    run_launch_agent_opts(role, agent, cwd, false, None, false, None)
}

/// 절대지침(앵커1-b): 탭(타이틀) = 워크플로우 폴더명 — "{role}-{agent} · {폴더}".
/// 폴더를 알 수 없으면(루트 등) 역할-에이전트만. 순수 함수 — 회귀 핀.
/// `/`·`\`를 모두 구분자로 취급해 플랫폼과 무관하게 마지막 컴포넌트를 폴더명으로 쓴다
/// (std::path::Path는 Unix에서 `\`를 구분자로 보지 않아 Windows 경로가 통째로 잡힌다 —
/// 데몬·클라이언트가 OS를 교차할 수 있으므로 수동 분할이 결정론적·이식 가능하다).
fn workflow_title(role: &str, agent: &str, cwd: &Option<String>) -> String {
    cwd.as_deref()
        .map(|s| s.trim_end_matches(['/', '\\']))
        .and_then(|s| s.rsplit(['/', '\\']).next())
        .filter(|f| !f.is_empty())
        // Windows 드라이브 루트(`C:\` → 트림 후 `C:`)는 폴더명이 아니다 — 폴백.
        .filter(|f| !(f.len() == 2 && f.ends_with(':') && f.as_bytes()[0].is_ascii_alphabetic()))
        .map(|folder| format!("{role}-{agent} · {folder}"))
        .unwrap_or_else(|| format!("{role}-{agent}"))
}

#[allow(clippy::too_many_arguments)]
fn run_launch_agent_opts(
    role: &str,
    agent: &str,
    cwd: Option<String>,
    resume: bool,
    session_id: Option<String>,
    restore: bool,
    // (W1) restore가 topology에 기록된 원 계정 config_dir을 넘긴다(재해소 금지). 신규 기동은 None.
    config_dir_override: Option<String>,
) -> i32 {
    // ★(W2 · G12) LAUNCH 경로의 boot 락 참여 — 별도 프로세스로 도는 `cys launch-agent`
    //   (javis_boot_node → boot-reviewers 경로)를 GUI/훅 `cys boot` 와 직렬화한다.
    //   run_boot 이 이미 쥐고 있으면 None(재진입 방어). Drop 에서 flock 자동 해제.
    let _launch_lock = acquire_launch_lock();
    // 절대지침(앵커1-b): 워커는 워크플로우 폴더에서 산다 — cwd 미지정이면 호출 폴더가
    // 워크플로우 폴더다 (데몬 기본값 home에 맡기지 않는다. 명시 --cwd는 그대로 우선).
    // 빈 문자열은 None으로 정규화 — 구버전 topology의 "cwd": "" 가 PTY 생성을 깨거나
    // 잘못된 타이틀을 만드는 것을 차단(restore 경로 방어).
    // ★루트 cwd 교정(오너 2026-07-15 실사고): Finder 런칭 GUI·launchd 소유 cysd의 cwd는 "/"라
    // ▶CEO/▶부서장 버튼·cys boot 경유 노드가 루트에서 태어나고, 그 값이 phoenix roster에
    // "진실"로 영속돼 이후 복원까지 오염시켰다. 루트는 이 제품에서 워크플로우 폴더일 수 없다
    // — home으로 교정한다(명시 --cwd "/"도 교정 대상 · restore가 넘기는 오염 roster 값도 치유).
    let cwd = cwd
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .map(sanitize_launch_cwd);
    // 기동 실패 시 정리용 — 만들어 둔 surface가 role을 점유한 채 남으면 재기동이 차단된다
    let mut created: Option<u64> = None;
    let result = (|| -> Result<BootVerdict, String> {
        let spec = load_agent_spec(agent)?;
        // (E-f) 멱등 기동 키 — 같은 role+agent+cwd 재시도가 중복 surface를 만들지 않게
        // 데몬이 단기 캐시(create_idem)로 기존 surface를 재반환하도록. 단일 머신·단일
        // 사용자라 단순 해시로 충분(설계 §4.1.5).
        let idem = format!(
            "la-{}-{}-{}",
            role,
            agent,
            cwd.as_deref()
                .unwrap_or("")
                .chars()
                .map(|c| c as u32)
                .fold(0u64, |a, c| a.wrapping_mul(31).wrapping_add(c as u64))
        );
        // RC-3(B′): Windows는 해소된 env(CLAUDE_CONFIG_DIR 등)를 surface.create로 넘겨 데몬이
        // PTY spawn 시 builder.env로 주입한다(순수 cmd send와 짝). unix는 빈 맵 — 셸 인라인 전개가
        // 진실원(무회귀). render_launch와 동일 규약이라 두 경로 결정론 일치.
        // ★D5(v4 · W4): 두 소비처(여기 surface.create env 맵 · boot_agent_on_surface 인라인
        // 재조립)가 **모두 lib 헬퍼를 경유**한다(사본 금지).
        // · mac: unix 는 인라인 `KEY="val" cmd` 전개가 진실원이라 render_launch 가 이 맵을 비워
        //   보낸다 → 여기 삽입은 무영향(규약 단일화 목적).
        // · Windows: 게이트가 **옵트인**(`~/.cys/win-no-alt-screen` · `CYS_WIN_NO_ALT_SCREEN=1`)
        //   일 때만 참이므로, **기본값에서는 여기서 아무것도 삽입되지 않는다**(강등 전 출고본과
        //   동일 — 2026-08-17 강등, 근거·승격 절차는 lib.rs `d5_gate_for_os` doc).
        //   옵트인한 경우에만 삽입되고, 그때는 실제로 pane 에 실린다 — Windows 는 순수 cmd send 라
        //   env 를 인라인으로 못 싣고 데몬이 surface.create 의 이 맵을 PTY spawn 시 builder.env 로
        //   주입하기 때문이다. ★즉 **이 경로가 D5 env 벨트가 Windows 에서 도달하는 유일한 경로**다
        //   (새 surface 를 만들며 기동하는 launch-agent 한정 — 기존 pane 에 붙는
        //   boot_agent_on_surface 는 env 를 폐기하고, Tauri GUI 의 create_surface 는 env 를 아예
        //   넘기지 않는다: src-tauri/src/main.rs 의 surface.create 페이로드 참조).
        //   그래서 이것은 '벨트'이고 본체는 UI 가드(ui/src/wheelgate.ts)다 — lib 헬퍼 주석 정본.
        let mut create_env_pairs = agent_env_pairs(&spec);
        cys::inject_claude_alt_screen_default(
            &mut create_env_pairs,
            extract_bin(spec["cmd"].as_str().unwrap_or(""), agent),
        );
        let (_, inject_env) = render_launch("", &create_env_pairs);
        let env_obj: serde_json::Map<String, Value> = inject_env
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        let r = request(
            "surface.create",
            json!({"cwd": cwd, "title": workflow_title(role, agent, &cwd), "role": role,
                   "rows": 40, "cols": 140, "idempotency_key": idem, "env": env_obj,
                   // ★SEAT: launch-agent 는 '이 역할의 노드를 실제로 띄우겠다'는 명시 의사다 —
                   // 보유자가 빈 좌석(agent 없는 셸)이면 승계를 요청한다. 데몬이 그 좌석이 정말
                   // 비었는지 결정론으로 재판정하므로(seat_claimable), 이 플래그는 '요청'일 뿐
                   // 강제가 아니다. agent 가 붙은 정당한 보유자는 종전대로 claim_denied 로 보호된다.
                   "takeover_empty_seat": true,
                   // (W1) restore 원값 전달(부재=신규는 데몬이 자기 env로 결정론 해소·기록).
                   "claude_config_dir": config_dir_override}),
        )?;
        let sid = r["surface_id"].as_u64().ok_or("create returned no id")?;
        created = Some(sid);
        eprintln!("[launch-agent] {} created (role={role})", surface_ref(sid));
        // (W1) 데몬이 기록·반환한 권위 config_dir을 resume 게이트·restore 인라인의 결정론 소스로 쓴다.
        let recorded_cfg = r["claude_config_dir"].as_str().map(String::from);
        // ★T-0147-5(W3): 그 config dir 의 settings.json 에 각성 훅이 없으면 **1분 내 원인 가시화**.
        //   이 노드는 뜨지만 `/clear` 후 지침 재주입도, 마스터 선언 부트도 발화하지 않는다 —
        //   종전엔 그 사실이 어디에도 나타나지 않아 "노드는 살아있는데 각성만 안 되는" 침묵 고장이 됐다.
        warn_if_awakening_hooks_missing(recorded_cfg.as_deref(), role, agent);
        let verdict = boot_agent_on_surface(
            sid,
            role,
            agent,
            &spec,
            resume,
            session_id.as_deref(),
            restore,
            cwd.as_deref(),
            recorded_cfg.as_deref(),
        )?;
        // ★(W4 · B5) stdout 계약: **보류에서도** 생성한 surface ref 를 낸다. GUI(start_master)와
        //   `javis_bootstrap` 이 이 값으로 ③claim-role 을 그 pane 에 귀속시키므로, 보류를 침묵으로
        //   처리하면 pane 은 살아 있는데 소비부가 그것을 못 찾는 침묵 고장이 된다. 진단·처방·
        //   보류 사유는 전부 stderr 로 간다(계약은 "stdout 마지막 줄 = surface ref" 하나다).
        println!("{}", surface_ref(sid));
        Ok(verdict)
    })();
    match result {
        Ok(BootVerdict::Ready) => 0,
        // ★관문 보류: close 0 · 좌석 보존 · 표식(데몬 기록은 판정 지점에서 이미 끝났다) · 처방.
        //   종료코드는 성공(0)도 실패(1)도 아닌 전용 값이다 — 0 이면 소비부가 '노드를 세웠다'로
        //   읽어 디렉티브·티켓을 태우고(그 주입 Return 이 관문 창을 누른다), 1 이면 '깨졌다'로
        //   읽어 살아 있는 좌석을 회수·파괴하려 든다.
        Ok(BootVerdict::GatePending { gate, tail }) => {
            if let Some(sid) = created {
                print_gate_pending_prescription(sid, role, agent, &gate, &tail);
            }
            cys::EXIT_GATE_PENDING
        }
        // ★(U-11) `LaunchFailed` 는 종전의 `Err`(데몬 왕복 실패 등 절차 미완)와 **완전히 같은
        //   귀결**이다 — or-패턴으로 합류시켜 롤백 close 블록을 한 벌로 유지한다. 두 벌이 되면
        //   한쪽만 고쳐지는 날 '실패했는데 좌석이 남는' 또는 '보류인데 닫히는' 비대칭이 생긴다.
        //   (귀결이 갈리는 것은 `GatePending` 하나뿐이라는 사실이 이 구조에 그대로 드러난다.)
        Ok(BootVerdict::LaunchFailed { evidence: e }) | Err(e) => {
            eprintln!("error: {e}");
            if let Some(sid) = created {
                // close 결과를 정직히 보고한다 — 실패를 'closed'로 거짓 보고하면 role이
                // 좀비 surface에 점유된 채 남아 재기동이 claim_denied로 막힌다(이번 회귀의 근원).
                // ★W2/P0-6: cause="reap" — launch 실패 롤백은 역할을 묘비화하지 않는다(부활 대상 유지). 과거
                // 고정 OwnerClose 라 실패한 worker launch 1회가 역할을 영구 오묘비화하던 우회로를 끊는다.
                match request("surface.close", json!({"surface_id": sid, "cause": "reap"})) {
                    Ok(_) => eprintln!(
                        "[launch-agent] failed surface {} closed (role 점유 해제)",
                        surface_ref(sid)
                    ),
                    Err(e) => eprintln!(
                        "[launch-agent] failed surface {} close 실패: {e} — \
                         `cys close-surface {}`로 수동 정리 필요(role 점유 잔존 가능)",
                        surface_ref(sid),
                        surface_ref(sid)
                    ),
                }
            }
            1
        }
    }
}

// ---------- 온보딩③: 상시 가동 등록 (launchd / Task Scheduler) ----------
// plist 포맷·경로·LABEL은 `cys::launchd`(앱 자동등록과 단일 소스) 위임 — 드리프트 방지.

fn run_daemon_cmd(action: DaemonAction) -> i32 {
    let result: Result<(), String> = (|| {
        #[cfg(target_os = "macos")]
        {
            match action {
                DaemonAction::Install { takeover } => {
                    let daemon = sibling_daemon_path()
                        .ok_or("cysd binary not found next to cys (같은 폴더에 동봉 필요)")?;
                    let running = connect_raw().is_ok();
                    if running && !takeover {
                        return Err(
                            "데몬이 이미 가동 중 — 등록만 하면 launchd 인스턴스가 flock에 막혀 재시도 루프가 된다.\n\
                             기존 데몬을 정지하고 소유권을 이관하려면: cys daemon install --takeover\n\
                             (주의: 가동 중인 세션이 소멸한다 — `cys list`로 먼저 확인)"
                                .into(),
                        );
                    }
                    let plist = cys::launchd::render_plist(&daemon, &cys::launchd::log_path());
                    let path = cys::launchd::plist_path();
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&path, plist).map_err(|e| e.to_string())?;
                    if running && takeover {
                        // 소유권 이관: 기존 데몬 정상 종료 (SIGTERM — scoped 정리·소켓 제거).
                        eprintln!("[daemon] 기존 데몬 정지 중 (소유권 이관)…");
                        // ★기존 job이 이미 launchd 적재 상태면 KeepAlive가 kill 직후 재기동해
                        // 폴링이 영영 down을 못 본다 → kill 전에 먼저 unload(KeepAlive 해제).
                        if cys::launchd::is_loaded() {
                            let _ = std::process::Command::new("launchctl")
                                .args(["unload", "-w"])
                                .arg(&path)
                                .output();
                        }
                        // ⚠ `pkill -x cysd`는 macOS comm이 15자로 잘려(/Applications/cy…)
                        // 매칭에 실패한다 → 데몬이 보고하는 self-pid로 정확히 종료한다.
                        let pid = request("system.identify", json!({}))
                            .ok()
                            .and_then(|v| v["daemon_pid"].as_u64());
                        if let Some(pid) = pid {
                            let _ = std::process::Command::new("kill")
                                .args(["-TERM", &pid.to_string()])
                                .output();
                        } else {
                            // 폴백: 전체 인자 경로 매칭(comm 절단 무관).
                            let _ = std::process::Command::new("pkill")
                                .args(["-TERM", "-f", "MacOS/cysd"])
                                .output();
                        }
                        // 고정 sleep 대신 flock 해제(=소켓 연결 불가)까지 폴링(최대 5초).
                        let mut down = false;
                        for _ in 0..50 {
                            if connect_raw().is_err() {
                                down = true;
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                        if !down {
                            return Err(
                                "기존 데몬이 5초 내 종료되지 않음 — launchctl load 보류(수동 확인 필요)"
                                    .into(),
                            );
                        }
                    }
                    let _ = std::process::Command::new("launchctl")
                        .args(["unload", "-w"])
                        .arg(&path)
                        .output(); // 재등록 대비 (실패 무시)
                    let out = std::process::Command::new("launchctl")
                        .args(["load", "-w"])
                        .arg(&path)
                        .output()
                        .map_err(|e| e.to_string())?;
                    if !out.status.success() {
                        return Err(format!(
                            "launchctl load failed: {}",
                            String::from_utf8_lossy(&out.stderr).trim()
                        ));
                    }
                    // 기동 확인
                    let mut up = false;
                    for _ in 0..40 {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        if connect_raw().is_ok() {
                            up = true;
                            break;
                        }
                    }
                    println!(
                        "launchd 등록 완료: {} (로그인 자동 기동 + 사망 시 자동 재기동)",
                        path.display()
                    );
                    println!("데몬 가동: {}", if up { "확인됨" } else { "미확인 — log 확인" });
                    println!("⚠ 이후 nohup 수동 기동과 병행 금지 (flock 충돌 — launchd가 단독 소유)");
                    Ok(())
                }
                DaemonAction::Uninstall => {
                    let path = cys::launchd::plist_path();
                    let _ = std::process::Command::new("launchctl")
                        .args(["unload", "-w"])
                        .arg(&path)
                        .output();
                    if path.exists() {
                        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                    }
                    println!("launchd 등록 해제 완료 (데몬 정지됨 — 세션도 함께 종료)");
                    Ok(())
                }
                DaemonAction::Status => {
                    let path = cys::launchd::plist_path();
                    let registered = path.exists();
                    let loaded = std::process::Command::new("launchctl")
                        .args(["list", cys::launchd::LAUNCHD_LABEL])
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false);
                    let alive = connect_raw().is_ok();
                    println!(
                        "registered={} loaded={} socket_alive={}",
                        registered, loaded, alive
                    );
                    if alive && !loaded {
                        println!("(데몬은 살아있지만 launchd 소유가 아님 — 수동/앱 기동 인스턴스)");
                    }
                    Ok(())
                }
            }
        }
        #[cfg(windows)]
        {
            const TASK: &str = "cysd";
            match action {
                DaemonAction::Install { takeover: _ } => {
                    let daemon = sibling_daemon_path()
                        .ok_or("cysd.exe not found next to cys.exe")?;
                    // ★진짜 KeepAlive 패리티(mac launchd KeepAlive 대응): schtasks 명령줄 등록(ONLOGON뿐)엔
                    // RestartOnFailure 플래그가 없어 사망 시 자동 재기동이 불가했다(구: "미지원"). 태스크 XML 로
                    // RestartOnFailure(PT1M×10) + ExecutionTimeLimit PT0S(무제한·기본 72h 제한이 데몬을 죽인다) +
                    // IgnoreNew(중복 억제) + 배터리 제약 해제로 전환한다.
                    // ─ 종료코드 의미론(cysd/main.rs 소스 확인): graceful shutdown(콘솔 이벤트 → shutdown_cleanup →
                    //   std::process::exit(0))=성공→스케줄러 재기동 없음 / taskkill /F(TerminateProcess·exit≠0)
                    //   ·크래시=실패→RestartOnFailure 재기동. 즉 의도적 정지는 안 되살리고, 죽음만 되살린다.
                    // ─ 상호작용①(install_update): stop_running_daemon 이 taskkill /F(exit≠0)로 데몬을 멈추면
                    //   스케줄러가 PT1M 뒤 재기동을 시도할 수 있으나, 그 무렵 앱 ensure_daemon 이 새 cysd 를
                    //   이미 띄워 파이프를 점유했으면 스케줄러 cysd 는 first_pipe_instance(true) 가드에 막혀 즉시
                    //   종료한다(이중 데몬·자원누수 없음). 재시도는 Count 로 상한(장기 폭주 불가). 스케줄러 실패
                    //   이력만 남는 cosmetic 사안 — 파이프 단일인스턴스 가드가 정합성을 보장한다.
                    // ─ 상호작용②(phoenix deploy): _win_restart_daemon 의 taskkill 뒤 스케줄러가 먼저 재기동해도
                    //   재기동 유발(/Run)은 IgnoreNew 라 무해하고, 최종 판정은 boot-epoch delta(어느 소스가
                    //   되살렸든 새 세대면 성공)로 확증돼 정합.
                    let user = current_user_id()
                        .ok_or("현재 사용자(whoami/USERNAME) 확인 실패 — 태스크 XML principal 생성 불가")?;
                    let xml = cysd_task_xml(&daemon, &user);
                    let xml_path = std::env::temp_dir().join("cysd-task.xml");
                    write_utf16le_bom(&xml_path, &xml).map_err(|e| format!("태스크 XML 기록 실패: {e}"))?;
                    let out = std::process::Command::new("schtasks")
                        .args(["/Create", "/XML"])
                        .arg(&xml_path)
                        .args(["/TN", TASK, "/F"])
                        .output()
                        .map_err(|e| e.to_string())?;
                    let _ = std::fs::remove_file(&xml_path); // 임시 XML 정리
                    if !out.status.success() {
                        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
                    }
                    println!("작업 스케줄러 등록 완료 (로그온 시 자동 기동 + 사망 시 자동 재기동 지원 — RestartOnFailure PT1M×10·실행시간 무제한).");
                    Ok(())
                }
                DaemonAction::Uninstall => {
                    let out = std::process::Command::new("schtasks")
                        .args(["/Delete", "/TN", TASK, "/F"])
                        .output()
                        .map_err(|e| e.to_string())?;
                    if !out.status.success() {
                        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
                    }
                    println!("작업 스케줄러 등록 해제 완료");
                    Ok(())
                }
                DaemonAction::Status => {
                    let registered = std::process::Command::new("schtasks")
                        .args(["/Query", "/TN", TASK])
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false);
                    // restart 정책 표시 — /Query /XML 에 RestartOnFailure 존재 여부(KeepAlive 켜짐).
                    let restart_on_failure = registered && task_has_restart_on_failure(TASK);
                    let alive = connect_raw().is_ok();
                    println!(
                        "registered={registered} restart_on_failure={restart_on_failure} socket_alive={alive}"
                    );
                    Ok(())
                }
            }
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = action;
            Err("이 OS에서는 미지원 (macOS launchd / Windows 작업 스케줄러만)".into())
        }
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn fmt_secs(s: u64) -> String {
    if s >= 3600 {
        format!("{}h{}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m{}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

/// T1-2 관제 보드 렌더링: org.status 1콜 → 사람/AI 모두 읽는 표
/// statusline stdin JSON에서 usage.report 파라미터(surface 제외)를 추출한다 — 순수 함수(테스트 핀).
/// `context_window.used_percentage`(서버 진실 ctx%)·`context_window_size`·`current_usage` 합(ctx_tokens,
/// input+cache_creation+cache_read = Phase 1 transcript 공식과 동일)·`rate_limits.five_hour/seven_day`
/// → rate 배열. 누락 필드는 안전하게 생략(rate 부재=무료/세션 첫 응답 전이면 빈 벡터).
fn statusline_to_report_params(v: &Value) -> Value {
    let cw = v.get("context_window");
    let ctx_pct = cw
        .and_then(|c| c.get("used_percentage"))
        .and_then(|x| x.as_f64());
    let ctx_window = cw
        .and_then(|c| c.get("context_window_size"))
        .and_then(|x| x.as_u64());
    let ctx_tokens = cw
        .and_then(|c| c.get("current_usage"))
        .map(|cu| {
            let g = |k: &str| cu.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            g("input_tokens") + g("cache_creation_input_tokens") + g("cache_read_input_tokens")
        })
        .filter(|&t| t > 0)
        .or_else(|| {
            cw.and_then(|c| c.get("total_input_tokens"))
                .and_then(|x| x.as_u64())
        });
    let mut rate = Vec::new();
    if let Some(rl) = v.get("rate_limits") {
        for (key, label) in [("five_hour", "5h"), ("seven_day", "7d")] {
            if let Some(used) = rl
                .get(key)
                .and_then(|w| w.get("used_percentage"))
                .and_then(|x| x.as_f64())
            {
                let mut entry = json!({"label": label, "used_pct": used});
                if let Some(r) = rl
                    .get(key)
                    .and_then(|w| w.get("resets_at"))
                    .and_then(|x| x.as_f64())
                {
                    entry["resets_at"] = json!(r);
                }
                rate.push(entry);
            }
        }
    }
    let mut params = json!({ "rate": rate });
    if let Some(p) = ctx_pct {
        params["ctx_pct"] = json!(p);
    }
    if let Some(t) = ctx_tokens {
        params["ctx_tokens"] = json!(t);
    }
    if let Some(w) = ctx_window {
        params["ctx_window"] = json!(w);
    }
    // CC v2 WS-A: statusline stdin의 transcript_path → session_file(기존 usage.report 파라미터).
    // 데몬이 프로필 dir→계정(accountUuid) 귀속에 쓴다. 부재 시 생략(하위호환·필드 없는 구버전 무해).
    if let Some(t) = v.get("transcript_path").and_then(|x| x.as_str()) {
        params["session_file"] = json!(t);
    }
    params
}

/// statusline JSON → 사람이 읽는 한 줄 (`<model> · CTX n% · 5h n% · 7d n%`). rate는 있을 때만.
/// claude UI statusline에 그대로 표시된다(pane 헤더 배지와 별개·추가 표면).
fn statusline_human_line(v: &Value) -> String {
    let model = v
        .get("model")
        .and_then(|m| m.get("display_name"))
        .and_then(|x| x.as_str())
        .unwrap_or("claude");
    let mut parts = vec![model.to_string()];
    if let Some(p) = v
        .get("context_window")
        .and_then(|c| c.get("used_percentage"))
        .and_then(|x| x.as_f64())
    {
        parts.push(format!("CTX {p:.0}%"));
    }
    if let Some(rl) = v.get("rate_limits") {
        for (key, label) in [("five_hour", "5h"), ("seven_day", "7d")] {
            if let Some(u) = rl
                .get(key)
                .and_then(|w| w.get("used_percentage"))
                .and_then(|x| x.as_f64())
            {
                parts.push(format!("{label} {u:.0}%"));
            }
        }
    }
    parts.join(" · ")
}

/// cys-statusline.sh 래퍼 전용 — stdin의 claude statusline JSON을 읽어 usage.report로 push하고,
/// (quiet가 아니면) 사람용 statusline 한 줄을 stdout으로 출력한다.
/// ★불변: statusline 경로는 **절대 claude를 막지 않는다** — 빈 입력·파싱 실패·surface 미해결·
/// 데몬 부재 전부 exit 0으로 무해하게 흘린다.
fn run_usage_report_stdin(surface: &Option<String>, quiet: bool) -> i32 {
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() || buf.trim().is_empty() {
        return 0;
    }
    let Ok(v) = serde_json::from_str::<Value>(&buf) else {
        return 0;
    };
    // push (surface 미해결·데몬 부재는 조용히 스킵 — 사람용 줄은 여전히 출력한다)
    if let Ok(sid) = target_surface(surface, &None) {
        let mut params = statusline_to_report_params(&v);
        params["surface_id"] = json!(sid);
        let _ = request("usage.report", params);
    }
    if !quiet {
        println!("{}", statusline_human_line(&v));
    }
    0
}

/// hook stdin JSON → usage.event 파라미터(surface 제외) — 순수 함수(테스트 핀).
/// PreToolUse/PostToolUse/Stop/SubagentStop만 매핑, 그 외 hook은 None(무시).
/// PostToolUse는 tool_response.is_error로 exit_code(실패 신호)를 best-effort 추출(E3 반복실패).
fn hook_to_event_params(v: &Value) -> Option<Value> {
    let raw = v.get("hook_event_name").and_then(|x| x.as_str())?;
    let event_type = match raw {
        "PreToolUse" => "PRE_TOOL",
        "PostToolUse" => "POST_TOOL",
        "Stop" => "STOP",
        "SubagentStop" => "SUBAGENT_STOP",
        // E-b: actionable 이벤트(PermissionRequest/ExitPlanMode/AskUserQuestion)를 버리지 않고
        //   raw 그대로 event_type에 싣는다. 데몬은 raw_hook_event(아래 동봉)로 분류한다.
        "PermissionRequest" | "ExitPlanMode" | "AskUserQuestion" => raw,
        _ => return None,
    };
    // E-b: raw hook_event_name을 그대로 동봉 → 데몬 분류기가 CLI 변환명이 아닌 raw로 분류.
    //   event_type(PRE_TOOL 등)은 SQLite 적재용으로 유지(record_event 무손상).
    let mut p = json!({ "event_type": event_type, "raw_hook_event": raw });
    if let Some(t) = v.get("tool_name").and_then(|x| x.as_str()) {
        p["tool_name"] = json!(t);
    }
    if let Some(ti) = v.get("tool_input") {
        p["tool_input"] = ti.clone();
    }
    if let Some(s) = v.get("session_id").and_then(|x| x.as_str()) {
        p["session_id"] = json!(s);
    }
    if let Some(a) = v.get("agent_id").and_then(|x| x.as_str()) {
        p["agent_id"] = json!(a);
    }
    if event_type == "POST_TOOL" {
        let err = v
            .get("tool_response")
            .and_then(|r| r.get("is_error"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        p["exit_code"] = json!(if err { 1 } else { 0 });
    }
    Some(p)
}

/// cys-hook.sh 전용 — hook stdin을 읽어 usage.event로 push. ★불변: 절대 에이전트를 막지 않는다
/// (빈 입력·파싱 실패·관심 없는 hook·surface 미해결·데몬 부재 전부 exit 0).
fn run_usage_event_stdin(surface: &Option<String>) -> i32 {
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() || buf.trim().is_empty() {
        return 0;
    }
    let Ok(v) = serde_json::from_str::<Value>(&buf) else {
        return 0;
    };
    let Some(mut params) = hook_to_event_params(&v) else {
        return 0;
    };
    if let Ok(sid) = target_surface(surface, &None) {
        params["surface_id"] = json!(sid);
        let _ = request("usage.event", params);
    }
    0
}

/// 지정 소켓에 단발 RPC — fan-out 집계용(부서 소켓 순회). autostart 안 함(부서 다운=정상 정보·도달불가 표기).
///
/// ★U-6 잔여 구멍 보강(2026-08-24): 종전 이 경로는 전용 와이어 로직(`rpc_over`)의 **무상한**
/// `read_line` 을 썼다 — `RpcDeadline` 을 타지 않으므로, 부서 데몬이 accept 후 wedge 되면
/// `cys org status` 가 **영구 정지**한다(`request_on_timeout` 이 이미 문서화한 A1-F2 와 같은
/// 클래스이며, 그쪽만 고쳐 두고 이쪽을 남긴 상태였다. "RPC 무진행 상한" 단위가 닫혔다고
/// 선언하려면 같은 기구를 타야 한다).
/// 상한 계산은 `request()` 와 **같은 정책 함수**(`rpc_idle_timeout`)를 쓴다 — 부서라고 다른
/// 정책을 두면 그 순간 정책이 두 벌이 되고, 롤백 노브(`CYS_RPC_TIMEOUT_SECS=0` → `None` =
/// 상한 해제 = 개정 전 거동)도 여기서 그대로 유효하다.
fn request_on(socket: &std::path::Path, method: &str, params: Value) -> Result<Value, String> {
    #[cfg(unix)]
    let mut stream = std::os::unix::net::UnixStream::connect(socket)
        .map_err(|e| format!("connect {}: {e}", socket.display()))?;
    // busy-retry: 부서 fan-out 도 ERROR_PIPE_BUSY(231)를 다운으로 오판하지 않는다(connect_raw 대칭).
    #[cfg(windows)]
    let mut stream =
        open_pipe_busy_retry(socket).map_err(|e| format!("open {}: {e}", socket.display()))?;
    // 선언 순서 주의 — deadline 이 stream 보다 먼저 drop 돼야 한다(request() 와 동일 계약).
    let deadline = RpcDeadline::arm(&stream, rpc_idle_timeout(method, &params))?;
    let out = rpc_roundtrip(&mut stream, &deadline, method, params);
    drop(deadline);
    out
}

/// request_on의 타임아웃판 — connect 후 read/write 상한을 강제한다. drain --verify fan-out은
/// hung 소켓(데몬이 accept 후 무응답)에서 request_on의 무타임아웃 read가 영구 정지[A1-F2]하므로 필수.
///
/// ★U-6: 종전 Windows arm 은 `request_on` 위임 = **no-op** 이었다(파이프에는 상한이 아예 없었다).
/// 지금은 두 플랫폼이 같은 기구(`RpcDeadline` — unix 는 소켓 타임아웃, Windows 는 `CancelIoEx`
/// 워치독)를 쓰므로 cfg 분기 자체가 사라졌다. 상한 의미는 `request()` 와 동일한 **무진행 구간**이다.
fn request_on_timeout(
    socket: &std::path::Path,
    method: &str,
    params: Value,
    timeout: std::time::Duration,
) -> Result<Value, String> {
    // 연결 자체는 종전 경로 유지(unix=UnixStream::connect · windows=busy-retry open).
    #[cfg(unix)]
    let mut stream = std::os::unix::net::UnixStream::connect(socket)
        .map_err(|e| format!("connect {}: {e}", socket.display()))?;
    #[cfg(windows)]
    let mut stream =
        open_pipe_busy_retry(socket).map_err(|e| format!("open {}: {e}", socket.display()))?;
    // 선언 순서 주의 — deadline 이 stream 보다 먼저 drop 돼야 한다(request() 와 동일 계약).
    let deadline = RpcDeadline::arm(&stream, Some(timeout))?;
    let out = rpc_roundtrip(&mut stream, &deadline, method, params);
    drop(deadline);
    out
}

// ============================ drain --verify (기능 1) ============================
// 재시작 전 전 노드의 증류 체크포인트(SESSION_STATE)를 nonce 마커로 결정론 확인한다. 무인자 plain drain은
// best-effort 저장 신호(거동 불변)이고, --verify만 이 경로로 분기해 노드별 결과 JSON+exit code를 낸다.
// ★설계 v3: 소켓별 병렬 fan-out + connect/read 타임아웃 + 전역 하드캡(=timeout+마진), nonce 마커
// (HTML 주석형 — 체크박스/denylist 토큰 회피), 복원 중 가드, live_cwd canonical 경로(무음 폴백 금지).

/// 체크포인트 nonce 마커 한 줄 — HTML 주석형 전용. 체크박스 문법(`- [ ]`) 금지(javis_report.py 진행% 오염),
/// session-start.sh denylist 토큰 회피. 신선도 판정은 mtime이 아니라 이 nonce 존재/일치로 한다[A1-F5]
/// (마커 쓰기가 mtime 소비자에게 '실작업'으로 오인되는 드리프트 회피).
fn checkpoint_marker(nonce: &str, ts: u64) -> String {
    format!("<!-- cys-checkpoint: {nonce} {ts} -->")
}

/// 파일에 지정 nonce의 체크포인트 마커가 존재하는가(존재/일치 — mtime 아님).
/// ★[F2] 이 검증은 '마커 기입'만 확인한다 — 노드가 마커 앞에서 SESSION_STATE 내용을 실제로 최신화했는지는
/// 보증하지 못한다(형식적 순응 한계). Saved 결과·UI 라벨은 그래서 "저장 확인"이 아니라 "마커 확인"으로
/// 표기한다. 내용 최신성은 노드(LLM) 협조 책임이며, 이 도구가 낼 수 있는 결정론 신호는 마커 존재까지다.
fn file_has_checkpoint_nonce(path: &std::path::Path, nonce: &str) -> bool {
    let needle = format!("cys-checkpoint: {nonce}");
    std::fs::read_to_string(path)
        .map(|s| s.contains(&needle))
        .unwrap_or(false)
}

/// 노드 canonical 체크포인트 파일 = <live_cwd>/_round/SESSION_STATE.md (단일 복원 진실).
fn canonical_checkpoint_file(live_cwd: &str) -> std::path::PathBuf {
    std::path::Path::new(live_cwd)
        .join("_round")
        .join("SESSION_STATE.md")
}

/// [F1] 소켓 경로별 안정 구별자(FNV-1a) — nonce에 섞어 크로스소켓(같은 sid) 충돌을 막는다. 결정론(런타임
/// 무관)이라 같은 소켓은 항상 같은 값·다른 소켓은 사실상 다른 값(충돌 확률 무시 가능). 단일 run 내
/// 유일성만 필요하므로 짧은 64bit로 충분.
fn socket_discriminator(socket: &std::path::Path) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in socket.to_string_lossy().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// 화면에서 '현재 입력창 영역'만 잘라낸다 — 마지막 입력 앵커(입력 박스 상단 '╭' 또는 줄 시작 '> ' 프롬프트)
/// 이후 끝까지. 제출된 텍스트는 이 영역 **위**(스크롤백)에 렌더되고, 미제출 입력은 이 영역 **안**에 잔류한다.
/// 앵커가 없으면(구/미지 TUI) 전체 화면을 반환한다(보수적 — 놓친 wedge=저장 유실이 과검출보다 위험).
fn input_region(screen: &str) -> &str {
    let box_top = screen.rfind('╭');
    let prompt = if screen.starts_with("> ") {
        Some(0)
    } else {
        screen.rfind("\n> ").map(|i| i + 1)
    };
    match box_top.into_iter().chain(prompt).max() {
        Some(i) => &screen[i..],
        None => screen,
    }
}

/// 전달확정 게이트 판정 — 제출되면 주입 텍스트가 위로 스크롤되고 하단 입력창엔 스피너/빈 프롬프트만 남는다.
/// Return 미발화(known bug)면 주입 텍스트가 입력창에 잔류하므로 sentinel이 입력창에 남는다.
/// ★[F1] 실터미널은 긴 지시문을 물리적으로 줄바꿈(래핑)하고 입력창 하단에 테두리·단축키·토큰카운터 UI가
///   따라붙어 sentinel이 최하단에서 밀리거나 물리 행 경계에서 쪼개진다 — 그래도 검출하려 공백·개행을
///   제거하고 매치한다(구 tail-4행 스캔은 놓쳐 Return 재전송 미발화·저장 유실).
/// ★[R2·R3 수리] 단 '화면 어디든'이 아니라 **입력창 영역(input_region)** 안에서만 매치한다 — 제출된
///   스크롤백 에코(nonce 포함)를 wedge로 오검출하면 ①승인 프롬프트 대기 노드에 잉여 Return을 쏘아 의도외
///   확정 위험(R2), ②delivery_failed↔timeout 라벨 변별 소실(R3). 미제출 입력은 입력창 영역에·제출 에코는
///   그 위(스크롤백)에 있으므로 영역 한정 매치가 둘을 가른다. 앵커 부재 TUI는 전체 매치로 폴백(F1 보존).
fn delivery_wedged(screen: &str, sentinel: &str) -> bool {
    let region = input_region(screen);
    let flat: String = region.chars().filter(|c| !c.is_whitespace()).collect();
    let needle: String = sentinel.chars().filter(|c| !c.is_whitespace()).collect();
    !needle.is_empty() && flat.contains(&needle)
}

/// 저장 지시문 생성 — ★[R1] 마커 기입을 '정지' 지시보다 **앞**(단계 ①)에 둔다. 지시문을 순서대로
/// 리터럴 실행하는 노드가 '정지'를 먼저 만나면 이후 마커 기입을 건너뛰어 '저장했으나 timeout' 오판정이
/// 났다(직전 F1 수리가 마커를 끝으로 옮기며 역전됨). F1의 wedge 검출은 위치 무관 전체 매치라 마커가
/// 지시문 끝일 필요가 없으므로, 마커를 ①로 되돌려도 F1은 유지된다.
/// ★[F4 위생] 기존 `<!-- cys-checkpoint:` 마커 라인은 지우고 새 1줄만 남기게 지시한다 — append-only면
/// 재시작마다 죽은 마커가 무한 증식한다. 검증 로직은 무변경(새 nonce 존재 확인이라 옛 마커 잔존과 무관).
fn drain_verify_instruction(marker: &str) -> String {
    format!(
        "[DRAIN-VERIFY] 재시작 전 체크포인트 검증. 지금 즉시 순서대로: ① _round/SESSION_STATE.md에 현재 작업 상태·미해결 게이트·다음 액션을 최신화해 저장하고, 그 파일에 이미 있는 `<!-- cys-checkpoint:` 로 시작하는 옛 마커 라인은 모두 삭제한 뒤, 맨 끝에 정확히 이 한 줄만 추가하라(문자 그대로·수정 금지): {marker} ② ①의 저장·기입을 모두 마친 뒤에 작업을 멈추고 재시작·복원을 기다려라(승인 프롬프트 대기 중이면 이 메시지는 무시하라)."
    )
}

/// phoenix 복원이 이 소켓의 이 역할에 대해 진행 중인가 — 진행 중이면 Some(사유), 아니면 None.
/// 저널 = <소켓 부모 디렉토리(realpath)>/phoenix/journal-*.json
/// (phoenix.py state_dir_for=realpath(dirname(socket))[:389] 정합 — ★[F4] canonicalize 적용, 실패 시 원경로).
/// 판정: 신선한(mtime 최근) 저널에서 대상 역할 stage가 기록됐고 g2_ack 미완료 → 복원 중(해당 role skip).
/// ★[F2 수리] fail-CLOSED: 신선한 저널이 존재하는데 판독/파싱 실패, 또는 기대 스키마(roles 객체·해당
///   role의 stages 객체) 부재면 '복원 중'으로 취급한다 — pack과 바이너리 릴리스 라인이 달라 저널 스키마
///   스큐가 실재 위험이고, 각성 파손(디렉티브 재주입 × "작업 중단" 주입 교차)은 비가역이라 안전 방향=주입
///   보류. 단 저널이 대상 role을 언급하지 않으면 이 role의 복원이 아니므로 다른 저널을 계속 본다(무관
///   role over-skip 금지). 저널 파일 자체가 없으면(디렉토리 부재·비-저널) None(복원 아님 — 무해).
/// ★[F3] EPOCH_GATE divergence: phoenix stage_done(javis_phoenix.py:1283-1296)은 완료 마킹의 epoch가
///   현재 세대와 일치할 때만 done으로 인정한다(재부팅 넘긴 stale done 무효화). 이 가드는 phoenix 런타임
///   epoch(_ACTIVE_EPOCH)를 알 수 없어 epoch 대조 없이 done 표기만 본다. 방향 차이: ①started&&!g2_ack
///   분기는 over-skip(안전) ②g2_ack done 분기의 stale-epoch under-skip 위험은 mtime 신선도 창(≤RECENCY)이
///   흡수한다 — stale-epoch done은 이전 세대(재부팅 전) 저널이라 대개 창 밖(무시)이므로 실질 발산 없음.
fn restore_guard_reason(socket: &std::path::Path, role: &str) -> Option<String> {
    const RESTORE_RECENCY_SECS: u64 = 300;
    let dir = socket.parent()?;
    // [F4] phoenix state_dir_for와 정렬 — 심링크 소켓 디렉토리 대응(실패 시 원경로 폴백).
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let phoenix = dir.join("phoenix");
    let entries = std::fs::read_dir(&phoenix).ok()?; // 디렉토리 부재 = 저널 없음 = 복원 아님(None)
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("journal-") && name.ends_with(".json")) {
            continue;
        }
        let fresh = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .map(|d| d.as_secs() <= RESTORE_RECENCY_SECS)
            .unwrap_or(false);
        if !fresh {
            continue; // stale 저널(오래된 실패 복원)은 영구 차단 방지 위해 무시
        }
        // [F2] 신선한 저널 — 여기서부터 판독/파싱/스키마 실패는 fail-CLOSED(복원 중 취급).
        // ★[R4 INFO] 판독·파싱 불능인 신선 저널은 그것이 '이 role'의 복원인지조차 알 수 없다 — 그래도
        //   보수적으로 skip한다(무관 role까지 over-skip 가능). 근거: over-skip의 대가는 verify 1회 보류(그
        //   노드는 이번 재시작 창에서 저장 미검증으로 표기)뿐이나, under-skip은 복원 중 노드에 "작업 중단"을
        //   주입해 각성을 비가역 파손시킨다 — 비대칭 위험이라 안전 방향(skip)을 택한다. 코드 변경 불요.
        let Ok(txt) = std::fs::read_to_string(e.path()) else {
            return Some(format!("신선한 저널 판독 실패({name}) — fail-closed(복원 중 취급)"));
        };
        let Ok(j) = serde_json::from_str::<Value>(&txt) else {
            return Some(format!(
                "신선한 저널 파손(JSON 파싱 실패, {name}) — fail-closed(복원 중 취급)"
            ));
        };
        if !j["roles"].is_object() {
            return Some(format!(
                "신선한 저널 스키마 이상(roles 비객체, {name}) — fail-closed(복원 중 취급)"
            ));
        }
        let role_entry = &j["roles"][role];
        if role_entry.is_null() {
            continue; // 이 저널은 대상 role 무관 — over-skip 금지, 다음 저널 확인
        }
        let stages = &role_entry["stages"];
        if !stages.is_object() {
            return Some(format!(
                "신선한 저널 role '{role}' stages 스키마 이상({name}) — fail-closed(복원 중 취급)"
            ));
        }
        let done = |s: &str| stages[s]["done"].as_bool() == Some(true);
        let started = ["spawn", "ready", "resume", "reinject"]
            .iter()
            .any(|s| done(s));
        if started && !done("g2_ack") {
            return Some(format!("phoenix 복원 진행 중(stage<g2_ack, {name})"));
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VerifyOutcome {
    Saved,
    Timeout,
    DeliveryFailed,
    Unverifiable,
    SkippedRestoring,
}
impl VerifyOutcome {
    fn as_str(&self) -> &'static str {
        match self {
            VerifyOutcome::Saved => "saved",
            VerifyOutcome::Timeout => "timeout",
            VerifyOutcome::DeliveryFailed => "delivery_failed",
            VerifyOutcome::Unverifiable => "unverifiable",
            VerifyOutcome::SkippedRestoring => "skipped_restoring",
        }
    }
}

/// verify 대상 1노드 — 소켓별 org.status에서 추출(surface_id는 데몬별 네임스페이스라 socket과 쌍으로 보유).
#[derive(Clone)]
struct VerifyTarget {
    socket: std::path::PathBuf,
    dept: String,
    display: String,
    surface_id: u64,
    surface_ref: String,
    role: String,
    live_cwd: Option<String>,
    pending_undelivered: u64,
}

/// drain --verify 소켓 I/O 추상화 — 프로덕션은 request_on_timeout, 테스트는 노드 상태(saved/no-save/
/// wedge/hung)를 모사하는 fake로 주입한다(producer≠evaluator 검증 용이).
trait VerifyIo {
    fn inject(
        &self,
        socket: &std::path::Path,
        sid: u64,
        text: &str,
        timeout: std::time::Duration,
    ) -> Result<(), String>;
    fn read_screen(
        &self,
        socket: &std::path::Path,
        sid: u64,
        lines: u64,
        timeout: std::time::Duration,
    ) -> Result<String, String>;
    fn send_return(
        &self,
        socket: &std::path::Path,
        sid: u64,
        timeout: std::time::Duration,
    ) -> Result<(), String>;
}

/// 소켓 지정 주입(inject_text의 socket+timeout판): bracketed paste → 0.8s → Return. 기본 소켓 하드바인딩인
/// inject_text와 달리 부서 소켓 대상[A1-F1] · request_on_timeout으로 hung 방어.
fn inject_text_on(
    socket: &std::path::Path,
    sid: u64,
    text: &str,
    timeout: std::time::Duration,
) -> Result<(), String> {
    // ★U-14 관문 가드 — 부서 소켓 판. `inject_text` 와 **같은 술어**를 쓰되 관측만 소켓 경유다.
    //   실사용상 이 경로의 대상은 이미 각성한 노드라 창은 대개 닫혀 있지만, 그렇다고 그물에
    //   구멍을 남기면 그 구멍이 다음 사고의 자리가 된다(이 저장소에서 살아남는 결함은 전부
    //   이음매에 있다). 관측 실패는 종전대로 전송(fail-open) — 아래 헬퍼의 doc 참조.
    gate_guard_check_on(socket, sid, timeout, "디렉티브 주입(부서)")?;
    let wrapped = format!("\x1b[200~{text}\x1b[201~");
    request_on_timeout(
        socket,
        "surface.send_text",
        json!({"surface_id": sid, "text": wrapped, "quiet": true, "authoritative": true}),
        timeout,
    )?;
    std::thread::sleep(std::time::Duration::from_millis(800));
    gate_guard_check_on(socket, sid, timeout, "제출 Return(부서)")?;
    request_on_timeout(
        socket,
        "surface.send_key",
        json!({"surface_id": sid, "key": "Return", "authoritative": true}),
        timeout,
    )?;
    Ok(())
}

/// `gate_guard_check` 의 **부서 소켓 판**(관측 경로만 다르고 판정 술어는 동일하다 — 사본 0).
fn gate_guard_check_on(
    socket: &std::path::Path,
    sid: u64,
    timeout: std::time::Duration,
    stage: &str,
) -> Result<(), String> {
    // ★한 왕복의 스냅샷에서 창·어댑터를 함께 읽는다(기본 소켓 판과 동형 — 사본 0).
    let rows: Vec<Value> = request_on_timeout(socket, "surface.list", json!({}), timeout)
        .ok()
        .and_then(|r| r["surfaces"].as_array().cloned())
        .unwrap_or_default();
    let awakened = surface_awakened_in(&rows, sid);
    if awakened != Some(false) {
        return Ok(());
    }
    // ★(P4-6) 관측 실패는 fail-open 이되 **fail-silent 는 아니다**(기본 소켓 판과 같은 규율).
    let screen = match request_on_timeout(
        socket,
        "surface.read_text",
        json!({"surface_id": sid}),
        timeout,
    )
    .ok()
    .and_then(|r| r["text"].as_str().map(|s| s.to_string()))
    {
        Some(text) => text,
        None => {
            eprintln!(
                "[inject-guard] ⚠ 화면 관측 실패(부서 소켓 {} · surface.read_text 무응답 또는 \
                 응답 스키마 스큐) — 관문 판정을 **건너뛰고** 종전대로 {stage} 를 보낸다\
                 (fail-open) surface={sid}. ★'관문 없음' 이 아니라 **가드가 눈을 감은 것**이다",
                socket.display()
            );
            String::new()
        }
    };
    // ★(P4-4) 코퍼스 단일 소스 — 기본 소켓 판과 **같은 함수**를 지난다.
    let gates = gate_corpus_for_seat(surface_agent_in(&rows, sid).as_deref());
    match cys::inject_guard::decide(&cys::inject_guard::Observed {
        screen: &screen,
        gates: &gates,
        awakened,
        guard_off: cys::inject_guard::guard_off(),
    }) {
        cys::inject_guard::Decision::Send => Ok(()),
        cys::inject_guard::Decision::SendObserved(hit) => {
            eprintln!(
                "[inject-guard] ⚠ 가드 강등({}=0 또는 마스터 {}=0) — 관문({})을 관측했으나 \
                 종전대로 {stage} 를 보낸다 surface={sid}",
                cys::inject_guard::ENV_GUARD_OFF,
                cys::ENV_BOOT_GATES,
                hit.id
            );
            Ok(())
        }
        cys::inject_guard::Decision::Hold(hit) => Err(gate_hold_message(sid, &hit, stage)),
    }
}

struct RealVerifyIo;
impl VerifyIo for RealVerifyIo {
    fn inject(
        &self,
        socket: &std::path::Path,
        sid: u64,
        text: &str,
        timeout: std::time::Duration,
    ) -> Result<(), String> {
        inject_text_on(socket, sid, text, timeout)
    }
    fn read_screen(
        &self,
        socket: &std::path::Path,
        sid: u64,
        lines: u64,
        timeout: std::time::Duration,
    ) -> Result<String, String> {
        request_on_timeout(
            socket,
            "surface.read_text",
            json!({"surface_id": sid, "lines": lines}),
            timeout,
        )
        .map(|r| r["text"].as_str().unwrap_or("").to_string())
    }
    fn send_return(
        &self,
        socket: &std::path::Path,
        sid: u64,
        timeout: std::time::Duration,
    ) -> Result<(), String> {
        request_on_timeout(
            socket,
            "surface.send_key",
            json!({"surface_id": sid, "key": "Return", "authoritative": true}),
            timeout,
        )
        .map(|_| ())
    }
}

/// 1노드 검증: 복원 가드 → canonical 경로 → 저장 지시 주입 → 전달확정 게이트 → nonce 파일 폴링.
/// 반환=(결과, 상세). 소켓 hung은 timeout(파일 폴링은 로컬 FS라 소켓 무관)·미제출 wedge는 delivery_failed로 구분.
fn verify_one_node(
    io: &dyn VerifyIo,
    t: &VerifyTarget,
    nonce_prefix: &str,
    timeout: std::time::Duration,
    now: u64,
) -> (VerifyOutcome, String) {
    use std::time::{Duration, Instant};
    // 1) 복원 중 가드 — 각성 파손 방지(디렉티브 재주입과 "작업 중단"의 교차 차단). 사유를 JSON에 전달.
    if let Some(reason) = restore_guard_reason(&t.socket, &t.role) {
        return (VerifyOutcome::SkippedRestoring, reason);
    }
    // 2) canonical 경로 — live_cwd 미제공(구버전 부서 데몬)이면 검증불가(무음 cwd 폴백 금지)[A1-F6]
    let Some(cwd) = t.live_cwd.as_deref() else {
        return (
            VerifyOutcome::Unverifiable,
            "live_cwd 미제공(구버전 데몬) — 검증불가(무음 폴백 금지)".into(),
        );
    };
    let file = canonical_checkpoint_file(cwd);
    // ★[F1 수리] 크로스소켓 nonce 충돌 방지: surface_id는 데몬별 네임스페이스라 서로 다른 소켓의 두 노드가
    // 같은 sid + 같은 live_cwd(전 부서 cwd 수렴 실측 있음)면 nonce·파일이 동일해져, 주입 안 한 노드가 타
    // 노드 마커를 자기 것으로 오인해 Saved 위양성(→재시작 체크포인트 유실)이 났다. results가 idx 키잉으로
    // sid 충돌을 이미 회피했는데 nonce만 sid에 남아 불일치였다. 소켓 안정 구별자를 nonce에 추가해 정합화.
    let nonce = format!(
        "{nonce_prefix}-{:x}-{}",
        socket_discriminator(&t.socket),
        t.surface_id
    );
    let marker = checkpoint_marker(&nonce, now);
    // 이미 이 nonce가 있으면 즉시 통과(프로세스 내 재호출 idempotent)
    if file_has_checkpoint_nonce(&file, &nonce) {
        return (
            VerifyOutcome::Saved,
            format!("nonce 마커 확인: {}", file.display()),
        );
    }
    // 3) 저장 지시 주입 — nonce 마커 기입 + 작업 중단.
    let instr = drain_verify_instruction(&marker);
    let io_to = std::cmp::min(timeout, Duration::from_secs(8));
    if let Err(e) = io.inject(&t.socket, t.surface_id, &instr, io_to) {
        // ★(U-14) 관문 보류는 소켓 hung 이 **아니다** — 소켓은 멀쩡했고 우리가 스스로 안 보냈다.
        //   둘을 같은 `timeout` 으로 접으면 사람이 소켓·데몬을 뒤지게 된다(진단이 거짓 방향을
        //   가리킨다). 안전 방향은 같다: 어느 쪽이든 Saved 가 아니므로 `all_saved` 가 거짓이 되고
        //   재시작 게이트는 그대로 막힌다 — '측정 불능은 통과가 아니다' 규약 그대로다.
        if cys::inject_guard::is_hold_error(&e) {
            return (
                VerifyOutcome::Unverifiable,
                format!("관문 보류 — 저장 지시 미주입(좌석 보존 · Return 0발): {e}"),
            );
        }
        // 소켓 hung(RPC 타임아웃) — delivery_failed(노드 wedge)와 구분해 timeout으로 분류
        return (
            VerifyOutcome::Timeout,
            "소켓 hung — 저장 지시 RPC 타임아웃(전역 캡 내)".into(),
        );
    }
    // 4) 전달확정 게이트 — 빈 프롬프트+스피너 확인, wedge(하단 입력창 잔류)면 Return 재전송
    std::thread::sleep(Duration::from_millis(600));
    // ★[F1] 24행 읽기 — 래핑된 지시문 전체 + 하단 입력창 UI 행을 포괄(구 6행은 래핑 시 sentinel 유실).
    let mut wedged = io
        .read_screen(&t.socket, t.surface_id, 24, io_to)
        .map(|s| delivery_wedged(&s, &nonce))
        .unwrap_or(false);
    if wedged {
        let _ = io.send_return(&t.socket, t.surface_id, io_to);
        std::thread::sleep(Duration::from_millis(800));
        wedged = io
            .read_screen(&t.socket, t.surface_id, 24, io_to)
            .map(|s| delivery_wedged(&s, &nonce))
            .unwrap_or(wedged);
    }
    // 5) nonce 파일 폴링(로컬 FS — 소켓 hung과 무관하게 저장 검출)
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if file_has_checkpoint_nonce(&file, &nonce) {
            return (
                VerifyOutcome::Saved,
                format!("nonce 마커 확인: {}", file.display()),
            );
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    if wedged {
        (
            VerifyOutcome::DeliveryFailed,
            "입력 미제출(wedge) — Return 재전송에도 저장 미검출".into(),
        )
    } else {
        (
            VerifyOutcome::Timeout,
            format!(
                "{}s 내 nonce 마커 미검출: {}",
                timeout.as_secs(),
                file.display()
            ),
        )
    }
}

/// 소켓별 병렬 fan-out — 총 소요 ≈ 1×timeout(직렬 누적 아님). 노드별 detached 스레드로 verify를 스폰하고
/// 전역 하드캡(=timeout+마진) 내 결과를 수집한다. 미도착 노드는 timeout으로 분류(캡 초과=hung 방어).
fn drain_verify_fanout(
    io: std::sync::Arc<dyn VerifyIo + Send + Sync>,
    targets: Vec<VerifyTarget>,
    timeout: std::time::Duration,
    now: u64,
) -> Value {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};
    let global_cap = timeout + Duration::from_secs(5);
    let nonce_prefix = format!("{now}-{}", std::process::id());
    let (tx, rx) = std::sync::mpsc::channel::<(usize, VerifyOutcome, String)>();
    let total = targets.len();
    // surface_id는 데몬별 네임스페이스라 소켓 간 충돌 가능 → 인덱스로 키잉(nonce도 [F1] 소켓 구별자로 정합).
    let mut meta: HashMap<usize, VerifyTarget> = HashMap::new();
    for (idx, t) in targets.iter().enumerate() {
        meta.insert(idx, t.clone());
    }
    for (idx, t) in targets.into_iter().enumerate() {
        let tx = tx.clone();
        let io = io.clone();
        let np = nonce_prefix.clone();
        std::thread::spawn(move || {
            let (o, d) = verify_one_node(io.as_ref(), &t, &np, timeout, now);
            let _ = tx.send((idx, o, d));
        });
    }
    drop(tx);
    let deadline = Instant::now() + global_cap;
    let mut results: HashMap<usize, (VerifyOutcome, String)> = HashMap::new();
    while results.len() < total {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok((idx, o, d)) => {
                results.insert(idx, (o, d));
            }
            Err(_) => break,
        }
    }
    // 결과 JSON(원래 순서 안정) — 미도착 노드는 전역 캡 초과 timeout.
    let mut nodes: Vec<Value> = Vec::new();
    let mut pending_loss: Vec<Value> = Vec::new();
    let (mut c_saved, mut c_timeout, mut c_deliv, mut c_unver, mut c_skip) = (0, 0, 0, 0, 0);
    let mut all_saved = true;
    for idx in 0..total {
        let t = &meta[&idx];
        let (outcome, detail) = results.get(&idx).cloned().unwrap_or((
            VerifyOutcome::Timeout,
            "전역 하드캡 초과 — 결과 미도착(timeout)".into(),
        ));
        match outcome {
            VerifyOutcome::Saved => c_saved += 1,
            VerifyOutcome::Timeout => c_timeout += 1,
            VerifyOutcome::DeliveryFailed => c_deliv += 1,
            VerifyOutcome::Unverifiable => c_unver += 1,
            VerifyOutcome::SkippedRestoring => c_skip += 1,
        }
        if outcome != VerifyOutcome::Saved {
            all_saved = false;
        }
        if t.pending_undelivered > 0 {
            pending_loss.push(json!({
                "role": t.role, "surface": t.surface_ref, "dept": t.dept,
                "pending_undelivered": t.pending_undelivered,
            }));
        }
        nodes.push(json!({
            "dept": t.dept,
            "department": t.display,
            "role": t.role,
            "surface": t.surface_ref,
            "socket": t.socket.to_string_lossy(),
            "live_cwd": t.live_cwd,
            "checkpoint_file": t.live_cwd.as_deref().map(|c| canonical_checkpoint_file(c).to_string_lossy().into_owned()),
            "outcome": outcome.as_str(),
            "detail": detail,
            "pending_undelivered": t.pending_undelivered,
        }));
    }
    json!({
        "mode": "drain-verify",
        "timeout_secs": timeout.as_secs(),
        "total": total,
        "nodes": nodes,
        "summary": {
            "saved": c_saved, "timeout": c_timeout, "delivery_failed": c_deliv,
            "unverifiable": c_unver, "skipped_restoring": c_skip,
        },
        // ★재시작 창 큐 보존[A3-F3]: 인메모리 pending_queue는 데몬 재시작에 소실된다(handlers.rs). 디스크
        // flush는 데몬 RPC가 필요해(handlers 무접촉 목표) 대신 유실 예정분을 정직하게 가시화한다(무음 유실 금지).
        "pending_loss_warning": pending_loss,
        "all_saved": all_saved,
    })
}

/// depts.json + 본부 소켓을 순회해 verify 대상(살아있는 AI 역할 노드)을 수집한다(run_fleet 소스 동형).
/// 도달불가(다운) 부서·본부는 스킵(정상 정보). live_cwd는 org.status의 노드별 cd 추적값을 그대로 쓴다.
fn drain_verify_targets() -> Vec<VerifyTarget> {
    let home = cys::home_dir().to_string_lossy().into_owned();
    let mut sockets: Vec<(std::path::PathBuf, String, String)> =
        vec![(socket_path(), "main".to_string(), "본부 · CEO".to_string())];
    let reg = std::env::var("CYS_DEPTS_JSON")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(&home).join(".cys/depts.json"));
    if let Ok(s) = std::fs::read_to_string(&reg) {
        if let Ok(v) = serde_json::from_str::<Value>(&s) {
            if let Some(depts) = v["depts"].as_object() {
                for (name, meta) in depts {
                    let sock = meta["socket"]
                        .as_str()
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| cys::dept_socket_path(name));
                    let disp = meta["display_name"].as_str().unwrap_or(name).to_string();
                    sockets.push((sock, name.clone(), disp));
                }
            }
        }
    }
    let mut targets = Vec::new();
    for (sock, dept, disp) in sockets {
        let r = match request_on_timeout(
            &sock,
            "org.status",
            json!({}),
            std::time::Duration::from_secs(4),
        ) {
            Ok(r) => r,
            Err(_) => continue, // 다운·전이 중 소켓 스킵(무해)
        };
        for s in r["surfaces"].as_array().cloned().unwrap_or_default() {
            if s["exited"].as_bool() == Some(true) {
                continue;
            }
            let Some(role) = s["role"].as_str() else {
                continue;
            };
            if s["agent"].is_null() {
                continue; // AI 노드만(agent 메타 존재)
            }
            let Some(sid) = s["surface_id"].as_u64() else {
                continue;
            };
            targets.push(VerifyTarget {
                socket: sock.clone(),
                dept: dept.clone(),
                display: disp.clone(),
                surface_id: sid,
                surface_ref: s["surface_ref"].as_str().unwrap_or("").to_string(),
                role: role.to_string(),
                live_cwd: s["live_cwd"].as_str().map(String::from),
                pending_undelivered: s["queue_depth"].as_u64().unwrap_or(0),
            });
        }
    }
    targets
}

/// `cys drain --verify` 진입점 — 결정론 JSON을 stdout에, exit code로 전원 저장 여부를 반환한다
/// (전원 saved=0, 아니면 1). 0-노드는 우아한 no-op(exit 0)[A3-F5].
fn run_drain_verify(timeout: u64) -> i32 {
    // 백스톱 하드 워치독 — 메인 로직이 어떤 이유로든 멈춰도 프로세스가 영구 정지하지 않게(plain drain 12s 패턴).
    // fan-out은 timeout+5s 안에 반환하므로 정상 경로에선 절대 발화하지 않는다.
    let cap = timeout + 10;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(cap));
        std::process::exit(3);
    });
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let targets = drain_verify_targets();
    let io: std::sync::Arc<dyn VerifyIo + Send + Sync> = std::sync::Arc::new(RealVerifyIo);
    let report = drain_verify_fanout(io, targets, std::time::Duration::from_secs(timeout), now);
    let all_saved = report["all_saved"].as_bool() == Some(true);
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if all_saved {
        0
    } else {
        1
    }
}

/// Tasks Control Center(CLI) — depts.json을 읽어 본부+각 부서 소켓에 org.status를 순회 집계한다.
/// master 능동 모니터링: 모든 부서의 모든 노드가 지금 하는 업무를 1콜로 본다. 도달불가 부서는 표기.
fn run_fleet(as_json: bool) -> i32 {
    // RC-7: HOME 미설정(Windows) 함정 회피 — dirs 기반 공용 해소.
    let home = cys::home_dir().to_string_lossy().into_owned();
    // v2 부서 한정 키(DESIGN-dept-qualified-keys-v2 §4a): 항목마다 dept(slug=레지스트리 키)·
    // socket(경로 문자열) additive. 본부는 고정 slug "main"·socket=null(기본 소켓 사용).
    let mut targets: Vec<(std::path::PathBuf, String, String, Value)> =
        vec![(socket_path(), "본부 · CEO".to_string(), "main".to_string(), Value::Null)];
    let reg = std::env::var("CYS_DEPTS_JSON")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(&home).join(".cys/depts.json"));
    if let Ok(s) = std::fs::read_to_string(&reg) {
        if let Ok(v) = serde_json::from_str::<Value>(&s) {
            if let Some(depts) = v["depts"].as_object() {
                for (name, meta) in depts {
                    // RC-4: socket 필드 부재 시 공용 규약으로 폴백(Windows named pipe·unix .sock).
                    let sock = meta["socket"]
                        .as_str()
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| cys::dept_socket_path(name));
                    let disp = meta["display_name"].as_str().unwrap_or(name).to_string();
                    // 방출 socket = 실제 도달 소켓과 동일 경로 문자열(브리지가 cys --socket 로 재사용).
                    let sock_str = sock.to_string_lossy().into_owned();
                    targets.push((sock, disp, name.clone(), Value::String(sock_str)));
                }
            }
        }
    }
    let mut out: Vec<Value> = Vec::new();
    for (sock, disp, dept, emit_sock) in &targets {
        match request_on(sock, "org.status", json!({})) {
            Ok(r) => out.push(json!({"department": disp, "dept": dept, "socket": emit_sock,
                                     "surfaces": r["surfaces"].clone()})),
            Err(e) => out.push(json!({"department": disp, "dept": dept, "socket": emit_sock,
                                      "error": e, "surfaces": []})),
        }
    }
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "departments": out })).unwrap()
        );
        return 0;
    }
    for d in &out {
        let disp = d["department"].as_str().unwrap_or("");
        if let Some(e) = d["error"].as_str() {
            println!("\n■ {disp}  ⚠ 도달불가: {e}");
            continue;
        }
        let surfaces = d["surfaces"].as_array().cloned().unwrap_or_default();
        let working = surfaces
            .iter()
            .filter(|s| s["status"]["state"].as_str() == Some("working"))
            .count();
        println!("\n■ {disp}  (노드 {} · 작업중 {working})", surfaces.len());
        for s in surfaces {
            let role = s["role"].as_str().unwrap_or("-");
            let state = if s["exited"].as_bool() == Some(true) {
                "오프라인"
            } else {
                s["status"]["state"].as_str().unwrap_or("·파생")
            };
            let ctx = s["status"]["context_pct"]
                .as_u64()
                .map(|v| format!("{v}%"))
                .unwrap_or_else(|| "-".into());
            let task = s["status"]["task"]
                .as_str()
                .filter(|t| !t.is_empty())
                .or_else(|| s["title"].as_str())
                .unwrap_or("(업무 미보고)");
            println!(
                "   {:<14} {:<9} {:>4}  {}",
                role,
                state,
                ctx,
                task.chars().take(60).collect::<String>()
            );
        }
    }
    0
}

fn run_status(as_json: bool) -> i32 {
    let r = match request("org.status", json!({})) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if as_json {
        println!("{}", serde_json::to_string_pretty(&r).unwrap());
        return 0;
    }
    if r["paused"].as_bool() == Some(true) {
        println!(
            "⛔ PAUSED — {} (cys resume로 해제; 큐·스케줄 동결 중, 실행 중 에이전트 행동은 계속)",
            r["pause_info"]["reason"].as_str().unwrap_or("")
        );
    }
    let header = format!(
        "{:<14} {:<12} {:<8} {:<9} {:>4} {:>7} {:>5}  {}",
        "ROLE", "SURFACE", "AGENT", "STATE", "CTX", "IDLE", "QUEUE", "TASK/TITLE"
    );
    println!("{header}");
    for s in r["surfaces"].as_array().cloned().unwrap_or_default() {
        let exited = s["exited"].as_bool().unwrap_or(false);
        let state = if exited {
            "exited!".to_string()
        } else if s["agent_alive"].as_bool() == Some(false) {
            "agent✗".to_string()
        } else {
            s["status"]["state"].as_str().unwrap_or("-").to_string()
        };
        let ctx = s["status"]["context_pct"]
            .as_u64()
            .map(|v| format!("{v}%"))
            .unwrap_or_else(|| "-".into());
        let task = s["status"]["task"]
            .as_str()
            .filter(|t| !t.is_empty())
            .or(s["title"].as_str())
            .unwrap_or("");
        let queue_mark = if s["queue_paused"].as_bool() == Some(true) {
            format!("{}⏸", s["queue_depth"].as_u64().unwrap_or(0))
        } else {
            s["queue_depth"].as_u64().unwrap_or(0).to_string()
        };
        println!(
            "{:<14} {:<12} {:<8} {:<9} {:>4} {:>7} {:>5}  {}",
            s["role"].as_str().unwrap_or("-"),
            s["surface_ref"].as_str().unwrap_or("?"),
            s["agent"].as_str().unwrap_or("-"),
            state,
            ctx,
            fmt_secs(s["idle_secs"].as_u64().unwrap_or(0)),
            queue_mark,
            task.chars().take(40).collect::<String>(),
        );
    }
    let pending = r["feed"]["pending"].as_u64().unwrap_or(0);
    if pending > 0 {
        println!(
            "feed: {pending} pending (oldest {}) — `cys feed list --status pending`",
            fmt_secs(r["feed"]["oldest_pending_age_secs"].as_u64().unwrap_or(0))
        );
    }
    let health = r["health_recent"].as_array().cloned().unwrap_or_default();
    if !health.is_empty() {
        println!("health (최근 {}건):", health.len().min(5));
        for h in health.iter().take(5) {
            println!(
                "  surface:{} [{}] {}",
                h["surface_id"],
                h["rule"].as_str().unwrap_or("?"),
                h["line"].as_str().unwrap_or("").chars().take(80).collect::<String>(),
            );
        }
    }
    if let Some(todo) = r["todo"].as_object() {
        if !todo.is_empty() {
            println!("todo:");
            for (path, v) in todo {
                let name = path.rsplit('/').next().unwrap_or(path);
                println!(
                    "  {name}: {}/{} (updated {} ago)",
                    v["done"],
                    v["total"],
                    fmt_secs(v["age_secs"].as_u64().unwrap_or(0))
                );
            }
        }
    }
    0
}

/// role 우선, 없으면 --surface, 없으면 env 폴백으로 대상 결정 (cycle/recover/reinject 공용)
fn resolve_role_or_surface(
    role: &Option<String>,
    surface: &Option<String>,
) -> Result<u64, String> {
    if role.is_some() {
        return target_surface(&None, role);
    }
    let explicit = parse_explicit_surface(surface)?;
    match explicit {
        Some(sid) => Ok(sid),
        None => Err("need --role or --surface".into()),
    }
}

/// T2-4 컨텍스트 사이클 집행기 — 게이트는 화면 마커가 아니라 파일 mtime+해시.
#[allow(clippy::too_many_arguments)]
/// cycle-agent가 대상 surface를 quiescing(=채널 inbox 주입 보류)으로 마킹/해제한다(§2.2 S5).
/// clear 직전 on, resume 후(또는 실패해도) off로 호출해 clear·복원 구간의 채널 주입을 봉한다.
fn set_surface_quiescing(sid: u64, on: bool) -> Result<(), String> {
    request("surface.quiesce", json!({"surface_id": sid, "on": on})).map(|_| ())
}

/// C3 저장검증 대상 제외 판정 — 선언이 `retired`(은퇴) 또는 `foreign-scope`(실재하는 남의 팩)면 true.
///
/// **fail-open이 계약이다**(ADR-3): 미선언(`unclaimed`)·고아(`orphan-scope`)는 제외하지 **않는다**.
/// 판정 못 한다고 살아있을 수 있는 파일을 게이트에서 빼면 저장 누락을 조용히 통과시킨다 —
/// 놓치는 것보다 시끄러운 편이 안전하다. 파일을 못 열어도 같은 이유로 제외하지 않는다.
///
/// 읽기는 선두 `HEAD_BYTES`(1 KiB)뿐이다(G3) — cycle마다 _round의 파일 수만큼 도는 경로다.
/// `scope_exists`를 주입받는 이유는 파서와 같다: 판정에서 파일시스템을 분리해 테스트가 실재
/// 팩 배치에 의존하지 않게 한다(프로덕션 호출자는 `cys::pack::scope_exists`를 넘긴다).
fn todo_decl_excluded(
    path: &std::path::Path,
    my_scope: &str,
    scope_exists: &dyn Fn(&str) -> bool,
) -> bool {
    let Ok(f) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = Vec::new();
    if f.take(cys::todo_decl::HEAD_BYTES as u64)
        .read_to_end(&mut buf)
        .is_err()
    {
        return false;
    }
    // 예산(G3) 적용과 lossy 디코드는 계약 정본 `head_from_bytes`가 유일하게 수행한다.
    // ★W14 S15 — 여기서 `String::from_utf8_lossy`를 직접 부르면 그것이 곧 두 번째 읽기 규칙이
    // 되고, 언젠가 정본과 갈린다(실제로 C2 데몬이 그렇게 갈렸다). 경계가 멀티바이트 중간이어도
    // 대체문자를 남길 뿐 패닉하지 않는 성질은 그 함수가 보장한다.
    let head = cys::todo_decl::head_from_bytes(&buf);
    let decl = cys::todo_decl::parse(&head).ok();
    let verdict = cys::todo_decl::classify(decl.as_ref(), my_scope, scope_exists);
    matches!(
        verdict,
        cys::todo_decl::Verdict::Retired | cys::todo_decl::Verdict::ForeignScope
    )
}

/// ★C2 — cycle 저장 게이트 목록 확정(순수). `detected`=스윕으로 찾은 **실존** 후보,
/// `expected`=지시문이 생성을 명령하는 **기대 경로**(실존 여부 무관).
///
/// 협로가 무엇이었나: 신설 노드의 첫 cycle은 후보가 하나도 없다(`_round`도 팩 todo도 아직
/// 없다). 종전 코드는 그 상태를 "저장 검증 파일 없음" 에러로 끝내 **clear를 영영 실행하지
/// 못했다** — 정작 컨텍스트가 가장 급한 노드가 순환에서 배제되는 방향의 실패다. 기대 경로를
/// 감시 대상으로 삼으면 에이전트가 지시대로 파일을 만드는 순간 게이트가 통과한다(생성도
/// 갱신이다). 게이트 의미는 그대로 ANY-match이고, 감시 대상이 늘어도 **다른 노드가 건드릴 수
/// 없는 자기 역할 파일**뿐이라 거짓 통과 위험은 늘지 않는다.
///
/// 순서 보존 dedup: baseline·handshake 본문이 이 순서로 만들어지므로 안정적이어야 한다.
fn cycle_gate_files(detected: Vec<String>, expected: Vec<std::path::PathBuf>) -> Vec<String> {
    let mut out = detected;
    for p in expected {
        let s = p.to_string_lossy().into_owned();
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// ★A′ — [CYCLE] 저장 지시문 생성(순수): 지시문이 안내하는 경로 = 게이트가 감시하는 경로.
///
/// 종전 고정 산문("~/.cys/pack/round/<역할>_TODO.md" 틸드 하드코딩 + "_round/ 또는 pack
/// round/ 정본" 모호 안내)은 lease/게이트 실경로와 어긋날 수 있었다 — 노드가 산문을 따라
/// **목록 밖**에 저장하면 파일 게이트·ALL-match 검증자(javis_cycle_verifier)가 deny 를
/// 반복한다. 감시 목록(files)을 그대로 열거해 그 갈림을 원천 제거한다.
///
/// 재기록 지시는 목록 '전부'가 아니라 **네 역할 소관 파일**로 한정한다 — 수동 레인
/// (--save-file 미지정 기본 탐지)의 감시 목록에는 cwd/_round 의 **타 역할** *_TODO.md 가
/// 포함될 수 있어(todo_decl_excluded 통과분 전부 수집), '전부 재기록' 강제는 남의 역할
/// TODO 쓰기('같은 산출물 쓰기는 단일 스레드' 규율 위반)를 유도한다. 전자동 레인의
/// ALL-match 정합은 불변이다 — lease 는 애초에 역할 소관 파일로만 구성되기 때문이다.
/// ② CYCLE-SAVED 마커 문장(plain 한 줄)은 종전 계약 그대로 보존한다.
///
/// [codex R1 수용 2026-08-20] 역할 인지형 개정 — "네 역할 소관"의 해석을 LLM 에 맡기지 않고 role 인자로 소관을 문구에 결정론 명시한다(비master 노드의 공유 SESSION_STATE 오재기록 차단).
fn cycle_save_directive(role: &str, files: &[String]) -> String {
    let scope = if role == "master" {
        // master 소관 = 자기 TODO + SESSION_STATE (종전 취지 유지).
        "이 중 **네 역할 소관 파일**(자기 TODO·자기 SESSION_STATE)을 지금 즉시 물리적으로 재기록하라(현재 작업 상태·미해결 게이트·다음 액션 저장)."
    } else {
        "네 소관은 **자기 역할 TODO 파일만**이다 — 그것을 지금 즉시 물리적으로 재기록하라(현재 작업 상태·미해결 게이트·다음 액션 저장). 목록의 SESSION_STATE·타 역할 TODO는 감시(관찰) 대상일 뿐 **쓰기 금지**(단일 스레드 쓰기 규율)."
    };
    format!(
        "[CYCLE] 컨텍스트 순환 절차 개시. ① 아래는 저장 검증이 감시하는 파일 경로 목록이다 — {scope} 목록 밖 경로에 저장하면 검증에 인정되지 않는다: {} ② 저장 완료 후 다른 출력 없이 plain 한 줄로 CYCLE-SAVED 를 출력하라.",
        files.join(" · ")
    )
}

/// 저장 게이트 1틱 판정(ANY-match) — 하나라도 `start_time` 이후 갱신됐고 해시가 baseline과
/// 다르면 통과. 화면 마커(CYCLE-SAVED)는 참고 신호일 뿐이고 **파일 변화가 사실**이다
/// (reward-hack·stale 마커 차단).
///
/// ★C2: **비존재 파일의 baseline은 `None`**이므로, 지시대로 새로 생성된 파일은
/// mtime>start && `Some(해시)` != `None` 이 성립해 이 판정이 그대로 '변화'로 인정한다 —
/// 기대 경로를 게이트에 넣기 위해 판정 로직을 바꿀 필요가 없다는 것이 이 함수의 계약이다.
/// 루프에서 분리한 이유도 그 계약을 테스트가 직접 단정할 수 있게 하기 위함이다.
fn cycle_save_verified(
    baseline: &[(String, Option<String>)],
    start_time: std::time::SystemTime,
) -> bool {
    baseline.iter().any(|(f, base_hash)| {
        let mtime_ok = std::fs::metadata(f)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| t > start_time)
            .unwrap_or(false);
        mtime_ok && sha256_file(f) != *base_hash
    })
}

/// cycle 2단계(저장 검증)의 진로 — `--force-no-verify` 의 실효 의미를 타입으로 고정한다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CycleVerifyPlan {
    /// 파일 갱신을 기다린다(기본).
    Wait,
    /// 운영자가 명시적으로 검증을 건너뛴다(비상 탈출구).
    SkipForced,
    /// 감시할 파일 자체가 없다.
    SkipNoFiles,
}

/// ★E3(적대 리뷰 REVISE-4): C2 이후 `files` 가 절대 비지 않게 되면서 `--force-no-verify` 는
/// **死플래그**가 됐다 — 유일한 소비처였던 "빈 목록 거부" 분기에 도달할 수 없기 때문이다.
/// 그 결과 저장이 불가능한 상태(에이전트 무응답·hang)에서 clear 를 강행할 비상 탈출구가
/// 사라졌고, 운영자는 30분 timeout 을 기다린 뒤 실패를 받는 것 외에 방법이 없었다.
/// 플래그의 원래 의미(검증 없이 진행)를 **검증 대기 자체를 건너뛰는 것**으로 복원한다.
/// 저장 지시 주입은 그대로 한다 — 지시조차 안 하면 저장할 기회 자체가 사라진다.
fn cycle_verify_plan(force_no_verify: bool, baseline_len: usize) -> CycleVerifyPlan {
    if force_no_verify {
        CycleVerifyPlan::SkipForced
    } else if baseline_len > 0 {
        CycleVerifyPlan::Wait
    } else {
        CycleVerifyPlan::SkipNoFiles
    }
}

/// 2-phase handshake 본문의 파일 1줄(순수).
///
/// ★E8(N-5): 종전에는 `unwrap_or_default()` 로 **미존재 파일에 빈 해시**를 실었다. 검증자는
/// `"경로 (sha256: )"` 를 보고 "해시가 비었다 = 뭔가 잘못됐다"로 읽거나, 더 나쁘게는 다른
/// 미존재 파일과 **같은 빈 값**이라 구분하지 못했다. C2 이후 기대 경로(아직 없는 파일)가
/// 본문에 정상적으로 들어오므로, 그 상태를 사실대로 표기한다 — 이 파일은 **생성 자체가
/// 저장의 증거**이고, 검증자가 확인해야 할 것은 해시가 아니라 존재 전이다.
fn handshake_file_line(path: &str, hash: Option<String>) -> String {
    match hash {
        Some(h) => format!("{path} (sha256: {h})"),
        None => format!("{path} (미생성 — 생성 자체가 증거)"),
    }
}

/// ★W4-B(결함 7): 검증자 승인 **영수증** 판정 — 순수 함수(데몬 불요·full RPC 없이 테스트).
///
/// 승인 영수증 = (request_id 일치는 호출측 폴링이 보장, decision=allow 계열,
/// `resolver_surface` == **지정 검증자** surface). 종전 코드는 timeout 에는 안전했지만
/// "누가 allow 했나"를 검증하지 않아 CEO 자동결재·제3자 reply·GUI 버튼이 전부 '검증자
/// 승인'으로 통용됐다 — resolver 대조가 이를 봉인한다(무검증 clear 차단 · producer≠evaluator).
///
/// resolver_surface 의 정의처는 데몬 feed.reply 단일 해소 경로(state.rs
/// `resolve_feed_item_audited` · W4-A 각인)다. **비-pane 해소 경로는 전부 여기서 거부**되며,
/// 오진 처방(불필요한 데몬 재시작 등)을 막기 위해 사유를 3분류로 구분한다:
///   · 키 자체 부재            = 구 데몬(feed.list 가 resolver 를 직렬화하지 않음 — 재시작 필요)
///   · null + resolver_pid 有  = GUI operator 토큰 등 pane 미귀속 해소(지정 검증자 아님)
///   · null + resolver_pid 無  = 데몬 내부(stale-clear)·채널 경유 해소(지정 검증자 아님)
/// Err 는 전부 clear 미실행 안전 중단으로 수렴한다 — 기존 'timeout→clear 미실행' 경로
/// (호출측 receipt=None 분기)와 같은 방향이다. 거부(deny·dismissed)는 안전 방향이므로
/// resolver 없이도 즉시 중단한다(allow 한정 검증 — is_self_approval 과 동일 원칙).
///
/// ★검증자 워처 상호참조(성찰 MAJOR): javis_cycle_autopilot 이 기동하는 검증자 워처
/// (javis_cycle_verifier)는 검증자 pane 의 포그라운드 자식이라 `cys feed reply` 시 커널
/// peer pid 조상 추적(handlers.rs resolve_caller_surface)으로 resolver_surface 가 그 pane
/// 에 귀속된다(mac 실측 가능). Windows 는 조상 체인 단절이 빈발해(CI windows-health
/// H-IDENT-1) 귀속 실패 = 영수증 부재 = 안전 중단이 될 수 있다 — 실기 핀은 CI 이관,
/// 여기선 한계만 명기한다(커밋 Not-tested 참조).
/// ★위장 한계 정직 표기(3R-A): resolver 는 pane 귀속까지만 증명한다 — 같은 pane 안의 다른
/// 프로세스가 reply 하면 구분하지 못한다(pane 침해는 별도 위협 등급 · 감사 원장이 사후 추적).
fn cycle_receipt_ok(item: &Value, vsid: u64) -> Result<(), String> {
    if item["status"].as_str() != Some("resolved") {
        return Err("영수증 아님(미해소 항목) — clear 중단".into());
    }
    let decision = item["decision"].as_str().unwrap_or("(없음)");
    if !matches!(decision, "allow" | "yes" | "approve") {
        // deny 계열·dismissed(GUI 목록 치우기) 전부 — 거부는 영수증 불요·즉시 안전 중단.
        return Err(format!("검증자 거부({decision}) — cycle 중단"));
    }
    match item.get("resolver_surface") {
        None => Err(
            "allow 영수증에 resolver 없음 — 구 데몬(feed.list 가 resolver_surface 를 \
             직렬화하지 않음). 팩/바이너리 갱신 후 첫 cycle 전 cysd 재시작 필요 · \
             비상시 --force-no-verify(위험). clear 중단"
                .into(),
        ),
        Some(v) if v.is_null() => {
            let via = if item["resolver_pid"].as_u64().is_some() {
                "GUI operator 토큰 등 pane 미귀속 해소"
            } else {
                "데몬 내부(stale)·채널 경유 해소"
            };
            Err(format!(
                "allow 해소 주체가 지정 검증자가 아님({via}) — 판정은 지정 검증자 pane \
                 에서 `cys feed reply` 로만 유효하다. clear 중단"
            ))
        }
        Some(v) => match v.as_u64() {
            Some(s) if s == vsid => Ok(()),
            Some(s) => Err(format!(
                "resolver=surface:{s} ≠ 지정 검증자 surface:{vsid} — 제3자 해소는 \
                 영수증이 아니다. clear 중단"
            )),
            None => Err(format!("resolver_surface 형식 이상({v}) — clear 중단")),
        },
    }
}

fn run_cycle_agent(
    role: Option<String>,
    surface: Option<String>,
    verifier: Option<String>,
    save_files: Vec<String>,
    clear_cmd: Option<String>,
    resume_text: Option<String>,
    timeout: u64,
    force_no_verify: bool,
) -> i32 {
    let result = (|| -> Result<(), String> {
        let sid = resolve_role_or_surface(&role, &surface)?;
        let entry = surface_entry(sid)?;
        if entry["exited"].as_bool() == Some(true) {
            return Err(format!("surface:{sid} 이미 종료됨"));
        }
        let role_name = entry["role"].as_str().unwrap_or("worker").to_string();
        // soul 축2: master self-clear 금지 — 검증자 없는 master cycle 거부
        if role_name == "master" && verifier.is_none() {
            return Err(
                "master cycle엔 --verifier <role>이 필수 (self-clear 금지 — 2-phase handshake)"
                    .into(),
            );
        }
        // clear 명령 선확정 — 저장만 시키고 clear 못하는 어정쩡한 상태 방지
        let agent = entry["agent"].as_str().map(String::from);
        let clear = match clear_cmd {
            Some(c) => c,
            None => {
                let a = agent
                    .clone()
                    .ok_or("agent 메타 없음 — --clear-cmd 명시 필요")?;
                load_agent_spec(&a)?["clear_cmd"]
                    .as_str()
                    .ok_or_else(|| {
                        format!("agents.json '{a}'에 clear_cmd 없음 — --clear-cmd 명시 필요")
                    })?
                    .to_string()
            }
        };
        // 저장 검증 파일 확정 (기본: <cwd>/_round/SESSION_STATE.md + *_TODO.md 자동 탐지)
        let cwd = entry["live_cwd"]
            .as_str()
            .or(entry["cwd"].as_str())
            .unwrap_or(".")
            .to_string();
        let files: Vec<String> = if !save_files.is_empty() {
            save_files
        } else {
            // 기본 탐지: <cwd>/_round 전체 + pack/round의 '대상 역할 소유분'만 — 절대지침이
            // todo·SESSION_STATE 정본을 pack/round로 통일했으므로(앵커5·6) 거기 저장분도
            // 검증 대상이다. 단 pack/round는 전 노드 공유 디렉터리라 다른 노드의 갱신이
            // 저장 게이트를 거짓 통과시킬 수 있어(타이밍 의존) 대상 역할 파일로 한정한다.
            let mut v = Vec::new();
            let cwd_round = std::path::PathBuf::from(format!("{cwd}/_round"));
            let ss = cwd_round.join("SESSION_STATE.md");
            if ss.exists() {
                v.push(ss.to_string_lossy().into_owned());
            }
            // ★C3(설계 §4-5): cwd/_round도 전 노드가 함께 쓰는 디렉터리다. 바로 위 pack/round
            // 분기는 그 사실을 알고 대상 역할 파일로 한정하는데 **이 분기만 무방비**였던 비대칭이
            // 유령 todo 사고의 코드 수준 원인이다. 종결된 레인의 유산 파일(status=retired)과 남의
            // 팩 파일(foreign-scope)을 목록에서 빼 handshake 본문을 정화한다.
            // 판정 의미는 바꾸지 않는다 — 게이트는 여전히 ANY-match(하나라도 갱신되면 통과)다.
            let my_scope = cys::pack::scope_id();
            let scope_exists = |s: &str| cys::pack::scope_exists(s);
            if let Ok(entries) = std::fs::read_dir(&cwd_round) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.ends_with("_TODO.md")
                        && !todo_decl_excluded(&e.path(), &my_scope, &scope_exists)
                    {
                        v.push(e.path().to_string_lossy().into_owned());
                    }
                }
            }
            let pack_round = cys::pack::pack_dir().join("round");
            let role_todo = format!(
                "{}_TODO.md",
                role_name.to_uppercase().replace('-', "_")
            );
            // ★C2: 기대 경로(지시문이 **생성을 명령하는** 파일)는 실존 여부와 무관하게 넣는다.
            // 종전 `pt.exists()` 가드가 만든 협로: 신설 노드는 아직 이 파일이 없어 게이트에서
            // 빠지고, 지시문은 바로 그 파일을 만들라고 시킨다 — 순응해 저장해도 아무도 안 보므로
            // 검증 실패로 clear가 막힌다. 비존재 파일의 baseline은 `None`이고, 생성되면
            // mtime>start && 해시(Some) != None 이 성립해 현행 ANY-match 루프가 그대로 인정한다.
            // SESSION_STATE(pack 정본)는 master 소관이므로 master cycle에서만 기대 경로다.
            let mut expected = vec![pack_round.join(&role_todo)];
            if role_name == "master" {
                expected.push(pack_round.join("SESSION_STATE.md"));
            }
            cycle_gate_files(v, expected)
        };
        // ★E3 주석 정정: C2 폴백(`cycle_gate_files`)이 기대 경로를 무조건 넣으므로 `files` 는
        // 실질적으로 비지 않는다 — 이 분기는 폴백이 되돌려지는 미래를 대비한 **방어적 잔존**이지
        // "진짜 예외가 여기 남았다"는 종전 서술은 사실이 아니었다. 그래서 `--force-no-verify` 의
        // 실질 의미도 이 분기가 아니라 **아래 검증 대기 생략**에 있다(비상 탈출구).
        if files.is_empty() && !force_no_verify {
            return Err(
                "저장 검증 파일 없음 — --save-file로 지정하거나 --force-no-verify(위험)".into(),
            );
        }
        let start_time = std::time::SystemTime::now();
        let baseline: Vec<(String, Option<String>)> = files
            .iter()
            .map(|f| (f.clone(), sha256_file(f)))
            .collect();

        // 1) 저장 지시
        eprintln!("[cycle 1/5] 저장 지시 주입 → surface:{sid} ({role_name})");
        // ★A′: 고정 산문 대신 감시 목록(files) 실경로 열거 — 지시 경로↔게이트 경로 정합.
        inject_text(sid, &cycle_save_directive(&role_name, &files))?;

        // 2) 파일 변화 게이트 (화면 마커는 참고 신호일 뿐 — reward-hack·stale 마커 차단)
        match cycle_verify_plan(force_no_verify, baseline.len()) {
            CycleVerifyPlan::Wait => {
                eprintln!("[cycle 2/5] 저장 파일 검증 대기 (mtime+해시, 최대 {timeout}s)");
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
                let mut verified = false;
                while std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    if cycle_save_verified(&baseline, start_time) {
                        verified = true;
                        break;
                    }
                }
                if !verified {
                    return Err(format!(
                        "저장 검증 실패 — {timeout}s 내 파일 갱신 없음. cycle 중단 (clear 미실행)"
                    ));
                }
                eprintln!("[cycle] 저장 검증 통과");
            }
            CycleVerifyPlan::SkipForced => {
                eprintln!(
                    "[cycle 2/5] ⚠ 저장 검증 **생략** (--force-no-verify) — 저장 지시는 주입했지만 \
                     파일 갱신을 기다리지 않는다. 대상이 저장하지 못한 상태로 clear 될 수 있다."
                );
            }
            CycleVerifyPlan::SkipNoFiles => {
                eprintln!("[cycle 2/5] ⚠ 감시 대상 파일 없음 — 검증 생략");
            }
        }

        // 3) 2-phase handshake — 검증자 부재 시 clear 금지 (soul 규칙)
        if let Some(v) = &verifier {
            eprintln!("[cycle 3/5] 검증자 '{v}' handshake");
            let vr = request("system.resolve_role", json!({"role": v}))
                .map_err(|e| format!("검증자 '{v}' 부재 — clear 금지 (self-clear 차단): {e}"))?;
            let vsid = vr["surface_id"].as_u64().ok_or("bad verifier resolve")?;
            let body: String = baseline
                .iter()
                .map(|(f, _)| handshake_file_line(f, sha256_file(f)))
                .collect::<Vec<_>>()
                .join("\n");
            let push = request(
                "feed.push",
                json!({"kind": "cycle-verify",
                       "title": format!("[CYCLE-VERIFY] {role_name} 저장 검증 요청"),
                       "body": body, "surface_id": sid, "wait": false}),
            )?;
            let req_id = push["request_id"].as_str().unwrap_or("").to_string();
            inject_text(vsid, &format!("[CYCLE-VERIFY] role '{role_name}'(surface:{sid})의 컨텍스트 순환 전 저장 검증 요청. SESSION_STATE/TODO 파일이 방금 갱신되었는지 확인하고 `cys feed reply {req_id} allow` 또는 `cys feed reply {req_id} deny`로 판정하라."))?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
            // ★W4-B(결함 7): 해소 항목을 발견해도 decision 문자열로 즉석 판정하지 않고 영수증
            // 검증(cycle_receipt_ok — resolver==지정 검증자 대조)에 넘긴다. Err 는 전부 clear
            // 미실행 안전 중단(아래 match)이고, timeout(None)의 안전 중단은 종전 그대로다.
            let receipt = loop {
                if std::time::Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
                let items = request("feed.list", json!({}))?;
                let found = items["items"]
                    .as_array()
                    .and_then(|a| {
                        a.iter()
                            .find(|i| i["request_id"].as_str() == Some(req_id.as_str()))
                            .cloned()
                    });
                if let Some(item) = found {
                    if item["status"].as_str() == Some("resolved") {
                        break Some(cycle_receipt_ok(&item, vsid));
                    }
                }
            };
            match receipt {
                Some(Ok(())) => eprintln!(
                    "[cycle] 검증자 승인 — 영수증 확인(resolver=surface:{vsid}) → clear 진행"
                ),
                // 영수증 불충족(거부·구 데몬·비-pane 해소·제3자 스탬프) — 사유는 stderr 로
                // 그대로 전파되고(run_cycle_agent 말미 eprintln) clear 는 실행되지 않는다.
                Some(Err(e)) => return Err(e),
                None => return Err("검증자 응답 없음 (timeout) — clear 중단".into()),
            }
        } else {
            eprintln!("[cycle 3/5] (검증자 미지정 — handshake 생략)");
        }

        // S5(§2.2): clear 직전 대상 surface를 quiescing으로 마킹 → 채널 inbox 주입이 clear·복원
        // 구간 동안 보류된다(C0 배달기가 이 상태를 읽음). autopilot 60% clear가 상시 조건이므로
        // 이게 채널×clear 레이스의 실질 봉합이다.
        set_surface_quiescing(sid, true)?;
        let clear_resume = (|| -> Result<(), String> {
            // 4) 입력 버퍼 정리 + clear
            eprintln!("[cycle 4/5] 입력 버퍼 정리 + '{clear}'");
            request("surface.send_key", json!({"surface_id": sid, "key": "C-u"}))?;
            std::thread::sleep(std::time::Duration::from_millis(200));
            request(
                "surface.send_text",
                json!({"surface_id": sid, "text": clear, "quiet": true}),
            )?;
            request(
                "surface.send_key",
                json!({"surface_id": sid, "key": "Return"}),
            )?;
            std::thread::sleep(std::time::Duration::from_secs(4));

            // 5) 디렉티브 재주입 + 재개 포인터
            eprintln!("[cycle 5/5] 디렉티브 재주입 + 재개 포인터");
            let directive = compose_directive(&role_name)?;
            inject_text(sid, &directive)?;
            std::thread::sleep(std::time::Duration::from_secs(2));
            let resume = resume_text.unwrap_or_else(|| {
                "[RESUME] 컨텍스트 순환 완료. _round/SESSION_STATE.md와 자기 TODO를 읽고 직전 작업을 이어가라.".into()
            });
            inject_text(sid, &resume)?;
            Ok(())
        })();
        // 재개 성공/실패와 무관하게 quiescing 해제 — 실패로 master가 quiescing에 갇혀 채널이
        // 영구 보류되는 것을 막는다(master 자기보고 안전망과 별개의 결정론 해제).
        let _ = set_surface_quiescing(sid, false);
        clear_resume?;
        println!("cycle complete → surface:{sid} ({role_name})");
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// T2-5 노드 복구: 죽은 에이전트를 같은 surface에서 재기동 + 지침 재주입 + 복원 포인터
fn run_node_recover(surface: Option<String>, role: Option<String>) -> i32 {
    let result = (|| -> Result<BootVerdict, String> {
        let sid = resolve_role_or_surface(&role, &surface)?;
        let entry = surface_entry(sid)?;
        if entry["exited"].as_bool() == Some(true) {
            return Err(format!(
                "surface:{sid} 셸 자체가 종료됨 — `cys restore`로 재기동하라"
            ));
        }
        let agent = entry["agent"]
            .as_str()
            .ok_or("agent 메타 없음 (launch-agent로 기동된 pane만 복구 가능)")?
            .to_string();
        if entry["agent_alive"].as_bool() == Some(true) {
            return Err(format!(
                "agent '{agent}'가 살아있는 것으로 보임 — 강제 재기동은 close-surface 후 launch-agent"
            ));
        }
        // RC-3 잔여(T2.1·codex CONFIRMED): Windows node-recover는 기존 pane에 **순수 cmd**를 재기동한다
        // (RC-3 B′). 그 pane이 env 미주입(create_surface_with_env 경유 아님 — 수동 생성·구세션)이면
        // CLAUDE_CONFIG_DIR 등이 pane env에 없어 claude가 오염된 기본 config로 뜬다. fail-closed로 차단
        // (unix는 인라인 `KEY="val" cmd` 재조립이 env를 셸 전개하므로 무관 — Windows 한정 가드).
        #[cfg(windows)]
        if entry["env_injected"].as_bool() != Some(true) {
            return Err(format!(
                "surface:{sid}는 env 미주입 pane(수동 생성·구세션) — Windows에선 순수 cmd 재기동 시 \
                 CLAUDE_CONFIG_DIR 등이 실리지 않아 안전하지 않다. `cys restore` 또는 \
                 `cys close-surface {sid}` 후 `cys launch-agent`로 재기동하라"
            ));
        }
        let role_name = entry["role"].as_str().unwrap_or("worker").to_string();
        let spec = load_agent_spec(&agent)?;
        eprintln!("[node-recover] surface:{sid} 위에 {agent} 재기동 (role={role_name})");
        // 셸 입력 잔재 정리 후 기동 (resume 플래그로 대화 기억 복원 시도)
        request("surface.send_key", json!({"surface_id": sid, "key": "C-u"}))?;
        std::thread::sleep(std::time::Duration::from_millis(200));
        // (4b) topology에 영속된 session_id가 있으면 정확한 세션 재개(없으면 fallback)
        let sess = entry["session_id"].as_str().map(String::from);
        // (W1) 같은 pane 재기동(restore=false → 인라인 없음)이나 resume 게이트엔 기록된 config_dir·cwd를 쓴다.
        let rec_cwd = entry["cwd"].as_str().map(String::from);
        let rec_cfg = entry["claude_config_dir"].as_str().map(String::from);
        let verdict = boot_agent_on_surface(
            sid,
            &role_name,
            &agent,
            &spec,
            true,
            sess.as_deref(),
            false,
            rec_cwd.as_deref(),
            rec_cfg.as_deref(),
        )?;
        match &verdict {
            BootVerdict::Ready => {
                inject_text(sid, "[RECOVER] 너는 방금 재기동되었다. _round/SESSION_STATE.md와 자기 TODO 파일을 읽어 작업 기억을 복원한 뒤 master에게 복귀를 1줄 push로 보고하라. 작업 재개는 master 지시를 따른다.")?;
                println!("recovered surface:{sid} ({agent})");
            }
            // ★(U-11) 이 호출부의 귀결은 launch 와 **다르다** — 여기엔 닫을 새 surface 가 없다.
            //   대신 이 경로의 실패는 `run_boot` 에서 `escalate_reclaim`(=kill)으로 자동
            //   에스컬레이션된다. 그래서 보류를 rc 1 로 내면 **살아 있는 에이전트를 죽인다**.
            //   전용 종료코드로 갈라 그 체인을 타지 않게 한다(수신부 분기는 run_boot 에 있다).
            //   주입도 하지 않는다: 관문 창에 `[RECOVER]` 를 밀어 넣는 것은 화면 파괴이고,
            //   그 붙여넣기의 Return 이 실측상 면책 창의 종료 버튼을 누른다.
            BootVerdict::GatePending { gate, tail } => {
                print_gate_pending_prescription(sid, &role_name, &agent, gate, tail);
                println!(
                    "gate-pending surface:{sid} ({agent}) — 좌석 보존 · 주입 0 · 회수 0(사람 1회 조치 대기)"
                );
            }
            BootVerdict::LaunchFailed { .. } => {}
        }
        Ok(verdict)
    })();
    match result {
        Ok(BootVerdict::Ready) => 0,
        Ok(BootVerdict::GatePending { .. }) => cys::EXIT_GATE_PENDING,
        // 종전과 동일: 실패는 rc 1 이고, run_boot 이 그 위에서 reclaim 으로 에스컬레이션한다.
        Ok(BootVerdict::LaunchFailed { evidence }) => {
            eprintln!("error: {evidence}");
            1
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// ★W2 복원 디렉티브 분기: 워커·리뷰어는 master 지시를 기다리지만, master는 지시할 상위가 없다 —
/// RECOVERY 프로토콜로 스스로 상태를 복원하고 미해결 게이트부터 자율 재개한다(콜드부트
/// auto-restore가 master를 포함하는 경로).
/// ★SEAT: fresh 기동과 **좌석 내 재연결(in-seat)** 두 경로가 같은 문구를 쓰도록 함수로 둔다 —
/// 인라인 중복이면 한쪽만 고쳐지는 드리프트가 난다(복원 계약은 경로와 무관하게 하나다).
fn restore_directive(role: &str) -> &'static str {
    if role == "master" {
        "[RESTORE] 조직 복원 절차다(master). _round/RECOVERY.md → SESSION_STATE.md → 자기 TODO → memory → git 순으로 읽고, 노드 재기동·surface 재매핑·directive 각성 후 미해결 게이트부터 자율 재개하라."
    } else {
        "[RESTORE] 조직 복원 절차다. _round/SESSION_STATE.md와 자기 TODO를 읽고 상태를 복원하라. ★작업 재개는 하지 말고 master의 지시를 기다려라."
    }
}

/// T2-6 조직 복원: 토폴로지 스냅샷 기준으로 죽은 역할 일괄 재기동 (작업 재개는 master 판단)
fn run_restore(cwd: Option<String>, include_master: bool, no_resume: bool) -> i32 {
    let result = (|| -> Result<(usize, usize, usize), String> {
        let topo = request("system.topology", json!({}))?;
        // ★SEAT(2026-07-17 실사고 수리): '역할이 등록됨'과 '그 좌석에 누가 앉아 있음'을 구분한다.
        // 종전 live 집합은 role 등록만 보고 skip 해서, role=master 를 쥔 **빈 셸**(agent 없는 좌석)이
        // 있으면 master 를 영영 부활시키지 못했다(phoenix·▶부서장 버튼·부트가 동시에 잠김).
        // 이제 live = "좌석이 실제 점유된 역할"만. 빈 좌석 역할은 아래 seats 맵으로 넘겨,
        // **그 좌석에 직접 연결**(in-seat)하거나 승계 fresh 로 되살린다.
        let live_entries = topo["live"].as_array().cloned().unwrap_or_default();
        let live: std::collections::HashSet<String> = live_entries
            .iter()
            .filter(|e| e["seat"].as_str() != Some("empty"))
            .filter_map(|e| e["role"].as_str().map(String::from))
            .collect();
        // 빈 좌석 인덱스: role → (surface_id, env_injected). 부활 대상이면서 좌석이 이미 존재하는 경우.
        let empty_seats: std::collections::HashMap<String, (u64, bool)> = live_entries
            .iter()
            .filter(|e| e["seat"].as_str() == Some("empty"))
            .filter_map(|e| {
                let role = e["role"].as_str()?.to_string();
                let sid = e["surface_id"].as_u64()?;
                Some((role, (sid, e["env_injected"].as_bool().unwrap_or(false))))
            })
            .collect();
        let saved = topo["saved"].as_array().cloned().unwrap_or_default();
        // ★W2a 심층방어: 의도적으로 닫힌(surface.close 경유) 역할의 묘비 — raw restore도 절대 재스폰하지
        // 않는다(1급 원칙: 사고사만 부활, 의도삭제는 좀비 차단). phoenix가 desired_roster로 병합하는
        // 것과 별개로, 이 경로가 직접 호출돼도 좀비를 살리지 않도록 한 겹 더 막는다.
        let tombstones: std::collections::HashSet<String> = topo["tombstones"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|e| e.as_str().map(String::from))
            .collect();
        if saved.is_empty() {
            println!("(토폴로지 스냅샷 없음 — launch-agent로 역할을 기동하면 자동 기록된다)");
            return Ok((0, 0, 0));
        }
        // ★(U-11) `gated` = "살아 있는데 관문에 갇힌 좌석" — 성공도 실패도 아닌 제3의 사실이다.
        //   exit 회계에는 종전대로 **실패와 같이** 넣는다(아래 `fail > 0 || gated > 0`):
        //   복원이 끝났다고 말하면 안 되는 상태이기 때문이다. 다만 화면 문안은 갈라서, 사람이
        //   '기동이 깨졌다'(재시도)와 '관문을 통과시켜라'(1회 조치)를 혼동하지 않게 한다.
        let (mut ok, mut fail, mut gated) = (0usize, 0usize, 0usize);
        for entry in saved {
            let Some(role) = entry["role"].as_str() else {
                continue;
            };
            // ★W2a: 묘비 역할은 include_master 여부와 무관하게 건너뛴다(의도삭제>강제부활).
            if tombstones.contains(role) {
                println!("· {role}: 의도적 삭제(묘비) — 부활 안 함 (좀비 차단)");
                continue;
            }
            if role == "master" && !include_master {
                println!("· {role}: 제외 (restore 실행자가 보통 master — --include-master로 포함)");
                continue;
            }
            if live.contains(role) {
                println!("· {role}: 이미 가동 중 — 건너뜀");
                continue;
            }
            let Some(agent) = entry["agent"].as_str() else {
                // 스냅샷에 agent 가 없으면 무엇을 띄울지 결정론으로 알 수 없다(claim-role 로만 등록된
                // pane). 좌석 유무와 무관하게 여기서 멈추는 것이 옳다 — 임의 기본값(claude) 추정은
                // 다른 에이전트를 쓰는 좌석에 엉뚱한 CLI 를 띄운다.
                let hint = if empty_seats.contains_key(role) {
                    " (좌석은 비어 있음 — 그 pane 에서 직접 agent 를 실행하면 등록된다)"
                } else {
                    ""
                };
                println!("· {role}: agent 미상 — 건너뜀 (claim-role로 등록된 pane){hint}");
                continue;
            };
            let target_cwd = cwd
                .clone()
                .or_else(|| entry["cwd"].as_str().map(String::from));
            // (4b) saved entry의 session_id를 꺼내 정확한 세션 재개(없으면 fallback)
            let sess = entry["session_id"].as_str().map(String::from);
            // (W1) topology에 기록된 원 계정 config_dir을 넘긴다(구 topology=None → 기존 템플릿 동작).
            let cfg = entry["claude_config_dir"].as_str().map(String::from);
            // ★SEAT in-seat 연결(오너 의도: "최초로 만들어지는 surface에 클로드가 연결되고 마스터로
            // 부활"): 그 역할의 좌석이 이미 있고 비어 있으면 **새 surface 를 만들지 않고 그 좌석에
            // 직접** 에이전트를 기동한다. 좌석이 늘지 않고(796형 잔존 pane 0) 사용자가 보는 그 pane 이
            // 그대로 부서장이 된다.
            //
            // ★계정격리 가드(E8): 빈 셸은 `cys new-surface` 산물이라 **pane env 가 비어 있다**.
            // unix 는 boot_agent_on_surface 가 `KEY="val" cmd` 인라인으로 env 를 실어 안전하지만,
            // Windows 는 순수 cmd 를 보내므로 CLAUDE_CONFIG_DIR 이 실리지 않아 **계정 격리가 깨진다**
            // (node-recover 가 같은 이유로 fail-closed 하는 지점). 안전하지 않으면 in-seat 를 포기하고
            // fresh 로 폴백한다 — 기능은 회복되고 격리는 보존된다(좌석이 하나 늘 뿐이며 승계가 정리한다).
            let in_seat = empty_seats.get(role).and_then(|&(sid, env_injected)| {
                let safe = cfg!(unix) || env_injected;
                safe.then_some(sid)
            });
            if let Some(sid) = in_seat {
                println!("· {role}: {agent} 좌석 내 재연결(surface:{sid})…");
                let spec = match load_agent_spec(agent) {
                    Ok(s) => s,
                    Err(e) => {
                        println!("· {role}: agent spec 해석 실패({e}) — 건너뜀");
                        fail += 1;
                        continue;
                    }
                };
                let seat_cwd = target_cwd.clone();
                match boot_agent_on_surface(
                    sid,
                    role,
                    agent,
                    &spec,
                    !no_resume,
                    sess.as_deref(),
                    false,
                    seat_cwd.as_deref(),
                    cfg.as_deref(),
                ) {
                    Ok(BootVerdict::Ready) => {
                        ok += 1;
                        let directive = restore_directive(role);
                        // ★(N7) 좌석 내 재연결은 성공했어도 복원 디렉티브가 유실될 수 있다 —
                        //   종전엔 그 유실이 조용해서 '재기동 ok' 로만 집계됐다. 방향은 무변.
                        if let Err(e) = inject_text(sid, directive) {
                            eprintln!(
                                "· {role}: 좌석 내 재연결은 됐으나 **복원 디렉티브 미주입** \
                                 (좌석 보존 · 계속 진행) — {e}"
                            );
                        }
                        continue;
                    }
                    // ★(U-11) 이 호출부의 귀결은 앞의 둘과 또 다르다 — **fresh 폴백을 하지 않는다**.
                    //   폴백하면 살아 있는(관문에 갇힌) 에이전트가 이미 쥔 역할로 새 surface 를
                    //   만들게 되고, 결과는 claim_denied 아니면 좌석 증식이다. 게다가 새 pane 은
                    //   같은 프로필로 같은 관문에 다시 들어가므로 **관문 재진입 루프**(폭주)의
                    //   씨앗이다. 복원의 정답은 "그 좌석을 사람이 통과시키게 두는 것" 이다.
                    Ok(BootVerdict::GatePending { gate, tail }) => {
                        gated += 1;
                        print_gate_pending_prescription(sid, role, agent, &gate, &tail);
                        println!(
                            "· {role}: 좌석 내 재연결 보류(관문 gate={gate}) — 좌석 보존 · 주입 0 · fresh 폴백 0"
                        );
                        continue;
                    }
                    Ok(BootVerdict::LaunchFailed { evidence }) => {
                        // in-seat 실패는 치명이 아니다 — fresh 폴백이 가용성을 지킨다(정직히 알린다).
                        println!("· {role}: 좌석 내 재연결 실패({evidence}) — fresh 기동으로 폴백");
                    }
                    Err(e) => {
                        // in-seat 실패는 치명이 아니다 — fresh 폴백이 가용성을 지킨다(정직히 알린다).
                        println!("· {role}: 좌석 내 재연결 실패({e}) — fresh 기동으로 폴백");
                    }
                }
            }
            println!("· {role}: {agent} 재기동…");
            let rc = run_launch_agent_opts(role, agent, target_cwd, !no_resume, sess, true, cfg);
            if rc == cys::EXIT_GATE_PENDING {
                // 새 pane 은 떴고 프로세스도 살아 있다 — 닫지 않고, 디렉티브도 넣지 않는다.
                // (처방 문안은 run_launch_agent_opts 가 stderr 로 이미 냈다.)
                gated += 1;
                println!("· {role}: 관문 보류 — 좌석 보존 · 주입 0(사람 1회 조치 후 `cys boot`)");
            } else if rc == 0 {
                ok += 1;
                if let Ok(r) = request("system.resolve_role", json!({"role": role})) {
                    if let Some(sid) = r["surface_id"].as_u64() {
                        // ⑪ pack-reinject 마커 seed — session_id를 resume 핀으로 복원하는 것과
                        // 동일 지점. 영속된 마커를 재생성 surface에 reinject.mark(단일 write path)로
                        // 다시 심어, 복원 후에도 동일 팩 버전 중복 재주입을 막는다. 부재(구 topology)면 skip.
                        if let (Some(pv), Some(dh)) = (
                            entry["pack_reinject"]["pack_version"].as_str(),
                            entry["pack_reinject"]["directive_hash"].as_str(),
                        ) {
                            let _ = request(
                                "reinject.mark",
                                json!({"surface_id": sid, "pack_version": pv, "directive_hash": dh}),
                            );
                        }
                        // ★(N7) 재기동 뒤 복원 디렉티브 유실도 조용하지 않다(방향 무변).
                        if let Err(e) = inject_text(sid, restore_directive(role)) {
                            eprintln!(
                                "· {role}: 재기동은 됐으나 **복원 디렉티브 미주입** \
                                 (좌석 보존 · 계속 진행) — {e}"
                            );
                        }
                    }
                }
            } else {
                fail += 1;
                println!("· {role}: 기동 실패 — 나머지 역할 계속 진행");
            }
        }
        Ok((ok, fail, gated))
    })();
    match result {
        Ok((ok, fail, gated)) => {
            println!(
                "restore 완료: 재기동 {ok} · 실패 {fail} · 관문 보류 {gated} · 현황 `cys status`"
            );
            if fail > 0 || gated > 0 {
                1
            } else {
                0
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// T2-7 디렉티브 드리프트 감지·재주입: --check면 각성 핑 먼저, 무응답 시에만 재주입
/// Tier R check-path 빈 셸 게이트(순수 함수·단위테스트 대상). surface_entry(topology) 엔트리에서
/// live agent 부재를 판정한다: live agent = agent 등록 present ∧ !exited ∧ agent_alive(데몬이
/// agent_seen ∧ !agent_exit_notified로 산출). 부재(빈 셸·크래시투셸·미부팅)면 true(=check-ping과
/// fall-through 주입을 둘 다 skip). 실 topology 엔트리는 exited/agent/agent_alive를 항상 포함한다;
/// 조회 자체 실패는 상위 surface_entry(sid)? 가 이미 처리하므로 이 게이트가 새 조회를 하지 않는다.
/// forced reinject(check=false)에는 호출하지 않는다 — CEO 강제주입 skip 금지.
fn reinject_check_should_skip_bare_shell(entry: &Value) -> bool {
    let agent_present = entry["agent"].is_string();
    let not_exited = entry["exited"].as_bool() != Some(true);
    let agent_live = entry["agent_alive"].as_bool() == Some(true);
    !(agent_present && not_exited && agent_live)
}

fn run_reinject(
    role: Option<String>,
    surface: Option<String>,
    check: bool,
    timeout: u64,
) -> i32 {
    let result = (|| -> Result<(), String> {
        let sid = resolve_role_or_surface(&role, &surface)?;
        let entry = surface_entry(sid)?;
        let role_name = role
            .clone()
            .or_else(|| entry["role"].as_str().map(String::from))
            .ok_or("role 미상 — --role 지정 필요")?;
        if check {
            // ── Tier R gate(에러①): 빈 셸(라이브 에이전트 부재)이면 핑·fall-through 주입 둘 다 skip. ──
            // 크래시투셸(exited=true)·미부팅 bare 셸에 디렉티브 전문을 뿌리는 소음/오염을 차단한다.
            // 블록 최상단 early-return이라 핑(inject_text) 前에 빠져나가 fall-through 주입도 안 일어난다.
            // forced reinject(check=false)는 이 블록 밖이라 CEO 강제주입이 유지된다(skip 금지).
            if reinject_check_should_skip_bare_shell(&entry) {
                println!("빈 셸(라이브 에이전트 부재) — check reinject skip (surface:{sid})");
                return Ok(());
            }
            // 마커를 핑 텍스트에 통째로 넣지 않는다 — 주입 텍스트의 터미널 에코가
            // wait_for에 매칭되는 false ACK(자기-에코 오탐)를 차단 (토큰 분리 조합 지시)
            let marker = format!("DIRECTIVE-ACK-{}", std::process::id());
            let cursor = request("surface.read_text", json!({"surface_id": sid}))?
                ["latest_cursor"]
                .as_u64()
                .unwrap_or(0);
            inject_text(sid, &format!("지침 각성 확인 핑: 너의 절대지침(디렉티브)이 컨텍스트에 살아있다면, 다음 두 토큰을 공백 없이 이어붙인 한 줄을 plain으로 출력하라: 'DIRECTIVE-ACK-' 그리고 '{}'", std::process::id()))?;
            let r = request(
                "surface.wait_for",
                json!({"surface_id": sid, "pattern": marker,
                       "timeout_secs": timeout, "since_line": cursor}),
            )?;
            if r["matched"].as_bool() == Some(true) {
                println!("디렉티브 생존 확인 (ACK 수신) — 재주입 불필요");
                return Ok(());
            }
            eprintln!("[reinject] ACK 없음 ({timeout}s) — 드리프트 판정, 재주입 진행");
        }
        let directive = compose_directive(&role_name)?;
        inject_text(sid, &directive)?;
        println!(
            "reinjected {} bytes → surface:{sid} ({role_name})",
            directive.len()
        );
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 무중단 팩 업데이트 (cys pack-update, DESIGN-noshutdown-pack-update §2-②/§7)
// ─────────────────────────────────────────────────────────────────────────────

/// 버전 3축 게이트(§7-④) 판정. 순수 함수 — 단위테스트 대상.
#[derive(Debug, Clone, PartialEq, Eq)]
enum VersionGate {
    /// remote 신버전 + 바이너리 호환 → 반영.
    Apply,
    /// remote가 디스크보다 새것이 아님(파싱 실패 포함) → 멱등 no-op.
    UpToDate,
    /// min_binary_version > 실행 바이너리 → 무중단 거부(바이너리 재시작 경로 안내).
    BinaryTooOld,
}

/// 3축 버전 비교(§7-④ + free/pro v6 §3 튜플 확장) — remote→disk 반영 판정
/// ((base semver, pro_revision) 튜플 strictly-newer, fail-CLOSED) ∧ remote→running
/// 호환 게이트(min_binary ≤ running). disk→embed 다운그레이드 가드는 install_from_iter가 담당.
/// min_binary가 빈 문자열이면 제약 없음(manifest #[serde(default)] 호환 — 단 channel=pro는
/// packsig ⓐ-2가 min_binary 필수를 이미 강제해 여기 도달 전 거부된다).
fn version_gates(
    remote_pack: (&str, u32),
    disk_pack: (&str, u32),
    min_binary: &str,
    running: &str,
) -> VersionGate {
    // 축1 반영 판정: remote 튜플이 디스크 튜플보다 strictly-newer 여야(파싱 실패=거부=no-op).
    if !cys::pack::remote_is_newer_tuple(remote_pack, disk_pack) {
        return VersionGate::UpToDate;
    }
    // 축2 호환 게이트: min_binary ≤ running. 빈 값=제약 없음. 파싱 실패·초과=거부.
    let min = min_binary.trim();
    if min.is_empty() {
        return VersionGate::Apply;
    }
    match (cys::pack::parse_semver(min), cys::pack::parse_semver(running)) {
        (Some(m), Some(r)) if m <= r => VersionGate::Apply,
        _ => VersionGate::BinaryTooOld,
    }
}

/// surface별 마지막 reinject 마커(P3 reinject.mark가 set, system.topology가 노출).
#[derive(Debug, Clone)]
struct ReinjectMarker {
    pack_version: String,
    directive_hash: String,
}

/// reinject 3단 게이트(§7-②) 결정. 순수 함수 — 단위테스트 대상.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReinjectDecision {
    /// 디렉티브 변경 + idle/ready + 신버전 → 주입.
    Inject,
    /// ⓐ해시 선검사: 합성 디렉티브 해시 == 마커 해시 → 주입 자체 스킵(토큰 0).
    SkipUnchanged,
    /// ⓒ버전 dedup: 마커 pack_version >= 새 버전 → 이미 주입됨, 스킵.
    SkipDedup,
    /// ⓑidle 게이트 미통과(working/미준비) → 다음 폴링까지 보류.
    Defer,
}

/// run_pack_reinject 집계 보고. injected/skipped/deferred/failed 카운트에 더해, busy로 보류된
/// 노드(surface_id, role) 목록을 함께 실어 pending 영속(다음 pack-update 재시도 가시화)에 쓴다.
#[derive(Debug, Default, PartialEq, Eq)]
struct ReinjectReport {
    injected: usize,
    skipped: usize,
    deferred: usize,
    failed: usize,
    /// Defer로 판정된 라이브 노드들 — pending 파일에 (surface_id, role)로 영속한다.
    deferred_nodes: Vec<(u64, String)>,
}

/// deferred reinject 대상 영속 경로 — pack_state_base(=~/.cys) 아래 .pack-reinject-pending.json.
fn reinject_pending_path(base: &std::path::Path) -> std::path::PathBuf {
    base.join(".pack-reinject-pending.json")
}

/// deferred(busy) 노드를 pending 파일에 영속하거나(>0), 더 이상 없으면 stale pending을 제거한다(0).
/// {pack_version, deferred:[{surface_id, role}]} 형식. 디스크 반영·reinject 성공 여부와 독립한
/// 가시화/재시도 힌트라 best-effort(critical 아님)다. 다음 pack-update는 토폴로지 마커를 새로 읽어
/// deferred 노드를 자연히 재평가(재주입)하므로, 이 파일은 외부 재시도·관측용 SOT다.
fn persist_reinject_pending(
    base: &std::path::Path,
    pack_version: &str,
    deferred_nodes: &[(u64, String)],
) -> std::io::Result<()> {
    let path = reinject_pending_path(base);
    if deferred_nodes.is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    } else {
        let nodes: Vec<serde_json::Value> = deferred_nodes
            .iter()
            .map(|(sid, role)| json!({"surface_id": sid, "role": role}))
            .collect();
        let doc = json!({"pack_version": pack_version, "deferred": nodes});
        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap_or_default())
    }
}

/// pending 파일(.pack-reinject-pending.json)을 읽어 (pack_version, [(surface_id, role)])로 파싱한다.
/// 파일 부재 → Ok(None). 손상(JSON 파싱 불가·pack_version 부재) → Ok(None)(best-effort: 손상 pending은
/// 무시하고 다음 pack-update가 새로 기록). LOW#1 능동 소비 경로의 reader (persist_reinject_pending의 역).
fn read_reinject_pending(
    base: &std::path::Path,
) -> std::io::Result<Option<(String, Vec<(u64, String)>)>> {
    let path = reinject_pending_path(base);
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&raw) else { return Ok(None) };
    let ver = doc["pack_version"].as_str().unwrap_or_default().to_string();
    if ver.is_empty() {
        return Ok(None);
    }
    let nodes = doc["deferred"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|n| {
                    let sid = n["surface_id"].as_u64()?;
                    let role = n["role"].as_str()?.to_string();
                    Some((sid, role))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(Some((ver, nodes)))
}

/// reinject 집계 → pack-update 종료코드. failed>0이면 EXIT_REINJECT_DEGRADED(디스크는 반영됐으나
/// 라이브 일부 미각성 — 성공 침묵 포장 금지), 아니면 0(deferred만 있어도 디스크 반영은 성공이라 0).
fn reinject_exit_code(failed: usize) -> i32 {
    if failed > 0 {
        cys::pack::EXIT_REINJECT_DEGRADED
    } else {
        0
    }
}

/// reinject 결정(§7-② 순서 고정): ⓐ해시 선검사(SkipUnchanged) → ⓒ버전 dedup(SkipDedup) →
/// ⓑidle 게이트(Defer) → Inject. 스킵(terminal)을 보류(Defer)보다 먼저 판정해, 주입할 게
/// 없는 노드를 헛되이 deferral 시키지 않는다.
/// ⓑ idle 게이트는 §7-② step2의 3신호 AND다: `idle`(ⓐ derive_node_state==idle) ∧
/// `self_idle`(ⓑ 자기보고 agent_status≠working) ∧ `ready`(ⓒ 어댑터 prompt-ready). 셋 중 하나라도
/// 불충족이면 Defer — long-thinking·자기보고 working 노드의 강제 주입(컨텍스트 오염)을 차단한다.
fn reinject_decision(
    marker: Option<&ReinjectMarker>,
    new_ver: &str,
    new_hash: &str,
    idle: bool,
    self_idle: bool,
    ready: bool,
) -> ReinjectDecision {
    // ⓐ 해시 선검사 — 디렉티브 무변경이면 주입 불요(스킬/스크립트만 바뀐 릴리스).
    if let Some(m) = marker {
        if m.directive_hash == new_hash {
            return ReinjectDecision::SkipUnchanged;
        }
        // ⓒ 버전 dedup — 같은(또는 더 높은) 버전을 이미 주입한 노드는 재주입 안 함.
        if let (Some(mv), Some(nv)) =
            (cys::pack::parse_semver(&m.pack_version), cys::pack::parse_semver(new_ver))
        {
            if mv >= nv {
                return ReinjectDecision::SkipDedup;
            }
        }
    }
    // ⓑ idle 게이트(§7-② step2 3신호 AND) — derive_node_state idle ∧ 자기보고≠working ∧ 준비됨.
    // 하나라도 불충족(busy·자기보고 working·미보고·미준비) = 보류(컨텍스트 오염 차단).
    if !(idle && self_idle && ready) {
        return ReinjectDecision::Defer;
    }
    ReinjectDecision::Inject
}

/// sha256 hex — 디렉티브 해시(§7-② ⓐ 선검사용). pack.rs content_hash와 동일 산식.
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

/// 임베드 PACK+PACK_SKILLS에서 권위 manifest Value를 산출(DESIGN-noshutdown §2-①). files는
/// rel→sha256(content_hash 동일산식: sha256_hex). 임베드 콘텐츠에서 파생되므로 standalone 팩
/// manifest의 단일 SOT다(같은 cysjavis-pack/ 소스 → tree와 일치 보장). key_id/signed_at/expires_at는
/// 주입되면 채우고 미지정이면 생략한다(CI 서명단계가 채움 — 미서명 manifest는 packsig 필수필드라
/// 무중단 검증에서 거부됨). 결정론: files는 BTreeMap(정렬), top-level은 serde_json Map(정렬).
fn build_pack_manifest_value(
    key_id: Option<String>,
    signed_at: Option<i64>,
    expires_at: Option<i64>,
    min_binary_version: &str,
    pack_version: Option<&str>,
) -> serde_json::Value {
    let mut files: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (rel, content) in cys::pack::PACK.iter().chain(cys::pack::PACK_SKILLS.iter()) {
        files.insert((*rel).to_string(), sha256_hex(content));
    }
    let mut obj = serde_json::Map::new();
    // 팩-only 릴리스 레인(2026-07-13 오너 승인): pack_version을 바이너리 버전과 분리 지정 가능.
    // 미지정=기존과 바이트 동일(CARGO_PKG_VERSION) — 본체 릴리스 경로 회귀 0.
    obj.insert(
        "pack_version".into(),
        json!(pack_version.unwrap_or(env!("CARGO_PKG_VERSION"))),
    );
    obj.insert("min_binary_version".into(), json!(min_binary_version));
    if let Some(k) = key_id {
        obj.insert("key_id".into(), json!(k));
    }
    if let Some(s) = signed_at {
        obj.insert("signed_at".into(), json!(s));
    }
    if let Some(e) = expires_at {
        obj.insert("expires_at".into(), json!(e));
    }
    obj.insert("files".into(), json!(files));
    serde_json::Value::Object(obj)
}

/// `cys pack-manifest` 진입점 — 권위 manifest를 stdout으로 방출(§2-①). CI가 standalone 팩
/// manifest.json의 단일 SOT로 캡처한다.
fn run_pack_manifest(
    key_id: Option<String>,
    signed_at: Option<i64>,
    expires_at: Option<i64>,
    min_binary_version: &str,
    pack_version: Option<String>,
) -> i32 {
    // 오버라이드는 semver 파싱 가능해야 한다(fail-loud) — 비교 게이트(check_pack_update·
    // version_gates)가 파싱 불가 버전을 만나 무음 오동작하는 경로를 방출 시점에 차단.
    if let Some(ref pv) = pack_version {
        if cys::pack::parse_semver(pv).is_none() {
            eprintln!("[pack-manifest] --pack-version 파싱 불가(semver 아님): {pv:?}");
            return 2;
        }
    }
    let v = build_pack_manifest_value(key_id, signed_at, expires_at, min_binary_version,
                                      pack_version.as_deref());
    match serde_json::to_string_pretty(&v) {
        Ok(s) => {
            println!("{s}");
            0
        }
        Err(e) => {
            eprintln!("[pack-manifest] 직렬화 실패: {e}");
            1
        }
    }
}

/// tar.gz를 dest에 in-Rust로 하드닝 전개(WP-6 R-SIG-1 ③-2). 외부 `tar -xzf`는 심링크/`..`/절대경로
/// 하드닝 플래그가 0이라 미검증 엔트리가 staging 밖으로 traversal-write할 수 있었다. 여기서는
/// tar+flate2로 ★엔트리별★ 검증한다: 정규 파일·디렉터리만 허용하고 심링크·하드링크·디바이스·FIFO·
/// 소켓 등 특수 엔트리와 절대경로·`..`·루트/prefix 성분·staging 경계 이탈 경로를 전건 fail-closed
/// 거부한다. 소유자·setuid 등 특수비트는 승계하지 않는다(File::create 기본 — `--no-same-owner` 동치).
fn extract_tar_gz(tar_gz: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("staging 생성 실패 {}: {e}", dest.display()))?;
    // dest를 정규화(canonicalize)해 심링크·상대성분 없는 경계 기준을 확보한다.
    let dest_canon = std::fs::canonicalize(dest)
        .map_err(|e| format!("staging 정규화 실패 {}: {e}", dest.display()))?;
    let f = std::fs::File::open(tar_gz)
        .map_err(|e| format!("tar 열기 실패 {}: {e}", tar_gz.display()))?;
    let gz = flate2::read::GzDecoder::new(std::io::BufReader::new(f));
    let mut ar = tar::Archive::new(gz);
    // ★unpack 편의함수(심링크/소유자 따라감) 대신 엔트리별 수동 처리 — 하드닝의 핵심.
    let entries = ar.entries().map_err(|e| format!("tar 엔트리 열거 실패: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("tar 엔트리 읽기 실패: {e}"))?;
        let etype = entry.header().entry_type();
        // ── 타입 게이트: 정규 파일·디렉터리만. 그 외(심링크/하드링크/디바이스/FIFO/…) 전건 거부. ──
        let is_dir = etype.is_dir();
        let is_regular = matches!(etype, tar::EntryType::Regular);
        if !is_dir && !is_regular {
            let name = entry
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            return Err(format!(
                "위험 tar 엔트리 타입 {etype:?} 거부(심링크/하드링크/특수파일): {name}"
            ));
        }
        // ── 경로 게이트: Normal/CurDir 성분만. 절대경로·`..`(ParentDir)·루트/prefix 거부. ──
        let raw = entry
            .path()
            .map_err(|e| format!("tar 경로 파싱 실패: {e}"))?
            .into_owned();
        for comp in raw.components() {
            match comp {
                std::path::Component::Normal(_) | std::path::Component::CurDir => {}
                _ => {
                    return Err(format!(
                        "위험 tar 경로 성분(절대/../루트/prefix) 거부: {}",
                        raw.display()
                    ));
                }
            }
        }
        let target = dest_canon.join(&raw);
        // 방어심층: 성분검사 우회 대비 join 결과가 staging 경계 밖이면 거부.
        if !target.starts_with(&dest_canon) {
            return Err(format!("tar 경로가 staging 경계 이탈: {}", raw.display()));
        }
        if is_dir {
            std::fs::create_dir_all(&target)
                .map_err(|e| format!("디렉터리 생성 실패 {}: {e}", target.display()))?;
            continue;
        }
        // 정규 파일: 부모 생성 후 내용 복사. 아카이브 내 심링크는 위 타입게이트가 전건 거부하므로
        // create_dir_all이 아카이브발 심링크 부모를 따라갈 여지가 없다.
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("부모 생성 실패 {}: {e}", parent.display()))?;
        }
        let mut out = std::fs::File::create(&target)
            .map_err(|e| format!("파일 생성 실패 {}: {e}", target.display()))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("파일 쓰기 실패 {}: {e}", target.display()))?;
    }
    Ok(())
}

/// staging 트리를 (rel, content) 쌍으로 수집(install_from_iter 입력원). 모든 팩 파일은 UTF-8
/// 텍스트(디렉티브·json·py·sh) — 비UTF8 파일은 fail-closed 에러. 디렉터리 재귀 walk.
fn collect_tree(root: &std::path::Path) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    fn walk(
        base: &std::path::Path,
        dir: &std::path::Path,
        out: &mut Vec<(String, String)>,
    ) -> Result<(), String> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("read_dir 실패 {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("dir entry 실패: {e}"))?;
            let path = entry.path();
            let ft = entry.file_type().map_err(|e| format!("file_type 실패: {e}"))?;
            if ft.is_dir() {
                walk(base, &path, out)?;
            } else if ft.is_file() {
                let rel = path
                    .strip_prefix(base)
                    .map_err(|e| format!("rel 경로 실패: {e}"))?
                    .to_string_lossy()
                    .replace('\\', "/");
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| format!("비UTF8/읽기 실패 {}: {e}", path.display()))?;
                out.push((rel, content));
            }
        }
        Ok(())
    }
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// flock(LOCK_EX) 임계영역에서 f를 실행(§7-⑧ 폴백 apply-lock — per-file write_atomic + writer 배타).
/// non-unix는 잠금 없이 실행.
///
/// ⚠보장 범위(정직 명시 · 층위 분리):
/// 1) 이 락은 **writer 측 상호배제(serialization)만** 제공한다 — 동시 writer가 같은 pack_dir를
///    겹쳐 쓰는 것을 직렬화할 뿐이다.
/// 2) **트랜잭션 rollback/commit marker는 이 락의 책임이 아니라 apply_pack_transactional의 책임이다**
///    — backup journal + `.pack-version` hard commit marker로 부분커밋 0(all-or-nothing)을 보장한다
///    (pack-update 경로). 이 락은 그 트랜잭션을 writer 배타 창 안에서 단독 실행시키는 역할만 한다.
/// 3) 그러나 §6-4 심링크(pack_dir) 1회 마이그레이션이 보류된 현재(디렉터리 일괄 atomic 스왑 미구현),
///    **외부 동시 live READER의 snapshot atomic(multi-file SET 일관성·torn-read)은 여전히 보장되지
///    않는다.** §7-⑧ 폴백이 요구한 reader-측 차단(공유 flock)을 load-bearing 리더(compose_directive —
///    MASTER_DIRECTIVE/soul.md/MEMORY.md/각 SKILL.md 순차 읽기 · Tauri read_board_catalog)가 취하지
///    않기 때문이다. 그 결과 apply 창 동안 외부 리더는 신규-directive + 구-soul 같은 혼재(torn) 집합을
///    관측할 수 있다. pack-update 자신의 reinject는 apply 이후 실행되어 안전하고, 노출 대상은 외부 동시
///    리더뿐이다. 진짜 reader 집합 원자성은 §6-4 심링크 스왑 도입 시 확보된다.
fn with_apply_lock<T>(lock_path: &std::path::Path, f: impl FnOnce() -> T) -> Result<T, String> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // CI fresh 환경엔 ~/.cys/ 가 없어 lock 파일 open이 ENOENT로 실패한다.
        // 락 파일 열기 직전 부모 디렉토리를 보장한다(이미 있으면 무해).
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("apply-lock 부모 디렉토리 생성 실패 {}: {e}", parent.display())
            })?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock_path)
            .map_err(|e| format!("apply-lock 열기 실패 {}: {e}", lock_path.display()))?;
        let fd = file.as_raw_fd();
        if unsafe { libc::flock(fd, libc::LOCK_EX) } != 0 {
            return Err(format!("flock 실패: {}", std::io::Error::last_os_error()));
        }
        let out = f();
        unsafe {
            libc::flock(fd, libc::LOCK_UN);
        }
        Ok(out)
    }
    #[cfg(not(unix))]
    {
        let _ = lock_path;
        Ok(f())
    }
}

/// pack-update 코어 결과(§2-② 흐름 1~5). reinject(6)는 라이브 데몬 단계로 분리.
#[derive(Debug, Clone)]
struct PackUpdateOutcome {
    gate: VersionGate,
    pack_version: String,
    written: usize,
    kept: usize,
    /// post-commit accepted 기록 성공 여부(v5 §3) — false = 디스크 반영은 성공했으나 replay
    /// 기준선이 낡음. run_pack_update가 EXIT_ACCEPTED_DEGRADED로 구분 보고(침묵 포장 금지).
    accepted_recorded: bool,
}

/// `--from` 핵심 경로(검증+버전게이트+apply). 테스트 가능: keyring/now/running/accepted_path를
/// 주입받고 라이브 데몬·embed 상수에 의존하지 않는다(do_apply=false면 검증·게이트만).
/// 순서(§2-②): 소스읽기→staging 압축해제→서명검증(P2 fail-closed)→파일 sha256 대조→버전 3축
/// 게이트→apply-lock+apply_pack_transactional(backup journal→install_from_iter→record_accepted[필수]
/// →.pack-version commit marker→저널 삭제; 실패 시 rollback·부분적용 0).
fn pack_update_from_dir(
    from_dir: &std::path::Path,
    staging: &std::path::Path,
    lock_path: &std::path::Path,
    accepted_path: &std::path::Path,
    now_unix: i64,
    running_binary: &str,
    keyring: &cys::packsig::Keyring,
    do_apply: bool,
) -> Result<PackUpdateOutcome, String> {
    let manifest_path = from_dir.join("pack-manifest.json");
    let sig_path = from_dir.join("pack-manifest.json.minisig");
    let tar_path = from_dir.join("pack.tar.gz");
    let manifest_bytes = std::fs::read(&manifest_path)
        .map_err(|e| format!("manifest 읽기 실패 {}: {e}", manifest_path.display()))?;
    let sig_bytes = std::fs::read(&sig_path)
        .map_err(|e| format!("서명 읽기 실패 {}: {e}", sig_path.display()))?;

    // ── WP-6 R-SIG-1 재배치: 서명·digest 검증을 tar 전개 ★이전★에 수행한다. 미검증 tarball의
    //    심링크/`..` 엔트리가 서명검증 전 staging 밖으로 pre-auth 임의쓰기하던 CRIT을 차단한다. ──
    // ⓐ 서명·신선도·replay 검증(P2, fail-closed) — 전개 전. staging 무변경 상태에서 fail-closed.
    let manifest = cys::packsig::verify_with_keyring(
        &manifest_bytes,
        &sig_bytes,
        now_unix,
        accepted_path,
        keyring,
    )
    .map_err(|e| format!("manifest 검증 실패: {e}"))?;

    // ⓐ' tar.gz digest 대조(전개 전) — 서명된 manifest.digest와 실제 tarball sha256 일치 강제.
    //     digest는 서명 안에 있어 forge 불가라 tar↔서명을 이 한 줄이 바인딩한다. digest 비어있음 =
    //     cutover 이전 서명본(verify가 signed_at<cutover만 허용) → 대조 불가라 skip(하위호환).
    if !manifest.digest.trim().is_empty() {
        let tar_sha = sha256_file(&tar_path.to_string_lossy())
            .ok_or_else(|| format!("tar.gz sha256 산출 실패: {}", tar_path.display()))?;
        if tar_sha != manifest.digest.trim() {
            return Err(format!(
                "tar.gz digest 불일치: 기대 {} 실제 {tar_sha} — 미검증/변조 tarball 거부",
                manifest.digest.trim()
            ));
        }
    }

    // ⓑ 검증 통과 후에만 staging 비우고 ★하드닝★ 전개(엔트리별 절대/../심링크/하드링크/특수파일 거부).
    let _ = std::fs::remove_dir_all(staging);
    extract_tar_gz(&tar_path, staging)?;

    // ⓒ 파일별 sha256 대조(P2 verify_files) — manifest.files → staging 전방 무결성.
    if let Err(e) = cys::packsig::verify_files(&manifest, staging) {
        let _ = std::fs::remove_dir_all(staging);
        return Err(format!("파일 무결성 검증 실패: {e}"));
    }

    // ⓒ' 역방향 커버리지(§7-①) — staging 트리의 전 파일이 서명 manifest.files에 등재돼야.
    // tarball 미서명이라 전방 검증만으로는 미등재 파일 추가 변조(악성 bin/*.py 등)를 못 막는다.
    // 전방+역방향으로 manifest ⇔ staging 집합 동치를 강제(fail-closed) — install_from_iter 진입 전 차단.
    if let Err(e) = cys::packsig::verify_no_extra_files(&manifest, staging) {
        let _ = std::fs::remove_dir_all(staging);
        return Err(format!("staging 트리 커버리지 검증 실패: {e}"));
    }

    // ─ free/pro 채널·상태 게이트(v6 §3·§5) — 버전 게이트 전에 디스크 상태를 확정한다. ─
    let pack_dir = cys::pack::pack_dir();
    let disk_version = std::fs::read_to_string(pack_dir.join(".pack-version"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let (disk_channel, disk_pro_rev) = match cys::pack::read_pack_state(&pack_dir) {
        cys::pack::PackStateRead::Absent => ("free".to_string(), 0u32),
        cys::pack::PackStateRead::Corrupt(e) => {
            // 손상 상태의 튜플은 신뢰 불가 — typed 거부, repair 선행 요구(v4 §5).
            return Err(format!(
                "[pack-state-corrupt] .pack-state.json 손상({e}) — pack-update 거부. \
                 cys pack-repair-channel 로 복구 후 재시도하라"
            ));
        }
        cys::pack::PackStateRead::Valid(st) => {
            if st.base_version != disk_version {
                // 정합 불일치 = 손상 간주(v4 §3). cysd 기동/init-pack의 제한적 자가치유가
                // 선행 경로 — pack-update는 보수적으로 거부한다.
                return Err(format!(
                    "[pack-state-mismatch] state.base {:?} ≠ .pack-version {:?} — pack-update 거부. \
                     cys init-pack(자가치유) 또는 cys pack-repair-channel 후 재시도하라",
                    st.base_version, disk_version
                ));
            }
            (st.channel, st.pro_revision)
        }
    };
    // 채널 전이 규칙: pro 설치에 free 번들 = 다운그레이드 시도 — 전용 명령만 허용(v2 §5).
    if disk_channel == "pro" && manifest.channel == "free" {
        return Err(
            "[pack-channel-refused] pro 설치에 free 번들 — pro→free 전환은 \
             cys pack-downgrade-to-free 전용 명령만 허용된다"
                .to_string(),
        );
    }

    // 버전 3축 게이트(§7-④ · v6 튜플).
    let gate = version_gates(
        (&manifest.pack_version, manifest.pro_revision),
        (&disk_version, disk_pro_rev),
        &manifest.min_binary_version,
        running_binary,
    );

    let mut written = 0;
    let mut kept = 0;
    let mut accepted_recorded = true;
    if gate == VersionGate::Apply && do_apply {
        // 반영: apply-lock 배타 → apply_pack_transactional(backup journal → install_from_iter →
        // .pack-state.json[journal 편입] → .pack-version=마지막 hard commit marker →
        // ★post-commit record_accepted(v4 — R3 codex blocking 결착: 커밋 이후로 이동. 실패 =
        // rollback 없음·loud·EXIT_ACCEPTED_DEGRADED 구분 보고·self-heal 수렴) → 저널 삭제.
        let tree = collect_tree(staging)?;
        let pv = manifest.pack_version.clone();
        let manifest_acc = manifest.clone();
        let acc_path = accepted_path.to_path_buf();
        let new_state = cys::pack::PackState {
            channel: manifest.channel.clone(),
            base_version: manifest.pack_version.clone(),
            pro_revision: manifest.pro_revision,
        };
        let res = with_apply_lock(lock_path, move || {
            let items: Vec<(&str, &str)> =
                tree.iter().map(|(r, c)| (r.as_str(), c.as_str())).collect();
            // W0-d: pack-update는 라이브 팩 쓰기 프로덕션 진입점 — 인가 부여.
            cys::pack::apply_pack_transactional(
                &items,
                &pv,
                &new_state,
                Some(cys::pack::PackWriteAuth::production()),
                || cys::packsig::record_accepted(&acc_path, &manifest_acc),
            )
        })?;
        let (w, k, post_ok) = res?;
        written = w;
        kept = k;
        accepted_recorded = post_ok;
    } else if gate == VersionGate::UpToDate
        && do_apply
        && manifest.channel == disk_channel
        && manifest.pro_revision == disk_pro_rev
        && manifest.pack_version == disk_version
    {
        // ─ self-heal(v5 §3 — 4조건·apply lock 보유 중): 동일 튜플 + 더 새 서명(1차 게이트가
        // 이미 보장 — 낡은 signed_at이면 verify가 replay 거부) 번들로 accepted 기준선만 수렴.
        // 조건③ "적용된 콘텐츠 == manifest.files"의 판정 기준 = `.install-manifest.json`
        // (설치-당시 해시 기록 = '무엇이 적용됐나'의 SOT). 라이브 디스크 대조는 정당한 사용자
        // 수정 파일(preserve-gate 철학)이 오탐을 만든다 — 구현 정밀화. 불일치 = **self-heal
        // 거부**(accepted 미갱신 = 드리프트 은닉 없음·R4 codex 결착) + loud typed 진단.
        // 명령 자체는 UpToDate no-op 성공(무해 케이스: 구설치본·재제안 번들을 에러로 만들지 않음).
        let manifest_acc = manifest.clone();
        let acc_path = accepted_path.to_path_buf();
        let pd = pack_dir.clone();
        with_apply_lock(lock_path, move || {
            let installed: Option<std::collections::BTreeMap<String, String>> =
                std::fs::read_to_string(pd.join(".install-manifest.json"))
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok());
            match installed {
                Some(m) if m == manifest_acc.files => {
                    match cys::packsig::record_accepted(&acc_path, &manifest_acc) {
                        Ok(()) => eprintln!(
                            "[pack-update] self-heal: 동일 튜플·적용 콘텐츠 일치 — accepted 기준선 갱신"
                        ),
                        Err(e) => eprintln!("[pack-update] ⚠ self-heal accepted 기록 실패: {e}"),
                    }
                }
                Some(_) => eprintln!(
                    "[pack-update] ⚠ same-version-content-mismatch: 동일 튜플 번들의 파일 해시가 \
                     설치 기록(.install-manifest.json)과 불일치 — self-heal 거부(기준선 미갱신 = \
                     드리프트 은닉 없음). 재서명 드리프트면 새 pro_revision 발급이 필요하다."
                ),
                None => eprintln!(
                    "[pack-update] self-heal 생략: 설치 기록 부재(구설치본) — 기준선 미갱신."
                ),
            }
        })?;
    }

    Ok(PackUpdateOutcome {
        gate,
        pack_version: manifest.pack_version,
        written,
        kept,
        accepted_recorded,
    })
}

/// ~/.cys (pack_dir의 부모) — 무중단 채널 상태파일(.pack-staging·.pack-apply.lock·.pack-accepted.json) 루트.
fn pack_state_base() -> std::path::PathBuf {
    cys::pack::pack_dir()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// `cys pack-downgrade-to-free`(free/pro v3 §5) — 유일한 pro→free 전환 경로. license-aware:
/// 유효 pro 라이선스 실재 시 기본 거부(--override-valid-license로만 통과). 실행 = state를
/// free로 전환 후 내장 팩 재설치(prune이 pro 전용 파일 제거 — 의도된 강등 동작).
fn run_pack_downgrade_to_free(yes: bool, override_valid_license: bool) -> i32 {
    let dir = cys::pack::pack_dir();
    let now = chrono::Utc::now().timestamp();
    let license_line = cys::license::render_status(now);
    println!("라이선스: {license_line}");
    let st = match cys::pack::read_pack_state(&dir) {
        cys::pack::PackStateRead::Absent => {
            println!("팩 상태: state 부재(=free) — 강등 대상 없음. no-op.");
            return 0;
        }
        cys::pack::PackStateRead::Valid(st) if st.channel == "free" => {
            println!("팩 상태: 이미 channel=free (base {}) — no-op.", st.base_version);
            return 0;
        }
        cys::pack::PackStateRead::Valid(st) => st,
        cys::pack::PackStateRead::Corrupt(e) => {
            eprintln!("팩 상태 손상({e}) — 먼저 cys pack-repair-channel 로 복구하라.");
            return 1;
        }
    };
    println!(
        "팩 상태: channel=pro (base {}, pro.{}) — free 강등 시 pro 전용 파일이 제거된다.",
        st.base_version, st.pro_revision
    );
    // license-aware 게이트(R2 양 리뷰어 합의): 유효 pro 라이선스 실재 = 기본 거부.
    if cys::license::is_pro(now) && !override_valid_license {
        eprintln!(
            "거부 — 유효 pro 라이선스가 실재한다(팩만 free로 강등되면 pro 앱 기능과 불일치). \
             정말 강등하려면 --override-valid-license 를 함께 지정하라."
        );
        return 1;
    }
    if !yes {
        println!("계획만 출력했다. 실제 강등은 --yes 를 지정하라.");
        return 0;
    }
    // 실행: state → free(base = 현재 .pack-version, rev 0) → 내장 팩 재설치(prune 포함).
    let disk_v = std::fs::read_to_string(dir.join(".pack-version"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let free_state = cys::pack::PackState {
        channel: "free".to_string(),
        base_version: disk_v,
        pro_revision: 0,
    };
    if let Err(e) = cys::pack::write_pack_state(&dir, &free_state) {
        eprintln!("error: state 전환 실패 — {e}");
        return 1;
    }
    // W0-d: pack-downgrade는 라이브 팩 재설치 프로덕션 진입점 — 인가 부여.
    match cys::pack::install(false, Some(cys::pack::PackWriteAuth::production())) {
        Ok((written, kept)) => {
            println!("[downgrade] free 전환 완료 — 내장 팩 재설치: {written} written, {kept} preserved.");
            0
        }
        Err(e) => {
            eprintln!(
                "[downgrade] ⚠ state는 free로 전환됐으나 내장 재설치 실패: {e} — cys init-pack 으로 재시도하라."
            );
            1
        }
    }
}

/// `cys pack-repair-channel`(free/pro v4 §5) — 채널 상태 진단·복구. 재기록 권위 =
/// accepted 기록(서명 검증 이력) + pro 전용 파일 증거. 라이선스는 정보 표시(단독 권위 아님).
fn run_pack_repair_channel(to: Option<String>, yes: bool, expert_override: bool) -> i32 {
    let dir = cys::pack::pack_dir();
    let base = pack_state_base();
    let now = chrono::Utc::now().timestamp();
    // ─ 진단 리포트 ─
    let disk_v = std::fs::read_to_string(dir.join(".pack-version"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let state_desc = match cys::pack::read_pack_state(&dir) {
        cys::pack::PackStateRead::Absent => "부재(=free/0)".to_string(),
        cys::pack::PackStateRead::Valid(st) => format!(
            "channel={} base={} pro.{}{}",
            st.channel,
            st.base_version,
            st.pro_revision,
            if st.base_version == disk_v { "" } else { " ⚠ .pack-version 불일치" }
        ),
        cys::pack::PackStateRead::Corrupt(e) => format!("★손상: {e}"),
    };
    let accepted_path = base.join(".pack-accepted.json");
    let accepted = cys::packsig::read_accepted_evidence(&accepted_path);
    let accepted_desc = match &accepted {
        Ok(None) => "부재(pack-update 이력 없음)".to_string(),
        Ok(Some((ch, rev, v))) => format!("channel={ch} {v} pro.{rev}"),
        Err(e) => format!("★손상: {e}"),
    };
    let pro_files = cys::pack::pro_file_evidence(&dir);
    println!("── pack channel 진단 ──");
    println!(".pack-version : {disk_v}");
    println!(".pack-state   : {state_desc}");
    println!("accepted 기록 : {accepted_desc}");
    println!("pro 파일 증거 : {}", if pro_files { "있음(임베드 외 설치 파일 실재)" } else { "없음" });
    println!("라이선스      : {}", cys::license::render_status(now));

    let Some(to) = to else {
        println!("(진단만 출력 — 복구는 --to free|pro --yes)");
        return 0;
    };
    if to != "free" && to != "pro" {
        eprintln!("error: --to 는 free|pro 만 유효");
        return 1;
    }
    // ─ 권위 규칙(v4 §5) ─
    let accepted_pro = matches!(&accepted, Ok(Some((ch, _, _))) if ch == "pro");
    if to == "pro" && !accepted_pro && !expert_override {
        eprintln!(
            "거부 — pro 재기록은 accepted 기록(서명 검증 이력)의 channel=pro 증거가 필요하다. \
             (순수 free 설치의 pro 자가 마킹 = 내장 갱신 자가 차단 사고 방지) \
             정말 강행하려면 --expert-override."
        );
        return 1;
    }
    if to == "free" {
        if cys::license::is_pro(now) && !expert_override {
            eprintln!(
                "거부 — 유효 pro 라이선스 실재 중 free 재기록은 downgrade와 동일한 위험 \
                 (다음 내장 install이 pro 파일을 prune). 강등은 cys pack-downgrade-to-free, \
                 강행은 --expert-override."
            );
            return 1;
        }
        if (accepted_pro || pro_files) && !expert_override {
            eprintln!(
                "거부 — pro 증거(accepted={accepted_pro}·pro 파일={pro_files})가 실재한다. \
                 free 재기록 시 다음 내장 install이 pro 파일을 제거한다. 강행은 --expert-override."
            );
            return 1;
        }
    }
    if !yes {
        println!("(계획만 — 실제 재기록은 --yes)");
        return 0;
    }
    // ─ 재기록: base = 현재 .pack-version(정합 복원), rev = accepted(pro) 또는 0 ─
    let pro_rev = match &accepted {
        Ok(Some((ch, rev, _))) if ch == "pro" && to == "pro" => *rev,
        _ => 0,
    };
    if to == "pro" && !accepted_pro {
        eprintln!("⚠ expert-override: accepted 증거 없는 pro 재기록 — pro_revision=0으로 기록한다.");
    }
    let st = cys::pack::PackState {
        channel: to.clone(),
        base_version: disk_v,
        pro_revision: pro_rev,
    };
    match cys::pack::write_pack_state(&dir, &st) {
        Ok(()) => {
            println!("[repair] 재기록 완료: channel={} base={} pro.{}", st.channel, st.base_version, st.pro_revision);
            0
        }
        Err(e) => {
            eprintln!("error: 재기록 실패 — {e}");
            1
        }
    }
}

/// 어댑터 prompt-ready predicate(§7-⑨): ready_marker 정의 어댑터(claude·gemini)는 화면에
/// 마커가 보이면 ready. 미정의 어댑터(codex)는 fallback = idle AND quiet ≥ 임계(영구 deferral 방지).
///
/// ★(U-13) **두 번째 소비처도 같은 술어를 경유한다.** 종전 본문은 `scrollback_tail.contains(marker)`
///   한 줄이고 가드가 0이었다 — 부트 폴링을 아무리 고쳐도 이 경로는 여전히 관문 화면(6종 전부에
///   `❯` 가 있다)을 ready 로 보고 **관문 창에 디렉티브를 재주입**했다. 그 붙여넣기의 Return 이
///   면책 창의 `No, exit` 를 누른다. 그래서 판정을 `cys::readiness::judge` 로 옮기고 관문 문면
///   AND 항을 붙였다. 종전 두 갈래(마커 어댑터는 꼬리 매칭만·미정의 어댑터만 idle 폴백)의
///   **의미는 그대로**이고, 추가된 것은 관문 항 하나다(미충족의 귀결도 종전과 같다 — 재주입
///   보류이지 좌석 파괴가 아니다).
fn adapter_ready(agent: &Option<String>, idle: bool, idle_secs: u64, scrollback_tail: &str) -> bool {
    const QUIET_THRESHOLD_SECS: u64 = 8; // ACK timeout 근사 — turn-boundary 근사 quiet 창
    let spec = agent.as_ref().and_then(|a| load_agent_spec(a).ok());
    let marker = spec
        .as_ref()
        .and_then(|s| s["ready_marker"].as_str().map(|s| s.to_string()));
    // 관문 코퍼스는 어댑터 스펙에서 해소한다(문면 SOT = src/first_run_gates.rs · 여기선 읽기만).
    // 스펙을 못 읽으면 빈 코퍼스 = 관문 축 없음 = 종전 판정(가용성 우선 — 재주입을 영구 봉쇄하지
    // 않는다. 이 경로의 오탐은 '주입 안 함'이라 안전 방향이지만, 영구 미주입도 결함이다).
    // ★(P4-4) 부트 폴링·주입 그물과 **같은 소스**를 지난다(사본 0). 어댑터 **미상**일 때
    //   빈 코퍼스(관문 축 없음)로 두는 종전 정책은 그대로다 — 바뀐 것은 스펙 **판독 실패**가
    //   조용한 '관문 부재' 로 접히지 않는다는 것 하나다(`resolve_gate_corpus` doc 참조).
    let gates = agent
        .as_deref()
        .map(|a| resolve_gate_corpus(a).gates)
        .unwrap_or_default();
    let obs = cys::readiness::Observed {
        site: cys::readiness::Site::Reinject,
        // 이 경로에는 커널 생존·화면 꼬리 관측이 없다 — '부재 ≠ 부정' 규약대로 그 축을 요구하는
        // 증거(밸브)는 발화하지 않고, 요구하지 않는 증거만 종전대로 흐른다.
        agent_alive: None,
        screen: scrollback_tail,
        delta: "",
        marker: marker.as_deref(),
        gates: &gates,
        tail_is_shell_prompt: None,
        bare_shell: None,
        time_fallback_reached: false,
        idle_quiet: Some(idle && idle_secs >= QUIET_THRESHOLD_SECS),
        legacy_v1: cys::readiness::legacy_v1(),
    };
    cys::readiness::judge(&obs).is_ready()
}

/// 살아있는 노드에 무중단 reinject(§7-②) — control.dashboard(state)·system.topology(마커)를 읽어
/// reinject_decision으로 판정, Inject만 디렉티브 주입 후 reinject.mark RPC로 기록(P3).
/// ★라이브 데몬 필요 — 실주입 검증은 P7. 여기선 결정 로직 배선만(베스트에포트).
fn run_pack_reinject(new_version: &str) -> Result<ReinjectReport, String> {
    // 마커(role → ReinjectMarker)는 system.topology.saved가 노출(P3가 pack_reinject 영속).
    let topo = request("system.topology", json!({}))?;
    let mut markers: std::collections::HashMap<String, ReinjectMarker> = std::collections::HashMap::new();
    if let Some(saved) = topo["saved"].as_array() {
        for e in saved {
            if let (Some(role), Some(pr)) = (e["role"].as_str(), e.get("pack_reinject")) {
                if let (Some(pv), Some(dh)) =
                    (pr["pack_version"].as_str(), pr["directive_hash"].as_str())
                {
                    markers.insert(
                        role.to_string(),
                        ReinjectMarker { pack_version: pv.to_string(), directive_hash: dh.to_string() },
                    );
                }
            }
        }
    }
    // 라이브 노드 상태: control.dashboard(fleet[].state=derive_node_state·idle_secs).
    let dash = request("control.dashboard", json!({}))?;
    let fleet = dash["fleet"].as_array().cloned().unwrap_or_default();
    let (mut injected, mut skipped, mut deferred, mut failed) = (0usize, 0usize, 0usize, 0usize);
    let mut deferred_nodes: Vec<(u64, String)> = Vec::new();
    for node in &fleet {
        let Some(sid) = node["surface_id"].as_u64() else { continue };
        let Some(role) = node["role"].as_str() else { continue };
        let agent = node["agent"].as_str().map(|s| s.to_string());
        let idle = node["state"].as_str() == Some("idle");
        let idle_secs = node["idle_secs"].as_u64().unwrap_or(0);
        // ⓑ 자기보고 게이트(§7-② step2) — agent_status≠working. 미보고(null)는 보수적으로
        // '비idle' 취급(working일 수 있음 → 주입 안 함, 컨텍스트 오염 차단).
        let self_idle = match node["agent_status"].as_str() {
            Some(st) => st != "working",
            None => false,
        };
        // 디렉티브 해시 — 합성 실패(비표준 역할 등)는 스킵.
        let Ok(directive) = compose_directive(role) else { continue };
        let new_hash = sha256_hex(&directive);
        // ready predicate(§7-⑨) — ready_marker 어댑터는 화면 tail로, 아니면 idle+quiet fallback.
        let tail = request("surface.read_text", json!({"surface_id": sid}))
            .ok()
            .and_then(|r| r["text"].as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let ready = adapter_ready(&agent, idle, idle_secs, &tail);
        match reinject_decision(markers.get(role), new_version, &new_hash, idle, self_idle, ready) {
            ReinjectDecision::Inject => {
                // per-node 에러 격리(Fix3): 한 노드의 transient 실패가 나머지 건강 노드의 reinject를
                // 중단시키지 않게 `?` 전파 대신 count+continue 한다.
                if let Err(e) = inject_text(sid, &directive) {
                    eprintln!("[pack-update] reinject 주입 실패(surface {sid}, role {role}): {e} — 다음 노드로 계속");
                    failed += 1;
                    continue;
                }
                // 주입 성공 후에만 마커 기록(P3 단일 write path). 마커 기록 실패는 '이미 주입됨'을
                // 의미하므로 다음 pack-update에서 같은 버전이 재주입(중복 주입)될 수 있다 — 그 창을
                // 가시화하도록 명시 경고하되 루프는 계속한다(나머지 노드 reinject 보장).
                if let Err(e) = request(
                    "reinject.mark",
                    json!({"surface_id": sid, "pack_version": new_version,
                           "directive_hash": new_hash}),
                ) {
                    eprintln!("[pack-update] ⚠ reinject.mark 기록 실패(surface {sid}, role {role}): {e} — \
                               주입은 됐으나 마커 미기록 → 다음 pack-update에서 중복 주입 가능");
                    failed += 1;
                    continue;
                }
                injected += 1;
            }
            ReinjectDecision::SkipUnchanged | ReinjectDecision::SkipDedup => skipped += 1,
            ReinjectDecision::Defer => {
                deferred += 1;
                deferred_nodes.push((sid, role.to_string()));
            }
        }
    }
    Ok(ReinjectReport { injected, skipped, deferred, failed, deferred_nodes })
}

/// LOW#1 pending 소비 핵심 — 라이브 토폴로지(markers)·플릿(fleet)·주입을 인자/클로저로 받아
/// 데몬 비의존 단위테스트가 가능하다. 각 pending 노드를 run_pack_reinject와 동일한 신호로
/// reinject_decision 재평가한다: Inject→주입+마크 성공 시 해소 / Skip*(이미 최신)→해소 /
/// 노드 부재(닫힘)·합성 실패(비표준 역할)→해소(무한 잔존 방지) / Defer(여전히 busy)·주입·마크
/// 실패→pending 잔존. 잔존 0이면 파일 삭제, 아니면 잔존 노드로 재기록(pack_version 보존).
/// pending_ver를 새 버전으로 쓰므로(현재 디스크 팩 == 보류 당시 버전), version gate와 독립이다.
/// 반환=(resolved, kept).
#[allow(clippy::too_many_arguments)]
fn consume_reinject_pending_core(
    base: &std::path::Path,
    pending_ver: &str,
    pending_nodes: &[(u64, String)],
    markers: &std::collections::HashMap<String, ReinjectMarker>,
    fleet: &[serde_json::Value],
    compose: impl Fn(&str) -> Result<String, String>,
    read_tail: impl Fn(u64) -> String,
    inject: impl Fn(u64, &str) -> Result<(), String>,
    mark: impl Fn(u64, &str, &str) -> Result<(), String>,
) -> std::io::Result<(usize, usize)> {
    let mut kept: Vec<(u64, String)> = Vec::new();
    let mut resolved = 0usize;
    for (sid, role) in pending_nodes {
        // 라이브 플릿에서 해당 surface 조회 — 부재(닫힘)면 재시도 대상 자체가 없으므로 해소 처리.
        let Some(node) = fleet.iter().find(|n| n["surface_id"].as_u64() == Some(*sid)) else {
            resolved += 1;
            continue;
        };
        let agent = node["agent"].as_str().map(|s| s.to_string());
        let idle = node["state"].as_str() == Some("idle");
        let idle_secs = node["idle_secs"].as_u64().unwrap_or(0);
        // 자기보고 게이트(§7-② step2) — null(미보고)은 보수적으로 비idle.
        let self_idle = match node["agent_status"].as_str() {
            Some(st) => st != "working",
            None => false,
        };
        // 디렉티브 합성 실패(비표준 역할)는 영영 주입 불가 → 해소(stale 잔존 방지).
        let Ok(directive) = compose(role) else {
            resolved += 1;
            continue;
        };
        let new_hash = sha256_hex(&directive);
        let ready = adapter_ready(&agent, idle, idle_secs, &read_tail(*sid));
        match reinject_decision(markers.get(role.as_str()), pending_ver, &new_hash, idle, self_idle, ready)
        {
            ReinjectDecision::Inject => {
                // per-node 에러 격리 — 한 노드의 실패가 나머지 재시도를 막지 않게 잔존 처리 후 계속.
                if inject(*sid, &directive).is_err() {
                    kept.push((*sid, role.clone()));
                    continue;
                }
                if mark(*sid, pending_ver, &new_hash).is_err() {
                    kept.push((*sid, role.clone()));
                    continue;
                }
                resolved += 1;
            }
            ReinjectDecision::SkipUnchanged | ReinjectDecision::SkipDedup => resolved += 1,
            ReinjectDecision::Defer => kept.push((*sid, role.clone())),
        }
    }
    persist_reinject_pending(base, pending_ver, &kept)?;
    Ok((resolved, kept.len()))
}

/// LOW#1 능동 소비 진입점 — run_pack_update 착수 시 1회 호출. 디스크 pending이 있으면 지금 idle인
/// 보류 노드에 reinject를 재시도한다(write-only였던 pending을 능동 소비). pending 부재/빈 목록 →
/// no-op(데몬 접속 없이 즉시 반환). 데몬 미가동 → Err(호출자가 로깅·계속, pending 보존 = graceful).
fn consume_reinject_pending(base: &std::path::Path) -> Result<(usize, usize), String> {
    let Some((ver, nodes)) = read_reinject_pending(base).map_err(|e| e.to_string())? else {
        return Ok((0, 0));
    };
    if nodes.is_empty() {
        // 빈 deferred만 남은 stale 파일 → 정리(데몬 접속 불요).
        let _ = std::fs::remove_file(reinject_pending_path(base));
        return Ok((0, 0));
    }
    // 라이브 토폴로지(마커)·플릿(상태) — 데몬 필요. 미가동이면 ?로 Err 전파(graceful 스킵·pending 보존).
    let topo = request("system.topology", json!({}))?;
    let mut markers: std::collections::HashMap<String, ReinjectMarker> =
        std::collections::HashMap::new();
    if let Some(saved) = topo["saved"].as_array() {
        for e in saved {
            if let (Some(role), Some(pr)) = (e["role"].as_str(), e.get("pack_reinject")) {
                if let (Some(pv), Some(dh)) =
                    (pr["pack_version"].as_str(), pr["directive_hash"].as_str())
                {
                    markers.insert(
                        role.to_string(),
                        ReinjectMarker {
                            pack_version: pv.to_string(),
                            directive_hash: dh.to_string(),
                        },
                    );
                }
            }
        }
    }
    let dash = request("control.dashboard", json!({}))?;
    let fleet = dash["fleet"].as_array().cloned().unwrap_or_default();
    consume_reinject_pending_core(
        base,
        &ver,
        &nodes,
        &markers,
        &fleet,
        compose_directive,
        |sid| {
            request("surface.read_text", json!({"surface_id": sid}))
                .ok()
                .and_then(|r| r["text"].as_str().map(|s| s.to_string()))
                .unwrap_or_default()
        },
        inject_text,
        |sid, ver, hash| {
            request(
                "reinject.mark",
                json!({"surface_id": sid, "pack_version": ver, "directive_hash": hash}),
            )
            .map(|_| ())
        },
    )
    .map_err(|e| e.to_string())
}

/// `cys pack-update` 진입점(§2-② 전체 흐름). --from(핵심)·--manifest-url(부차).
/// ④ 투명성: 내장 팩 반영 드라이런 — install_into 와 **같은 판정 함수**(pack::decide_file_action)를
/// 쓰는 pack::plan_install 로 갱신/보존/치유/병합대기/정리를 설치 전에 보여준다(쓰기 0·플랜≠실제 드리프트 0).
fn run_pack_plan(force: bool) -> i32 {
    let dir = cys::pack::pack_dir();
    let items: Vec<(&str, &str)> = cys::pack::PACK_ALL.iter().map(|(r, c)| (*r, *c)).collect();
    let plan = cys::pack::plan_install(&dir, &items, force, env!("CARGO_PKG_VERSION"));
    if let Some(reason) = &plan.blocked {
        println!("⛔ 설치 차단: {reason}");
        return 1;
    }
    let section = |title: &str, rels: &[String], note: &str| {
        if rels.is_empty() {
            return;
        }
        println!("\n{title} ({}건){}", rels.len(), if note.is_empty() { String::new() } else { format!(" — {note}") });
        for r in rels {
            println!("  {r}");
        }
    };
    println!("팩 반영 플랜 (대상: {} · 바이너리 {} · 쓰기 없음)", dir.display(), env!("CARGO_PKG_VERSION"));
    section("🔄 자동 갱신", &plan.update, "비수정 — 그대로 갱신됨");
    section("✨ 신규 생성", &plan.create, "");
    section("🛠 강제 치유", &plan.heal, "system 수정본 — 덮기 전 사용자본을 <파일>.user 로 보존");
    section("⏸ 보존+병합 대기", &plan.merge_new, "user-owned 수정본 유지 + 신버전 <파일>.new 병치 → cys pack-merge");
    section("🔒 보존", &plan.keep_user, "user-owned 수정본 — 건드리지 않음");
    section("🗑 정리(폐기 파일)", &plan.prune_delete, "임베드에서 제거된 비수정 파일");
    section("🗑→🔒 폐기지만 보존", &plan.prune_keep_modified, "수정본이라 삭제하지 않음");
    println!("\n= 변화 없음(최신) {}건", plan.unchanged);
    let pending = cys::pack::load_merge_pending(&dir);
    if !pending.is_empty() {
        println!("※ 기존 병합 대기 {}건 — `cys pack-merge` 로 검토", pending.len());
    }
    println!("※ 사용자 전용 오버레이(~/.cys/local — 디렉티브 append·스킬 shadowing·훅 후행)는 업데이트가 절대 건드리지 않음");
    report_overlay_skill_drift();
    0
}

/// ★W-D2(커스텀 생존 설계 2026-07-17): 오버레이 shadowing 스킬의 vendor 전진 감지(읽기 전용 —
/// local 에는 절대 쓰지 않음·⑥층 불가침 유지). 오버레이는 업데이터가 존재를 모르는 영역이라
/// "불가침의 대가 = 드리프트 무감지"였다 — --to-local 승격 시 기록한 `.vendor-base`(승격 당시
/// vendor SKILL.md 해시)와 현 임베드 해시를 대조해, 사용자가 가리고 있는 vendor 스킬이 그 뒤
/// 전진했으면 알린다. 기준 미기록(구 승격본·수동 생성)은 판정 불가로 정직하게 표시.
fn report_overlay_skill_drift() {
    let local_skills = cys::pack::local_dir().join("skills");
    let Ok(entries) = std::fs::read_dir(&local_skills) else { return };
    let mut lines: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !entry.path().join("SKILL.md").is_file() {
            continue;
        }
        let rel = format!("skills/{name}/SKILL.md");
        let Some(embed) = cys::pack::PACK_ALL.iter().find(|(r, _)| *r == rel).map(|(_, c)| *c) else {
            continue; // vendor 대응물 없는 순수 자작 스킬 — 드리프트 개념 없음
        };
        let embed_hash = cys::pack::content_hash_pub(embed);
        match std::fs::read_to_string(entry.path().join(".vendor-base")) {
            Ok(base) if base.trim() == embed_hash => {} // vendor 미전진 — 조용히 통과
            Ok(_) => lines.push(format!(
                "  {name}: vendor 전진 있음 — 내 오버레이본이 낡은 vendor 를 기준으로 함. \
                 대조: `cys skill show {name}`(오버레이) vs 팩본 {rel}"
            )),
            Err(_) => lines.push(format!("  {name}: 승격 기준 미기록 — vendor 전진 여부 판정 불가(수동 대조 권장)")),
        }
    }
    if !lines.is_empty() {
        println!("\n⚠ 오버레이 shadowing 스킬의 vendor 드리프트 ({}건):", lines.len());
        for l in lines {
            println!("{l}");
        }
    }
}

/// ③ 커스터마이즈 병합: 병합 대기 원장 목록·해소. 해소 경로 4종 —
///   --take-new(신버전 채택) · --keep-mine(내 수정 유지·이번 신버전 소화) ·
///   diff3/--ai 3-way 병합(base=.pristine 조상) · --to-local(healed system 파일을 오버레이로 이동).
/// system(healed) 파일은 rel 로 되쓰기 금지 — 다음 기동 install 이 다시 치유(P0-4)하므로
/// 지원 경로는 to-local(스킬 shadowing)뿐임을 명시한다.
/// ★A12 코드 가드(v4 · W4 — 결정 D8: override 플래그명 `--force-vendor`).
///
/// CEO 승격 중(= `<pack>/directives/MASTER_DIRECTIVE.md.pre-ceo` 실재)의 base MASTER 를
/// vendor/보존 본으로 **덮는** 두 동사를 기계 거부한다: `pack-merge --take-new` 와
/// `pack-rollback --file` (R2 가 치명 분류한 승격 파괴 벡터 — 후자는 롤백 '도구'라서 더
/// 위험하다). '금지 명문화'(runbook)를 기계 집행으로 승격한 것이며, keep-mine(승격본 유지)
/// 절차는 종전대로 통과한다. 반환 = Some(stderr 전문·exit 비0) / None = 통과.
#[derive(Clone, Copy, PartialEq)]
enum CeoGuardVerb {
    TakeNew,  // pack-merge --take-new
    Rollback, // pack-rollback --file
}

const MASTER_DIRECTIVE_REL: &str = "directives/MASTER_DIRECTIVE.md";

fn ceo_vendor_overwrite_rejection(
    pack_dir: &std::path::Path,
    rel: &str,
    force_vendor: bool,
    verb: CeoGuardVerb,
) -> Option<String> {
    if rel != MASTER_DIRECTIVE_REL || force_vendor {
        return None;
    }
    let pre_ceo = pack_dir.join(format!("{MASTER_DIRECTIVE_REL}.pre-ceo"));
    if !pre_ceo.exists() {
        return None;
    }
    let (verb_label, procedure) = match verb {
        CeoGuardVerb::TakeNew => (
            "--take-new",
            // D1(a) MASTER 정본 절차 — ①② 순서 역전 금지(keep-mine 이 .new 를 삭제한다).
            format!(
                " 올바른 절차(A12 runbook · keep-mine):\n\
                 \x20  ① cp <pack>/{MASTER_DIRECTIVE_REL}.new <pack>/{MASTER_DIRECTIVE_REL}.pre-ceo   # 복원 백업을 신본으로 갱신\n\
                 \x20  ② cys pack-merge --file {MASTER_DIRECTIVE_REL} --keep-mine   # 승격본 유지(.new 해소 — ①② 역전 금지)"
            ),
        ),
        CeoGuardVerb::Rollback => (
            "pack-rollback",
            " 정본 롤백 경로(A12): 직전 릴리스 재설치 → 같은 세션에서 promote-ceo 재실행\n\
             \x20 (md 파일 단위 수동/후진 복원 금지 — '비정형 승격 FAIL' 자가 제조 경로)"
                .to_string(),
        ),
    };
    Some(format!(
        "⛔ 거부: '{MASTER_DIRECTIVE_REL}' 는 CEO 승격 중이다({} 존재) — {verb_label} 는 승격본을 \
         덮어 강등 가역성(.pre-ceo↔md 쌍)을 파괴한다.\n{procedure}\n\
         \x20 정말 덮으려면(승격 파괴 승인) --force-vendor 를 명시하라.",
        pre_ceo.display()
    ))
}

/// ★G3-축3 안전핵 게이트 거부 exit — 예약 규약({0,1,2,64} 충돌 금지 · clap 사용오류=2)과
/// 분리해, 신 팩+구 바이너리 스큐에서 '플래그 부재(clap 2)'와 '게이트 거부'가 소비 스크립트에서
/// 구분되게 한다. 값 7 = claim-role 정당거부(rc=7) 선례 계열(타입드 거부).
const EXIT_UNSAFE_CORE_REFUSED: i32 = 7;

/// ★G3-축3 위험 diff 요약(순수) — 소실 키워드와, 교체로 사라질 ours 쪽 조항 줄만 적시한다.
/// 전체 diff 가 아닌 이유: 게이트 화면의 판단 재료는 "무엇이 사라지는가" 하나이고, 전문 diff 는
/// 헌법 파일에서 수백 줄이라 오히려 소실 조항을 묻는다.
fn takeover_risk_summary(ours: &str, lost: &[String]) -> String {
    let mut s = format!(
        "⚠ 안전핵 소실 {}건: {}\n  교체로 사라지는 현행 조항:\n",
        lost.len(),
        lost.join(", ")
    );
    for line in ours.lines() {
        let ll = line.to_lowercase();
        let hits: Vec<&str> = lost
            .iter()
            .filter(|k| ll.contains(k.as_str()))
            .map(|k| k.as_str())
            .collect();
        if !hits.is_empty() {
            s.push_str(&format!("  - [{}] {}\n", hits.join(","), line.trim()));
        }
    }
    s
}

/// ★G3-축3 감사 엔트리 조립(순수 형태 — 시각·env 읽기만). actor 는 os_user 하나만 기록한다:
/// env 유래(USER/USERNAME)라 자기신고 값이며 인증이 아니라 참고 정보다 — cys_role 등 추가
/// 자기신고 신원은 신뢰 불가 데이터라 싣지 않는다.
fn merge_audit_entry(
    rel: &str,
    action: &str,
    before: &str,
    after: &str,
    verify_result: &str,
    flags: &[String],
) -> serde_json::Value {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let os_user = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" })
        .unwrap_or_else(|_| "unknown".into());
    json!({
        "ts": ts,
        "file": rel,
        "action": action,
        "actor_os_user": os_user,
        "before_sha256": cys::pack::content_hash_pub(before),
        "after_sha256": cys::pack::content_hash_pub(after),
        "verify_result": verify_result,
        "flags": flags,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_pack_merge(
    file: Option<String>,
    take_new: bool,
    keep_mine: bool,
    ai: bool,
    to_local: bool,
    propose: bool,
    yes: bool,
    force_vendor: bool,
    dry_run: bool,
    force_unsafe_core: bool,
) -> i32 {
    let dir = cys::pack::pack_dir();
    let mut pending = cys::pack::load_merge_pending(&dir);
    // ★G3-축3 플래그 결합 제약 — 파괴 승인·드라이런 플래그의 침묵 무시는 오사용을 감추므로
    //   해당 동사 밖에서는 fail-closed 로 거절한다(적용된 척 0).
    if force_unsafe_core && !take_new {
        eprintln!("--force-unsafe-core 는 --take-new 전용(안전핵 소실 승인 플래그) — 조합을 확인하라");
        return 1;
    }
    if dry_run && !(take_new || keep_mine) {
        eprintln!("--dry-run 은 --take-new/--keep-mine 과 함께 사용(해소 예정 판정·쓰기 0)");
        return 1;
    }
    let Some(rel) = file else {
        // 목록 모드
        if pending.is_empty() {
            println!("병합 대기 없음 — 커스터마이즈와 vendor 팩이 정합 상태입니다.");
            return 0;
        }
        println!("병합 대기 {}건:", pending.len());
        for (rel, e) in pending.iter() {
            let kind = e.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
            let side = e.get("side").and_then(|v| v.as_str()).unwrap_or("?");
            let ver = e.get("version").and_then(|v| v.as_str()).unwrap_or("?");
            match kind {
                "new-pending" => println!(
                    "  ⏸ {rel} — 내 수정본 유지 중, vendor {ver} 신버전이 {side} 에 대기\n     → cys pack-merge --file {rel} [--take-new|--keep-mine|--ai|--propose]"
                ),
                "healed" => println!(
                    "  🛠 {rel} — vendor {ver} 로 치유됨, 내 수정본은 {side} 에 보존\n     → cys pack-merge --file {rel} [--to-local|--propose|--keep-mine(보존본 정리)]"
                ),
                _ => println!("  ? {rel} ({kind})"),
            }
        }
        return 0;
    };
    // rel 검증(원장 기반 — 경로 traversal 차단: 원장에 있는 키만 처리).
    let Some(entry) = pending.get(&rel).cloned() else {
        eprintln!("'{rel}' 은 병합 대기 목록에 없음 — `cys pack-merge` 로 목록 확인");
        return 1;
    };
    let kind = entry.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
    let target = dir.join(&rel);
    let embed_now: Option<&str> = cys::pack::PACK_ALL
        .iter()
        .find(|(r, _)| *r == rel.as_str())
        .map(|(_, c)| *c);
    // ★W-F2: --propose 는 해소가 아니라 환류 — patch 파일 생성만 하고 원장은 건드리지 않는다.
    if propose {
        return run_pack_propose(&dir, &rel, kind, embed_now);
    }
    // ★W-D4: 헌법 파일(디렉티브·soul·CLAUDE)은 --yes 여도 대화형 확인 필수 — 자동 병합·자동
    // 교체 금지(자율주행 denylist 정합). 비대화형(stdin EOF)에서는 빈 입력 → 거절되므로 안전측.
    let is_const = cys::pack::is_constitution_file(&rel);
    let confirm = |prompt: &str| -> bool {
        if yes && !is_const {
            return true;
        }
        if yes && is_const {
            println!("(헌법 파일 — --yes 무시, 확인 필수)");
        }
        print!("{prompt} [y/N] ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        matches!(line.trim(), "y" | "Y" | "yes")
    };
    // 원장·병치 파일 해소 공통부.
    let resolve = |pending: &mut serde_json::Map<String, serde_json::Value>, side_suffix: &str| {
        pending.remove(&rel);
        cys::pack::save_merge_pending(&dir, pending);
        let _ = std::fs::remove_file(dir.join(format!("{rel}{side_suffix}")));
    };
    // ★G3-축3 감사 원장 — 해소·거부 사실의 append 기록. 기록 실패는 loud 경고 후 계속
    //   (감사는 관측이지 검증 게이트가 아니다 — 기록 불능이 해소 자체를 봉쇄하면 병합 원장이
    //   영구 적체된다. save_merge_pending best-effort 전례와 같은 결).
    let audit = |entry: &serde_json::Value| {
        if let Err(e) = cys::pack::append_merge_audit(&dir, entry) {
            eprintln!("⚠ 감사 원장 기록 실패(해소는 계속): {e}");
        }
    };
    // 감사 flags — 판단에 영향을 준 승인 플래그만(추적의 재료).
    let flag_list = |extra: &[&str]| -> Vec<String> {
        let mut f: Vec<String> = Vec::new();
        if yes {
            f.push("yes".into());
        }
        if force_vendor {
            f.push("force-vendor".into());
        }
        if force_unsafe_core {
            f.push("force-unsafe-core".into());
        }
        f.extend(extra.iter().map(|s| s.to_string()));
        f
    };
    // user-owned 해소 시 매니페스트 base 전진(같은 vendor 버전으로 .new 재병치 방지).
    let advance_manifest_base = |content: &str| {
        let mpath = dir.join(cys::pack::INSTALL_MANIFEST);
        let mut m: std::collections::BTreeMap<String, String> = std::fs::read_to_string(&mpath)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        m.insert(rel.clone(), cys::pack::content_hash_pub(content));
        if let Ok(json) = serde_json::to_string_pretty(&m) {
            let _ = cys::pack::write_atomic(&mpath, json.as_bytes());
        }
    };
    match kind {
        "new-pending" => {
            let new_path = dir.join(format!("{rel}.new"));
            let theirs = match std::fs::read_to_string(&new_path) {
                Ok(s) => s,
                Err(_) => match embed_now {
                    Some(c) => c.to_string(),
                    None => {
                        eprintln!("{rel}.new 부재 + 임베드에도 없음 — 원장만 정리");
                        resolve(&mut pending, ".new");
                        return 0;
                    }
                },
            };
            let ours = std::fs::read_to_string(&target).unwrap_or_default();
            if take_new {
                // ★A12 승격 가드 — 승격 중 base MASTER 의 vendor 채택은 confirm 이전에 기계 거부.
                if let Some(msg) =
                    ceo_vendor_overwrite_rejection(&dir, &rel, force_vendor, CeoGuardVerb::TakeNew)
                {
                    eprintln!("{msg}");
                    return 1;
                }
                // ★G3-축3(결함 5): 헌법 파일 전량 교체는 takeover 술어로 안전핵 소실을 선검증.
                //   기존 verify_constitution_merge 는 merged==theirs 에서 구조적 항진(⊥ 조건)이라
                //   이 경로의 가드가 되지 못한다 — ours-only 소실 검출 전용 술어를 fail-closed 배선.
                let takeover_verify: Result<(), Vec<String>> = if is_const {
                    cys::overrides::verify_constitution_takeover(&ours, &theirs)
                } else {
                    Ok(())
                };
                let verify_label = match &takeover_verify {
                    Ok(()) if is_const => "ok".to_string(),
                    Ok(()) => "n/a".to_string(),
                    Err(lost) => format!("unsafe-core-lost:{}", lost.join(",")),
                };
                if let Err(lost) = &takeover_verify {
                    eprint!("{}", takeover_risk_summary(&ours, lost));
                    if !force_unsafe_core {
                        if dry_run {
                            println!(
                                "(dry-run · 쓰기 0) 판정: 게이트 거부(rc={EXIT_UNSAFE_CORE_REFUSED}) — 적용하려면 --force-unsafe-core 명시"
                            );
                        } else {
                            // 거부도 감사 라인으로 남긴다 — 원장은 시도·거부·실행의 전체 사실.
                            audit(&merge_audit_entry(
                                &rel, "take-new", &ours, &ours, &verify_label,
                                &flag_list(&["refused"]),
                            ));
                            eprintln!(
                                "⛔ 거부(rc={EXIT_UNSAFE_CORE_REFUSED}): 헌법 안전핵 소실 — 소실을 승인하려면 --force-unsafe-core 를 명시하라."
                            );
                        }
                        return EXIT_UNSAFE_CORE_REFUSED;
                    }
                    eprintln!("⚠ --force-unsafe-core: 안전핵 소실 승인 상태로 진행(감사 원장 기록).");
                } else if is_const {
                    println!("✔ 헌법 안전핵 승계 확인(ours-only 소실 0)");
                }
                if dry_run {
                    println!("(dry-run · 쓰기 0) '{rel}' ← vendor 신버전 채택 예정");
                    return 0;
                }
                if confirm(&format!("'{rel}' 을 vendor 신버전으로 교체(내 수정 폐기)?")) {
                    if let Err(e) = cys::pack::write_atomic(&target, theirs.as_bytes()) {
                        eprintln!("쓰기 실패: {e}");
                        return 1;
                    }
                    advance_manifest_base(&theirs);
                    resolve(&mut pending, ".new");
                    audit(&merge_audit_entry(
                        &rel, "take-new", &ours, &theirs, &verify_label, &flag_list(&[]),
                    ));
                    println!("✅ {rel} ← vendor 신버전 채택");
                }
                return 0;
            }
            if keep_mine {
                if dry_run {
                    println!("(dry-run · 쓰기 0) '{rel}' — 내 수정 유지·이번 vendor 신버전 해소 예정");
                    return 0;
                }
                advance_manifest_base(&theirs); // 이번 신버전은 '본 것'으로 — vendor 재전진 시에만 재병치
                resolve(&mut pending, ".new");
                audit(&merge_audit_entry(&rel, "keep-mine", &ours, &ours, "n/a", &flag_list(&[])));
                println!("✅ {rel} — 내 수정 유지(이번 vendor 신버전 해소)");
                return 0;
            }
            // 3-way 병합: base = .pristine 조상(사용자가 fork 한 시점의 vendor 본).
            let base_path = dir.join(cys::pack::PRISTINE_DIR).join(&rel);
            let base = std::fs::read_to_string(&base_path).ok();
            let merged: Option<String> = if ai {
                ai_three_way_merge(&rel, base.as_deref(), &ours, &theirs)
            } else {
                diff3_merge(base.as_deref(), &ours, &theirs)
            };
            match merged {
                Some(m) if m == ours => {
                    println!("병합 결과 = 현재 내 수정본과 동일(vendor 변경이 이미 반영됨) — 해소만 수행");
                    advance_manifest_base(&theirs);
                    resolve(&mut pending, ".new");
                    0
                }
                Some(m) => {
                    // ★W-D3: 헌법 파일 병합은 안전핵 소실 결정론 검증을 통과해야 적용 가능
                    // (fail-closed — AI 병합의 미묘한 왜곡·통째 소실로부터 안전핵을 지킨다).
                    if is_const {
                        if let Err(lost) = cys::overrides::verify_constitution_merge(&ours, &theirs, &m) {
                            eprintln!(
                                "⛔ 병합 거부(헌법 안전핵 소실 검출): {} — 병합본에서 안전핵 조항이 사라짐.\n\
                                 \x20 수동 병합 후 --keep-mine 으로 해소하거나 --take-new(vendor 본)를 쓰세요.",
                                lost.join(", ")
                            );
                            return 1;
                        }
                        println!("✔ 헌법 안전핵 검증 통과(소실 0)");
                    }
                    println!("── 병합 제안 diff (내 수정본 → 병합본) ──");
                    print_unified_diff(&ours, &m);
                    if confirm(&format!("'{rel}' 에 병합본 적용?")) {
                        if let Err(e) = cys::pack::write_atomic(&target, m.as_bytes()) {
                            eprintln!("쓰기 실패: {e}");
                            return 1;
                        }
                        advance_manifest_base(&theirs);
                        // 병합본의 새 조상 = 이번 vendor 본(다음 3-way 정확성).
                        let _ = std::fs::create_dir_all(base_path.parent().unwrap_or(&dir));
                        let _ = cys::pack::write_atomic(&base_path, theirs.as_bytes());
                        resolve(&mut pending, ".new");
                        println!("✅ {rel} ← 3-way 병합 적용");
                    } else {
                        println!("보류 — 원장 유지. --take-new/--keep-mine 또는 수동 편집 후 재실행");
                    }
                    0
                }
                None => {
                    println!(
                        "자동 병합 불가(충돌 또는 도구 부재). 선택지:\n\
                         \x20 cys pack-merge --file {rel} --take-new   # vendor 신버전 채택\n\
                         \x20 cys pack-merge --file {rel} --keep-mine # 내 수정 유지\n\
                         \x20 cys pack-merge --file {rel} --ai        # AI 3-way 병합 제안\n\
                         \x20 수동: {rel} 과 {rel}.new 를 직접 병합 후 --keep-mine 으로 해소"
                    );
                    1
                }
            }
        }
        "healed" => {
            let user_path = dir.join(format!("{rel}.user"));
            if to_local {
                // 스킬 등 system 파일의 사용자본을 오버레이로 승격 — vendor 무결성(치유)과 공존.
                let local_root = cys::pack::local_dir();
                let dest = local_root.join(&rel);
                if let Some(parent) = dest.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::read_to_string(&user_path) {
                    Ok(content) => {
                        if let Err(e) = cys::pack::write_atomic(&dest, content.as_bytes()) {
                            eprintln!("오버레이 쓰기 실패: {e}");
                            return 1;
                        }
                        // ⑥ 사용자 스킬 WARN 게이트(BLOCK 아님) — 로컬 승격 시 1회 정적 스캔(스킬 디렉토리 단위).
                        if rel.starts_with("skills/") {
                            if let Some(skill_dir) = dest.parent() {
                                skillscan_warn(skill_dir);
                                // ★W-D2: 승격 당시 vendor 해시를 기록 — pack-plan 이 이후 vendor
                                // 전진(shadowing 드리프트)을 결정론 감지하는 기준점. 사용자의 명시적
                                // 승격 명령에 딸린 메타 기록이라 "업데이터의 local 불가침"과 무관.
                                if let Some(embed) = embed_now {
                                    let _ = cys::pack::write_atomic(
                                        &skill_dir.join(".vendor-base"),
                                        cys::pack::content_hash_pub(embed).as_bytes(),
                                    );
                                }
                            }
                        }
                        resolve(&mut pending, ".user");
                        println!(
                            "✅ {rel} 사용자본 → {} (오버레이 — 업데이트 불가침{})",
                            dest.display(),
                            if rel.starts_with("skills/") { " · 동명 스킬 shadowing" } else { "" }
                        );
                        0
                    }
                    Err(e) => {
                        eprintln!("{} 읽기 실패: {e}", user_path.display());
                        1
                    }
                }
            } else if keep_mine || take_new {
                // healed 의 '해소' = 보존본 정리(vendor 본 유지가 이미 디스크 상태).
                if dry_run {
                    println!("(dry-run · 쓰기 0) '{rel}' 보존본({rel}.user) 정리(vendor 본 유지 확정) 예정");
                    return 0;
                }
                if confirm(&format!("'{rel}' 보존본({rel}.user) 정리(vendor 본 유지 확정)?")) {
                    resolve(&mut pending, ".user");
                    // 디스크 본문 무변경 해소 — before==after 로 '정리' 사실만 원장에 남긴다.
                    let cur = std::fs::read_to_string(&target).unwrap_or_default();
                    let action = if take_new { "take-new" } else { "keep-mine" };
                    audit(&merge_audit_entry(
                        &rel, action, &cur, &cur, "n/a", &flag_list(&["healed-cleanup"]),
                    ));
                    println!("✅ {rel} — vendor 본 유지 확정, 보존본 정리");
                }
                0
            } else {
                println!(
                    "'{rel}' 은 system 파일 — 직접 되쓰기는 다음 기동 때 다시 치유(P0-4)되므로 지원하지 않음.\n\
                     \x20 cys pack-merge --file {rel} --to-local  # 사용자본을 ~/.cys/local 오버레이로(스킬 shadowing)\n\
                     \x20 cys pack-merge --file {rel} --keep-mine # vendor 본 유지 확정(보존본 정리)\n\
                     보존본 위치: {}",
                    user_path.display()
                );
                0
            }
        }
        other => {
            eprintln!("알 수 없는 원장 kind '{other}' — 수동 확인 필요");
            1
        }
    }
}

/// ★W-F2(개선 환류 채널): 내 수정본과 vendor 본의 diff 를 제안 patch 파일로 생성.
/// 배포 사용자의 system 커스텀에는 upstream 이 없다 — 가치 있는 개조가 제품으로 돌아올 유일한
/// 길은 제안이다. **자동 전송 없음**(외부 발행은 항상 사용자 수동) — 파일 생성+안내까지만.
/// 비밀 유출 가드: 홈 경로·키 패턴을 스캔해 검출 시 경고를 함께 출력(제출 전 사용자 확인 의무).
fn run_pack_propose(dir: &std::path::Path, rel: &str, kind: &str, embed_now: Option<&str>) -> i32 {
    // mine = 사용자 쪽 내용(healed → .user 보존본 / new-pending → 현재 디스크본).
    let (mine_path, vendor): (std::path::PathBuf, String) = match kind {
        "healed" => (
            dir.join(format!("{rel}.user")),
            match embed_now {
                Some(c) => c.to_string(),
                None => {
                    eprintln!("'{rel}' 은 현 임베드에 없음(폐기된 파일) — 제안 대상 아님");
                    return 1;
                }
            },
        ),
        "new-pending" => (
            dir.join(rel),
            match std::fs::read_to_string(dir.join(format!("{rel}.new"))) {
                Ok(s) => s,
                Err(_) => embed_now.map(str::to_string).unwrap_or_default(),
            },
        ),
        other => {
            eprintln!("알 수 없는 원장 kind '{other}' — 제안 생성 불가");
            return 1;
        }
    };
    let mine = match std::fs::read_to_string(&mine_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} 읽기 실패: {e}", mine_path.display());
            return 1;
        }
    };
    // 비밀 스캔(안내용 WARN — 차단 아님: 제출은 어차피 사용자 수동이라 최종 책임 지점이 명확).
    let mut secret_hits: Vec<String> = Vec::new();
    for (i, line) in mine.lines().enumerate() {
        let l = line.to_lowercase();
        if l.contains("/users/") || l.contains("c:\\users\\") || l.contains("private key")
            || l.contains("api_key") || l.contains("apikey") || l.contains("password")
            || l.contains("secret")
        {
            secret_hits.push(format!("  {}행: {}", i + 1, line.trim()));
        }
    }
    let out_dir = cys::pack::local_dir().join("proposals");
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("proposals 디렉터리 생성 실패: {e}");
        return 1;
    }
    let ver = env!("CARGO_PKG_VERSION");
    let fname = format!("{}-v{ver}.patch", rel.replace('/', "_"));
    let out = out_dir.join(&fname);
    // diff -u(맥·리눅스·git-bash 공통) 시도 — 부재 시 전문(全文) 제안으로 폴백(내용 손실 0).
    let body = unified_diff_via_cmd(&vendor, &mine, rel).unwrap_or_else(|| {
        format!(
            "# unified diff 도구 부재 — 전문 제안(위=vendor {ver} 기준, 아래=내 수정 전문)\n\
             # ── 내 수정 전문 ──\n{mine}"
        )
    });
    let header = format!(
        "# cys 개선 제안 patch\n# 대상: {rel} (vendor {ver} 기준)\n# 생성: cys pack-merge --file {rel} --propose\n\n"
    );
    if let Err(e) = cys::pack::write_atomic(&out, format!("{header}{body}").as_bytes()) {
        eprintln!("patch 쓰기 실패: {e}");
        return 1;
    }
    println!("✅ 제안 patch 생성: {}", out.display());
    if !secret_hits.is_empty() {
        println!(
            "⚠ 비밀/개인정보 의심 {}건 — 제출 전 반드시 검토·마스킹하세요:\n{}",
            secret_hits.len(),
            secret_hits.join("\n")
        );
    }
    println!("제출: 위 파일을 지원 채널(홈페이지 문의/저장소 이슈)에 첨부 — 자동 전송은 하지 않습니다.");
    0
}

/// diff -u 외부 명령으로 unified diff 생성(결정론) — 도구 부재·실행 실패면 None(호출측 폴백).
fn unified_diff_via_cmd(vendor: &str, mine: &str, rel: &str) -> Option<String> {
    let tmp = std::env::temp_dir().join(format!("cys-propose-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).ok()?;
    let a = tmp.join("vendor");
    let b = tmp.join("mine");
    std::fs::write(&a, vendor).ok()?;
    std::fs::write(&b, mine).ok()?;
    let out = std::process::Command::new("diff")
        .arg("-u")
        .arg("--label").arg(format!("vendor/{rel}"))
        .arg("--label").arg(format!("mine/{rel}"))
        .arg(&a)
        .arg(&b)
        .output();
    let _ = std::fs::remove_dir_all(&tmp);
    match out {
        // diff 종료코드: 0=동일, 1=차이(정상), 2+=오류.
        Ok(o) if o.status.code() == Some(0) || o.status.code() == Some(1) => {
            Some(String::from_utf8_lossy(&o.stdout).into_owned())
        }
        _ => None,
    }
}

/// ★W-E1(신뢰의 결정론 증명): 직전 설치 보존본(<pack>.prev — atomic_swap 이 1세대 보존)에서
/// **파일 단위** 복원. 전량 스왑 롤백은 업데이트 후 쌓인 런타임 상태(memory/·SESSION_STATE)까지
/// 과거로 되돌리는 신규 소실 사고를 만들므로 v1 은 파일 단위만 지원한다(전량은 오너 결정 보류).
/// seed-once 경로는 복원 대상에서 제외(상태 불가침 대칭). system 파일 복원은 다음 부트 스윕이
/// 재치유함을 정직하게 고지 — 영속 경로(--to-local/--propose)로 안내한다.
fn run_pack_rollback(file: Option<String>, yes: bool, force_vendor: bool, force_unsafe_core: bool) -> i32 {
    let dir = cys::pack::pack_dir();
    let prev = cys::pack::pack_prev_dir(&dir);
    if !prev.is_dir() {
        eprintln!("보존본 없음({}) — .prev 는 설치·업데이트가 1회 이상 실행된 뒤 생깁니다.", prev.display());
        return 1;
    }
    let prev_ver = std::fs::read_to_string(prev.join(".pack-version")).unwrap_or_else(|_| "?".into());
    let cur_ver = std::fs::read_to_string(dir.join(".pack-version")).unwrap_or_else(|_| "?".into());
    let Some(rel) = file else {
        // 목록 모드: .prev 와 현재 팩에서 내용이 다른 파일(복원 후보)을 표시. 읽기 전용.
        println!(
            "보존본: {} (팩 {} → 현재 {})\n차이 파일(복원 후보):",
            prev.display(), prev_ver.trim(), cur_ver.trim()
        );
        let mut count = 0usize;
        let mut stack = vec![prev.clone()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    // 내부 관리 디렉터리(.pristine)는 복원 대상 아님.
                    if p.file_name().and_then(|n| n.to_str()) == Some(".pristine") {
                        continue;
                    }
                    stack.push(p);
                    continue;
                }
                let Ok(rel_path) = p.strip_prefix(&prev) else { continue };
                let rel_s = rel_path.to_string_lossy().replace('\\', "/");
                // 관리 파일·병치 파일은 후보에서 제외(사용자 멘탈모델 = 팩 본 파일).
                if rel_s.starts_with('.') || rel_s.ends_with(".user") || rel_s.ends_with(".new") {
                    continue;
                }
                let prev_bytes = std::fs::read(&p).unwrap_or_default();
                let cur_bytes = std::fs::read(dir.join(rel_path)).unwrap_or_default();
                if prev_bytes != cur_bytes {
                    // ★G3 축2: 스코프 인지 라벨 — dept 팩의 soul.md 는 seed-once 로 표시(분류
                    // SOT 단일 통과 — 설치가 seed-once 로 다루는 파일을 목록이 user 로 말하면 안 된다).
                    let own = cys::pack::ownership_name_scoped(&rel_s, &dir);
                    println!("  [{own}] {rel_s}");
                    count += 1;
                    if count >= 200 {
                        println!("  … (200건 초과 — 생략)");
                        return 0;
                    }
                }
            }
        }
        if count == 0 {
            println!("  (없음 — 보존본과 현재 팩이 동일)");
        } else {
            println!("복원: cys pack-rollback --file <경로>");
        }
        return 0;
    };
    if rel.contains("..") || rel.starts_with('/') {
        eprintln!("잘못된 경로: {rel}");
        return 1;
    }
    // ★A12 승격 가드 — 롤백 '도구' 자체가 승격 파괴 벡터(R2 치명 분류): 승격 중 MASTER 후진 거부.
    if let Some(msg) = ceo_vendor_overwrite_rejection(&dir, &rel, force_vendor, CeoGuardVerb::Rollback) {
        eprintln!("{msg}");
        return 1;
    }
    // ★G3 축2: 스코프 인지 등급 — dept 팩의 soul.md 는 seed-once(승계 후 불가침)라 rollback
    // 덮어쓰기도 함께 거부된다(base 레인 거동·기존 메시지는 불변, dept soul 분기만 additive).
    let own = cys::pack::ownership_name_scoped(&rel, &dir);
    if own == "seed-once" {
        let dept_soul = cys::pack::dept_scope_of(&dir).is_some()
            && (rel == "soul.md" || rel.ends_with("/soul.md"));
        if dept_soul {
            eprintln!(
                "⛔ '{rel}' 은 부서 soul(seed-once) — base 헌장 승계 후 불가침이라 파일 단위 복원 \
                 대상에서 제외합니다(의도적 재시드는 파일 삭제 후 init-pack)."
            );
        } else {
            eprintln!(
                "⛔ '{rel}' 은 런타임 상태(seed-once) — 롤백이 업데이트 후 쌓인 기억·상태를 지우는 \
                 역방향 소실을 만들므로 파일 단위 복원 대상에서 제외합니다."
            );
        }
        return 1;
    }
    let src = prev.join(&rel);
    let content = match std::fs::read(&src) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("보존본에 '{rel}' 없음({e}) — cys pack-rollback 으로 후보 목록 확인");
            return 1;
        }
    };
    // ★G3-축3 대칭 지점: 롤백도 '현재본 → 보존본 전량 교체'라 take-new 와 같은 안전핵 소실
    //   벡터다(롤백 '도구'라서 더 위험 — A12 각주와 동일 취지). 헌법 파일이면 현재본(ours)
    //   대비 보존본(theirs)의 ours-only 소실을 같은 술어로 선검증한다(fail-closed).
    //   강제 진행의 실행 감사 라인은 실제 쓰기 성공 **후**에만 남긴다(미실행 사실의 원장 오염 금지).
    let mut forced_unsafe: Option<(String, String)> = None; // (verify_label, 교체 전 현재본)
    if cys::pack::is_constitution_file(&rel) {
        let cur = std::fs::read_to_string(dir.join(&rel)).unwrap_or_default();
        let prev_s = String::from_utf8_lossy(&content);
        if let Err(lost) = cys::overrides::verify_constitution_takeover(&cur, &prev_s) {
            eprint!("{}", takeover_risk_summary(&cur, &lost));
            let verify_label = format!("unsafe-core-lost:{}", lost.join(","));
            if !force_unsafe_core {
                let flags: Vec<String> = vec!["refused".into()];
                if let Err(e) = cys::pack::append_merge_audit(
                    &dir,
                    &merge_audit_entry(&rel, "rollback", &cur, &cur, &verify_label, &flags),
                ) {
                    eprintln!("⚠ 감사 원장 기록 실패(거부는 유효): {e}");
                }
                eprintln!(
                    "⛔ 거부(rc={EXIT_UNSAFE_CORE_REFUSED}): 보존본 복원이 현행 안전핵을 소실 — 승인하려면 --force-unsafe-core 를 명시하라."
                );
                return EXIT_UNSAFE_CORE_REFUSED;
            }
            eprintln!("⚠ --force-unsafe-core: 안전핵 소실 승인 상태로 복원 진행(감사 원장 기록).");
            forced_unsafe = Some((verify_label, cur));
        }
    }
    let confirm = |prompt: &str| -> bool {
        if yes {
            return true;
        }
        print!("{prompt} [y/N] ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        matches!(line.trim(), "y" | "Y" | "yes")
    };
    if !confirm(&format!("'{rel}' 을 보존본(팩 {})으로 복원?", prev_ver.trim())) {
        println!("취소됨");
        return 0;
    }
    let dest = dir.join(&rel);
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = cys::pack::write_atomic(&dest, &content) {
        eprintln!("복원 쓰기 실패: {e}");
        return 1;
    }
    // ★G3-축3: 안전핵 소실 승인 복원만 원장 기록 — 게이트가 발화한 파괴적 교체의 사후 추적.
    if let Some((label, before)) = &forced_unsafe {
        let after = String::from_utf8_lossy(&content).to_string();
        let flags: Vec<String> = vec!["force-unsafe-core".into()];
        if let Err(e) = cys::pack::append_merge_audit(
            &dir,
            &merge_audit_entry(&rel, "rollback", before, &after, label, &flags),
        ) {
            eprintln!("⚠ 감사 원장 기록 실패(복원은 완료): {e}");
        }
    }
    match own {
        "user" => println!("✅ {rel} 복원(user 소유 — 업데이트가 덮지 않으므로 이대로 유지됩니다)"),
        _ => println!(
            "✅ {rel} 복원(system 소유 — 다음 부트 설치 스윕이 vendor 본으로 재치유하며, 그때 \
             이 복원본은 {rel}.user 로 보존됩니다. 영속화: cys pack-merge --file {rel} --to-local(스킬) \
             또는 --propose(개선 제안)"
        ),
    }
    0
}

/// ★W-F3(표본 수집 도구): 이 기계의 커스터마이즈 실태를 로컬 리포트로 산출 — 병합 원장·보존본·
/// 오버레이·자작 파일 통계. 배포 사용자가 "무엇을 만들다 잃는가"의 실분포 표본을 자발 제출할 수
/// 있게 한다(자동 전송 없음 — 파일 생성+출력까지만. 개인 경로 등은 파일명 수준만 담는다).
fn run_doctor_custom_report() -> i32 {
    let dir = cys::pack::pack_dir();
    let local = cys::pack::local_dir();
    let mut md = String::new();
    md.push_str(&format!(
        "# cys 커스터마이즈 실태 리포트\n\n- 바이너리: {}\n- 팩: {}\n\n",
        env!("CARGO_PKG_VERSION"),
        std::fs::read_to_string(dir.join(".pack-version")).unwrap_or_else(|_| "?".into()).trim()
    ));
    // ① 병합 원장(무엇이 치유/병치됐나 — 소실 체감의 1차 표본).
    let pending = cys::pack::load_merge_pending(&dir);
    md.push_str(&format!("## 병합 대기 원장 ({}건)\n", pending.len()));
    for (rel, e) in pending.iter() {
        let kind = e.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
        let ver = e.get("version").and_then(|v| v.as_str()).unwrap_or("?");
        md.push_str(&format!("- [{kind}] {rel} (vendor {ver})\n"));
    }
    // ② 보존본(.user)·병치본(.new) 잔존 — 원장 밖 잔존물 포함(파일명만).
    let mut users: Vec<String> = Vec::new();
    let mut news: Vec<String> = Vec::new();
    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if p.file_name().and_then(|n| n.to_str()) == Some(".pristine") {
                    continue;
                }
                stack.push(p);
                continue;
            }
            let rel = p.strip_prefix(&dir).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            if rel.ends_with(".user") {
                users.push(rel);
            } else if rel.ends_with(".new") {
                news.push(rel);
            }
        }
    }
    users.sort();
    news.sort();
    md.push_str(&format!("\n## 보존본 .user ({}건)\n", users.len()));
    for r in &users {
        md.push_str(&format!("- {r}\n"));
    }
    md.push_str(&format!("\n## 병치본 .new ({}건)\n", news.len()));
    for r in &news {
        md.push_str(&format!("- {r}\n"));
    }
    // ③ 오버레이(~/.cys/local) 실사용 — 카테고리별 파일 수만(내용 비수집).
    md.push_str("\n## 오버레이(~/.cys/local) 사용\n");
    for cat in ["skills", "directives", "hooks", "bin", "notes", "proposals"] {
        let n = walk_count(&local.join(cat));
        if n > 0 {
            md.push_str(&format!("- {cat}: {n}개 파일\n"));
        }
    }
    // ④ 팩 안 비임베드 자작 파일(생존 보증 대상) — 임베드·관리 파일 제외.
    let embedded: std::collections::HashSet<&str> =
        cys::pack::PACK_ALL.iter().map(|(r, _)| *r).collect();
    let mut customs: Vec<String> = Vec::new();
    let mut stack2 = vec![dir.clone()];
    while let Some(d) = stack2.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if p.file_name().and_then(|n| n.to_str()) == Some(".pristine") {
                    continue;
                }
                stack2.push(p);
                continue;
            }
            let rel = p.strip_prefix(&dir).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            if rel.starts_with('.') || rel.ends_with(".user") || rel.ends_with(".new")
                || rel.starts_with("memory/") || rel.starts_with("round/")
                || embedded.contains(rel.as_str())
            {
                continue;
            }
            customs.push(rel);
        }
    }
    customs.sort();
    md.push_str(&format!("\n## 팩 안 비임베드 자작 파일 ({}건 — 업데이트 불가침 보증 대상)\n", customs.len()));
    for r in customs.iter().take(100) {
        md.push_str(&format!("- {r}\n"));
    }
    if customs.len() > 100 {
        md.push_str(&format!("- … 외 {}건\n", customs.len() - 100));
    }
    let out_dir = local.join("reports");
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("reports 디렉터리 생성 실패: {e}");
        return 1;
    }
    let out = out_dir.join(format!(
        "custom-report-{}.md",
        chrono::Local::now().format("%Y%m%dT%H%M%S")
    ));
    if let Err(e) = cys::pack::write_atomic(&out, md.as_bytes()) {
        eprintln!("리포트 쓰기 실패: {e}");
        return 1;
    }
    print!("{md}");
    println!("\n✅ 저장: {} — 개선에 도움이 됩니다: 지원 채널에 자발 제출(자동 전송 없음)", out.display());
    0
}

/// 디렉터리 트리의 파일 수(재귀·읽기 전용) — custom-report 보조.
fn walk_count(root: &std::path::Path) -> usize {
    let mut n = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                n += 1;
            }
        }
    }
    n
}

/// diff3 -m 3-way 병합(결정론) — base 부재·diff3 부재·충돌이면 None(호출측이 대안 안내).
fn diff3_merge(base: Option<&str>, ours: &str, theirs: &str) -> Option<String> {
    let base = base?;
    let tmp = std::env::temp_dir().join(format!("cys-merge-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).ok()?;
    let (po, pb, pt) = (tmp.join("ours"), tmp.join("base"), tmp.join("theirs"));
    std::fs::write(&po, ours).ok()?;
    std::fs::write(&pb, base).ok()?;
    std::fs::write(&pt, theirs).ok()?;
    let out = std::process::Command::new("diff3")
        .arg("-m")
        .args([&po, &pb, &pt])
        .output()
        .ok()?;
    let _ = std::fs::remove_dir_all(&tmp);
    // exit 0 = 무충돌 병합, 1 = 충돌(마커 포함 출력), 2+ = 에러.
    if out.status.code() == Some(0) {
        String::from_utf8(out.stdout).ok()
    } else {
        None
    }
}

/// AI 3-way 병합(차별점 ③) — claude 헤드리스로 '사용자 커스텀 의도를 신버전 베이스라인에 재적용'.
/// 산출물은 제안일 뿐 — 호출측이 diff 를 보여주고 승인받아 적용한다(producer≠approver).
fn ai_three_way_merge(rel: &str, base: Option<&str>, ours: &str, theirs: &str) -> Option<String> {
    // 본문 인라인(파일 경로 금지) — 경로를 주면 헤드리스가 파일 읽기 도구 라운드·권한에 걸려
    // hang/지연한다(실측). 인라인이면 단발 생성으로 끝난다. 총량 상한으로 컨텍스트 폭주 방지.
    const AI_MERGE_MAX: usize = 200_000;
    if ours.len() + theirs.len() + base.map_or(0, |b| b.len()) > AI_MERGE_MAX {
        eprintln!("파일이 너무 커서 AI 인라인 병합 불가({AI_MERGE_MAX}B 초과) — 수동 병합 후 --keep-mine 로 해소하라");
        return None;
    }
    let base_block = match base {
        Some(b) => format!("<<<공통 조상(내가 수정을 시작한 시점의 vendor 본)>>>\n{b}\n<<<끝>>>\n"),
        None => String::from("(공통 조상 없음 — 2-way: 내 수정 의도를 추론해 보존하라)\n"),
    };
    let prompt = format!(
        "다음은 cys 팩 파일 '{rel}' 의 3-way 병합 요청이다.\n\
         {base_block}\
         <<<내 수정본(의도를 보존해야 할 대상)>>>\n{ours}\n<<<끝>>>\n\
         <<<vendor 신버전(새 베이스라인)>>>\n{theirs}\n<<<끝>>>\n\
         규칙: vendor 신버전을 베이스로 삼고, 내 수정본이 조상 대비 바꾼 **의도**를 신버전 위에 재적용하라. \
         충돌 시 내 수정 의도를 우선하되 vendor 의 구조 변화를 존중하라. \
         출력은 병합된 파일의 **전체 내용만** — 설명·코드펜스·머리말 금지."
    );
    println!("(AI 병합 제안 생성 중 — claude 헤드리스, 최대 180초…)");
    // ★세션 env 스크럽(cysd scrub_claude_session_env 동형): claude 세션 안에서 실행하면 자식이
    // child-session 으로 강등·hang 하는 문제 차단. + 폴링 타임아웃(무한 대기 금지).
    let child = std::process::Command::new("claude")
        .args(["-p", &prompt, "--output-format", "text"])
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("CLAUDE_CODE_CHILD_SESSION")
        .env_remove("CLAUDECODE")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let result = child.ok().and_then(|mut c| {
        // stdout 은 별도 스레드로 동시 드레인 — 자식이 파이프 버퍼(64KB+)를 채우고 write 블록,
        // 부모는 try_wait 대기하는 상호 데드락을 차단한다(병합 파일은 64KB 를 넘을 수 있음).
        let drain = c.stdout.take().map(|mut out| {
            std::thread::spawn(move || {
                use std::io::Read;
                let mut s = String::new();
                let _ = out.read_to_string(&mut s);
                s
            })
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        let status = loop {
            match c.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {
                    if std::time::Instant::now() > deadline {
                        let _ = c.kill();
                        let _ = c.wait(); // zombie 수거(드레인 스레드도 EOF 로 종료)
                        eprintln!("claude 헤드리스 180초 타임아웃 — diff3/수동 경로를 사용하라");
                        break None;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Err(_) => break None,
            }
        };
        let stdout = drain.and_then(|h| h.join().ok()).unwrap_or_default();
        match status {
            Some(st) if st.success() => {
                let s = stdout.trim_end().to_string();
                if s.is_empty() { None } else { Some(s) }
            }
            _ => None,
        }
    });
    if result.is_none() {
        eprintln!("claude 헤드리스 병합 제안 실패 — diff3/수동 경로를 사용하라");
    }
    result
}

/// 경량 unified diff 출력(외부 의존 0) — 병합 제안 검토용 시각화.
fn print_unified_diff(old: &str, new: &str) {
    let ol: Vec<&str> = old.lines().collect();
    let nl: Vec<&str> = new.lines().collect();
    // 단순 LCS 없이 앞뒤 공통 접두/접미 제거 후 중간 블록만 표시(검토용 — 정밀 diff 는 도구 몫).
    let mut start = 0;
    while start < ol.len() && start < nl.len() && ol[start] == nl[start] {
        start += 1;
    }
    let (mut oe, mut ne) = (ol.len(), nl.len());
    while oe > start && ne > start && ol[oe - 1] == nl[ne - 1] {
        oe -= 1;
        ne -= 1;
    }
    if start == oe && start == ne {
        println!("(변경 없음)");
        return;
    }
    println!("@@ 줄 {}~ (구 {}줄 → 신 {}줄) @@", start + 1, oe - start, ne - start);
    for l in &ol[start..oe] {
        println!("- {l}");
    }
    for l in &nl[start..ne] {
        println!("+ {l}");
    }
}

/// ⑥ 사용자 스킬 정적 스캔 WARN 게이트(BLOCK 금지 — SkillSpector 연구의 WARN 원칙).
/// javis_skillscan.py(`scan <스킬 디렉토리>`)가 팩에 있으면 스캔해 발견사항을 경고로만 출력한다.
/// 사용자 오버레이는 사용자 책임 영역 — 차단하면 자기발화·커스터마이즈가 막힌다(WARN-not-BLOCK).
fn skillscan_warn(skill_dir: &std::path::Path) {
    let scanner = cys::pack::pack_dir().join("bin/javis_skillscan.py");
    if !scanner.exists() {
        return;
    }
    // ★SEAL-1: 동봉 python 해소 시 `.pyc` 번들 오염 차단(팩토리가 env 주입 — lib.rs SOT).
    match cys::python_command("python3")
        .arg(&scanner)
        .arg("scan")
        .arg(skill_dir)
        .output()
    {
        Ok(o) => {
            let out = String::from_utf8_lossy(&o.stdout);
            let flagged = out.contains("[BLOCK]")
                || out.contains("CRITICAL")
                || out.contains("HIGH")
                || out.contains("MEDIUM");
            if flagged {
                eprintln!("⚠ skillscan WARN — 차단 아님(사용자 오버레이는 사용자 책임 영역), 검토 권장:");
                for line in out.trim().lines().take(20) {
                    eprintln!("  {line}");
                }
            }
        }
        Err(_) => {} // python3 부재 등 — WARN 게이트는 best-effort
    }
}

fn run_pack_update(from: Option<String>, manifest_url: Option<String>, dry_run: bool) -> i32 {
    // 성공 경로는 종료코드(i32)를 싣는다: 0=완전 성공, EXIT_REINJECT_DEGRADED=디스크는 반영됐으나
    // 라이브 노드 reinject 실패(성공 침묵 포장 금지). 에러 경로(Err)는 외부에서 1로 매핑.
    let result = (|| -> Result<i32, String> {
        let base = pack_state_base();
        let staging = base.join(".pack-staging");
        let lock_path = base.join(".pack-apply.lock");
        let accepted_path = base.join(".pack-accepted.json");

        // 착수 시 crash recovery(§7-⑤): 직전 pack-update가 apply 도중 죽어 orphan 저널이 남았으면
        // 먼저 자가치유한다(미커밋=rollback / 커밋완료=정리). dry-run·UpToDate 경로도 거치도록
        // 소스 해석 전에, apply-lock 보유 하에 1회 수행한다.
        with_apply_lock(&lock_path, cys::pack::recover_pack_journal)??;

        // LOW#1: 착수 시 1회 — 직전 pack-update가 busy로 보류(deferred)한 노드를 능동 재시도한다.
        // version gate 판정 전·독립(디스크 팩이 이미 그 버전이라 UpToDate여도 동작): 보류 당시 busy였던
        // 노드가 지금 idle이면 reinject를 완료하고 pending에서 제거한다. dry-run은 부작용 없음 계약이라
        // 생략. 데몬 미가동이면 graceful 스킵(Err 로깅·pending 보존).
        if !dry_run {
            match consume_reinject_pending(&base) {
                Ok((resolved, kept)) if resolved > 0 || kept > 0 => {
                    println!(
                        "[pack-update] pending reinject 소비: {resolved} 해소, {kept} 잔존."
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[pack-update] pending reinject 소비 스킵(데몬 점검 필요): {e}")
                }
            }
        }

        // 소스 해석: --from(로컬 디렉터리) 우선. --manifest-url은 staging에 fetch(부차).
        let from_dir: std::path::PathBuf = match (from, manifest_url) {
            (Some(d), _) => std::path::PathBuf::from(d),
            (None, Some(url)) => fetch_remote_pack(&url, &base)?,
            (None, None) => return Err("--from <dir> 또는 --manifest-url <url> 필요".into()),
        };

        let now_unix = chrono::Utc::now().timestamp();
        let running = env!("CARGO_PKG_VERSION");
        let keyring = cys::packsig::embedded_keyring()?;
        let outcome = pack_update_from_dir(
            &from_dir,
            &staging,
            &lock_path,
            &accepted_path,
            now_unix,
            running,
            &keyring,
            !dry_run,
        )?;

        match outcome.gate {
            VersionGate::UpToDate => {
                println!(
                    "[pack-update] 이미 최신 — 반영 0 (remote {} ≤ 디스크). no-op.",
                    outcome.pack_version
                );
                return Ok(0);
            }
            VersionGate::BinaryTooOld => {
                eprintln!(
                    "[pack-update] 거부 — 팩 {}이 더 새 바이너리를 요구한다(min_binary > 실행 {running}). \
                     바이너리 업데이트(재시작) 경로로 진행하세요.",
                    outcome.pack_version
                );
                return Err("binary-too-old".into());
            }
            VersionGate::Apply => {}
        }

        if dry_run {
            println!(
                "[pack-update] dry-run: 검증·게이트 통과(팩 {} 반영 가능) — 디스크 반영·reinject 생략.",
                outcome.pack_version
            );
            return Ok(0);
        }

        println!(
            "[pack-update] 팩 {} 반영 완료 ({} written, {} preserved). 노드 reinject 점검…",
            outcome.pack_version, outcome.written, outcome.kept
        );
        // v5 §3: post-commit accepted 실패는 디스크 반영 성공과 구분 보고(침묵 포장 금지) —
        // 아래 reinject 결과와 무관하게 최종 종료코드를 EXIT_ACCEPTED_DEGRADED로 승격한다.
        let accepted_degraded = !outcome.accepted_recorded;

        // 6) 살아있는 노드 reinject(§7-②) — 베스트에포트(데몬 미가동 시 경고만).
        //    디스크 반영은 이미 성공(commit). reinject 결과는 별도 신호로 전파한다:
        //    failed>0 → 종료코드 EXIT_REINJECT_DEGRADED + 경고(성공 침묵 포장 금지),
        //    deferred>0 → pending 영속(다음 pack-update/노드 idle 시 재시도) + 경고.
        match run_pack_reinject(&outcome.pack_version) {
            Ok(rep) => {
                println!(
                    "[pack-update] reinject: {} injected, {} skipped, {} deferred, {} failed.",
                    rep.injected, rep.skipped, rep.deferred, rep.failed
                );
                // 구조화 출력(Tauri 브리지가 failed/deferred를 파싱해 update-warning emit).
                println!(
                    "{} pack_version={} injected={} skipped={} deferred={} failed={}",
                    cys::pack::REINJECT_RESULT_PREFIX,
                    outcome.pack_version,
                    rep.injected,
                    rep.skipped,
                    rep.deferred,
                    rep.failed
                );
                // deferred(busy) 노드 pending 영속 / 없으면 stale 제거(가시화·재시도 SOT).
                if let Err(e) =
                    persist_reinject_pending(&base, &outcome.pack_version, &rep.deferred_nodes)
                {
                    eprintln!("[pack-update] ⚠ deferred pending 영속 실패: {e}");
                }
                if rep.deferred > 0 {
                    eprintln!(
                        "[pack-update] ⚠ {} 노드 busy → reinject 보류(pending 영속: {}). \
                         다음 pack-update 또는 노드 idle 시 재시도됩니다.",
                        rep.deferred,
                        reinject_pending_path(&base).display()
                    );
                }
                if rep.failed > 0 {
                    eprintln!(
                        "[pack-update] ⚠ {} 노드 reinject 실패 — 디스크 팩은 {} 로 갱신됐으나 해당 \
                         노드는 미각성(이전 지침으로 동작). 디스크 반영은 성공이라 롤백하지 않음. \
                         다음 pack-update에서 재시도됩니다(성공으로 침묵 포장하지 않음).",
                        rep.failed, outcome.pack_version
                    );
                }
                if accepted_degraded {
                    return Ok(cys::pack::EXIT_ACCEPTED_DEGRADED);
                }
                Ok(reinject_exit_code(rep.failed))
            }
            // 데몬 미가동 등으로 reinject 자체를 못 함 — 디스크 반영은 성공(무중단 정책상 0).
            Err(e) => {
                eprintln!("[pack-update] reinject 스킵(데몬 점검 필요): {e}");
                if accepted_degraded {
                    return Ok(cys::pack::EXIT_ACCEPTED_DEGRADED);
                }
                Ok(0)
            }
        }
    })();
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// 원격 팩 fetch(부차) — 시스템 curl shell-out으로 manifest·sig·tar를 staging 형제 디렉터리에 받는다.
/// 핵심 검증·반영 로직은 --from과 동일 경로(pack_update_from_dir)를 탄다.
fn fetch_remote_pack(manifest_url: &str, base: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let dl = base.join(".pack-download");
    let _ = std::fs::remove_dir_all(&dl);
    std::fs::create_dir_all(&dl).map_err(|e| format!("download dir 생성 실패: {e}"))?;
    // manifest_url 형제 경로로 sig·tar URL 유도(같은 디렉터리에 동봉).
    let base_url = manifest_url
        .rsplit_once('/')
        .map(|(b, _)| b.to_string())
        .ok_or("manifest-url 형식 오류")?;
    for (url, name) in [
        (manifest_url.to_string(), "pack-manifest.json"),
        (format!("{base_url}/pack-manifest.json.minisig"), "pack-manifest.json.minisig"),
        (format!("{base_url}/pack.tar.gz"), "pack.tar.gz"),
    ] {
        let out = dl.join(name);
        // R-CLI-3: URL 앞에 `--`(옵션 종결자)를 둔다. manifest_url이 원격/입력 유래라 `-`로 시작하면
        // curl 플래그로 해석되던 인자 주입을 차단(옵션 파싱 종료 후 URL을 위치 인자로 강제).
        let status = std::process::Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&out)
            .arg("--")
            .arg(&url)
            .status()
            .map_err(|e| format!("curl 실행 실패: {e}"))?;
        if !status.success() {
            return Err(format!("fetch 실패({name}): {url}"));
        }
    }
    Ok(dl)
}

/// 완화책 ③: scoped 실행 — 새 프로세스 그룹에서 실행하고 원장에 등록,
/// 종료 시 그룹 전체를 강제 종료하여 서버가 절대 누적되지 않게 한다.
/// 자식의 종료 코드를 그대로 반환한다 (시그널 사망 = 128+signo).
fn run_scoped(surface: Option<String>, command: Vec<String>) -> Result<i32, String> {
    use cys::SpawnPolicy;
    if command.is_empty() {
        return Err("no command given".into());
    }
    let sid = parse_explicit_surface(&surface)?
        .or_else(|| cys::env_compat(ENV_SURFACE_ID).and_then(|s| parse_surface_ref(&s)));

    let mut cmd = std::process::Command::new(&command[0]);
    cmd.args(&command[1..]);
    // ★SEAL-1 방어심도: `cys run -- <명령>` 은 CLAUDE.md 가 워커 표준 실행 형태로 규정한 경로라
    // 임의의 명령(그 안의 python 포함)이 여기로 들어온다. 임의 명령 스폰이라 python_command
    // 팩토리를 못 쓰므로 같은 상수를 직접 소비한다(규약 산재 아님 · 정본 = lib.rs
    // ENV_PY_NO_BYTECODE). python 이 아닌 자식에겐 무해한 무시 변수다.
    cmd.env(cys::ENV_PY_NO_BYTECODE, cys::PY_NO_BYTECODE_ON);
    // ★U-7: 등급 `ConsoleScoped` — unix 는 setsid 로 떼고(아래 SIGINT/SIGTERM/SIGHUP 핸들러가
    // killpg 로 회수한다), **Windows 는 일부러 떼지 않는다**. 근거는 회수 수단의 비대칭이다:
    // 이 함수의 kill_group 은 `child.wait()` 이 돌아온 **뒤에만** 실행되는데, Windows 에는
    // 대응하는 콘솔 컨트롤 핸들러가 없어 CLI 가 Ctrl-C 로 죽으면 그 지점에 영영 도달하지
    // 못한다. 지금은 자식이 같은 콘솔 그룹에 있어 Ctrl-C 가 자식에게도 전파되는 것이 유일한
    // 안전망이다 — 회수 수단 없는 분리는 개선이 아니라 **영구 고아**(자원 누적)다.
    // `CREATE_NO_WINDOW` 도 금지: 사용자가 실행을 요청한 명령의 출력을 가린다.
    // (Windows 콘솔 핸들러가 생기면 이 등급을 GroupScoped 로 올린다 — 그 전엔 아니다.)
    cmd.spawn_policy(cys::ChildLifetime::ConsoleScoped);
    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    let pid = child.id();
    let pgid = pid as i64; // setsid → pgid == pid (unix); ignored on windows

    // setsid로 분리된 자식은 터미널 시그널(Ctrl-C 등)에 면역 — CLI가 죽기 전에
    // 그룹을 대신 죽여야 '종료 시 그룹 강제 종료' 보장이 유지된다.
    // (원장 deregister는 핸들러에서 생략 — dead-pid 항목은 watchdog이 자동 회수)
    #[cfg(unix)]
    {
        SCOPED_PGID.store(pgid as i32, std::sync::atomic::Ordering::SeqCst);
        let handler =
            scoped_cleanup_handler as extern "C" fn(libc::c_int) as *const () as libc::sighandler_t;
        unsafe {
            libc::signal(libc::SIGINT, handler);
            libc::signal(libc::SIGTERM, handler);
            libc::signal(libc::SIGHUP, handler);
        }
    }

    if let Err(e) = request(
        "ledger.register",
        json!({"pid": pid, "pgid": pgid, "cmd": command.join(" "), "surface_id": sid, "scoped": true}),
    ) {
        // 등록 실패 = 데몬이 생명주기를 보장할 수 없음 → 그룹 즉시 강제 종료.
        // 살려두면 어떤 거버넌스(watchdog·reap_orphan_ledger)에도 안 보이는 영구 고아가 된다.
        kill_group(pid, pgid);
        let _ = child.wait();
        return Err(format!(
            "ledger.register failed — scoped group killed (pid={pid}): {e}"
        ));
    }
    eprintln!("[scoped pid={pid} registered in ledger]");

    let wait_res = child.wait();

    // Force-kill the whole group: anything the command left behind dies with it.
    // wait가 Err여도 정리는 무조건 수행한다.
    kill_group(pid, pgid);
    let _ = request("ledger.deregister", json!({"pid": pid}));

    let status = wait_res.map_err(|e| e.to_string())?;
    #[cfg(unix)]
    let code = status.code().unwrap_or_else(|| {
        use std::os::unix::process::ExitStatusExt;
        status.signal().map(|s| 128 + s).unwrap_or(1)
    });
    #[cfg(not(unix))]
    let code = status.code().unwrap_or(1);
    eprintln!("[scoped pid={pid} exited ({status}); process group force-killed and deregistered]");
    Ok(code)
}

fn kill_group(pid: u32, pgid: i64) {
    #[cfg(unix)]
    {
        let _ = pid;
        unsafe {
            libc::killpg(pgid as i32, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let _ = pgid;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}

#[cfg(unix)]
static SCOPED_PGID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// async-signal-safe 핸들러: killpg·_exit만 호출 (소켓 I/O·할당 금지)
#[cfg(unix)]
extern "C" fn scoped_cleanup_handler(sig: libc::c_int) {
    let pgid = SCOPED_PGID.load(std::sync::atomic::Ordering::SeqCst);
    if pgid > 0 {
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
    unsafe { libc::_exit(128 + sig) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★A12 승격 가드 단위 테스트(v4 · W4): 승격 중(.pre-ceo 존재) base MASTER 를 덮는
    /// 두 동사(take-new·rollback)만 거부 — keep-mine 경로·타 파일·비승격 상태·--force-vendor
    /// 는 통과. 거부 문안에 keep-mine 절차(merge)/재설치·promote-ceo(rollback) 안내가 실린다.
    #[test]
    fn ceo_vendor_overwrite_rejection_truth_table() {
        let td = std::env::temp_dir().join(format!("cys-a12-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(td.join("directives")).unwrap();
        let rel = MASTER_DIRECTIVE_REL;

        // ① 비승격(.pre-ceo 부재): 양 동사 모두 통과.
        for verb in [CeoGuardVerb::TakeNew, CeoGuardVerb::Rollback] {
            assert!(
                ceo_vendor_overwrite_rejection(&td, rel, false, verb).is_none(),
                ".pre-ceo 부재면 가드가 발화하면 안 된다"
            );
        }

        // 승격 상태 진입: .pre-ceo 실재.
        std::fs::write(td.join(format!("{rel}.pre-ceo")), "backup").unwrap();

        // ② 승격 중 take-new → 거부 + keep-mine 절차 안내.
        let msg = ceo_vendor_overwrite_rejection(&td, rel, false, CeoGuardVerb::TakeNew)
            .expect("승격 중 --take-new 는 거부돼야 한다");
        assert!(msg.contains("--keep-mine"), "keep-mine 절차 안내 누락: {msg}");
        assert!(msg.contains("--force-vendor"), "override 안내 누락: {msg}");

        // ③ 승격 중 rollback → 거부 + 재설치·promote-ceo 정본 경로 안내.
        let msg = ceo_vendor_overwrite_rejection(&td, rel, false, CeoGuardVerb::Rollback)
            .expect("승격 중 pack-rollback 은 거부돼야 한다");
        assert!(msg.contains("promote-ceo"), "정본 롤백 경로 안내 누락: {msg}");

        // ④ --force-vendor override → 통과(승격 파괴 승인은 오너 명시 의사).
        for verb in [CeoGuardVerb::TakeNew, CeoGuardVerb::Rollback] {
            assert!(
                ceo_vendor_overwrite_rejection(&td, rel, true, verb).is_none(),
                "--force-vendor 는 가드를 통과해야 한다"
            );
        }

        // ⑤ 타 파일(WORKER 등)은 승격 중에도 무간섭.
        assert!(
            ceo_vendor_overwrite_rejection(
                &td,
                "directives/WORKER_DIRECTIVE.md",
                false,
                CeoGuardVerb::TakeNew
            )
            .is_none(),
            "MASTER 외 파일은 가드 대상이 아니다"
        );
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★G3-축3 게이트 exit 계약 핀: 게이트 거부는 예약 exit({0,1,2,64} — clap 사용오류=2)과
    /// 충돌 금지. 신 팩+구 바이너리 스큐에서 '플래그 부재(clap 2)'와 '게이트 거부'가 구분돼야
    /// 소비 스크립트가 오진하지 않는다(claim-role 정당거부 rc=7 선례 계열).
    #[test]
    fn unsafe_core_refused_exit_is_unreserved() {
        for reserved in [0, 1, 2, 64] {
            assert_ne!(
                EXIT_UNSAFE_CORE_REFUSED, reserved,
                "게이트 거부 exit 가 예약 코드와 충돌: {reserved}"
            );
        }
        assert_eq!(EXIT_UNSAFE_CORE_REFUSED, 7, "claim-role 정당거부(7) 계열 고정 — 소비부 파리티");
    }

    /// ★G1(W2-E) queue.deliver 게이트 exit 계약 핀 — [성찰 BLOCKER: 설계 원문의 exit 2 는
    /// clap 사용오류(2)와 충돌 → 7 계열로 확정] 게이트 거부 6코드 = exit 7(예약 {0,1,2,64}
    /// 비충돌 · claim-role rc=7 선례 계열), 조준 실패·경합·통신 오류 = 일반 오류 1.
    /// 판정은 request() 에러 문면("code: message")의 code **접두** — 데몬
    /// ForceDeliverDenied::code() + handlers 게이트 ①②와 1:1 계약이다.
    #[test]
    fn queue_deliver_gate_exit_is_seven_and_unreserved() {
        for reserved in [0, 1, 2, 64] {
            assert_ne!(
                EXIT_QUEUE_GATE_REFUSED, reserved,
                "게이트 거부 exit 가 예약 코드와 충돌: {reserved}"
            );
        }
        assert_eq!(EXIT_QUEUE_GATE_REFUSED, 7, "claim-role 정당거부(7) 선례 계열 고정");
        // 안전 게이트 거부 6종 → 7 (kill-switch·ACL·좌석·사람·헬스 pause·출력 quiet 하한).
        for gate in [
            "paused: daemon paused (kill-switch)",
            "acl_denied: reviewer-* → worker*",
            "empty_seat: role seat has no agent",
            "typing_guard: human typed recently",
            "queue_paused: health action",
            "output_busy: output streaming",
        ] {
            assert_eq!(queue_deliver_exit_code(gate), EXIT_QUEUE_GATE_REFUSED, "{gate}");
        }
        // 조준 실패·경합·통신 오류 → 1 (게이트 거부와 구분 — 소비 스크립트 오진 방지).
        for err in [
            "not_found: entry_id not in pending queue",
            "queue_empty: pending queue is empty",
            "not_head_requires_allow_reorder: entry is at index 2",
            "delivery_failed: delivery raced",
            "process_exited: surface process has exited",
            "invalid_params: missing surface_id",
            "abi: LenMismatch",
            "connect: no daemon",
        ] {
            assert_eq!(queue_deliver_exit_code(err), 1, "{err}");
        }
        // 접두 판정 핀: 게이트 코드가 문자열 중간·유사 접두에 있어도 오분류하지 않는다.
        assert_eq!(queue_deliver_exit_code("paused_x: y"), 1, "유사 접두는 게이트 아님");
        assert_eq!(queue_deliver_exit_code("error: paused: nested"), 1, "중간 등장은 게이트 아님");
    }

    /// ★G3 축1 hooks-prune 게이트 exit 계약 핀 — base 팩 대상 + --allow-base 부재 = 7
    /// (claim-role 정당거부(7) 계열 · 예약 {0,1,2,64} 비충돌), IO·파싱 거부 = 1, 정상/대상없음 = 0.
    /// 게이트 순수부(hooks_prune_gate_refused) 진리표 동봉 — 부서 전용 기본(fail-closed).
    #[test]
    fn hooks_prune_gate_exit_is_seven_and_dept_only_by_default() {
        for reserved in [0, 1, 2, 64] {
            assert_ne!(
                EXIT_HOOKS_PRUNE_GATE_REFUSED, reserved,
                "게이트 거부 exit 가 예약 코드와 충돌: {reserved}"
            );
        }
        assert_eq!(EXIT_HOOKS_PRUNE_GATE_REFUSED, 7, "claim-role 정당거부(7) 계열 고정");
        let dept = std::path::Path::new("/h/.cys/pack-dept-d1");
        let base = std::path::Path::new("/h/.cys/pack");
        assert!(!hooks_prune_gate_refused(dept, false), "부서 팩은 기본 통과");
        assert!(!hooks_prune_gate_refused(dept, true));
        assert!(hooks_prune_gate_refused(base, false), "base 팩은 --allow-base 없이는 거부");
        assert!(!hooks_prune_gate_refused(base, true), "--allow-base 명시 시 통과");
        assert!(
            hooks_prune_gate_refused(std::path::Path::new("/h/.cys/pack-dept-"), false),
            "빈 부서명(불량 레인)은 부서로 인정하지 않는다"
        );
    }

    /// ★G3 축1 dept-hook-residue 탐지 순수부 — 훅 명령에서 부서 팩 루트 추출(경계·정규화).
    #[test]
    fn dept_pack_of_command_matrix() {
        let base = std::path::Path::new("/h/.cys");
        assert_eq!(
            dept_pack_of_command("sh /h/.cys/pack-dept-d1/hooks/session-start.sh", base),
            Some(std::path::PathBuf::from("/h/.cys/pack-dept-d1"))
        );
        // Windows 훅 명령(역슬래시·quote) 정규화 — 백슬래시 base 와 명령 모두
        assert_eq!(
            dept_pack_of_command(
                "bash \"C:/Users/user/.cys/pack-dept-sales/hooks/a.sh\"",
                std::path::Path::new("C:\\Users\\user\\.cys")
            ),
            Some(std::path::PathBuf::from("C:\\Users\\user\\.cys").join("pack-dept-sales"))
        );
        assert_eq!(dept_pack_of_command("sh /h/.cys/pack/hooks/a.sh", base), None, "base 팩 무탐지");
        assert_eq!(dept_pack_of_command("sh /h/.cys/pack-dept-", base), None, "빈 부서명");
        assert_eq!(
            dept_pack_of_command("sh /elsewhere/.cys/pack-dept-d1/h.sh", base),
            None,
            "타 base 아래는 이 설치의 부서가 아니다"
        );
    }

    /// ★G3 축1 부서 acctdir 해소(agents.json 시드값) — cys-dept `pack_seeded_acct` 동일 규약 핀.
    #[test]
    fn dept_seeded_acct_dir_resolves_env_and_legacy_cmd() {
        let td = std::env::temp_dir().join(format!("cys-acctdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let pack = td.join("pack-dept-d1");
        std::fs::create_dir_all(&pack).unwrap();
        // ① 신구조: env 맵
        std::fs::write(
            pack.join("agents.json"),
            r#"{"claude":{"cmd":"claude","env":{"CLAUDE_CONFIG_DIR":"/h/.cys/claude-d1"}}}"#,
        )
        .unwrap();
        assert_eq!(
            dept_seeded_acct_dir(&pack),
            Some(std::path::PathBuf::from("/h/.cys/claude-d1"))
        );
        // ② 레거시: cmd 인라인 리터럴
        std::fs::write(
            pack.join("agents.json"),
            r#"{"claude":{"cmd":"CLAUDE_CONFIG_DIR=\"/h/.cys/claude-legacy\" claude"}}"#,
        )
        .unwrap();
        assert_eq!(
            dept_seeded_acct_dir(&pack),
            Some(std::path::PathBuf::from("/h/.cys/claude-legacy"))
        );
        // ③ 미시드·부재 = None(계정격리 미사용 부서 — 실측 불가는 제거 부적격으로 흐른다)
        std::fs::write(pack.join("agents.json"), r#"{"claude":{"cmd":"claude"}}"#).unwrap();
        assert_eq!(dept_seeded_acct_dir(&pack), None);
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★G4(W4-C) reap-surface 게이트 exit 계약 핀 — reap_denied(사유 8종 어느 것이든) = 7
    /// (claim-role rc=7 선례 계열 · 예약 {0,1,2,64} 비충돌), 그 외(not_found·invalid·통신) = 1.
    /// 소비 스크립트(javis_reap_exited.py)가 rc=7 + stderr 사유 코드로 분기하는 계약의 CLI 측.
    #[test]
    fn reap_surface_gate_exit_is_seven_and_unreserved() {
        // 게이트 거부(reap_denied 접두) — 사유 코드 8종 전부 exit 7.
        for reason in [
            "caller_unresolved",
            "caller_role_forbidden",
            "active_surface",
            "agent_still_alive",
            "queue_not_empty",
            "daemon_ancestor",
            "grace_not_elapsed",
            "state_changed",
        ] {
            let err = format!("reap_denied: surface.reap denied: {reason}");
            assert_eq!(reap_surface_exit_code(&err), 7, "{err}");
        }
        // 게이트 밖(대상 없음·파라미터·통신·프레임) → 일반 오류 1.
        for err in [
            "not_found: surface 9 not found",
            "invalid_params: missing surface_id",
            "abi: LenMismatch",
            "connect: no daemon",
        ] {
            assert_eq!(reap_surface_exit_code(err), 1, "{err}");
        }
        // 접두 판정 핀 — 중간 등장은 게이트 아님.
        assert_eq!(reap_surface_exit_code("error: reap_denied: nested"), 1);
    }

    /// ★G3-축3 위험 요약 핀: 소실 키워드 계수와 사라질 ours 조항 줄이 적시되고, 무관 줄은
    /// 혼입되지 않는다 — 게이트 화면의 판단 재료 최소 완비.
    #[test]
    fn takeover_risk_summary_lists_lost_lines_only() {
        let ours = "- autopilot denylist 준수\n- kill-switch 즉시 정지\n- 무관 조항";
        let lost = vec!["denylist".to_string(), "kill-switch".to_string()];
        let s = takeover_risk_summary(ours, &lost);
        assert!(s.contains("소실 2건"), "소실 계수 누락: {s}");
        assert!(s.contains("denylist 준수"), "사라질 조항 줄 미적시: {s}");
        assert!(s.contains("kill-switch 즉시 정지"), "사라질 조항 줄 미적시: {s}");
        assert!(!s.contains("무관 조항"), "무관 줄 혼입: {s}");
    }

    /// ★G3-축3 감사 엔트리 스키마 핀: 8필드 계약({ts,file,action,actor_os_user,before_sha256,
    /// after_sha256,verify_result,flags}) + sha256 은 pack 해시 SOT(content_hash_pub)와 동일.
    #[test]
    fn merge_audit_entry_has_contract_fields() {
        let e = merge_audit_entry("soul.md", "take-new", "A", "B", "ok", &["yes".to_string()]);
        for k in [
            "ts", "file", "action", "actor_os_user", "before_sha256", "after_sha256",
            "verify_result", "flags",
        ] {
            assert!(e.get(k).is_some(), "감사 필드 누락: {k}");
        }
        assert_eq!(e["before_sha256"], json!(cys::pack::content_hash_pub("A")));
        assert_eq!(e["after_sha256"], json!(cys::pack::content_hash_pub("B")));
        assert_eq!(e["flags"], json!(["yes"]));
        // 직렬화 1줄 보장(원장 라인 규율) — 개행이 이스케이프돼 물리 개행 0.
        let line = serde_json::to_string(&e).unwrap();
        assert!(!line.contains('\n'), "엔트리 직렬화에 물리 개행 혼입");
    }

    /// ★(W4 · D5 관측) alt_screen_notice 진리표 — 필드 부재(None)=판정 불가·무발화(FAIL 금지),
    /// mac claude fullscreen=WARN+reason 부기, win claude fullscreen=힌트(경보 아님·부기 없음),
    /// false/타 에이전트=무발화.
    #[test]
    fn alt_screen_notice_truth_table() {
        // 구 데몬(필드 부재) → 판정 불가, 어떤 OS 에서도 무발화.
        assert!(alt_screen_notice(None, "claude", true, false).is_none());
        assert!(alt_screen_notice(None, "claude", false, true).is_none());
        // 정상(inline) → 무발화.
        assert!(alt_screen_notice(Some(false), "claude", true, false).is_none());
        // mac ∧ claude ∧ true → WARN + verify reason 부기.
        let (line, attach) = alt_screen_notice(Some(true), "claude", true, false)
            .expect("mac claude fullscreen 은 WARN");
        assert!(line.contains("WARN"), "WARN 표식 누락: {line}");
        assert!(attach, "mac WARN 은 directive.verify reason 에 부기해야 한다");
        // win ∧ claude ∧ true → 힌트(경보 아님) + 부기 없음.
        let (line, attach) = alt_screen_notice(Some(true), "claude", false, true)
            .expect("win claude fullscreen 은 힌트");
        assert!(line.contains("hint"), "힌트는 경보(WARN)가 아니어야 한다: {line}");
        assert!(!attach, "win 힌트는 reason 부기 없음");
        // 타 에이전트(codex 등) → 무발화.
        assert!(alt_screen_notice(Some(true), "codex", true, false).is_none());
        // 기타 OS(linux 등) → 무발화.
        assert!(alt_screen_notice(Some(true), "claude", false, false).is_none());
    }

    // ★루트 cwd 교정(2026-07-15 실사고): 루트류는 home으로, 정상 경로는 불변.
    #[test]
    fn sanitize_launch_cwd_truth_table() {
        let home = cys::home_dir().to_string_lossy().into_owned();
        assert_eq!(sanitize_launch_cwd("/".into()), home);
        assert_eq!(sanitize_launch_cwd("\\".into()), home);
        assert_eq!(sanitize_launch_cwd("C:\\".into()), home);
        assert_eq!(sanitize_launch_cwd("/Users/x".into()), "/Users/x");
        assert_eq!(sanitize_launch_cwd("/Users/x/".into()), "/Users/x/");
        assert_eq!(sanitize_launch_cwd("C:\\work".into()), "C:\\work");
    }

    // pack-update·compose 통합테스트는 동일 전역 env(ENV_PACK_DIR/ENV_CONFIG_DIR/ENV_SOCKET)를
    // set/remove하므로 단일 뮤텍스로 직렬화한다. 옛 PACK_UPDATE_ENV_LOCK·COMPOSE_ENV_LOCK가 별개라
    // 두 그룹이 병렬 교차하면 None 복원 시 remove_var가 실행 중 테스트를 실 ~/.cys/pack으로
    // 폴백시켜 삭제하던 레이스를 차단한다(HIGH 감사).
    //
    // ★poison 내성(결함 2 · 2026-08-24 이종 리뷰어 · handlers.rs U-18 과 같은 패턴):
    //   `lock().unwrap()` 은 **한 건의 실패를 여러 건의 실패로 증폭**한다 — 이 뮤텍스를 쥔 채
    //   패닉한 검체가 하나 나오면 뒤따르는 검체 전원이 `PoisonError` 로 죽고, 그 순간 **어느
    //   핀이 실제로 발화했는지 읽을 수 없게 된다**(릴리스 레인 실행에서 실측: 1 건의 실패가
    //   2 건을 더 죽였다). 이 락이 지키는 것은 전역 env 의 **직렬화**이지 데이터 불변식이 아니다
    //   — 앞 검체가 env 복원을 못 하고 죽었다면 그 사실은 그 검체의 실패로 이미 보고되고,
    //   뒤 검체는 자기 값을 다시 `set_var` 하므로 poison 을 무시하는 것이 정확하다.
    //   ★규약: 이 파일의 env 뮤텍스는 **전부** `unwrap_or_else(|e| e.into_inner())` 를 쓴다
    //   (소스 핀 `env_mutexes_are_poison_tolerant_source_pin` 이 집행).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn sha256_of(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes))
    }

    /// ★W-B 보완 핀(성찰 2 산물): agents.json 을 user 승격하면 사용자 수정본이 동결돼 vendor
    /// **신규 어댑터**가 영영 안 보이는 게 대가다(schedule.json 의 ensure_builtin_jobs 등가물이
    /// agents.json 엔 없음). load_agent_spec 이 ①디스크 정의 우선(수정 보존) ②디스크에 없는
    /// 키만 임베드 폴백(신기능 즉시 사용) ③둘 다 없으면 거부 — 합집합을 박제한다.
    #[test]
    fn load_agent_spec_disk_wins_and_embed_fills_missing() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-agentspec-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&td);
        std::env::set_var(cys::pack::ENV_PACK_DIR, &td);

        // 임베드에 실재하는 어댑터 키 2개를 사료로 삼는다(하드코딩 금지 — 팩 진실에서 취득).
        let embed: serde_json::Value = cys::pack::PACK_ALL
            .iter()
            .find(|(r, _)| *r == "agents.json")
            .map(|(_, c)| serde_json::from_str(c).expect("임베드 agents.json 파싱"))
            .expect("임베드에 agents.json 존재");
        let keys: Vec<String> = embed
            .as_object()
            .expect("객체")
            .keys()
            .filter(|k| !k.starts_with('_'))
            .cloned()
            .collect();
        assert!(keys.len() >= 2, "어댑터 2개 이상 전제: {keys:?}");
        let (mine_key, vendor_only_key) = (&keys[0], &keys[1]);

        // 사용자본: 첫 어댑터만 보유 + 수정 흔적. 둘째 어댑터는 없음(구버전 파일 재현).
        let mut mine = serde_json::Map::new();
        let mut spec = embed[mine_key].clone();
        spec["notes"] = serde_json::Value::String("MY-EDIT".into());
        mine.insert(mine_key.clone(), spec);
        std::fs::write(
            td.join("agents.json"),
            serde_json::to_string(&serde_json::Value::Object(mine)).unwrap(),
        )
        .unwrap();

        // ① 디스크 정의 우선 — 사용자 수정이 임베드에 덮이지 않는다.
        let got = load_agent_spec(mine_key).expect("디스크 어댑터 로드");
        assert_eq!(got["notes"].as_str(), Some("MY-EDIT"), "디스크본이 이겨야(수정 보존)");
        // ② 디스크에 없는 vendor 신규 어댑터 → 임베드 폴백(동결 해소).
        let filled = load_agent_spec(vendor_only_key).expect("임베드 폴백으로 로드돼야");
        assert_eq!(filled, embed[vendor_only_key], "폴백은 임베드 정의 그대로");
        // ③ 양쪽 모두 없음 → 거부(무음 통과 금지).
        assert!(load_agent_spec("nosuch-agent-xyz").is_err(), "미지 어댑터는 거부");

        let _ = std::fs::remove_dir_all(&td);
        match saved {
            Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
        }
    }

    /// ★(W4 · 재감사 CS-1③/비평2 C-1) **필드 단위 계층** 핀 — whole-object 폴백의 사각을 메운다:
    /// 유저가 커스터마이즈해 둔 어댑터는 vendor 가 새로 출하한 `ready_marker`·`approval_patterns`
    /// 를 못 받아 동결됐다(readiness 시간폴백 퇴화 · 폴더신뢰 자동확인 불발).
    /// ①키 결손 → 임베드 값 보강 ②디스크 선언 존재 → 무접촉(위 테스트의 '디스크가 이긴다' 불변식
    /// 을 필드 층에서도 유지) ③명시 `null` = 의도적 없음 → 보강 안 함 ④계층 대상 아닌 키 무접촉.
    #[test]
    fn load_agent_spec_field_layer_fills_marker_and_trust_pattern() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-agentfield-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&td);
        std::env::set_var(cys::pack::ENV_PACK_DIR, &td);

        let embed = embedded_agents_json().expect("임베드 agents.json 파싱");
        // 계층 2키를 실제로 가진 어댑터를 임베드에서 취득(하드코딩 금지 — 팩 진실이 사료).
        let key = embed
            .as_object()
            .expect("객체")
            .iter()
            .find(|(k, v)| {
                !k.starts_with('_')
                    && v.get("ready_marker").is_some()
                    && v.get("approval_patterns").is_some()
            })
            .map(|(k, _)| k.clone())
            .expect("ready_marker+approval_patterns 를 가진 임베드 어댑터 존재");
        let write_one = |spec: Value| {
            let mut m = serde_json::Map::new();
            m.insert(key.clone(), spec);
            std::fs::write(
                td.join("agents.json"),
                serde_json::to_string(&Value::Object(m)).unwrap(),
            )
            .unwrap();
        };

        // ① 두 키를 뺀 사용자본(구 커스터마이즈 재현) → 임베드 값으로 보강돼야
        let mut stripped = embed[&key].clone();
        let o = stripped.as_object_mut().unwrap();
        o.remove("ready_marker");
        o.remove("approval_patterns");
        o.insert("notes".into(), json!("MY-EDIT"));
        write_one(stripped);
        let got = load_agent_spec(&key).expect("로드");
        assert_eq!(
            got["ready_marker"], embed[&key]["ready_marker"],
            "결손 ready_marker 가 임베드(vendor)로 채워져야 — 동결 해소"
        );
        assert_eq!(
            got["approval_patterns"], embed[&key]["approval_patterns"],
            "결손 approval_patterns 가 임베드(vendor)로 채워져야"
        );
        assert_eq!(
            got["notes"].as_str(),
            Some("MY-EDIT"),
            "계층 대상 아닌 키는 무접촉(사용자 수정 보존)"
        );
        // 디스크 파일은 고쳐 쓰지 않는다(★W-B: 사용자 소유 파일 무변경 — 메모리 보강만).
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(td.join("agents.json")).unwrap()).unwrap();
        assert!(
            on_disk[&key].get("ready_marker").is_none(),
            "보강이 디스크에 기록되면 안 된다(사용자 파일 무접촉)"
        );

        // ② 디스크 선언 우선 — 커스텀 마커·패턴을 임베드가 덮지 않는다
        let mut custom = embed[&key].clone();
        custom["ready_marker"] = json!("MY-MARKER");
        custom["approval_patterns"] = json!([]);
        write_one(custom);
        let got = load_agent_spec(&key).expect("로드");
        assert_eq!(got["ready_marker"].as_str(), Some("MY-MARKER"), "디스크가 이긴다");
        assert_eq!(got["approval_patterns"], json!([]), "빈 배열 선언도 존중");

        // ③ 명시 null = "의도적으로 없음" 선언 → 임베드로 덮지 않는다
        let mut nulled = embed[&key].clone();
        nulled["ready_marker"] = Value::Null;
        write_one(nulled);
        let got = load_agent_spec(&key).expect("로드");
        assert!(got["ready_marker"].is_null(), "명시 null 은 사용자 의도 — 보강 금지");

        let _ = std::fs::remove_dir_all(&td);
        match saved {
            Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
        }
    }

    /// ★★H-DELIVER-1 (U-12 · K-1 해소의 **유일 합격 기준**) — **배달성**.
    ///
    /// 기존 설치 기계의 `agents.json` 에는 `ready_marker`·`approval_patterns` 가 **값으로 이미
    /// 있다**. `fill_missing_fields` 는 "키가 아예 없을 때만" 채우므로, 벤더가 그 값을 고쳐
    /// 출하해도 **결함이 있는 바로 그 기계들에는 영영 도달하지 않는다**(K-1).
    /// 그래서 관문 데이터는 **신규 키**로 실어 보낸다 — 신규 키는 구 디스크 파일에 부재하므로
    /// 계층이 채우고, 그 순간 배달이 성립한다. 이 테스트는 그 명제를 **구 파일 픽스처로 실행**해
    /// 확인한다(문서가 아니라 동작으로).
    #[test]
    fn h_deliver_1_old_agents_json_receives_new_key_from_embed() {
        use cys::first_run_gates as frg;
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-deliver1-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&td);
        std::env::set_var(cys::pack::ENV_PACK_DIR, &td);

        let embed = embedded_agents_json().expect("임베드 agents.json 파싱");
        // ── 구 기계 재현: 계층 2키는 **값으로 존재**하고(그래서 값 수정은 도달 못 한다),
        //    신규 키만 없다. `_schema` 도 구 버전 그대로 둔다.
        let mut old = embed["claude"].clone();
        let o = old.as_object_mut().unwrap();
        o.remove(frg::ADAPTER_KEY);
        o.insert("notes".into(), json!("MY-EDIT"));
        assert!(o.contains_key("ready_marker") && o.contains_key("approval_patterns"));
        let mut disk = serde_json::Map::new();
        disk.insert("_schema".into(), json!(2));
        disk.insert("claude".into(), old);
        std::fs::write(
            td.join("agents.json"),
            serde_json::to_string(&Value::Object(disk)).unwrap(),
        )
        .unwrap();

        let got = load_agent_spec("claude").expect("로드");
        // ① 신규 키가 임베드 봉투로 **채워졌다** = 구 기계에 도달한다.
        assert_eq!(
            got[frg::ADAPTER_KEY],
            embed["claude"][frg::ADAPTER_KEY],
            "신규 키가 계층으로 채워지지 않았다 — 배달 경로 미성립(K-1 미해소)"
        );
        // ② 그 봉투로 해소한 코퍼스가 실제로 읽힌다(값이 살아서 소비 지점까지 간다).
        let resolved = frg::resolve_from_spec(&got);
        assert_eq!(resolved.gates, frg::builtin(), "봉투는 왔는데 코퍼스가 안 선다");
        assert!(
            resolved
                .gates
                .iter()
                .any(|g| g.id == "bypass-disclaimer" && g.default_index == Some(1)),
            "면책 관문의 실측 기본 포커스(No, exit)가 배달되지 않았다"
        );
        // ③ 사용자 주권 불변 — 디스크에 값이 있는 키는 그대로, 디스크 파일도 무접촉.
        assert_eq!(got["notes"].as_str(), Some("MY-EDIT"));
        assert_eq!(got["ready_marker"], embed["claude"]["ready_marker"]);
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(td.join("agents.json")).unwrap()).unwrap();
        assert!(
            on_disk["claude"].get(frg::ADAPTER_KEY).is_none(),
            "보강이 사용자 파일에 기록됐다(★W-B 위반)"
        );
        // ④ ★계측 타당성 대조군: **디스크에 값이 있는 키**는 임베드가 못 이긴다 —
        //    즉 "값 수정으로는 도달하지 않는다"는 K-1 의 전제 자체가 이 트리에서 참이다.
        //    (이 대조가 없으면 ①은 '원래 되는 일'을 확인한 공허한 초록일 수 있다.)
        let mut mutated = embed.clone();
        mutated["claude"]["ready_marker"] = json!("VENDOR-NEW-MARKER");
        let mut spec = load_agent_spec("claude").expect("로드");
        fill_missing_fields(&mut spec, mutated.get("claude"));
        assert_ne!(
            spec["ready_marker"].as_str(),
            Some("VENDOR-NEW-MARKER"),
            "디스크 값이 vendor 신값으로 덮였다 — 그렇다면 K-1 서사가 틀린 것이니 설계를 재확인하라"
        );
        // ⑤ ★기전 A/B 차분: **임베드가 그 키를 들고 있을 때만** 채워진다.
        //    (같은 함수·같은 입력에서 키 유무 하나만 바꾼다 — 배달이 우연이 아님을 보인다.)
        let mut a = json!({"cmd": "x"});
        let mut emb_without = embed["claude"].clone();
        emb_without
            .as_object_mut()
            .unwrap()
            .remove(frg::ADAPTER_KEY);
        fill_missing_fields(&mut a, Some(&emb_without));
        assert!(
            a.get(frg::ADAPTER_KEY).is_none(),
            "임베드에 없는 키가 어디선가 만들어졌다(배달원이 임베드가 아니다)"
        );
        let mut b = json!({"cmd": "x"});
        fill_missing_fields(&mut b, embed.get("claude"));
        assert_eq!(
            b[frg::ADAPTER_KEY],
            embed["claude"][frg::ADAPTER_KEY],
            "임베드가 들고 있는데도 안 채워졌다 — 배달 경로 단절"
        );
        // ⑥ 사용자 주권은 신규 키에도 그대로다: 디스크 선언이 있으면 그것이 이긴다.
        let mut mine = json!({"cmd": "x"});
        mine.as_object_mut().unwrap().insert(
            frg::ADAPTER_KEY.to_string(),
            json!({"source": "replace",
                   "gates": [{"id": "mine", "needles": ["Proceed with the migration?"]}]}),
        );
        fill_missing_fields(&mut mine, embed.get("claude"));
        let r = frg::resolve_from_spec(&mine);
        // ★N1 정정 — 정본은 "선언 1건 + 강제 복원된 Fatal 빌트인"이다. 종전 `len()==1` 은
        //   "replace 한 줄이 킬체인 관문을 전부 없애는 것이 정상"을 박제하고 있었다.
        //   주권 침해와 Fatal 바닥을 가르는 판별자는 개수가 아니라 **가역 관문**(theme)이다.
        let fatal: Vec<String> = frg::builtin()
            .into_iter()
            .filter(|g| g.absence_is_fatal())
            .map(|g| g.id)
            .collect();
        assert_eq!(
            r.gates.len(),
            1 + fatal.len(),
            "디스크 선언이 임베드에 덮였거나 Fatal 바닥이 사라졌다: {:?}",
            r.gates.iter().map(|g| g.id.as_str()).collect::<Vec<_>>()
        );
        assert!(
            !r.gates.iter().any(|g| g.id == "theme"),
            "가역 관문까지 되살아났다 = 코드 정본 폴백(사용자 주권 침해)의 형태다"
        );
        for id in &fatal {
            assert!(r.gates.iter().any(|g| &g.id == id), "Fatal 관문 {id} 소실");
        }
        assert_eq!(r.gates[0].id, "mine");

        let _ = std::fs::remove_dir_all(&td);
        match saved {
            Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
        }
    }

    /// ★(W4 · CS-1③) 감지 오라클 핀: extract_bin(env-prefix 건너뛰기) + 경로형 실재 + **실행권**.
    /// 종전 부트 인라인 판정은 `exists()` 만 봤다 — 실행권 없는 파일을 '설치됨'으로 오탐하고
    /// 기동에서야 EACCES 로 죽었다. python 오라클(os.access X_OK)과 판정이 어긋난 지점이기도 하다.
    #[test]
    fn detect_agent_binary_requires_exec_bit_and_skips_env_prefix() {
        let td = std::env::temp_dir().join(format!("cys-agentdetect-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&td);
        let exec = td.join("fakeagent");
        let noexec = td.join("fakeagent-noexec");
        std::fs::write(&exec, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::write(&noexec, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exec, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::set_permissions(&noexec, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let agents = json!({
            // env-prefix 가 붙어도 바이너리 토큰을 정확히 고른다(claude 어댑터 형태)
            "ok": {"cmd": format!("FOO=bar {} --flag", exec.display())},
            "noexec": {"cmd": noexec.display().to_string()},
            "absent": {"cmd": td.join("nope-nonexistent").display().to_string()},
        });
        let d = detect_agent_binary("ok", &agents);
        assert!(d.installed, "실행권 있는 경로형 바이너리는 installed({})", d.reason);
        assert_eq!(
            d.bin,
            exec.display().to_string(),
            "env-prefix 를 건너뛴 토큰이어야(extract_bin 단일 진실)"
        );
        assert_eq!(d.resolved.as_deref(), Some(exec.as_path()), "해소 경로 보고");
        let d = detect_agent_binary("absent", &agents);
        assert!(!d.installed, "부재 경로는 미설치");
        assert!(d.resolved.is_none());
        assert!(!d.hint.is_empty(), "미설치엔 안내 힌트가 붙는다");
        // 실행권 판정은 unix 전용(Windows 는 실행권 개념이 없어 실재로만 본다)
        #[cfg(unix)]
        {
            let d = detect_agent_binary("noexec", &agents);
            assert!(
                !d.installed,
                "실행권 없는 파일은 미설치여야(X_OK 강화 — exists() 오탐 차단): {}",
                d.reason
            );
        }
        // agents.json 에 정의가 없으면 agent 이름을 바이너리로 보고 PATH 를 본다(종전 동형)
        let d = detect_agent_binary("no-such-agent-xyz", &agents);
        assert_eq!(d.bin, "no-such-agent-xyz");
        assert!(!d.installed);
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★(W4 · B19) 폴더신뢰 판정이 **어댑터 선언을 실제로 소비**하고, 내장 needle 폴백을
    /// 잃지 않았음을 핀한다(무회귀 = 종전 감지의 상위집합).
    #[test]
    fn trust_prompt_hit_consumes_adapter_pattern_and_keeps_legacy_needles() {
        fn flat(s: &str) -> String {
            s.chars().filter(|c| !c.is_whitespace()).collect()
        }
        let embed = embedded_agents_json().expect("임베드 agents.json");
        let spec = &embed["claude"];
        let re = trust_prompt_regex(spec).expect("claude 어댑터에 trust-prompt 선언 존재");
        // ★(U-15) 폴백 축의 소스 = U-12 관문 코퍼스 정본(사본 0).
        let gs = cys::first_run_gates::builtin();
        let none: &[cys::first_run_gates::Gate] = &[];

        // ① 선언 문면 그대로
        let t = "  Do you trust the files in this folder?  \n";
        assert!(trust_prompt_hit(Some(&re), none, t, &flat(t), false));
        // ② TUI 폭에 따라 접힌 프롬프트 — 공백 정규화가 흡수한다(원문 정규식이면 여기서 깨진다)
        let t = "Do you trust the files\n   in this folder?";
        assert!(trust_prompt_hit(Some(&re), none, t, &flat(t), false));
        // ③ 구 문면(선언 패턴엔 없다) — 코퍼스 폴백이 잡는다
        let t = "Do you trust this folder?";
        assert!(trust_prompt_hit(Some(&re), &gs, t, &flat(t), false), "선언+코퍼스 병존");
        assert!(trust_prompt_hit(None, &gs, t, &flat(t), false), "패턴 부재 시 코퍼스 단독 폴백");
        // ③′ ★2.1.241 실측 문면 — 선언 패턴에는 **없고**(claude 2.1.236~241 어디에도 없다)
        //     코퍼스 폴백만이 잡는다. 이 축이 없으면 현행 claude 에서 자동확인이 통째로 죽는다.
        let t = "Quick safety check: Is this a project you created or one you trust?";
        assert!(
            !trust_prompt_hit(Some(&re), none, t, &flat(t), false),
            "선언 패턴이 실측 문면을 잡으면 U-15 의 전제(구 패턴은 실재하지 않는다)가 틀린 것"
        );
        assert!(
            trust_prompt_hit(Some(&re), &gs, t, &flat(t), false),
            "실측 문면을 코퍼스 폴백이 놓친다 — 현행 claude 에서 폴더신뢰 자동확인 불발"
        );
        // ③″ ★확인 에코는 감지 근거가 아니다(킬체인의 형태 — 2026-07-29 실사고).
        let echo = "Yes, I trust this folder ✔";
        assert!(
            !trust_prompt_hit(Some(&re), &gs, echo, &flat(echo), false),
            "확인 에코가 신뢰 프롬프트로 재매칭된다 — 2발째 Return 이 면책 창을 누른다"
        );
        assert!(
            trust_prompt_hit(Some(&re), &gs, echo, &flat(echo), true),
            "롤백(V1)에서 구 하드코딩 needle 이 되살아나지 않는다 — 스위치가 아무것도 안 되돌린다"
        );
        // ④ 선언 패턴이 코퍼스와 **무관한 문면**이어도 소비된다(하드코딩 탈출 증명)
        let custom = json!({"approval_patterns": [
            {"name": "trust-prompt", "pattern": "Vertraust du (diesem|dem) Ordner"},
            {"name": "tool-permission", "pattern": "NEVER-AUTO-ANSWER"}
        ]});
        let re2 = trust_prompt_regex(&custom).expect("커스텀 trust-prompt 패턴");
        let t = "Vertraust du\n  diesem Ordner?";
        assert!(trust_prompt_hit(Some(&re2), &gs, t, &flat(t), false), "어댑터 선언 소비 실패(B19)");
        assert!(
            !trust_prompt_hit(None, &gs, t, &flat(t), true),
            "그 문면은 코퍼스·구 needle 어느 쪽으로도 안 잡힌다 — 선언 소비가 유일 경로임을 증명"
        );
        // ⑤ trust-prompt 외 패턴은 소비하지 않는다(사람 판단 보존 — 자동응답 금지 계약)
        let t = "NEVER-AUTO-ANSWER";
        assert!(
            !trust_prompt_hit(Some(&re2), &gs, t, &flat(t), false),
            "tool-permission 을 소비하면 안 된다"
        );
        // ⑥ 무관한 출력에 오탐 금지
        let t = "worker ready. no prompts here.";
        assert!(!trust_prompt_hit(Some(&re), &gs, t, &flat(t), true));
        assert!(!trust_prompt_hit(None, &gs, t, &flat(t), true));
        // ⑦ 패턴 부재·깨진 정규식 → None(내장 needle 폴백 경로)
        assert!(trust_prompt_regex(&json!({})).is_none(), "approval_patterns 부재");
        assert!(
            trust_prompt_regex(&json!({"approval_patterns": [{"name": "approve", "pattern": "x"}]}))
                .is_none(),
            "trust-prompt 항목 부재"
        );
        assert!(
            trust_prompt_regex(
                &json!({"approval_patterns": [{"name": "trust-prompt", "pattern": "(unclosed"}]})
            )
            .is_none(),
            "깨진 정규식은 폴백(부트 중단 금지)"
        );
    }

    /// minisign keypair 생성 → (pubkey_base64_rawline, sign_fn).
    fn gen_signer() -> (String, impl Fn(&[u8]) -> String) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().expect("keypair");
        let pk_b64 = kp.pk.to_base64();
        let sk = kp.sk;
        let signer = move |data: &[u8]| -> String {
            let cursor = std::io::Cursor::new(data.to_vec());
            minisign::sign(None, &sk, cursor, None, None)
                .expect("sign")
                .into_string()
        };
        (pk_b64, signer)
    }

    /// from_dir에 (pack.tar.gz + pack-manifest.json + .minisig)를 짓는다. 반환: manifest 바이트.
    fn build_signed_pack(
        from_dir: &std::path::Path,
        files: &[(&str, &str)],
        key_id: &str,
        pack_version: &str,
        min_binary: &str,
        signed_at: i64,
        expires_at: i64,
        sign: &impl Fn(&[u8]) -> String,
    ) {
        let tree = from_dir.join("tree");
        let _ = std::fs::remove_dir_all(&tree);
        std::fs::create_dir_all(&tree).unwrap();
        let mut files_map = serde_json::Map::new();
        for (rel, content) in files {
            let p = tree.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
            files_map.insert(rel.to_string(), json!(sha256_of(content.as_bytes())));
        }
        // tar czf pack.tar.gz -C tree .
        let status = std::process::Command::new("tar")
            // macOS bsdtar가 xattr AppleDouble(._*) 사이드카를 tar에 넣지 않게 한다 — 프로덕션
            // 결정론 tar(GNU/python)는 이런 엔트리가 없으므로 픽스처를 프로덕션 포맷과 일치시킨다.
            .env("COPYFILE_DISABLE", "1")
            .arg("-czf")
            .arg(from_dir.join("pack.tar.gz"))
            .arg("-C")
            .arg(&tree)
            .arg(".")
            .status()
            .expect("tar czf");
        assert!(status.success(), "tar czf 실패");
        let manifest = json!({
            "pack_version": pack_version,
            "min_binary_version": min_binary,
            "key_id": key_id,
            "signed_at": signed_at,
            "expires_at": expires_at,
            "files": files_map,
        });
        let mbytes = serde_json::to_vec(&manifest).unwrap();
        std::fs::write(from_dir.join("pack-manifest.json"), &mbytes).unwrap();
        let sig = sign(&mbytes);
        std::fs::write(from_dir.join("pack-manifest.json.minisig"), sig).unwrap();
    }

    /// pro 채널 서명 번들(v6 §3 — channel/pro_revision 포함). build_signed_pack의 pro 변형.
    #[allow(clippy::too_many_arguments)]
    fn build_signed_pack_pro(
        from_dir: &std::path::Path,
        files: &[(&str, &str)],
        key_id: &str,
        pack_version: &str,
        pro_revision: u32,
        min_binary: &str,
        signed_at: i64,
        expires_at: i64,
        sign: &impl Fn(&[u8]) -> String,
    ) {
        let tree = from_dir.join("tree");
        let _ = std::fs::remove_dir_all(&tree);
        std::fs::create_dir_all(&tree).unwrap();
        let mut files_map = serde_json::Map::new();
        for (rel, content) in files {
            let p = tree.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
            files_map.insert(rel.to_string(), json!(sha256_of(content.as_bytes())));
        }
        let status = std::process::Command::new("tar")
            // macOS bsdtar가 xattr AppleDouble(._*) 사이드카를 tar에 넣지 않게 한다 — 프로덕션
            // 결정론 tar(GNU/python)는 이런 엔트리가 없으므로 픽스처를 프로덕션 포맷과 일치시킨다.
            .env("COPYFILE_DISABLE", "1")
            .arg("-czf")
            .arg(from_dir.join("pack.tar.gz"))
            .arg("-C")
            .arg(&tree)
            .arg(".")
            .status()
            .expect("tar czf");
        assert!(status.success(), "tar czf 실패");
        let manifest = json!({
            "pack_version": pack_version,
            "min_binary_version": min_binary,
            "key_id": key_id,
            "signed_at": signed_at,
            "expires_at": expires_at,
            "channel": "pro",
            "pro_revision": pro_revision,
            "files": files_map,
        });
        let mbytes = serde_json::to_vec(&manifest).unwrap();
        std::fs::write(from_dir.join("pack-manifest.json"), &mbytes).unwrap();
        let sig = sign(&mbytes);
        std::fs::write(from_dir.join("pack-manifest.json.minisig"), sig).unwrap();
    }

    fn test_keyring(key_id: &str, pubkey: &str) -> cys::packsig::Keyring {
        cys::packsig::Keyring {
            keys: vec![cys::packsig::TrustedKey {
                key_id: key_id.to_string(),
                pubkey: pubkey.to_string(),
                not_after: "2099-01-01T00:00:00Z".to_string(),
            }],
            revoked_key_ids: vec![],
        }
    }

    /// pack-manifest emit(§2-①) — files 키가 PACK+PACK_SKILLS 전부 포함 + sha256이 content_hash
    /// (sha256_hex 동일산식)와 일치. 플래그 주입 채움·미지정 생략(fail-closed) 검증.
    #[test]
    fn pack_manifest_emits_embedded_files_with_content_hash() {
        // 플래그 전건 주입.
        let v = build_pack_manifest_value(Some("39E60A702949D6C3".into()), Some(100), Some(200), "0.4.1", None);
        assert_eq!(v["pack_version"], json!(env!("CARGO_PKG_VERSION")));
        // 팩-only 레인: pack_version 오버라이드가 그대로 방출되고, 미지정은 기존과 동일(회귀 0).
        let vo = build_pack_manifest_value(None, None, None, "", Some("9.9.9"));
        assert_eq!(vo["pack_version"], json!("9.9.9"), "pack_version 오버라이드 미반영");
        assert_eq!(v["min_binary_version"], json!("0.4.1"));
        assert_eq!(v["key_id"], json!("39E60A702949D6C3"));
        assert_eq!(v["signed_at"], json!(100));
        assert_eq!(v["expires_at"], json!(200));
        let files = v["files"].as_object().expect("files object");
        // PACK+PACK_SKILLS 전부 포함 + sha256 == content_hash 동일산식.
        for (rel, content) in cys::pack::PACK.iter().chain(cys::pack::PACK_SKILLS.iter()) {
            let got = files
                .get(*rel)
                .and_then(|x| x.as_str())
                .unwrap_or_else(|| panic!("manifest files에 누락: {rel}"));
            assert_eq!(got, sha256_hex(content), "sha256 불일치: {rel}");
        }
        // 임베드 외 항목이 끼지 않는다(rel 중복 없으므로 합집합 크기 == 항목 수).
        let embedded: std::collections::BTreeSet<&str> = cys::pack::PACK
            .iter()
            .chain(cys::pack::PACK_SKILLS.iter())
            .map(|(r, _)| *r)
            .collect();
        assert_eq!(files.len(), embedded.len(), "manifest files에 임베드 외 항목 존재");
        // 미지정 플래그는 생략(fail-closed: 미서명 manifest는 무중단 검증에서 거부됨).
        let v2 = build_pack_manifest_value(None, None, None, "", None);
        assert!(v2.get("key_id").is_none(), "미지정 key_id가 방출됨");
        assert!(v2.get("signed_at").is_none(), "미지정 signed_at가 방출됨");
        assert!(v2.get("expires_at").is_none(), "미지정 expires_at가 방출됨");
        assert_eq!(v2["min_binary_version"], json!(""), "min_binary_version 기본 빈문자열");
    }

    /// 버전 3축 게이트 — 반영 판정·호환 게이트·빈 min_binary·파싱 실패 (v6 튜플 확장).
    #[test]
    fn version_gates_three_axes() {
        // remote newer + min_binary ok → Apply
        assert_eq!(version_gates(("1.1.0", 0), ("1.0.0", 0), "0.4.1", "1.0.0"), VersionGate::Apply);
        // remote 같음/낮음 → UpToDate(멱등)
        assert_eq!(version_gates(("1.0.0", 0), ("1.0.0", 0), "", "1.0.0"), VersionGate::UpToDate);
        assert_eq!(version_gates(("0.9.0", 0), ("1.0.0", 0), "", "1.0.0"), VersionGate::UpToDate);
        // remote 파싱 실패 → UpToDate(fail-CLOSED 반영거부)
        assert_eq!(version_gates(("garbage", 0), ("1.0.0", 0), "", "1.0.0"), VersionGate::UpToDate);
        // min_binary 초과 → BinaryTooOld
        assert_eq!(version_gates(("2.0.0", 0), ("1.0.0", 0), "99.0.0", "1.0.0"), VersionGate::BinaryTooOld);
        // min_binary 빈 값 → 제약 없음(Apply)
        assert_eq!(version_gates(("2.0.0", 0), ("1.0.0", 0), "", "0.4.1"), VersionGate::Apply);
        // min_binary == running → Apply (≤)
        assert_eq!(version_gates(("2.0.0", 0), ("1.0.0", 0), "1.0.0", "1.0.0"), VersionGate::Apply);
        // min_binary 파싱 실패 → BinaryTooOld(fail-CLOSED)
        assert_eq!(version_gates(("2.0.0", 0), ("1.0.0", 0), "junk", "1.0.0"), VersionGate::BinaryTooOld);
    }

    /// v6 튜플 전이 케이스(설계 §3 의무): free→pro / pro.N→pro.N+1 / pro 역행 / base rebase.
    #[test]
    fn version_gates_pro_revision_tuple_transitions() {
        // free→pro 전환(동일 base + pro.1) → Apply — 구 parse_semver 접미 절단이 이중 차단하던 경로.
        assert_eq!(version_gates(("0.8.0", 1), ("0.8.0", 0), "0.8.0", "0.8.0"), VersionGate::Apply);
        // pro.N → pro.N+1 (동일 base 증분) → Apply — R1 실증 결함(replay/UpToDate 이중 차단)의 교정 핀.
        assert_eq!(version_gates(("0.8.0", 2), ("0.8.0", 1), "0.8.0", "0.8.0"), VersionGate::Apply);
        // pro 역행(pro.1 ← pro.2 설치) → UpToDate(반영 거부).
        assert_eq!(version_gates(("0.8.0", 1), ("0.8.0", 2), "0.8.0", "0.8.0"), VersionGate::UpToDate);
        // base rebase: 0.8.0-pro.5 설치 위에 0.9.0-pro.1 → Apply (base 우선 비교).
        assert_eq!(version_gates(("0.9.0", 1), ("0.8.0", 5), "0.9.0", "0.9.0"), VersionGate::Apply);
        // 동일 튜플 → UpToDate (self-heal 후보 — 파일 반영은 없다).
        assert_eq!(version_gates(("0.8.0", 1), ("0.8.0", 1), "0.8.0", "0.8.0"), VersionGate::UpToDate);
    }

    /// reinject 3단 게이트 결정 — unchanged·dedup·defer·inject.
    #[test]
    fn reinject_decision_gate() {
        let m = ReinjectMarker { pack_version: "1.0.0".into(), directive_hash: "HASH_A".into() };
        // 인자 순서: (marker, new_ver, new_hash, idle, self_idle, ready)
        // ⓐ 해시 동일 → SkipUnchanged (게이트 신호 무관)
        assert_eq!(
            reinject_decision(Some(&m), "1.1.0", "HASH_A", true, true, true),
            ReinjectDecision::SkipUnchanged
        );
        assert_eq!(
            reinject_decision(Some(&m), "1.1.0", "HASH_A", false, false, false),
            ReinjectDecision::SkipUnchanged
        );
        // ⓒ 해시 변경이지만 마커 버전 >= 새 버전 → SkipDedup
        assert_eq!(
            reinject_decision(Some(&m), "1.0.0", "HASH_B", true, true, true),
            ReinjectDecision::SkipDedup
        );
        assert_eq!(
            reinject_decision(Some(&m), "0.9.0", "HASH_B", true, true, true),
            ReinjectDecision::SkipDedup
        );
        // ⓑ 해시 변경 + 신버전이지만 busy/자기보고working/미준비 → Defer (3신호 AND 각 축)
        assert_eq!(
            reinject_decision(Some(&m), "1.1.0", "HASH_B", false, true, true),
            ReinjectDecision::Defer
        );
        assert_eq!(
            reinject_decision(Some(&m), "1.1.0", "HASH_B", true, false, true),
            ReinjectDecision::Defer
        );
        assert_eq!(
            reinject_decision(Some(&m), "1.1.0", "HASH_B", true, true, false),
            ReinjectDecision::Defer
        );
        // 통과: 해시 변경 + 신버전 + idle + self_idle + ready → Inject
        assert_eq!(
            reinject_decision(Some(&m), "1.1.0", "HASH_B", true, true, true),
            ReinjectDecision::Inject
        );
        // 마커 부재(첫 주입): 3신호 모두 true면 Inject, 하나라도 false면 Defer
        assert_eq!(
            reinject_decision(None, "1.0.0", "HASH_X", true, true, true),
            ReinjectDecision::Inject
        );
        assert_eq!(
            reinject_decision(None, "1.0.0", "HASH_X", false, true, true),
            ReinjectDecision::Defer
        );
        assert_eq!(
            reinject_decision(None, "1.0.0", "HASH_X", true, false, true),
            ReinjectDecision::Defer
        );
    }

    /// reinject 집계 → pack-update 종료코드: failed>0이면 degraded(성공 침묵 포장 금지),
    /// failed==0이면 0(deferred만 있어도 디스크 반영은 성공이라 0). #3 핵심 신호 계약.
    #[test]
    fn reinject_failed_signals_degraded_exit() {
        assert_eq!(reinject_exit_code(0), 0, "실패 0 → 성공(0)");
        assert_eq!(
            reinject_exit_code(1),
            cys::pack::EXIT_REINJECT_DEGRADED,
            "실패>0 → degraded 종료코드(성공으로 침묵 포장 금지)"
        );
        assert_eq!(reinject_exit_code(5), cys::pack::EXIT_REINJECT_DEGRADED);
        // 0(완전 성공)·1(일반 실패)과 구분되는 신호여야 호출자가 디스크 반영+미각성을 분간한다.
        assert_ne!(cys::pack::EXIT_REINJECT_DEGRADED, 0);
        assert_ne!(cys::pack::EXIT_REINJECT_DEGRADED, 1);
    }

    /// deferred(busy) 노드 pending 영속: deferred>0 → {pack_version, deferred:[{surface_id, role}]}
    /// 기록, deferred==0 → stale pending 제거(없으면 no-op). #3 deferred 가시화·재시도 계약.
    #[test]
    fn reinject_pending_persists_and_clears() {
        let base = std::env::temp_dir().join(format!("cys-reinject-pending-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let path = reinject_pending_path(&base);

        // deferred 없으면 기존 파일 없을 때 no-op(에러 아님).
        assert!(!path.exists());
        persist_reinject_pending(&base, "2.0.0", &[]).unwrap();
        assert!(!path.exists(), "deferred 0·기존 부재 → 파일 생성 안 함");

        // deferred>0 → pending 영속(버전·노드 목록 보존).
        let deferred = vec![(7u64, "worker".to_string()), (9u64, "cso".to_string())];
        persist_reinject_pending(&base, "2.0.0", &deferred).unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc["pack_version"], "2.0.0");
        let nodes = doc["deferred"].as_array().unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0]["surface_id"], 7);
        assert_eq!(nodes[0]["role"], "worker");
        assert_eq!(nodes[1]["surface_id"], 9);
        assert_eq!(nodes[1]["role"], "cso");

        // 이후 deferred 0 → stale pending 제거(다음 실행이 해소됐음을 반영).
        persist_reinject_pending(&base, "2.1.0", &[]).unwrap();
        assert!(!path.exists(), "deferred 해소 → stale pending 제거");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// LOW#1 능동 소비: pending에 보류된 2노드 중 지금 idle인 노드는 재주입(inject+mark)·해소하고,
    /// 여전히 busy(자기보고 working)인 노드는 pending에 잔존시킨다. 잔존 노드만 재기록되는지 확인.
    #[test]
    fn pending_consume_retries_idle_keeps_busy() {
        let base = std::env::temp_dir().join(format!("cys-pending-c1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let path = reinject_pending_path(&base);

        // 보류된 2노드 영속(둘 다 직전 pack-update에서 busy였다).
        persist_reinject_pending(
            &base,
            "2.0.0",
            &[(7u64, "worker".to_string()), (9u64, "cso".to_string())],
        )
        .unwrap();
        let (ver, nodes) = read_reinject_pending(&base).unwrap().unwrap();
        assert_eq!(ver, "2.0.0");
        assert_eq!(nodes.len(), 2);

        // 라이브 플릿: surface 7=idle·ready(agent 부재→idle+quiet fallback), surface 9=working.
        let fleet = vec![
            json!({"surface_id":7, "role":"worker", "state":"idle", "idle_secs":30, "agent_status":"idle"}),
            json!({"surface_id":9, "role":"cso", "state":"idle", "idle_secs":30, "agent_status":"working"}),
        ];
        let markers = std::collections::HashMap::new(); // 마커 부재(첫 주입) → 3신호 AND면 Inject.

        let injected = std::cell::Cell::new(0u32);
        let marked = std::cell::Cell::new(0u32);
        let (resolved, kept) = consume_reinject_pending_core(
            &base,
            &ver,
            &nodes,
            &markers,
            &fleet,
            |_role| Ok("DIRECTIVE-BODY".to_string()),
            |_sid| String::new(), // tail 빈값 — ready_marker 부재 어댑터는 idle+quiet fallback.
            |_sid, _t| {
                injected.set(injected.get() + 1);
                Ok(())
            },
            |_sid, _v, _h| {
                marked.set(marked.get() + 1);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(resolved, 1, "idle 노드 1개 해소");
        assert_eq!(kept, 1, "busy 노드 1개 잔존");
        assert_eq!(injected.get(), 1, "idle 노드만 주입");
        assert_eq!(marked.get(), 1, "주입 성공 노드만 마크");
        // pending은 busy 노드(surface 9)만 남아 재기록.
        assert!(path.exists(), "잔존 노드 있음 → pending 유지");
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let remaining = doc["deferred"].as_array().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0]["surface_id"], 9);
        assert_eq!(remaining[0]["role"], "cso");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// LOW#1: 보류 노드가 전부 해소되면(모두 idle 주입 성공) pending 파일을 삭제한다.
    #[test]
    fn pending_consume_clears_file_when_all_resolved() {
        let base = std::env::temp_dir().join(format!("cys-pending-c2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let path = reinject_pending_path(&base);

        persist_reinject_pending(&base, "2.0.0", &[(7u64, "worker".to_string())]).unwrap();
        let (ver, nodes) = read_reinject_pending(&base).unwrap().unwrap();
        let fleet = vec![
            json!({"surface_id":7, "role":"worker", "state":"idle", "idle_secs":30, "agent_status":"idle"}),
        ];
        let markers = std::collections::HashMap::new();
        let (resolved, kept) = consume_reinject_pending_core(
            &base,
            &ver,
            &nodes,
            &markers,
            &fleet,
            |_role| Ok("DIRECTIVE-BODY".to_string()),
            |_sid| String::new(),
            |_sid, _t| Ok(()),
            |_sid, _v, _h| Ok(()),
        )
        .unwrap();
        assert_eq!(resolved, 1);
        assert_eq!(kept, 0);
        assert!(!path.exists(), "전부 해소 → pending 삭제");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// LOW#1: pending 파일이 없으면 consume_reinject_pending은 데몬 접속 없이 즉시 no-op(0,0).
    #[test]
    fn pending_consume_noop_when_absent() {
        let base = std::env::temp_dir().join(format!("cys-pending-c3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        assert!(!reinject_pending_path(&base).exists());
        // 데몬 접속 없이 즉시 반환(요청 함수 호출 없음).
        let r = consume_reinject_pending(&base).unwrap();
        assert_eq!(r, (0, 0));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// LOW#1: pending이 있는데 데몬 미가동이면 graceful — Err 반환·pending 보존(소실 없음).
    #[test]
    fn pending_consume_graceful_when_daemon_absent() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 존재하지 않는 소켓으로 강제 + autostart 차단 → request 결정론적 실패(실데몬 비접촉).
        let saved_sock = std::env::var(cys::ENV_SOCKET).ok();
        let saved_noauto = std::env::var("CYS_NO_AUTOSTART").ok();
        let base = std::env::temp_dir().join(format!("cys-pending-c4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var(cys::ENV_SOCKET, base.join("nonexistent.sock"));
        std::env::set_var("CYS_NO_AUTOSTART", "1");

        let path = reinject_pending_path(&base);
        persist_reinject_pending(&base, "2.0.0", &[(7u64, "worker".to_string())]).unwrap();
        assert!(path.exists());

        let res = consume_reinject_pending(&base);

        // env 복원(assert 전).
        match saved_sock {
            Some(v) => std::env::set_var(cys::ENV_SOCKET, v),
            None => std::env::remove_var(cys::ENV_SOCKET),
        }
        match saved_noauto {
            Some(v) => std::env::set_var("CYS_NO_AUTOSTART", v),
            None => std::env::remove_var("CYS_NO_AUTOSTART"),
        }
        let preserved = path.exists();
        let _ = std::fs::remove_dir_all(&base);

        assert!(res.is_err(), "데몬 미가동 → Err(graceful 스킵 신호)");
        assert!(preserved, "데몬 부재 시 pending 보존(소실 금지)");
    }

    /// ★오프라인 통합: 서명된 테스트 팩을 --from 코어로 적용 → .pack-version·파일·accepted 반영.
    #[test]
    fn pack_update_from_dir_applies_signed_pack() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(cys::pack::ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-pu-apply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let pack_dir = td.join("pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &pack_dir);
        std::env::set_var(cys::pack::ENV_CONFIG_DIR, td.join("cysclaude"));
        // 이미 설치된 팩(구버전) 시뮬 — .pack-version 선존.
        std::fs::write(pack_dir.join(".pack-version"), "0.0.1").unwrap();

        let (pk, sign) = gen_signer();
        let kr = test_keyring("TESTKEY", &pk);
        let from_dir = td.join("from");
        std::fs::create_dir_all(&from_dir).unwrap();
        let files = [
            ("soul.md", "SOUL v2 content\n"),
            ("directives/MASTER_DIRECTIVE.md", "MASTER v2\n"),
        ];
        build_signed_pack(&from_dir, &files, "TESTKEY", "1.0.0", "0.4.1", 1000, 9_000_000_000, &sign);

        let staging = td.join("staging");
        let lock = td.join(".lock");
        let accepted = td.join(".accepted.json");
        let res = pack_update_from_dir(
            &from_dir, &staging, &lock, &accepted, 5000, "0.4.1", &kr, true,
        );

        // env 복원(assert 전).
        let restore = || {
            match &saved {
                Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
                None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
            }
            match &saved_cfg {
                Some(v) => std::env::set_var(cys::pack::ENV_CONFIG_DIR, v),
                None => std::env::remove_var(cys::pack::ENV_CONFIG_DIR),
            }
        };
        let outcome = match res {
            Ok(o) => o,
            Err(e) => {
                restore();
                let _ = std::fs::remove_dir_all(&td);
                panic!("적용 실패: {e}");
            }
        };
        let disk_ver = std::fs::read_to_string(pack_dir.join(".pack-version")).unwrap();
        let soul = std::fs::read_to_string(pack_dir.join("soul.md")).unwrap();
        let acc_exists = accepted.is_file();
        let acc = std::fs::read_to_string(&accepted).unwrap_or_default();
        restore();
        let _ = std::fs::remove_dir_all(&td);

        assert_eq!(outcome.gate, VersionGate::Apply);
        assert_eq!(disk_ver.trim(), "1.0.0", ".pack-version 반영");
        assert_eq!(soul, "SOUL v2 content\n", "파일 내용 반영");
        assert!(outcome.written >= 2, "written {}", outcome.written);
        assert!(acc_exists, "accepted 기록 부재");
        assert!(acc.contains("1.0.0"), "accepted에 pack_version 부재");
    }

    /// ★오프라인 통합 거부 케이스: 위조 서명·만료·구버전·min_binary 초과.
    #[test]
    fn pack_update_from_dir_rejects_invalid() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(cys::pack::ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-pu-reject-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let pack_dir = td.join("pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &pack_dir);
        std::env::set_var(cys::pack::ENV_CONFIG_DIR, td.join("cysclaude"));
        std::fs::write(pack_dir.join(".pack-version"), "1.0.0").unwrap();

        let (pk, sign) = gen_signer();
        let (_pk_other, sign_other) = gen_signer();
        let kr = test_keyring("TESTKEY", &pk);
        let files = [("soul.md", "S\n")];
        let staging = td.join("staging");
        let lock = td.join(".lock");

        // ① 위조 서명(다른 키) → 거부 (do_apply=false로 충분, 검증 단계에서 막힘)
        let d1 = td.join("from1");
        std::fs::create_dir_all(&d1).unwrap();
        build_signed_pack(&d1, &files, "TESTKEY", "2.0.0", "0.4.1", 1000, 9_000_000_000, &sign_other);
        let acc1 = td.join(".acc1.json");
        let r1 = pack_update_from_dir(&d1, &staging, &lock, &acc1, 5000, "0.4.1", &kr, false);

        // ② 만료(now > expires_at) → 거부
        let d2 = td.join("from2");
        std::fs::create_dir_all(&d2).unwrap();
        build_signed_pack(&d2, &files, "TESTKEY", "2.0.0", "0.4.1", 1000, 2000, &sign);
        let acc2 = td.join(".acc2.json");
        let r2 = pack_update_from_dir(&d2, &staging, &lock, &acc2, 5000, "0.4.1", &kr, false);

        // ③ 구버전(remote 1.0.0 == disk 1.0.0) → UpToDate(no-op, 거부 아님이지만 미반영)
        let d3 = td.join("from3");
        std::fs::create_dir_all(&d3).unwrap();
        build_signed_pack(&d3, &files, "TESTKEY", "1.0.0", "0.4.1", 3000, 9_000_000_000, &sign);
        let acc3 = td.join(".acc3.json");
        let r3 = pack_update_from_dir(&d3, &staging, &lock, &acc3, 5000, "0.4.1", &kr, true);

        // ④ min_binary 초과 → BinaryTooOld(미반영)
        let d4 = td.join("from4");
        std::fs::create_dir_all(&d4).unwrap();
        build_signed_pack(&d4, &files, "TESTKEY", "2.0.0", "99.0.0", 3000, 9_000_000_000, &sign);
        let acc4 = td.join(".acc4.json");
        let r4 = pack_update_from_dir(&d4, &staging, &lock, &acc4, 5000, "0.4.1", &kr, true);

        let restore = || {
            match &saved {
                Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
                None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
            }
            match &saved_cfg {
                Some(v) => std::env::set_var(cys::pack::ENV_CONFIG_DIR, v),
                None => std::env::remove_var(cys::pack::ENV_CONFIG_DIR),
            }
        };
        let disk_after = std::fs::read_to_string(pack_dir.join(".pack-version")).unwrap_or_default();
        restore();
        let _ = std::fs::remove_dir_all(&td);

        assert!(r1.is_err(), "위조 서명 통과");
        assert!(r2.is_err(), "만료 서명 통과");
        assert_eq!(r3.expect("구버전 검증 자체는 통과").gate, VersionGate::UpToDate);
        assert_eq!(r4.expect("min_binary 검증 자체는 통과").gate, VersionGate::BinaryTooOld);
        assert_eq!(disk_after.trim(), "1.0.0", "거부/no-op인데 디스크 버전 변경됨");
    }

    /// ★free/pro e2e(v6 §3·§5 전이 의무 테스트): free 설치 → pro.1 전환(Apply) → pro.2 증분
    /// (Apply — R1 실증 이중 차단의 교정 핀) → free 번들 거부(전용 명령 강제) → pro 역행 거부.
    /// 각 단계에서 state·accepted가 계약대로 영속되는지 검증.
    #[test]
    fn pack_update_pro_channel_e2e() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(cys::pack::ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-pu-pro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let pack_dir = td.join("pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &pack_dir);
        std::env::set_var(cys::pack::ENV_CONFIG_DIR, td.join("cysclaude"));
        std::fs::write(pack_dir.join(".pack-version"), "1.0.0").unwrap();

        let (pk, sign) = gen_signer();
        let kr = test_keyring("TESTKEY", &pk);
        let staging = td.join("staging");
        let lock = td.join(".lock");
        let accepted = td.join("base").join(".pack-accepted.json");
        std::fs::create_dir_all(td.join("base")).unwrap();

        // ① free(1.0.0) → pro.1(동일 base) 전환 — Apply여야 한다.
        let d1 = td.join("pro1");
        std::fs::create_dir_all(&d1).unwrap();
        let files1 = [("soul.md", "SOUL\n"), ("pro-only/skill.md", "PRO v1\n")];
        build_signed_pack_pro(&d1, &files1, "TESTKEY", "1.0.0", 1, "0.4.1", 2000, 9_000_000_000, &sign);
        let r1 = pack_update_from_dir(&d1, &staging, &lock, &accepted, 5000, "0.4.1", &kr, true);

        // ② pro.1 → pro.2 증분(동일 base) — Apply여야 한다(구현 전: replay+UpToDate 이중 차단).
        let d2 = td.join("pro2");
        std::fs::create_dir_all(&d2).unwrap();
        let files2 = [("soul.md", "SOUL\n"), ("pro-only/skill.md", "PRO v2\n")];
        build_signed_pack_pro(&d2, &files2, "TESTKEY", "1.0.0", 2, "0.4.1", 3000, 9_000_000_000, &sign);
        let r2 = pack_update_from_dir(&d2, &staging, &lock, &accepted, 5000, "0.4.1", &kr, true);

        // ③ pro 설치에 free 번들(1.1.0 신버전이어도) → 전용 명령 강제 typed 거부.
        let d3 = td.join("free-on-pro");
        std::fs::create_dir_all(&d3).unwrap();
        build_signed_pack(&d3, &[("soul.md", "FREE\n")], "TESTKEY", "1.1.0", "0.4.1", 4000, 9_000_000_000, &sign);
        let r3 = pack_update_from_dir(&d3, &staging, &lock, &accepted, 5000, "0.4.1", &kr, true);

        // ④ pro 역행(pro.1 재배포·신서명) → replay 튜플 거부.
        let d4 = td.join("pro-regress");
        std::fs::create_dir_all(&d4).unwrap();
        build_signed_pack_pro(&d4, &files1, "TESTKEY", "1.0.0", 1, "0.4.1", 5000, 9_000_000_000, &sign);
        let r4 = pack_update_from_dir(&d4, &staging, &lock, &accepted, 5000, "0.4.1", &kr, true);

        let restore = || {
            match &saved {
                Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
                None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
            }
            match &saved_cfg {
                Some(v) => std::env::set_var(cys::pack::ENV_CONFIG_DIR, v),
                None => std::env::remove_var(cys::pack::ENV_CONFIG_DIR),
            }
        };
        let pro_content = std::fs::read_to_string(pack_dir.join("pro-only/skill.md")).unwrap_or_default();
        let state = cys::pack::read_pack_state(&pack_dir);
        let acc_ev = cys::packsig::read_accepted_evidence(&accepted);
        restore();
        let _ = std::fs::remove_dir_all(&td);

        let o1 = r1.expect("① free→pro.1 실패");
        assert_eq!(o1.gate, VersionGate::Apply, "① free→pro.1이 Apply가 아님");
        assert!(o1.accepted_recorded, "① accepted 미기록");
        let o2 = r2.expect("② pro.1→pro.2 실패(R1 이중 차단 재발?)");
        assert_eq!(o2.gate, VersionGate::Apply, "② pro 증분이 Apply가 아님");
        assert_eq!(pro_content, "PRO v2\n", "② pro.2 콘텐츠 미반영");
        let e3 = r3.expect_err("③ pro 설치에 free 번들이 통과됨");
        assert!(e3.contains("pack-channel-refused"), "③ typed 사유 아님: {e3}");
        assert!(r4.is_err(), "④ pro 역행이 통과됨");
        assert!(
            matches!(state, cys::pack::PackStateRead::Valid(ref st)
                if st.channel == "pro" && st.base_version == "1.0.0" && st.pro_revision == 2),
            "state 계약 위반: {state:?}"
        );
        assert_eq!(
            acc_ev.expect("accepted 판독 실패"),
            Some(("pro".to_string(), 2, "1.0.0".to_string())),
            "accepted 채널·rev 계약 위반"
        );
    }

    /// ★오프라인 통합(Fix1 §7-① 역방향 커버리지): 서명 manifest에 없는 파일을 tarball에 주입한
    /// 팩은 거부되고 디스크는 불변이어야 한다. tarball 미서명이므로 verify_files(전방)만으로는
    /// 못 막던 '미등재 파일 추가' 변조를 verify_no_extra_files(역방향)가 fail-closed로 차단한다.
    #[test]
    fn pack_update_from_dir_rejects_extra_unlisted_file() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(cys::pack::ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-pu-extra-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let pack_dir = td.join("pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &pack_dir);
        std::env::set_var(cys::pack::ENV_CONFIG_DIR, td.join("cysclaude"));
        std::fs::write(pack_dir.join(".pack-version"), "1.0.0").unwrap();

        let (pk, sign) = gen_signer();
        let kr = test_keyring("TESTKEY", &pk);
        let from_dir = td.join("from");
        std::fs::create_dir_all(&from_dir).unwrap();
        // 서명 manifest는 soul.md만 등재(유효 서명·신선창·신버전 2.0.0).
        build_signed_pack(
            &from_dir, &[("soul.md", "S\n")], "TESTKEY", "2.0.0", "0.4.1", 3000, 9_000_000_000, &sign,
        );
        // tarball에 미등재 악성 파일(bin/evil.py with #!) 주입 후 재압축 — manifest·서명은 그대로.
        let tree = from_dir.join("tree");
        let evil = tree.join("bin/evil.py");
        std::fs::create_dir_all(evil.parent().unwrap()).unwrap();
        std::fs::write(&evil, "#!/usr/bin/env python3\nprint('pwned')\n").unwrap();
        let status = std::process::Command::new("tar")
            // macOS bsdtar가 xattr AppleDouble(._*) 사이드카를 tar에 넣지 않게 한다 — 프로덕션
            // 결정론 tar(GNU/python)는 이런 엔트리가 없으므로 픽스처를 프로덕션 포맷과 일치시킨다.
            .env("COPYFILE_DISABLE", "1")
            .arg("-czf")
            .arg(from_dir.join("pack.tar.gz"))
            .arg("-C")
            .arg(&tree)
            .arg(".")
            .status()
            .expect("tar czf");
        assert!(status.success(), "tar czf 실패");

        let staging = td.join("staging");
        let lock = td.join(".lock");
        let accepted = td.join(".accepted.json");
        let res =
            pack_update_from_dir(&from_dir, &staging, &lock, &accepted, 5000, "0.4.1", &kr, true);

        let restore = || {
            match &saved {
                Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
                None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
            }
            match &saved_cfg {
                Some(v) => std::env::set_var(cys::pack::ENV_CONFIG_DIR, v),
                None => std::env::remove_var(cys::pack::ENV_CONFIG_DIR),
            }
        };
        let disk_after = std::fs::read_to_string(pack_dir.join(".pack-version")).unwrap_or_default();
        let evil_installed = pack_dir.join("bin/evil.py").exists();
        let soul_installed = pack_dir.join("soul.md").exists();
        let acc_exists = accepted.is_file();
        restore();
        let _ = std::fs::remove_dir_all(&td);

        assert!(res.is_err(), "미등재 파일 포함 팩이 통과(서명/무결성 우회)");
        assert!(!evil_installed, "미등재 악성 파일이 설치됨(transitive-integrity 위반)");
        assert!(!soul_installed, "거부됐는데 등재 파일이 설치됨(원자성 위반)");
        assert!(!acc_exists, "거부됐는데 accepted 기록됨(replay 기준선 오염)");
        assert_eq!(disk_after.trim(), "1.0.0", "거부인데 디스크 버전 변경됨");
    }

    /// (2c) 회귀 박제: transient 화이트리스트가 cys connect()의 실제 에러 문자열과 정렬돼야
    /// (2a) slow_consumer return 후 재연결이 작동한다. cys connect_raw는 누락 소켓에
    /// "No such file or directory (os error 2)", 거부에 "Connection refused (os error 61)",
    /// half-open read에 "Broken pipe/Connection reset by peer"를 낸다. 그 외(invalid_params 등)는
    /// 비-transient라 즉시 반환돼야(무한루프 차단) 한다.
    #[test]
    fn transient_event_error_matches_real_connect_strings() {
        // cys connect_raw가 실제로 내는 형태
        assert!(is_transient_event_error(
            "cannot connect to cysd at /tmp/x.sock: No such file or directory (os error 2)"
        ));
        assert!(is_transient_event_error(
            "cannot connect to cysd at /tmp/x.sock: Connection refused (os error 61)"
        ));
        // half-open read 단절
        assert!(is_transient_event_error("Broken pipe (os error 32)"));
        assert!(is_transient_event_error("Connection reset by peer (os error 54)"));
        // 정상 EOF·서버 (2a) 종료
        assert!(is_transient_event_error("event stream closed"));
        assert!(is_transient_event_error("slow_consumer"));
        // 비-transient는 재연결 금지(즉시 반환)
        assert!(!is_transient_event_error("invalid_params"));
        assert!(!is_transient_event_error("bad cursor in /tmp/cur"));
    }

    /// ★회귀 박제 (Windows named pipe busy-retry — ERROR_PIPE_BUSY 231 봉인):
    /// 231은 데몬 생존·listening 인스턴스 순간 소진(정상 혼잡)이라 재시도 없는 1회 open 은
    /// 멀티 노드 동시 RPC 에서 상시 실패하고("cannot connect to cysd pipe … os error 231" —
    /// 2026-07-10 Windows 실사고), 다운 오판이 sibling cysd autostart 헛발동까지 부른다.
    /// 간격이 0이면 busy spin, 마감 ≤ 간격이면 사실상 무재시도 — 정책 상수로 의도를 박제한다
    /// (Windows arm 은 이 호스트에서 컴파일/실행 불가 — cysd PIPE_ACCEPT_ERROR_BACKOFF 와 같은 방식).
    #[test]
    fn pipe_busy_retry_policy_is_bounded_and_nonzero() {
        assert_eq!(
            cys::PIPE_BUSY_ERROR, 231,
            "ERROR_PIPE_BUSY 는 Win32 상수 231 — 바뀌면 busy 분기가 영영 안 탄다"
        );
        assert!(
            !cys::PIPE_BUSY_RETRY_INTERVAL.is_zero(),
            "busy-retry 간격이 0이면 100% CPU busy spin: {:?}",
            cys::PIPE_BUSY_RETRY_INTERVAL
        );
        assert!(
            cys::PIPE_BUSY_RETRY_DEADLINE > cys::PIPE_BUSY_RETRY_INTERVAL,
            "마감({:?}) ≤ 간격({:?})이면 사실상 재시도 없는 1회 open 으로 회귀한다",
            cys::PIPE_BUSY_RETRY_DEADLINE,
            cys::PIPE_BUSY_RETRY_INTERVAL
        );
        // 커널 대기 슬라이스·jitter 캡 정책 핀: 슬라이스가 0이면 WaitNamedPipeW 가
        // NMPWAIT_USE_DEFAULT_WAIT(서버 기본값) 의미로 표변하고, 슬라이스·캡이 데드라인을
        // 잠식하면 open 재판정(비-busy 오류 즉시 반환 계약) 기회가 사라진다.
        assert!(
            !cys::PIPE_BUSY_WAIT_SLICE.is_zero(),
            "0ms 슬라이스는 서버 기본 타임아웃 의미(NMPWAIT_USE_DEFAULT_WAIT)로 오발"
        );
        assert!(
            cys::PIPE_BUSY_WAIT_SLICE + cys::PIPE_BUSY_BACKOFF_CAP < cys::PIPE_BUSY_RETRY_DEADLINE,
            "슬라이스({:?})+캡({:?})이 데드라인({:?})을 잠식 — 데몬 다운 재판정 기회 소멸",
            cys::PIPE_BUSY_WAIT_SLICE,
            cys::PIPE_BUSY_BACKOFF_CAP,
            cys::PIPE_BUSY_RETRY_DEADLINE
        );
        assert!(
            cys::PIPE_BUSY_BACKOFF_CAP >= cys::PIPE_BUSY_RETRY_INTERVAL,
            "캡({:?}) < 하한({:?})이면 next_busy_delay 클램프 구간이 성립하지 않는다",
            cys::PIPE_BUSY_BACKOFF_CAP,
            cys::PIPE_BUSY_RETRY_INTERVAL
        );
    }

    /// (3) 회귀 박제: cursor 파일은 write→read 라운드트립으로 seq를 정확히 보존하고,
    /// 부재 파일은 None(에러 아님)·비숫자는 Err로 구분돼야 한다.
    #[test]
    fn event_cursor_roundtrip_and_missing() {
        let dir = std::env::temp_dir().join(format!("cys-cursor-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("cursor");
        let p = path.to_str().unwrap();
        // 부재 파일 = None
        assert_eq!(read_event_cursor(p).unwrap(), None);
        // write→read 라운드트립
        write_event_cursor(p, 4242).unwrap();
        assert_eq!(read_event_cursor(p).unwrap(), Some(4242));
        // 갱신
        write_event_cursor(p, 9999).unwrap();
        assert_eq!(read_event_cursor(p).unwrap(), Some(9999));
        // 비숫자 = Err
        std::fs::write(&path, "garbage\n").unwrap();
        assert!(read_event_cursor(p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 회귀 박제: boot의 설치 판정이 경로형 cmd(틸드 절대경로 — agy)를 which로 넘기면
    /// 틸드 비확장으로 '미설치' 오판 → 4종 의무 부트가 조용히 3종이 된다.
    /// expand_tilde가 '~/'를 홈으로 확장해 파일 존재 판정이 성립해야 한다.
    #[test]
    fn expand_tilde_resolves_home_prefix() {
        let home = dirs::home_dir().expect("home dir");
        assert_eq!(expand_tilde("~/.local/bin/agy"), home.join(".local/bin/agy"));
        // 비틸드 경로·단순 명령어는 그대로
        assert_eq!(
            expand_tilde("/usr/bin/env"),
            std::path::PathBuf::from("/usr/bin/env")
        );
        assert_eq!(expand_tilde("codex"), std::path::PathBuf::from("codex"));
        // '~user' 형태는 확장하지 않는다 (보수적 — 그대로 존재 판정)
        assert_eq!(expand_tilde("~root/x"), std::path::PathBuf::from("~root/x"));
    }

    /// 회귀 박제: boot의 바이너리 존재 검사가 cmd의 env-prefix(KEY=VAL)를 바이너리명으로
    /// 오판하면 안 된다 — claude cmd `CLAUDE_CONFIG_DIR="..." claude ...`가 첫 토큰을
    /// 바이너리로 보고 '미설치'로 건너뛰어 CSO·worker가 조용히 누락되던 회귀를 차단한다.
    #[test]
    fn boot_bin_skips_env_prefix_tokens() {
        assert!(is_env_assignment("CLAUDE_CONFIG_DIR=\"$HOME/.cys/claude\""));
        assert!(is_env_assignment("FOO=bar"));
        assert!(!is_env_assignment("claude"));
        assert!(!is_env_assignment("~/.local/bin/agy"));
        assert!(!is_env_assignment("/usr/bin/codex"));
        // extract_bin은 boot 설치판정과 agent_bin 메타등록이 공유하는 단일 진실(codex R1 회귀).
        assert_eq!(
            extract_bin(
                "CLAUDE_CONFIG_DIR=\"$HOME/.cys/claude\" claude --dangerously-skip-permissions",
                "claude"
            ),
            "claude"
        );
        assert_eq!(
            extract_bin("~/.local/bin/agy --dangerously-skip-permissions", "gemini"),
            "~/.local/bin/agy"
        );
        assert_eq!(
            extract_bin("codex --dangerously-bypass-approvals-and-sandbox", "codex"),
            "codex"
        );
        // 토큰이 전부 env-assignment뿐이면 fallback(agent 이름)을 반환한다.
        assert_eq!(extract_bin("FOO=bar", "claude"), "claude");
        // 문서화된 한계 박제 (agy R1 지적2 — 비차단): 값에 공백 있는 따옴표 대입은 미지원.
        // split_whitespace가 쪼개 잘린 토큰(b")이 바이너리로 잡힌다 — 현 어댑터 cmd 3종은
        // 공백 없는 env 값이라 미발생. 이 박제는 향후 공백 cmd 도입 시 회귀를 즉시 드러낸다.
        assert_eq!(extract_bin("KEY=\"a b\" claude", "fallback"), "b\"");
    }

    // compose_directive 테스트들은 전역 ENV_PACK_DIR를 변경하므로 상단 ENV_LOCK으로 직렬화한다
    // (pack-update 테스트와 동일 전역 env 공유 — 별개 락 병렬 교차 레이스 차단, HIGH 감사).

    /// ★불변식 박제: compose_directive는 디렉티브 → soul.md → 장기메모리 색인 → 스킬 색인
    /// 순서로 조립한다. 메모리 색인 누락은 "리뷰어·워커 장기기억 0" 결함의 재발이므로
    /// 섹션 존재와 순서를 기계 검증한다 (launch/reinject/cycle 공용 경로).
    #[test]
    fn compose_directive_includes_memory_index_after_soul() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let td = std::env::temp_dir().join(format!("cys-compose-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        for sub in ["directives", "memory", "skills/demo"] {
            std::fs::create_dir_all(td.join(sub)).unwrap();
        }
        std::fs::write(td.join("directives/WORKER_DIRECTIVE.md"), "# WORKER 절대지침\n").unwrap();
        // worker compose는 이제 RSI 5번째 directive를 fail-closed로 요구 → fixture 동반.
        std::fs::write(td.join("directives/RSI_LEARNING_DIRECTIVE.md"), "# RSI 학습 절대지침\n").unwrap();
        std::fs::write(td.join("soul.md"), "soul-marker\n").unwrap();
        std::fs::write(td.join("memory/MEMORY.md"), "memory-index-marker\n").unwrap();
        std::fs::write(
            td.join("skills/demo/SKILL.md"),
            "name: demo\ndescription: d\n",
        )
        .unwrap();

        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &td);
        let out = compose_directive("worker").expect("compose 실패");
        match saved {
            Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
        }
        let _ = std::fs::remove_dir_all(&td);

        let pos = |needle: &str| out.find(needle).unwrap_or_else(|| panic!("누락: {needle}"));
        let d = pos("WORKER 절대지침");
        let s = pos("■ soul.md");
        let m = pos("■ 장기메모리 색인");
        let k = pos("■ 보유 스킬 색인");
        assert!(out.contains("memory-index-marker"), "메모리 색인 본문 미동봉");
        assert!(
            out.contains("memory/MEMORY.md") && out.contains(td.to_str().unwrap()),
            "메모리 절대경로 미표기 — 노드가 위치를 추론하게 된다"
        );
        assert!(d < s && s < m && m < k, "조립 순서 위반: 디렉티브<soul<메모리<스킬");
    }

    /// ★불변식 박제(Phase 2 배선): RSI_LEARNING_DIRECTIVE는 master·worker 주입물에만 포함되고
    /// cso·reviewer에는 포함되지 않는다. 단일-directive-per-role을 깨지 않고 RSI만 추가 주입함을
    /// 실측한다(추측 금지 — compose_directive 실출력에서 §1~§6 마커 존재/부재 검증).
    #[test]
    fn compose_directive_injects_rsi_only_for_master_worker() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let td = std::env::temp_dir().join(format!("cys-rsi-inject-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(td.join("directives")).unwrap();
        for (f, body) in [
            ("MASTER_DIRECTIVE.md", "# MASTER 절대지침\n"),
            ("WORKER_DIRECTIVE.md", "# WORKER 절대지침\n"),
            ("CSO_DIRECTIVE.md", "# CSO 절대지침\n"),
            ("REVIEWER_DIRECTIVE.md", "# REVIEWER 절대지침\n"),
        ] {
            std::fs::write(td.join("directives").join(f), body).unwrap();
        }
        // RSI directive — §1~§6 마커를 가진 본문(실주입 여부를 본문으로 판정)
        std::fs::write(
            td.join("directives/RSI_LEARNING_DIRECTIVE.md"),
            "# RSI 학습 루프 — 절대지침 (5번째 directive)\n\n## 1. '학습'의 조작적 정의\n## 6. 할루시네이션 원천 봉쇄장치\nRSI-BODY-MARKER\n",
        )
        .unwrap();

        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &td);
        let master = compose_directive("master").expect("master compose");
        let worker = compose_directive("worker").expect("worker compose");
        let worker2 = compose_directive("worker-2").expect("worker-2 compose");
        let cso = compose_directive("cso").expect("cso compose");
        let reviewer = compose_directive("reviewer-gemini").expect("reviewer compose");
        match saved {
            Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
        }
        let _ = std::fs::remove_dir_all(&td);

        assert!(master.contains("RSI-BODY-MARKER"), "master에 RSI 미주입");
        assert!(worker.contains("RSI-BODY-MARKER"), "worker에 RSI 미주입");
        assert!(worker2.contains("RSI-BODY-MARKER"), "worker-2(변형)에 RSI 미주입");
        assert!(!cso.contains("RSI-BODY-MARKER"), "cso에 RSI 오주입(대상 아님)");
        assert!(!reviewer.contains("RSI-BODY-MARKER"), "reviewer에 RSI 오주입(대상 아님)");
    }

    /// ★불변식 박제 (절대지침 앵커1-b): 탭 타이틀 = "{role}-{agent} · {워크플로우 폴더명}".
    /// 폴더를 알 수 없는 경계(루트·빈 문자열·None)는 역할-에이전트로 폴백.
    #[test]
    fn workflow_title_embeds_folder_name() {
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            workflow_title("worker", "claude", &some("/Users/x/Desktop/CYSjavis/cys-terminal")),
            "worker-claude · cys-terminal"
        );
        // 후행 슬래시 정규화
        assert_eq!(
            workflow_title("reviewer-gemini", "gemini", &some("/a/b/my-workflow/")),
            "reviewer-gemini-gemini · my-workflow"
        );
        // 상대 경로도 basename
        assert_eq!(workflow_title("worker", "claude", &some("proj")), "worker-claude · proj");
        // Windows 경로 + 후행 백슬래시 정규화 (file_name()이 None이 되는 케이스 방어)
        assert_eq!(
            workflow_title("worker", "claude", &some("C:\\Users\\x\\my-wf")),
            "worker-claude · my-wf"
        );
        assert_eq!(
            workflow_title("worker", "claude", &some("C:\\Users\\x\\my-wf\\")),
            "worker-claude · my-wf"
        );
        // 한글/유니코드 폴더명
        assert_eq!(
            workflow_title("worker", "claude", &some("/a/자비스-워크플로우")),
            "worker-claude · 자비스-워크플로우"
        );
        // 연속 구분자도 마지막 비공백 컴포넌트
        assert_eq!(workflow_title("worker", "claude", &some("//a//b")), "worker-claude · b");
        // 경계: 루트·빈 문자열·None·Windows 드라이브 루트·.. → 폴백
        assert_eq!(workflow_title("worker", "claude", &some("/")), "worker-claude");
        assert_eq!(workflow_title("worker", "claude", &some("")), "worker-claude");
        assert_eq!(workflow_title("worker", "claude", &None), "worker-claude");
        assert_eq!(workflow_title("worker", "claude", &some("C:\\")), "worker-claude");
        assert_eq!(workflow_title("worker", "claude", &some("D:/")), "worker-claude");
        // ".." 은 폴더명으로 부적절하지 않음 — 실제 디렉터리 참조라 그대로 표시(상위 폴더 기동 시)
        assert_eq!(workflow_title("worker", "claude", &some("/a/b/..")), "worker-claude · ..");
    }

    #[test]
    fn duration_basic_units() {
        assert_eq!(parse_duration_secs("90s"), Ok(90));
        assert_eq!(parse_duration_secs("20m"), Ok(1200));
        assert_eq!(parse_duration_secs("2h"), Ok(7200));
        assert_eq!(parse_duration_secs("1d"), Ok(86400));
    }

    #[test]
    fn duration_compound() {
        // 1h30m = 3600 + 1800
        assert_eq!(parse_duration_secs("1h30m"), Ok(5400));
        // 누적 순서 무관하게 합산
        assert_eq!(parse_duration_secs("1m30s"), Ok(90));
        assert_eq!(parse_duration_secs("1h2m3s"), Ok(3723));
    }

    #[test]
    fn duration_zero_is_ok() {
        // 0초는 형식상 유효 (값 검증은 호출부 책임)
        assert_eq!(parse_duration_secs("0s"), Ok(0));
    }

    #[test]
    fn duration_rejects_bad_input() {
        // 단위 없는 순수 숫자
        assert!(parse_duration_secs("5").is_err());
        // 빈 문자열
        assert!(parse_duration_secs("").is_err());
        // 숫자 없는 단위
        assert!(parse_duration_secs("s").is_err());
        // 알 수 없는 단위
        assert!(parse_duration_secs("5x").is_err());
        // 단위 뒤 trailing 숫자 (미완성)
        assert!(parse_duration_secs("5m3").is_err());
        assert!(parse_duration_secs("1h30").is_err());
        // 공백·기호
        assert!(parse_duration_secs("1 h").is_err());
        assert!(parse_duration_secs("-5s").is_err());
    }

    #[test]
    fn duration_overflow_is_error_not_panic() {
        // R3 버그 가드: n은 u64로 파싱되나 n*86400이 u64를 넘는 입력.
        // 과거: debug=패닉, release=silent wrap(엉뚱한 발화 epoch). 이제 Err로 거부.
        assert!(parse_duration_secs("9999999999999999d").is_err());
        // 곱셈은 안 넘쳐도 누적 합(checked_add)에서 넘치는 경로
        let near_max = format!("{}s", u64::MAX);
        assert_eq!(parse_duration_secs(&near_max), Ok(u64::MAX));
        assert!(parse_duration_secs(&format!("{}s1s", u64::MAX)).is_err());
        // u64::MAX 자체는 s 단위(×1)로 정확히 통과 — 상한 경계 보존
        assert!(parse_duration_secs(&format!("{}m", u64::MAX)).is_err()); // ×60 overflow
        // 정상 큰 값은 여전히 통과 (회귀 아님)
        assert_eq!(parse_duration_secs("100d"), Ok(100 * 86400));
    }

    #[test]
    fn cli_glob_anchored_full_match() {
        // 리터럴은 전체 일치만 (부분 일치 거부 — handlers::glob_match의 ^…$ 앵커와 동일 의미)
        assert!(cli_glob_match("reviewer", "reviewer"));
        assert!(!cli_glob_match("reviewer", "reviewer-gemini"));
        assert!(!cli_glob_match("reviewer", "xreviewer"));
        assert!(!cli_glob_match("view", "reviewer"));
    }

    #[test]
    fn cli_glob_star_semantics() {
        // '*'는 빈 문자열 포함 임의 길이 매치
        assert!(cli_glob_match("*", ""));
        assert!(cli_glob_match("*", "anything"));
        assert!(cli_glob_match("reviewer-*", "reviewer-gemini"));
        assert!(cli_glob_match("reviewer-*", "reviewer-")); // * = 빈 매치
        assert!(!cli_glob_match("reviewer-*", "reviewer")); // 하이픈 리터럴 불일치
        // 중간 '*'
        assert!(cli_glob_match("a*z", "az"));
        assert!(cli_glob_match("a*z", "abcz"));
        assert!(!cli_glob_match("a*z", "abc"));
    }

    #[test]
    fn cli_glob_backtracking_and_multistar() {
        // 백트래킹: 다중 '*'와 탐욕 매칭이 올바르게 되돌아오는지 (재귀 매처의 고전 버그 지점)
        assert!(cli_glob_match("*-*", "worker-2"));
        assert!(cli_glob_match("w*r*2", "worker-2"));
        assert!(cli_glob_match("**", "abc")); // 연속 '*'도 안전
        assert!(cli_glob_match("a**c", "abbbc"));
        // 매칭 실패 케이스 — '*'가 있어도 리터럴 제약 위반
        assert!(!cli_glob_match("a*c", "abd"));
        assert!(!cli_glob_match("*x", "abc"));
    }

    #[test]
    fn cli_glob_literal_star_in_pattern_only() {
        // value 안의 '*'는 리터럴로 취급 (패턴의 '*'만 와일드카드)
        assert!(cli_glob_match("a*", "a*literal"));
        assert!(!cli_glob_match("abc", "a*c")); // 패턴이 리터럴이면 value의 '*'와 불일치
    }

    /// handlers::glob_match(regex판, 데몬측)과 1:1 동일한 명세 (독립 오라클).
    /// '*'→".*", 나머지는 regex escape 후 ^…$ 앵커. 재귀 cli_glob_match가 이 명세에서
    /// 갈리면 CLI측 ACL(--to 글롭 브로드캐스트)이 데몬측과 비대칭 동작한다.
    fn regex_glob_oracle(pattern: &str, value: &str) -> bool {
        let mut re = String::from("^");
        for ch in pattern.chars() {
            if ch == '*' {
                re.push_str(".*");
            } else {
                re.push_str(&regex::escape(&ch.to_string()));
            }
        }
        re.push('$');
        regex::Regex::new(&re)
            .map(|r| r.is_match(value))
            .unwrap_or(false)
    }

    #[test]
    fn cli_glob_agrees_with_regex_oracle_over_corpus() {
        // 패턴·값 전수 곱집합에서 재귀 cli_glob_match와 regex 명세가 완전 일치해야 한다.
        // (handlers.rs의 대칭 테스트와 짝 — 두 바이너리 모두 같은 명세에 핀 고정.)
        // 단, regex '.'은 \n 미매치이므로 값에 개행을 넣지 않는다(역할명 무개행 전제와 일치).
        let patterns = [
            "", "*", "**", "a", "a*", "*a", "*a*", "a*b", "a**b", "a*b*c", "reviewer-*", "*-*",
            "w*r*2", "abc", "a.b", "a+b", "a?b", "[x]", "a*z", "**a**",
        ];
        let values = [
            "", "a", "ab", "abc", "a*literal", "reviewer-gemini", "reviewer-", "reviewer",
            "worker-2", "a.b", "axb", "a+b", "a?b", "[x]", "az", "abz", "abcz", "x", "-", "a-b-c",
        ];
        for p in patterns {
            for v in values {
                assert_eq!(
                    cli_glob_match(p, v),
                    regex_glob_oracle(p, v),
                    "glob 비대칭: pattern={p:?} value={v:?} (recursive={} regex={})",
                    cli_glob_match(p, v),
                    regex_glob_oracle(p, v),
                );
            }
        }
    }

    #[test]
    fn parse_explicit_surface_variants() {
        // None은 그대로 통과 (호출처가 의미 결정)
        assert_eq!(parse_explicit_surface(&None), Ok(None));
        // 유효 ref → Some
        assert_eq!(parse_explicit_surface(&Some("31".into())), Ok(Some(31)));
        assert_eq!(parse_explicit_surface(&Some("surface:7".into())), Ok(Some(7)));
        // 잘못된 형식 → Err
        assert!(parse_explicit_surface(&Some("nope".into())).is_err());
        assert!(parse_explicit_surface(&Some("-1".into())).is_err());
    }

    /// T5 Phase 2-A: claude statusline stdin JSON → usage.report 파라미터 추출 핀.
    /// 공식 stdin 스키마(used_percentage·current_usage 합·rate_limits)를 회귀 박제한다.
    #[test]
    fn statusline_params_full_schema() {
        let v = json!({
            "context_window": {
                "context_window_size": 200000,
                "used_percentage": 41.6,
                "current_usage": {
                    "input_tokens": 1000,
                    "cache_creation_input_tokens": 2000,
                    "cache_read_input_tokens": 80000,
                    "output_tokens": 5000
                }
            },
            "rate_limits": {
                "five_hour": {"used_percentage": 41.0, "resets_at": 1781314865},
                "seven_day": {"used_percentage": 12.0, "resets_at": 1781781650}
            }
        });
        let p = statusline_to_report_params(&v);
        assert_eq!(p["ctx_pct"].as_f64(), Some(41.6));
        assert_eq!(p["ctx_window"].as_u64(), Some(200000));
        // ctx_tokens = input + cache_creation + cache_read (output 제외) = 83000
        assert_eq!(p["ctx_tokens"].as_u64(), Some(83000));
        let rate = p["rate"].as_array().unwrap();
        assert_eq!(rate.len(), 2);
        assert_eq!(rate[0]["label"], json!("5h"));
        assert_eq!(rate[0]["used_pct"].as_f64(), Some(41.0));
        assert_eq!(rate[0]["resets_at"].as_f64(), Some(1781314865.0));
        assert_eq!(rate[1]["label"], json!("7d"));
    }

    /// rate_limits 부재(무료/세션 첫 응답 전): ctx만 추출, rate는 빈 벡터 — ctx 배지만 작동.
    #[test]
    fn statusline_params_no_rate_limits() {
        let v = json!({
            "context_window": {"context_window_size": 1000000, "used_percentage": 8.0}
        });
        let p = statusline_to_report_params(&v);
        assert_eq!(p["ctx_pct"].as_f64(), Some(8.0));
        assert_eq!(p["ctx_window"].as_u64(), Some(1000000));
        assert_eq!(p["rate"].as_array().unwrap().len(), 0);
        assert!(p.get("ctx_tokens").is_none(), "current_usage·total 없으면 ctx_tokens 생략");
        assert!(p.get("session_file").is_none(), "transcript_path 없으면 session_file 생략");
    }

    /// CC v2 WS-A: transcript_path → session_file 동봉 — 데몬의 계정(accountUuid) 귀속 경로.
    #[test]
    fn statusline_params_transcript_path() {
        let v = json!({
            "transcript_path": "/Users/x/.claude/projects/-a/s.jsonl",
            "context_window": {"used_percentage": 3.0}
        });
        let p = statusline_to_report_params(&v);
        assert_eq!(
            p["session_file"],
            json!("/Users/x/.claude/projects/-a/s.jsonl")
        );
    }

    /// 사람용 statusline 한 줄 포맷 — rate는 있을 때만, 모델명 부재 시 "claude" 폴백.
    #[test]
    fn statusline_human_line_format() {
        let v = json!({
            "model": {"display_name": "Opus 4.8"},
            "context_window": {"used_percentage": 42.0},
            "rate_limits": {
                "five_hour": {"used_percentage": 41.0},
                "seven_day": {"used_percentage": 12.0}
            }
        });
        assert_eq!(statusline_human_line(&v), "Opus 4.8 · CTX 42% · 5h 41% · 7d 12%");
        let v2 = json!({"context_window": {"used_percentage": 8.0}});
        assert_eq!(statusline_human_line(&v2), "claude · CTX 8%");
    }

    /// T7 E1-4: hook stdin → usage.event 파라미터 매핑 핀.
    #[test]
    fn hook_event_params_mapping() {
        let pre = json!({"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Skill","tool_input":{"skill":"commit"}});
        let p = hook_to_event_params(&pre).unwrap();
        assert_eq!(p["event_type"], json!("PRE_TOOL"));
        assert_eq!(p["raw_hook_event"], json!("PreToolUse"), "E-b: raw 동봉");
        assert_eq!(p["tool_name"], json!("Skill"));
        assert_eq!(p["tool_input"]["skill"], json!("commit"));
        assert_eq!(p["session_id"], json!("s1"));
        let post = json!({"hook_event_name":"PostToolUse","tool_name":"Bash","tool_response":{"is_error":true}});
        let pp = hook_to_event_params(&post).unwrap();
        assert_eq!(pp["event_type"], json!("POST_TOOL"));
        assert_eq!(pp["raw_hook_event"], json!("PostToolUse"), "E-b: raw 동봉");
        assert_eq!(pp["exit_code"], json!(1), "is_error→exit 1");
        assert!(hook_to_event_params(&json!({"hook_event_name":"Notification"})).is_none(), "관심 없는 hook 무시");
        // E-b: actionable 이벤트는 None으로 버려지지 않고 raw가 보존된다.
        let perm = json!({"hook_event_name":"PermissionRequest","tool_name":"Bash"});
        let pr = hook_to_event_params(&perm).unwrap();
        assert_eq!(pr["event_type"], json!("PermissionRequest"), "raw event_type 보존");
        assert_eq!(pr["raw_hook_event"], json!("PermissionRequest"));
        let epm = hook_to_event_params(&json!({"hook_event_name":"ExitPlanMode"})).unwrap();
        assert_eq!(epm["raw_hook_event"], json!("ExitPlanMode"));
        let auq = hook_to_event_params(&json!({"hook_event_name":"AskUserQuestion"})).unwrap();
        assert_eq!(auq["raw_hook_event"], json!("AskUserQuestion"));
    }

    #[test]
    fn hook_command_is_os_aware_and_targets_session_start() {
        // SessionStart hook 명령은 타깃 OS에서 실행 가능한 형태여야 한다.
        // 회귀 가드: 바닐라 Windows 셸은 `.sh`를 인터프리터 없이 실행 못 하고 "open with"
        // 대화상자를 띄운다(claude-code #21847·#24097) → /clear 후 자동 재주입(autopilot 축2)
        // 무력화. Unix는 기존 `sh` 동작을 그대로 보존(제로 회귀).
        let cmd = cys::pack::session_start_hook_command(std::path::Path::new("/pack"));
        // 어느 OS든 항상 동봉된 session-start.sh를 가리킨다
        assert!(
            cmd.contains("hooks/session-start.sh") || cmd.contains("hooks\\session-start.sh"),
            "must target the bundled hook script: {cmd:?}"
        );
        // 인터프리터를 통해 호출한다 — 스크립트 경로를 명령 선두에 그대로 두면(=`<path>.sh`)
        // Windows 셸이 파일 연결로 가로채므로 금지
        let interp = cmd.split_whitespace().next().unwrap_or("");
        assert!(
            interp == "sh" || interp == "bash",
            "hook must be invoked via a shell interpreter, got: {interp:?}"
        );

        #[cfg(unix)]
        {
            // Unix: 기존 계약 박제 — 정확히 `sh <path>` (동작 변경 없음)
            assert_eq!(cmd, "sh /pack/hooks/session-start.sh");
        }
        #[cfg(windows)]
        {
            // Windows: `sh` 맨 이름 대신 Git Bash가 보장하는 `bash`로 호출 —
            // Claude Code가 Windows에서 `.sh` 해석에 찾는 인터프리터와 일치
            assert!(cmd.starts_with("bash "), "windows must use bash: {cmd:?}");
        }
    }

    #[test]
    fn render_launch_os_aware_unix_byte_identical() {
        // RC-3(B′) 회귀 핀(master D5 조건): unix 렌더는 기존 agents.json 단일문자열과 byte-identical.
        let cmd = "claude --dangerously-skip-permissions";
        let env = vec![(
            "CLAUDE_CONFIG_DIR".to_string(),
            "${CYS_ACCOUNT_DIR:-$HOME/.cys/claude}".to_string(),
        )];
        let (send, inject) = render_launch(cmd, &env);
        #[cfg(not(windows))]
        {
            // 기존(RC-3 前) claude.cmd 단일문자열과 정확히 동일 — 셸이 ${:-}·$HOME 전개(무회귀)
            assert_eq!(
                send,
                "CLAUDE_CONFIG_DIR=\"${CYS_ACCOUNT_DIR:-$HOME/.cys/claude}\" claude --dangerously-skip-permissions"
            );
            assert!(inject.is_empty(), "unix는 env 주입 없음(셸 전개가 진실원)");
        }
        #[cfg(windows)]
        {
            // Windows: 순수 cmd만 send(POSIX env-assign 문자열 소멸) + env는 해소되어 주입 맵으로
            assert_eq!(send, "claude --dangerously-skip-permissions");
            assert_eq!(inject.len(), 1);
            assert_eq!(inject[0].0, "CLAUDE_CONFIG_DIR");
            assert!(!inject[0].1.contains("${"), "주입 값은 해소됨: {:?}", inject[0].1);
            assert!(!inject[0].1.contains("$HOME"), "HOME 전개됨: {:?}", inject[0].1);
        }
    }

    /// ★D5 두-소비처 회귀 핀(v4 · W4): 주입 로직이 lib 헬퍼 단일 SOT 라도, **두 소비처**
    /// (boot_agent_on_surface 인라인 재조립 · run_launch_agent_opts surface.create env 맵)의
    /// 합성 결과에서 각각 검증한다 — 사용자 "0"(agents.json env) 이 있으면 최종 산출 어디에도
    /// "1" 미출현, 키 부재 + claude 면 삽입. (CI 상주 핀은 lib claude_alt_screen_env_injection_pins.)
    ///
    /// ★lib.rs 핀과의 분업(중복 아님): **게이트 값**(OS × 옵트인 매핑)은 lib.rs 의
    /// `claude_alt_screen_env_injection_pins` ④ 가 어느 호스트에서든 전 행을 조회해 고정한다.
    /// 여기가 고정하는 것은 그 값이 **OS별 파이프라인의 실제 산출물**로 이어지는 배선이다 —
    /// 아래 ②(mac)·②-win(windows)은 서로 **다른 산출물**(인라인 send 문자열 vs surface.create
    /// env 맵)을 보므로 각 OS 의 CI 에서만 컴파일·실행된다. lib 핀이 초록이어도 여기가 빨갛다면
    /// '게이트는 맞는데 그 OS 에서 env 가 실리는 자리가 틀렸다'는 뜻이다.
    #[test]
    fn d5_env_injection_covers_both_consumers() {
        let cmd = "claude --dangerously-skip-permissions";
        // ① 사용자 "0" override — 두 소비처 최종 산출에 "1" 미출현.
        let spec = serde_json::json!({
            "cmd": cmd,
            "env": {
                "CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN": "0",
                "CLAUDE_CONFIG_DIR": "${CYS_ACCOUNT_DIR:-$HOME/.cys/claude}"
            }
        });
        // 소비처 1: 인라인 재조립(boot_agent_on_surface 동형 순서).
        let mut env_pairs = agent_env_pairs(&spec);
        cys::inject_claude_alt_screen_default(&mut env_pairs, extract_bin(cmd, "claude"));
        let (send, _) = render_launch(cmd, &env_pairs);
        assert!(
            !send.contains("CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=\"1\""),
            "사용자 '0' 이 있으면 인라인 문자열에 '1' 이 나오면 안 된다: {send}"
        );
        #[cfg(not(windows))]
        assert!(
            send.contains("CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=\"0\""),
            "사용자 '0' 은 인라인에 보존돼야 한다: {send}"
        );
        // 소비처 2: surface.create env 맵(run_launch_agent_opts 동형 순서).
        let mut create_pairs = agent_env_pairs(&spec);
        cys::inject_claude_alt_screen_default(&mut create_pairs, extract_bin(cmd, "claude"));
        let (_, inject_env) = render_launch("", &create_pairs);
        assert!(
            !inject_env
                .iter()
                .any(|(k, v)| k == cys::ENV_CLAUDE_NO_ALT_SCREEN && v == "1"),
            "사용자 '0' 이 있으면 create env 맵에도 '1' 미출현: {inject_env:?}"
        );
        // ② 키 부재 spec — mac claude 는 두 소비처 파이프라인에서 기본 "1" 이 산출에 실린다.
        #[cfg(target_os = "macos")]
        {
            let bare = serde_json::json!({"cmd": cmd, "env": {}});
            let mut pairs = agent_env_pairs(&bare);
            cys::inject_claude_alt_screen_default(&mut pairs, extract_bin(cmd, "claude"));
            let (send2, _) = render_launch(cmd, &pairs);
            assert!(
                send2.contains("CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=\"1\""),
                "mac claude 키 부재면 기본 '1' 이 인라인에 실려야 한다: {send2}"
            );
        }
        // ②-win ★Windows 파이프라인 핀(Windows 호스트에서만 컴파일·실행 — windows-health 의
        //  `cargo test --bin cys d5_env_injection` 스텝이 유일한 실행처). 검증 대상이 mac 과
        //  **다르다**: Windows 는 인라인 `KEY="val" cmd` 전개를 쓰지 않아 send 가 순수 cmd 이므로,
        //  주입이 실리는 곳은 surface.create 로 넘어가는 **env 맵**이다(데몬이 PTY spawn 시
        //  builder.env 로 주입한다). 그래서 키를 찾는 위치가 send2 가 아니라 inject2 다.
        //
        //  ★2026-08-17 강등 개정(의미가 바뀐 핀): Windows 의 D5 는 **기본 off · 옵트인 시에만**
        //   주입이다(lib.rs `d5_gate_for_os` doc — 앵커 ④ · 실기 스모크 B-5 미수행). 그래서 두 행을
        //   **모두** 고정한다: ⓐ기본(미옵트인) = 키 미출현 ⓑ옵트인 = "1" 출현.
        //  ★게이트 값을 `inject_claude_alt_screen_default`(래퍼)가 아니라 `d5_gate_for_os` 로
        //   **명시해 먹인다**: 래퍼는 러너 홈의 파일·env 를 읽으므로 CI 환경 상태에 따라 결과가
        //   흔들려 두 행을 결정론으로 고정할 수 없다. 여기서 보려는 것은 옵트인 판독기가 아니라
        //   **게이트 값 → 파이프라인 산출물** 배선이다(판독기 자체는 형제 게이트와 동형 1줄).
        #[cfg(windows)]
        {
            let bare = serde_json::json!({"cmd": cmd, "env": {}});
            // ⓐ 기본(옵트인 없음) — 강등 전 출고본과 동일하게 **아무것도 실리지 않는다**.
            let mut default_pairs = agent_env_pairs(&bare);
            cys::inject_claude_alt_screen_default_for(
                &mut default_pairs,
                extract_bin(cmd, "claude"),
                cys::d5_gate_for_os("windows", false),
            );
            let (send_d, inject_d) = render_launch(cmd, &default_pairs);
            assert_eq!(send_d, cmd, "windows send 는 순수 cmd 여야 한다(인라인 전개 없음): {send_d}");
            assert!(
                !inject_d
                    .iter()
                    .any(|(k, _)| k == cys::ENV_CLAUDE_NO_ALT_SCREEN),
                "windows 기본(미옵트인)은 create env 맵에 D5 키가 없어야 한다: {inject_d:?}"
            );
            // ⓑ 옵트인 — 그때는 create env 맵에 "1" 이 실린다(이 경로가 Windows 유일 도달 경로).
            let mut opt_in_pairs = agent_env_pairs(&bare);
            cys::inject_claude_alt_screen_default_for(
                &mut opt_in_pairs,
                extract_bin(cmd, "claude"),
                cys::d5_gate_for_os("windows", true),
            );
            let (send2, inject2) = render_launch(cmd, &opt_in_pairs);
            assert_eq!(send2, cmd, "windows send 는 순수 cmd 여야 한다(인라인 전개 없음): {send2}");
            assert!(
                inject2
                    .iter()
                    .any(|(k, v)| k == cys::ENV_CLAUDE_NO_ALT_SCREEN && v == "1"),
                "windows 옵트인 + 키 부재면 create env 맵에 '1' 이 실려야 한다: {inject2:?}"
            );
        }
        // ③ 타 에이전트(codex) — 어느 소비처에도 미삽입.
        let codex_cmd = "codex --dangerously-bypass-approvals-and-sandbox";
        let mut codex_pairs: Vec<(String, String)> = Vec::new();
        cys::inject_claude_alt_screen_default(&mut codex_pairs, extract_bin(codex_cmd, "codex"));
        assert!(codex_pairs.is_empty(), "타 에이전트 미삽입: {codex_pairs:?}");
    }

    #[test]
    fn render_launch_no_env_agent_unchanged() {
        // env 없는 에이전트(gemini/codex·레거시): 양 OS 모두 cmd 그대로, 주입 없음.
        let (send, inject) = render_launch("~/.local/bin/agy --dangerously-skip-permissions", &[]);
        assert_eq!(send, "~/.local/bin/agy --dangerously-skip-permissions");
        assert!(inject.is_empty());
    }

    #[test]
    fn resolve_env_value_expands_default_branch() {
        // ${VAR:-default}: VAR 설정 시 그 값, 미설정 시 default($HOME 전개).
        std::env::remove_var("CYS_TEST_ACCT_X");
        let r = resolve_env_value("${CYS_TEST_ACCT_X:-$HOME/.cys/claude}");
        assert!(r.ends_with("/.cys/claude"), "default+HOME 전개: {r}");
        assert!(!r.contains("${") && !r.contains("$HOME"), "잔여 미전개 없음: {r}");
        std::env::set_var("CYS_TEST_ACCT_X", "/acct/dir");
        assert_eq!(resolve_env_value("${CYS_TEST_ACCT_X:-$HOME/.cys/claude}"), "/acct/dir");
        std::env::remove_var("CYS_TEST_ACCT_X");
    }

    #[test]
    fn agent_env_pairs_reads_map_or_empty() {
        let spec = serde_json::json!({"cmd": "claude", "env": {"CLAUDE_CONFIG_DIR": "x", "A": "b"}});
        let pairs = agent_env_pairs(&spec);
        assert_eq!(pairs, vec![("A".into(), "b".into()), ("CLAUDE_CONFIG_DIR".into(), "x".into())]); // 정렬
        let no_env = serde_json::json!({"cmd": "agy"});
        assert!(agent_env_pairs(&no_env).is_empty());
    }

    /// ★G34(W3) — 소켓에서 **레인 팩을 결정론 유도**한다(cys-dept 명명 규약 미러).
    /// 부서 소켓+본부 팩 데몬이 생기면 부서 부트가 exit 8 로 영구 차단되고 팩이 교차 서빙된다.
    #[test]
    fn lane_pack_derivation_mirrors_dept_naming_convention() {
        let home = dirs::home_dir().unwrap();
        // unix 부서 소켓(디렉터리 성분) → ~/.cys/pack-dept-<name>
        let p = lane_pack_for_socket(std::path::Path::new(
            "/Users/x/.local/state/cys-dept-dept-2/cys.sock",
        ))
        .expect("부서 소켓에서 레인 팩 유도 실패");
        assert_eq!(p, home.join(".cys").join("pack-dept-dept-2"));
        // windows named pipe 형태(역슬래시)도 같은 규약
        let w = lane_pack_for_socket(std::path::Path::new(r"\\.\pipe\cys-dept-sales"))
            .expect("named pipe 에서 유도 실패");
        assert_eq!(w, home.join(".cys").join("pack-dept-sales"));
        // 명명 부서(dept-N 아님)도 커버 — cys-dept 는 임의 이름을 허용한다
        assert_eq!(
            lane_pack_for_socket(std::path::Path::new("/x/cys-dept-ceo/cys.sock")).unwrap(),
            home.join(".cys").join("pack-dept-ceo")
        );
        // base 소켓·커스텀 소켓·빈 부서명(불량 레인)은 유도 불가 → None(호출부가 거부/무동작)
        for bad in [
            "/Users/x/.local/state/cys/cys.sock",
            "/tmp/whatever.sock",
            "/x/cys-dept-/cys.sock",
        ] {
            assert!(
                lane_pack_for_socket(std::path::Path::new(bad)).is_none(),
                "유도 불가여야 한다: {bad}"
            );
        }
        // is_dept_socket 판정과 정합 — 부서로 판정되면 유도 가능해야 한다(빈 부서명 제외)
        assert!(cys::is_dept_socket(std::path::Path::new(
            "/Users/x/.local/state/cys-dept-dept-2/cys.sock"
        )));
        // ★W4: 구현은 lib 단일 소스다 — GUI(cys-app)가 같은 함수를 쓴다(사본 드리프트 봉인)
        assert_eq!(
            lane_pack_for_socket(std::path::Path::new("/x/cys-dept-ceo/cys.sock")),
            cys::pack::lane_pack_for_socket(std::path::Path::new("/x/cys-dept-ceo/cys.sock")),
            "cys.rs 별칭과 lib 정본의 판정이 갈렸다(중복 구현 재발)"
        );
    }

    /// ★W4(하드 제약 6-⑧) `cys boot` bare exit **의미 전환** 계약 박제.
    /// exit 은 `--json` 의 Fatal 판정과 **같은 사실**을 내야 한다: 1 ⟺ mandatory 중 failed|missing.
    /// busy 는 그 어느 쪽도 아니므로 별도 값(75)이고, Degrade-only 는 0 이다(B1 데드엔드 금지).
    #[test]
    fn boot_exit_matches_json_fatal_verdict() {
        // ① 세 의미가 서로 다른 값이다(뭉개짐 금지)
        assert_eq!(boot_exit_code(0, 0, false), 0, "Fatal 0건 = 성공(Degrade-only 포함)");
        assert_eq!(boot_exit_code(1, 0, false), 1, "Fatal 1건 = 1");
        assert_eq!(boot_exit_code(3, 0, false), 1, "Fatal 다건도 1(개수 아님·의미)");
        assert_eq!(boot_exit_code(0, 0, true), EXIT_BOOT_BUSY, "busy = 별도 비0");
        // ② busy 는 성공(0)·Fatal(1) 과 겹치지 않고, clap 사용오류(2)·EX_USAGE(64)와도 다르다
        assert_eq!(EXIT_BOOT_BUSY, 75, "EX_TEMPFAIL(75) 고정 — python·GUI 소비부와 파리티");
        for reserved in [0, 1, 2, 64] {
            assert_ne!(EXIT_BOOT_BUSY, reserved, "busy 값이 예약 exit 과 충돌: {reserved}");
        }
        // ③ busy 는 fatal 계수와 무관하게 busy 다(락을 못 잡았으면 아무 역할도 시도하지 않았다)
        assert_eq!(boot_exit_code(9, 0, true), EXIT_BOOT_BUSY);
        // ④ --json Fatal 판정 규칙과의 동등성 — 같은 fixture 를 양쪽 규칙으로 판정한다.
        //    (python 소비부 `_boot_fatal_verdict` 와 문자 그대로 같은 술어: mandatory ∧ failed|missing)
        let fatal_rule = |roles: &[Value]| -> usize {
            roles
                .iter()
                .filter(|r| {
                    r["mandatory"].as_bool().unwrap_or(false)
                        && matches!(r["outcome"].as_str(), Some("failed") | Some("missing"))
                })
                .count()
        };
        let degrade_only = vec![
            json!({"role":"cso","outcome":"launched","mandatory":true}),
            json!({"role":"reviewer-grok","outcome":"missing","mandatory":false}),
            json!({"role":"reviewer-gemini","outcome":"failed","mandatory":false}),
        ];
        assert_eq!(boot_exit_code(fatal_rule(&degrade_only), 0, false), 0);
        let fatal = vec![
            json!({"role":"cso","outcome":"missing","mandatory":true}),
            json!({"role":"worker","outcome":"launched","mandatory":true}),
        ];
        assert_eq!(boot_exit_code(fatal_rule(&fatal), 0, false), 1);
        // skipped_unconfirmed(죽음 미확정 보류)는 Fatal 이 아니다 — 파괴·스폰 둘 다 안 한 상태
        let unconfirmed = vec![json!({"role":"cso","outcome":"skipped_unconfirmed","mandatory":true})];
        assert_eq!(boot_exit_code(fatal_rule(&unconfirmed), 0, false), 0);

        // ⑤ ★M3 — **의무 관문 보류 축**. `--json` 의 gate 판정과 bare exit 이 같은 사실을 낸다.
        //    (①~④ 는 전부 `fatal_gate_pending=0` 으로 고정해 종전 계약이 한 톨도 안 바뀐 것을
        //     먼저 못 박고, 새 축은 여기서만 연다.)
        let gate_rule = |roles: &[Value]| -> usize {
            roles
                .iter()
                .filter(|r| {
                    r["mandatory"].as_bool().unwrap_or(false)
                        && r["outcome"].as_str() == Some("gate_pending")
                })
                .count()
        };
        // 회전2 실주행 재현: 로스터 5 중 의무 4 가 관문 보류. 종전 계약이면 **exit 0**.
        let live_run = vec![
            json!({"role":"cso","outcome":"gate_pending","mandatory":true}),
            json!({"role":"worker","outcome":"gate_pending","mandatory":true}),
            json!({"role":"reviewer-gemini","outcome":"gate_pending","mandatory":true}),
            json!({"role":"reviewer-codex","outcome":"gate_pending","mandatory":true}),
            json!({"role":"reviewer-grok","outcome":"launched","mandatory":false}),
        ];
        assert_eq!(fatal_rule(&live_run), 0, "드릴 전제: 이 로스터에 Fatal 은 0 이다");
        assert_eq!(gate_rule(&live_run), 4, "드릴 전제: 의무 4좌석이 관문 보류다");
        assert_eq!(
            boot_exit_code(fatal_rule(&live_run), gate_rule(&live_run), false),
            cys::EXIT_GATE_PENDING,
            "의무 0/4 인데 exit 이 '팀을 세웠다'(0)로 나간다 — M3 의 결함 그 자체"
        );
        // 선택 역할만 보류면 Degrade-only 다(0) — 새 축이 과잉 발화하지 않는다.
        let optional_only = vec![
            json!({"role":"cso","outcome":"launched","mandatory":true}),
            json!({"role":"reviewer-grok","outcome":"gate_pending","mandatory":false}),
        ];
        assert_eq!(
            boot_exit_code(fatal_rule(&optional_only), gate_rule(&optional_only), false),
            0
        );
        // 78 은 예약 exit·형제 코드와 겹치지 않는다(공유 상수 계약 — python 미러와 파리티).
        assert_eq!(cys::EXIT_GATE_PENDING, 78, "관문 보류 전용 종료코드는 78 고정");
        for reserved in [0, 1, 2, 64, EXIT_BOOT_BUSY] {
            assert_ne!(cys::EXIT_GATE_PENDING, reserved, "78 이 예약 exit 과 충돌: {reserved}");
        }
    }

    /// ★T-0147-6: 타이핑 가드 거부 판정이 **lib 단일 소스 문구**로 이뤄지는지.
    /// 와이어는 `error.message` 만 전달하므로 이 매처가 깨지면 `--queued` 폴백이 조용히 죽는다.
    #[test]
    fn typing_guard_matcher_binds_to_shared_message() {
        assert!(is_typing_guard_err(cys::MSG_TYPING_GUARD), "정본 메시지 미매칭");
        assert!(is_typing_guard_err(cys::ERR_TYPING_GUARD), "정본 코드 미매칭");
        // 실제 클라이언트가 받는 형태(데몬 message 그대로)
        assert!(is_typing_guard_err(
            "human is typing in this pane; retry later or use --queued"
        ));
        // 다른 거부는 절대 큐 전환을 트리거하지 않는다(오폴백 금지)
        for other in [
            "acl_denied: external→worker deny",
            "surface process has exited",
            "queue_full: pending queue cap (100) reached",
            "clear_first_unsupported",
        ] {
            assert!(!is_typing_guard_err(other), "무관 오류가 큐 전환을 유발: {other}");
        }
    }

    /// ★B3 회귀 핀(0.14.24): `cys send-key Return` 이 타이핑 가드에 막혔을 때 **소실되지 않고**
    /// 큐로 1회 전환되는가의 판정 표. 이 폴백이 없어서 노드 보고의 제출 Enter 가 조용히
    /// 사라졌다(본문만 프롬프트에 남고 미제출 — 결함3 의 마지막 층).
    /// 반대 방향도 같은 무게로 박는다: **타이핑 가드가 아닌 거부는 절대 큐로 바꾸지 않는다**
    /// (오폴백은 정당한 거부를 성공으로 위장한다).
    #[test]
    fn send_key_queue_fallback_fires_only_for_typing_guard_on_submit_keys() {
        let guard = cys::MSG_TYPING_GUARD;
        // ① 전환 대상 — 비-queued 제출 키 + 타이핑 가드.
        assert!(should_queue_fallback_send_key(false, "Return", guard));
        assert!(should_queue_fallback_send_key(false, "Enter", guard));
        assert!(should_queue_fallback_send_key(
            false,
            "Return",
            "human is typing in this pane; retry later or use --queued"
        ));
        // ② 이미 --queued 면 전환할 것이 없다(이중 적재 금지).
        assert!(!should_queue_fallback_send_key(true, "Return", guard));
        // ③ 제출 키가 아니면 전환 불가 — 데몬 계약상 텍스트 큐에는 Return/Enter 만 실린다.
        for k in ["Tab", "Escape", "Up", "BTab", "F5", "Space", "a"] {
            assert!(
                !should_queue_fallback_send_key(false, k, guard),
                "{k} 가 큐로 전환됐다 — 데몬이 invalid_params 로 되받는다"
            );
        }
        // ④ 타이핑 가드가 아닌 거부는 전부 그대로 실패해야 한다.
        for other in [
            "acl_denied: external→worker deny",
            "surface process has exited",
            "queue_full: pending queue cap (100) reached",
            "write_stalled: surface input channel full (pane not consuming input)",
            "not_found: surface 31 not found",
            "invalid_params: unknown key: Retrun",
        ] {
            assert!(
                !should_queue_fallback_send_key(false, "Return", other),
                "무관 거부가 큐 전환을 유발: {other}"
            );
        }
    }

    /// ★B3 회귀 핀(0.14.24): `cys send` 본문도 같은 규칙으로 1회 전환된다 — 단
    /// `--clear-first` 는 제외한다. 원자 clear+paste+submit 은 **직접 전달 전용**이고 데몬이
    /// `--queued` 와의 결합을 invalid_params 로 거부하므로(handlers send_text), 폴백으로
    /// 만들면 안내 대신 두 번째 오류가 난다.
    #[test]
    fn send_queue_fallback_excludes_already_queued_and_clear_first() {
        let guard = cys::MSG_TYPING_GUARD;
        assert!(should_queue_fallback_send(false, false, guard), "본문 큐 전환이 죽었다");
        assert!(!should_queue_fallback_send(true, false, guard), "이미 큐인데 또 전환");
        assert!(
            !should_queue_fallback_send(false, true, guard),
            "clear_first + queued 는 데몬이 거부하는 조합이다 — 폴백 금지"
        );
        assert!(!should_queue_fallback_send(false, true, "clear_first_unsupported"));
        for other in [
            "acl_denied: external→worker deny",
            "clear_first_unsupported: clear_first requires a launch-agent-registered pane",
            "queue_full: pending queue cap (100) reached",
        ] {
            assert!(
                !should_queue_fallback_send(false, false, other),
                "무관 거부가 큐 전환을 유발: {other}"
            );
        }
    }

    #[test]
    fn install_claude_hook_skips_backup_when_already_installed() {
        // RC-1 회귀 핀(D2 master 조건): 온보딩이 매 기동 init-pack을 호출(멱등)해도,
        // hook이 이미 있으면 `.bak-cys` 정상 백업이 클로버되면 안 된다(백업은 실제 write 시에만).
        let base =
            std::env::temp_dir().join(format!("cys-hookbak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let pack = base.join("pack");
        let settings = base.join("settings.json");
        let settings_path = settings.to_string_lossy().to_string();
        let backup = format!("{settings_path}.bak-cys");

        // 1) 최초 설치: hook 없음 → 등록 성공 + write 발생(기존 "{}" 존재하므로 이때 backup 1회 생성).
        std::fs::write(&settings, "{}").unwrap();
        let r1 = install_claude_hook(&settings_path, &pack).unwrap();
        assert!(r1.contains("registered"), "first install must register: {r1}");

        // 2) 정상 백업 sentinel을 심는다 — 매 기동 멱등 재실행이 이 "정상 상태 백업"을 클로버하면
        //    안 된다(D2 master 조건: 기존 hook 존재 시 .bak-cys 무변경). mtime보다 견고한 내용 비교.
        let sentinel = "{\"_sentinel\":\"good-backup-must-survive\"}";
        std::fs::write(&backup, sentinel).unwrap();

        // 3) 재실행(멱등): hook 이미 존재 → skip. backup 블록에 도달하지 않아야 sentinel이 보존된다.
        let r2 = install_claude_hook(&settings_path, &pack).unwrap();
        assert!(r2.contains("already"), "second call must skip: {r2}");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            sentinel,
            "already-installed skip must NOT clobber existing .bak-cys (정상 백업 무변경)"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 기동 화면의 평탄화(공백 제거)를 테스트에서 동일하게 재현하는 헬퍼.
    /// boot_agent_on_surface가 `text.chars().filter(|c| !c.is_whitespace())`로
    /// 만드는 입력과 1:1 동일해야 screen_shows_launch_failure 판정이 핀 고정된다.
    fn flatten_ws(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn launch_failure_detection_is_cross_platform() {
        // 회귀 가드: launch-agent 준비 폴링의 사망 감지가 Unix 셸 오류만 잡으면
        // Windows(PowerShell/cmd)에서 기동 실패를 못 보고 죽은 셸에 지침을 주입한다.
        // hook_command OS 대칭화와 같은 결: 양 OS의 "명령 못 찾음"을 모두 잡아야 한다.

        // --- Unix: 기존 계약 박제 (제로 회귀) ---
        // zsh: "command not found: foo"
        assert!(screen_shows_launch_failure(&flatten_ws(
            "zsh:1: command not found: claude-bogus"
        )));
        // bash: "foo: command not found"
        assert!(screen_shows_launch_failure(&flatten_ws(
            "bash: claude-bogus: command not found"
        )));
        // 직접 바이너리 실행 실패: "No such file or directory"
        assert!(screen_shows_launch_failure(&flatten_ws(
            "./claude-bogus: No such file or directory"
        )));
        // "not found in PATH" 표현
        assert!(screen_shows_launch_failure(&flatten_ws(
            "claude-bogus: not found in PATH"
        )));

        // --- Windows: 이번 수정으로 새로 잡혀야 하는 케이스 ---
        // PowerShell: 미존재 명령
        assert!(
            screen_shows_launch_failure(&flatten_ws(
                "claude-bogus : The term 'claude-bogus' is not recognized as the name of a cmdlet, \
                 function, script file, or operable program. Check the spelling of the name, ..."
            )),
            "PowerShell의 미존재 명령 오류를 감지하지 못함"
        );
        // cmd.exe: 미존재 명령
        assert!(
            screen_shows_launch_failure(&flatten_ws(
                "'claude-bogus' is not recognized as an internal or external command, \
                 operable program or batch file."
            )),
            "cmd.exe의 미존재 명령 오류를 감지하지 못함"
        );

        // --- 음성(negative): 정상 기동 화면은 사망으로 오판하지 않아야 함 ---
        // 정상 Claude Code 프롬프트(ready_marker ❯ 포함)
        assert!(!screen_shows_launch_failure(&flatten_ws(
            "Welcome to Claude Code\n\n❯ "
        )));
        // 폴더 신뢰 프롬프트
        assert!(!screen_shows_launch_failure(&flatten_ws(
            "Do you trust the files in this folder?"
        )));
        // 빈 화면
        assert!(!screen_shows_launch_failure(&flatten_ws("")));
    }

    /// U-9 · `screen_tail_is_shell_prompt` 진리표 (T-D4 / F4-cys-boot-launch-06)
    ///
    /// 이 술어는 "화면 꼬리가 셸 프롬프트인가"를 판정해, 에이전트 TUI가 안 떴는데
    /// 54KB 역할 디렉티브를 맨 셸에 오주입하는 것을 막는다(marker 없는 어댑터의
    /// 시간 폴백 직전 검사 · `:7547`). Windows 프롬프트(`PS C:\…>` · `C:\>`)가 `>`로
    /// 끝나는데 종결자 집합에 `>`가 없어 Windows에서 이 가드가 무력화됐다.
    ///
    /// **왜 진리표인가**: `>`를 무조건 종결자로 넣으면 본문 오탐(다이어그램 화살표
    /// `-->` · 화살표 함수 `=>` · 태그 `<div>` · autolink `<https://…>`)이 늘고,
    /// 오탐이 늘면 ready 선언이 줄어 `Err` → 롤백 close 가 늘어난다(U-11 미착지 상태에서는
    /// 건강한 pane 이 닫히는 방향 = 치명위험 ④). 그래서 `>` 는 **Windows 프롬프트 형태
    /// 요구와 AND** 로만 참이 되며, 아래 음성 축이 그 사실을 기계로 박제한다.
    #[test]
    fn screen_tail_truth_table() {
        // ── ① Unix 양성: 기존 4종 종결자 계약 박제(제로 회귀) ──────────────────
        // bash·sh
        assert!(screen_tail_is_shell_prompt("user@host:~/dev$"));
        assert!(screen_tail_is_shell_prompt("user@host:~/dev$ ")); // trim_end 후 판정
        // zsh
        assert!(screen_tail_is_shell_prompt("user@Mac cys-terminal-rel %"));
        // root
        assert!(screen_tail_is_shell_prompt("root@box:/#"));
        // powerlevel10k · starship (H-PRED-8 ⓔ 핀 — 이 항이 사라지면 claude 잔존 ❯ 오탐 차단이 뚫린다)
        assert!(screen_tail_is_shell_prompt("❯"));
        assert!(screen_tail_is_shell_prompt("~/dev/cys-terminal-rel ❯ "));

        // ── ② Unix 음성: 본문에 종결자 문자가 있어도 '끝문자'가 아니면 거짓 ─────
        // '마지막 비공백 줄의 끝문자' 규칙이 본문 오탐을 막는 유일한 장치다.
        assert!(!screen_tail_is_shell_prompt("export PATH=$HOME/.cargo/bin:$PATH 적용됨"));
        assert!(!screen_tail_is_shell_prompt("비용은 $12 이고 점유율은 40% 였다"));
        assert!(!screen_tail_is_shell_prompt("# 제목입니다"));
        assert!(!screen_tail_is_shell_prompt("Welcome to Claude Code"));

        // ── ③ Windows 양성: 이번 수정으로 새로 잡혀야 하는 축 ──────────────────
        // ★P1-5 핀 이사: Windows 축은 이제 **`windows=true` 인자**로 돈다(술어 자체는 무cfg 라
        //   진리표 전항이 mac CI 에서 그대로 돌아간다 — 항 하나도 지우지 않았고 값도 그대로다).
        //   `win(t)` = "Windows pane 이라면 셸 프롬프트로 판정하는가".
        let win = |t: &str| screen_tail_is_shell_prompt_on(t, true);
        // PowerShell 기본 프롬프트(실측 화면 · docs/plans/2026-07-29-win-two-defects-plan.md:319-323)
        assert!(
            win("PS C:\\Users\\x> "),
            "PowerShell 기본 프롬프트를 셸 프롬프트로 인식하지 못함(F4-cys-boot-launch-06)"
        );
        // 실측 캡처 — Parallels VM master pane(PROBE_RESULTS_WINDOWS.md WIN-2)
        assert!(win("PS C:\\WINDOWS\\system32\\WindowsPowerShell\\v1.0>"));
        // PowerShell 비파일시스템 공급자(레지스트리·Env 드라이브)
        assert!(win("PS HKLM:\\SOFTWARE>"));
        // 극단 축약 프롬프트(`function prompt { \"PS>\" }`)
        assert!(win("PS>"));
        // PowerShell 연속행 프롬프트도 프롬프트다
        assert!(win("PS C:\\Users\\x>>"));
        // cmd.exe — 드라이브 루트 / 하위 경로
        assert!(win("C:\\>"), "cmd.exe 드라이브 루트 프롬프트를 인식하지 못함");
        assert!(win("C:\\Users\\x>"));
        assert!(win("D:\\work\\cys>"));

        // ── ③′ ★P1-5 신설 축: **유닉스 pane 에서는 같은 텍스트가 프롬프트가 아니다** ─────
        // mac/Linux 워커가 Windows 안내문을 마지막 줄로 출력하는 것만으로 건강한 pane 이
        // 롤백 close 대상이 되던 결함(호출부 OS 게이트 부재)의 회귀 박제다.
        // 수정 전 코드에서는 아래 전항이 **참**이었다 → 이 축이 곧 적색 증명이다.
        let nix = |t: &str| screen_tail_is_shell_prompt_on(t, false);
        for t in [
            "PS C:\\Users\\x> ",
            "PS C:\\WINDOWS\\system32\\WindowsPowerShell\\v1.0>",
            "PS HKLM:\\SOFTWARE>",
            "PS>",
            "PS C:\\Users\\x>>",
            "C:\\>",
            "C:\\Users\\x>",
            "D:\\work\\cys>",
        ] {
            assert!(
                !nix(t),
                "유닉스 pane 의 Windows 안내문이 셸 프롬프트로 오판됐다(건강 pane close 방향): {t}"
            );
        }
        // 반대로 Unix 종결자 4종은 **게이트하지 않는다** — Windows 콘솔의 git-bash·WSL 프롬프트에
        // 주입하는 오부정을 만들지 않기 위함(위험 방향이 반대라 대칭으로 가두지 않는다).
        assert!(win("user@host:~/dev$"));
        assert!(win("~/dev ❯"));

        // ── ④ Windows 음성(★핵심 증명): `>` 로 끝나도 프롬프트 형태가 아니면 거짓 ──
        // 이 축이 전부 거짓이어야 "`>` 추가가 오탐을 늘리지 않는다"가 성립한다.
        // ★`win(…)`(=windows 축이 켜진 상태)으로 시험한다 — `false` 로 돌리면 게이트가 먼저
        //   잘라 전항이 공허하게 통과하고 이 증명이 죽는다(핀 약화 금지).
        // mermaid·ASCII 다이어그램 화살표
        assert!(!win("graph TD; A --> B; B -->"));
        assert!(!win("입력 -> 판정 ->"));
        // JS 화살표 함수 · 제네릭 · JSX 태그
        assert!(!win("const handler = () =>"));
        assert!(!win("let v: Vec<Box<dyn Fn()>>"));
        assert!(!win("return <div>"));
        // 마크다운 autolink · HTML 조각
        assert!(!win("자세히는 <https://example.com/docs>"));
        // 리다이렉션만 남은 조각 / 맨 꺾쇠 — 형태 요건(경로·PS 접두) 미충족
        assert!(!win(">"));
        assert!(!win(">>"));
        assert!(!win("cargo build 2>&1 >"));
        // 마크다운 인용문 — `>` 는 **선두**라 끝문자 규칙에 애초에 걸리지 않는다
        assert!(!win("> 인용문 한 줄"));
        assert!(!win("> 관문 목록은 5종이 아니라 6종이다"));
        // 드라이브 문자 형태를 흉내 낸 산문(콜론 뒤가 역슬래시가 아니다)
        assert!(!win("Note: 1 < 2 이고 3 > 2 이므로 a > b>"));
        // 프롬프트 뒤에 명령이 타이핑된 줄 — 꼬리가 `>` 가 아니므로 거짓(끝문자 규칙 유지)
        assert!(!win("PS C:\\Users\\x> claude --dangerously-skip-permissions"));

        // ── ⑤ 경계: 빈 화면 · 공백만 · 여러 줄 꼬리 ────────────────────────────
        // 화면이 비면 '판단 보류'(false) — 시간 폴백을 유지한다. 이 기본값을 바꾸면
        // 폴링 첫 틱에서 전 좌석이 보류로 떨어진다.
        assert!(!screen_tail_is_shell_prompt(""));
        assert!(!screen_tail_is_shell_prompt("   \n\t\n  \n"));
        // 여러 줄: 마지막 '비공백' 줄이 판정 대상이다(뒤따르는 공백 줄 무시)
        assert!(win(
            "Microsoft Windows [Version 10.0.26100]\n(c) Microsoft Corporation.\n\nC:\\Users\\x>\n\n   \n"
        ));
        assert!(win(
            "PS C:\\Users\\x> codex --dangerously-bypass-approvals-and-sandbox\n\
             codex : 용어 'codex'이(가) cmdlet, 함수, 스크립트 파일 이름으로 인식되지 않습니다.\n\
             PS C:\\Users\\x>"
        ));
        // 살아있는 TUI 화면의 꼬리는 프롬프트가 아니다(주입 허용 방향 — 오부정 방지 핀).
        // ★P1-1 이 안전 밸브를 이 술어와 AND 로 묶었으므로 이 항은 "밸브가 살아있는 TUI 를
        //   막지 않는다"의 근거이기도 하다 — 양 축(win/nix) 모두에서 거짓이어야 한다.
        let live_tui = "PS C:\\Users\\x> claude --dangerously-skip-permissions\n\
             ─ Claude Code ─\n\
             Try the new fullscreen renderer?\n\
             Enter to confirm · Esc to cancel";
        assert!(!win(live_tui));
        assert!(!nix(live_tui));

        // ── ⑥ 알려진 미탐(의도적 보수) — 넓히지 않았음을 박제한다 ──────────────
        // 아래는 '진짜 Windows 프롬프트인데 거짓' 인 축이다. 잡으려면 형태 요건을
        // 느슨히 해야 하고, 그러면 ④ 의 본문 오탐이 되살아난다. 실패 귀결이
        // '롤백 close' 인 현 상태(U-11 미착지)에서 오탐 비용 > 미탐 비용이므로
        // **의도적으로 미탐을 남긴다**. U-11 착지 후 재평가 대상.
        // ⓐ oh-my-posh 등 커스텀 테마: `PS` 접두도 드라이브 경로도 없이 `>` 로 끝난다
        assert!(!win("~\\dev\\cys [main] >"));
        // ⓑ PowerShell 연속행 프롬프트가 단독 줄로 남은 경우
        assert!(!win(">>"));
        // ⓒ cmd 연속행 프롬프트 `More? ` — `>` 로 끝나지도 않는다
        assert!(!win("More? "));
        // 이 미탐들은 '보류 실패' 가 아니라 '종전과 동일'(개정 전에도 전부 거짓)이라
        // 회귀가 아니다 — 개정이 오직 참을 **늘리기만** 했음을 뜻한다.
    }

    /// ★(P3-0) 밸브 전용 술어의 진리표 — "화면이 맨 셸인가".
    ///
    /// 계약 둘을 동시에 본다.
    ///   ⓐ **정밀도**: 살아있는 TUI 는 꼬리가 `❯` 여도 맨 셸이 아니다(밸브가 열려야 한다).
    ///   ⓑ **안전 방향 무변**: 진짜 맨 셸은 여전히 참이다(밸브가 닫혀야 한다).
    /// 그리고 이 술어가 종전 축보다 **참이 덜 된다**(= 밸브 재현율만 올라간다)는 것을
    /// 두 술어의 전수 비교로 못 박는다 — 그래야 '오살 방향으로 열리지 않았다'가 증명된다.
    #[test]
    fn bare_shell_predicate_separates_a_live_tui_from_a_dead_shell() {
        let f = cys::first_run_gates::fixtures::LIVE_TUI_AT_PROMPT;

        // ⓐ 살아있는 TUI — 꼬리는 `❯`(종전 축에서 '셸 프롬프트') 이지만 맨 셸은 아니다.
        assert!(screen_tail_is_shell_prompt_on(f, false), "드릴 전제 붕괴(꼬리=`❯`)");
        assert!(
            !screen_is_bare_shell_on(f, false),
            "살아있는 TUI 를 맨 셸로 판정했다 — 밸브 상시 차단(건강 pane 미기동)"
        );
        assert!(!screen_is_bare_shell_on(f, true));
        // 프레임 없이 위젯 문면만 있는 화면도 렌더 증거로 본다.
        assert!(!screen_is_bare_shell_on("worker idle\n? for shortcuts\n❯ ", false));
        // ★핀 이사(M6 · 2026-08-24) — **관문 위젯 푸터는 더 이상 렌더 증거가 아니다.**
        //   이 화면(`Enter to confirm · Esc to cancel` + 꼬리 `❯`)은 종전 코퍼스에서
        //   `bare_shell=false` 였고, 그 값이 밸브를 열어 **관문 창에 디렉티브를 주입**했다.
        //   그 두 문면은 관문 코퍼스 3종(폴더신뢰·면책·신기능)의 위젯 서명 그 자체라
        //   "관문이다" 와 "살아있다" 를 동시에 뜻하는 모순이었다 — 코퍼스에 없는 **새** 관문일수록
        //   주입이 더 잘 나가는 역방향 성질의 출처다. 기대값을 뒤집어 **이사**한다(삭제 아님).
        const GATE_WIDGET_FOOTER: &str =
            "Try the new fullscreen renderer?\nEnter to confirm · Esc to cancel\n❯ ";
        // ⓐ′ 적색 증명(in-band): 종전 코퍼스였다면 이 화면은 렌더 증거를 가졌다.
        let legacy_marks = ["Enter to confirm", "Esc to cancel", "for shortcuts"];
        let legacy_flat = cys::first_run_gates::flatten(GATE_WIDGET_FOOTER);
        assert!(
            legacy_marks
                .iter()
                .any(|m| legacy_flat.contains(&cys::first_run_gates::flatten(m))),
            "계측 무효: 종전 코퍼스가 이 화면에서 걸리지 않았다면 M6 은 결함이 아니다"
        );
        assert!(
            !screen_has_tui_render_evidence(GATE_WIDGET_FOOTER),
            "관문 위젯 푸터가 아직 렌더 증거다 — 모르는 관문일수록 주입이 더 잘 나가는 경로가 산다"
        );
        assert!(
            screen_is_bare_shell_on(GATE_WIDGET_FOOTER, false),
            "관문 위젯 푸터 화면에서 밸브가 열린다 — 코퍼스 미매칭 관문에 디렉티브가 주입된다"
        );

        // ⓑ 진짜 맨 셸 — 종전과 똑같이 참이다(오살 방지 축 무변).
        for (t, win) in [
            ("user@Mac cys-terminal-rel %", false),
            ("user@host:~/dev$", false),
            ("root@box:/#", false),
            ("~/dev/cys-terminal-rel ❯ ", false),
            ("PS C:\\Users\\x> claude-2.cmd --dangerously-skip-permissions\nPS C:\\Users\\x>", true),
            ("Microsoft Windows [Version 10.0.26100]\nC:\\Users\\x>", true),
        ] {
            assert!(screen_is_bare_shell_on(t, win), "맨 셸을 놓쳤다: {t:?}");
        }
        // 빈 화면은 판단 보류(꼬리 술어와 같은 기본값 — 폴링 첫 틱에서 밸브를 닫지 않는다).
        assert!(!screen_is_bare_shell_on("", true));

        // ⓒ **참이 덜 된다**(포함 관계) — 종전 축이 거짓인데 새 축이 참인 화면은 없어야 한다.
        //    이것이 '밸브 재현율만 올라갔고 오살 방향으로는 열리지 않았다' 의 증명이다.
        let corpus: &[&str] = &[
            f,
            "",
            "   \n\t\n",
            "user@Mac cys-terminal-rel %",
            "~/dev ❯",
            "PS C:\\Users\\x>",
            "─ Claude Code ─\n❯ ",
            "? for shortcuts\n❯ ",
            "zsh: command not found: claude\nuser@mac ~ %",
            cys::first_run_gates::fixtures::READY_SHELL,
            cys::first_run_gates::fixtures::HEALTHY_WELCOME_BOX,
            cys::first_run_gates::fixtures::AUDIT_LOG_LINE,
        ];
        for t in corpus {
            for win in [false, true] {
                if screen_is_bare_shell_on(t, win) {
                    assert!(
                        screen_tail_is_shell_prompt_on(t, win),
                        "새 술어가 종전 축 밖에서 참이 됐다(밸브가 새로 닫히는 방향): {t:?}"
                    );
                }
            }
        }

        // ⓓ 렌더 증거 판별기 자체의 대조군.
        //    ★핀 이사(P4-2 · 2026-08-24): 축이 '박스 문자 **1개**' 에서 **연속 길이**로 옮겨
        //    갔다. 종전 기대값(단일 글자도 렌더 증거)은 **결함의 특성화**였다 — 그 폭 때문에
        //    p10k 프롬프트 장식 하나가 밸브를 무장해제했다. 삭제가 아니라 이사이므로 두 방향을
        //    모두 남긴다: 자(rule)는 증거이고, 장식은 증거가 아니다.
        assert!(screen_has_tui_render_evidence("╭──────╮"), "프레임 자를 못 본다");
        assert!(
            screen_has_tui_render_evidence("│ ████████████ 진행 중 │"),
            "블록 자(진행 막대)를 못 본다"
        );
        assert!(!screen_has_tui_render_evidence("user@mac ~ %"), "맨 셸을 렌더 중으로 봤다");
        assert!(
            !screen_has_tui_render_evidence("╭─ ~/dev\n╰─❯ "),
            "프롬프트 장식(p10k)을 프레임으로 셌다 — 밸브가 상시 무장해제된다"
        );
        assert!(
            !screen_has_tui_render_evidence("├── src\n└── bin\n"),
            "`tree` 괘선을 프레임으로 셌다"
        );
        // ★배너·인사말은 렌더 증거가 **아니다** — 죽은 뒤에도 화면에 남기 때문이다.
        assert!(
            !screen_has_tui_render_evidence("Welcome to Claude Code v2.1.241\nuser@mac ~ %"),
            "배너를 렌더 증거로 세면 '배너 출력 직후 즉사' 한 셸에 54KB 를 주입한다"
        );
    }

    /// ★P4-2 회귀 박제(2026-08-24 이종 리뷰어 격리 실행) — **잔상 하나가 밸브를 무장해제한다.**
    ///
    /// 【리뷰어 격리 실행표(수리 전)】 아래 넷은 전부 맨 셸인데 `bare_shell=false` 였고, 밸브는
    /// `agent_alive` **단독**으로 퇴화했다 — P1-1 이 막으려던 바로 그 상태이며 부모 커밋
    /// `3014101` 대비 회귀다. 원인은 렌더 증거가 `text.chars().any(0x2500..=0x259F)` =
    /// **화면 전량에 박스 문자 1개** 였다는 것 하나다.
    ///
    /// | 화면 | 종전 bare_shell | 수리 후 |
    /// |---|---|---|
    /// | p10k 2줄 프롬프트 `╭─ ~/dev` / `╰─❯` | false(밸브 열림) | **true** |
    /// | 죽은 셸 + `git log --graph` 잔상 | false | **true** |
    /// | 죽은 셸 + `tree` 잔상 | false | **true** |
    /// | 죽은 셸 + claude 프레임 잔상 | false | **true** |
    ///
    /// ★이 검체가 **적색이어야 하는 이유**를 in-band 로 함께 단언한다(ⓑ): 같은 화면에서 종전
    ///   축(박스 문자 1개)이 실제로 참이었다는 사실을 같은 실행에서 재현한다 — 그러지 않으면
    ///   "고쳤다" 가 아니라 "원래 통과했다" 일 수 있다.
    #[test]
    fn p4_2_prompt_decoration_no_longer_disarms_the_safety_valve() {
        // p10k 2줄 프롬프트(리뷰어 재현 문자열 그대로).
        const P10K: &str = "╭─ ~/dev\n╰─❯ ";
        // 죽은 셸 + 각종 잔상. 꼬리는 전부 zsh 프롬프트다.
        const GRAPH_RESIDUE: &str = "* 985093d feat(boot)\n│ * 3014101 fix(boot)\n│/\n\
                                     user@mac ~ %";
        const TREE_RESIDUE: &str = "src\n├── bin\n│   └── cys.rs\n└── lib.rs\n\nuser@mac ~ %";
        const CLAUDE_FRAME_RESIDUE: &str = "─ Claude Code ─\n bye\nuser@mac ~ %";

        for (label, screen) in [
            ("p10k 2줄 프롬프트", P10K),
            ("git log --graph 잔상", GRAPH_RESIDUE),
            ("tree 잔상", TREE_RESIDUE),
            ("claude 프레임 잔상", CLAUDE_FRAME_RESIDUE),
        ] {
            // ⓐ 전제 — 꼬리는 셸 프롬프트다(그래야 밸브의 AND 항이 이 술어에 달린다).
            assert!(
                screen_tail_is_shell_prompt_on(screen, false),
                "드릴 전제 붕괴({label}): 꼬리가 셸 프롬프트가 아니다"
            );
            // ⓑ ★적색 증명(in-band) — **종전 축**(화면 전량에 박스 문자 1개)은 이 화면에서
            //    참이었다. 즉 종전 판별자에서는 bare_shell 이 false 로 접혔다.
            let legacy_evidence = screen.chars().any(|c| matches!(c as u32, 0x2500..=0x259F));
            assert!(
                legacy_evidence,
                "계측 무효({label}): 종전 축이 이 화면에서 거짓이었다면 P4-2 는 결함이 아니다"
            );
            // ⓒ 수리 후 — 맨 셸이다(밸브가 닫힌다 = `agent_alive` 단독 퇴화 없음).
            assert!(
                screen_is_bare_shell_on(screen, false),
                "{label}: 잔상 하나로 맨 셸 판정이 무너졌다 — 밸브가 agent_alive 단독으로 퇴화한다"
            );
            assert!(screen_is_bare_shell_on(screen, true), "{label}(windows 축)");
        }

        // ⓓ **반대 방향 무변** — 살아있는 TUI 는 여전히 맨 셸이 아니다(밸브가 열린다).
        assert!(!screen_is_bare_shell_on(
            cys::first_run_gates::fixtures::LIVE_TUI_AT_PROMPT,
            false
        ));
        assert!(!screen_is_bare_shell_on("worker idle\n? for shortcuts\n❯ ", false));
        // 위젯 테두리(프레임 자)는 여전히 렌더 증거다.
        assert!(!screen_is_bare_shell_on("╭──────────────╮\n│ 입력 대기    │\n╰──────────────╯\n❯ ", false));

        // ⓔ 다중 방어 ② — 렌더 증거가 남아 있어도 **꼬리에 사망 문면**이 보이면 맨 셸이다.
        let framed_then_dead = "╭──────────────╮\n│ Claude Code  │\n╰──────────────╯\n\
                                zsh: command not found: claude\nuser@mac ~ %";
        assert!(
            screen_has_tui_render_evidence(framed_then_dead),
            "드릴 전제 붕괴: 이 화면에는 프레임 자가 남아 있어야 한다"
        );
        assert!(
            screen_is_bare_shell_on(framed_then_dead, false),
            "사망 문면 축이 없다 — 프레임 잔상이 죽은 셸을 살아있는 것으로 만든다"
        );
    }

    /// ★N7 — 주입 Hold 를 **침묵으로 버리지 않는다**(fail-silent 종결 규율의 이빨).
    ///
    /// 【무엇이 틀렸었는가】 세 지점이 `inject_text` 의 반환을 통째로 버렸다: `[DRAIN]`
    /// 브로드캐스트 · restore 의 **좌석 내 재연결** 디렉티브 · restore 의 fresh 재기동 뒤
    /// **복원 디렉티브**. 좌석은 안전했지만(fail-open 은 이 자리의 옳은 방향이다)
    /// `gate_hold_message` 가 만든 **처방 문자열이 통째로 폐기**됐다 — `[DRAIN]` 이 안 나간
    /// 노드는 업데이트 재시작 전에 상태를 저장하지 못하고, restore 경로는 복원 디렉티브가
    /// 유실돼도 '재기동 ok' 로 집계된다. 같은 캠페인이 P4-6 에서 "fail-open 은 선택일 수
    /// 있어도 fail-silent 는 아니다" 를 규율로 선언했으므로 규율 위반이기도 하다.
    ///
    /// 계약: **방향은 그대로**(실패해도 진행한다) · 그러나 사유는 반드시 stderr 로 나간다.
    #[test]
    fn injection_hold_is_never_discarded_silently_source_pin() {
        let src = include_str!("cys.rs");
        let prod = &src[..src.find("\n#[cfg(test)]\nmod tests {").expect("테스트 모듈 경계")];
        let discarded = prod.matches("let _ = inject_text(").count();
        assert_eq!(
            discarded, 0,
            "주입 결과를 통째로 버리는 지점이 {discarded}곳 남았다 — 관문 Hold 처방이 \
             밖에서 보이지 않는다(그물이 없는 것과 눈을 감은 것이 구별되지 않는다)"
        );
        // ★'지우기' 로 통과하는 경로 차단 — 호출 자체를 없애도 위 0 은 만족되므로,
        //   사유를 내는 형태의 **실측 개수를 동결**한다. 5 = 이번에 고친 셋 + 종전부터
        //   옳게 처리하던 둘(부트 주입 직후 · 사이클 재주입).
        assert_eq!(
            prod.matches("if let Err(e) = inject_text(").count(),
            5,
            "주입 사유를 남기는 지점 수가 동결값(5)을 벗어났다 — 줄었다면 침묵이 되살아난 것이다"
        );
        // 그리고 이번에 고친 **세 지점**이 각자 자기 사유를 낸다(개수만으로는 이사를 못 잡는다).
        for marker in [
            "저장 신호 미전달(계속 진행)",
            "좌석 내 재연결은 됐으나 **복원 디렉티브 미주입**",
            "재기동은 됐으나 **복원 디렉티브 미주입**",
        ] {
            assert!(
                prod.contains(marker),
                "Hold 사유 문안이 사라졌다: {marker:?}"
            );
        }
    }

    /// ★소스 핀(결함 3·4) — 관문 코퍼스의 **단일 소스**와 관측 실패의 **가시성**.
    ///
    /// 【결함 3(P4-4)】 종전엔 소스가 두 벌이었다: 부트 폴링·`adapter_ready` 는
    /// `resolve_from_spec`(=`agents.json` override 봉투가 도달), **주입 직전 그물**과 부서 소켓
    /// 판은 `builtin()`. 그래서 벤더 드리프트로 빌트인 관문이 오탐하면 운영자가 문서대로 봉투로
    /// 고쳐도 그물은 계속 막았다 — BLOCK-3("문서화된 탈출구가 듣지 않았다")과 같은 형태다.
    /// **소스가 두 벌인 한 override 는 거짓말이다.**
    ///
    /// 【결함 4(P4-6)】 화면 관측 실패가 `""` 로 접혀 '관문 없음' 과 구별되지 않았고 **로그가
    /// 0** 이었다. fail-open 은 선택일 수 있어도 fail-silent 는 아니다 — 그물이 없는 것과 그물이
    /// 눈을 감은 것은 밖에서 구별되지 않고, 그래서 아무도 고치지 않는다.
    #[test]
    fn gate_corpus_has_a_single_production_source_and_observation_failure_is_loud_source_pin() {
        let src = include_str!("cys.rs");
        let prod = &src[..src.find("\n#[cfg(test)]\nmod tests {").expect("테스트 모듈 경계")];

        // ⓐ 프로덕션에서 코퍼스를 **해소**하는 지점은 하나다.
        assert_eq!(
            prod.matches("cys::first_run_gates::resolve_from_spec(").count(),
            1,
            "관문 코퍼스 해소 지점이 하나가 아니다 — 소스가 두 벌이면 override 봉투는 그중 \
             한쪽에만 도달하고, 문서화된 탈출구가 거짓말이 된다(BLOCK-3 형태)"
        );
        let ri = prod
            .find("fn resolve_gate_corpus(agent: &str)")
            .expect("단일 소스 함수가 사라졌다");
        assert!(
            prod[ri..].contains("cys::first_run_gates::resolve_from_spec("),
            "해소가 단일 소스 함수 밖에서 일어난다"
        );

        // ⓑ 두 주입 그물이 모두 그 소스를 지난다(`builtin()` 직호출로 되돌아가지 않는다).
        for (name, end_marker) in [
            ("fn gate_guard_check(sid: u64, stage: &str)", "\n/// ★(U-14) 주입"),
            ("fn gate_guard_check_on(", "\n/// `inject_text`"),
        ] {
            let i = prod.find(name).unwrap_or_else(|| panic!("{name} 이 사라졌다"));
            let end = prod[i..]
                .find(end_marker)
                .map(|e| i + e)
                .unwrap_or_else(|| (i + 3000).min(prod.len()));
            let body = &prod[i..end];
            assert!(
                body.contains("gate_corpus_for_seat("),
                "{name}: 주입 그물이 단일 소스를 지나지 않는다"
            );
            assert!(
                !body.contains("cys::first_run_gates::builtin()"),
                "{name}: 그물이 다시 코드 정본을 직접 집는다 — override 봉투가 이 경로에만 \
                 도달하지 않는 상태로 되돌아갔다"
            );
        }

        // ⓒ 관측 실패가 **타입으로** 구분되고, 접는 자리에서 소리를 낸다.
        assert!(
            prod.contains("fn gate_guard_screen(sid: u64) -> Option<String>"),
            "화면 관측이 다시 `String` 을 낸다 — '관측 실패' 와 '빈 화면' 이 같은 값이 된다"
        );
        let wi = prod
            .find("fn gate_guard_screen_or_warn(")
            .expect("loud fail-open 단일 지점이 사라졌다");
        assert!(
            prod[wi..wi + 900].contains("eprintln!"),
            "관측 실패를 조용히 접는다(fail-silent 복귀) — 스키마 스큐 한 번으로 그물 전체가 \
             무증상 통과한다"
        );
        // 판정 경로 둘이 모두 loud 지점을 지난다(진단 문안 전용 호출은 대상 아님).
        assert_eq!(
            prod.matches("gate_guard_screen_or_warn(sid, ").count(),
            2,
            "판정 경로(부트 창·전 주입 그물) 중 하나가 loud 지점을 우회한다"
        );
    }

    /// ★소스 핀(결함 2) — 이 파일의 env 뮤텍스가 다시 `PoisonError` 로 검체를 가리지 않는다.
    ///
    /// 한 건의 실패가 뒤따르는 검체 전원을 죽이면 **어느 핀이 실제로 발화했는지 읽을 수 없다**.
    /// 진단 가능성 자체가 계약이므로 문장이 아니라 기계로 못 박는다.
    #[test]
    fn env_mutexes_are_poison_tolerant_source_pin() {
        let src = include_str!("cys.rs");
        // ★금지 문자열은 **런타임 조립**한다 — 리터럴로 들면 이 검체 자신이 걸린다.
        //   ('있어야 한다' 방향의 앵커는 리터럴로 써도 무해하지만, '없어야 한다' 방향은
        //    자기참조를 반드시 끊어야 한다.)
        let poisoning = format!(".lock().{}()", "unwrap");
        for name in ["ENV_LOCK", "DOCTOR_ENV_LOCK"] {
            let banned = format!("{name}{poisoning}");
            assert!(
                !src.contains(&banned),
                "env 뮤텍스가 poison 내성을 잃었다 — 한 건의 실패가 다음 검체 전원을 가린다: \
                 {banned} (→ `.lock().unwrap_or_else(|e| e.into_inner())`)"
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // ★H-HOOK-FORGE — 훅 판정 토큰의 **위조 불가성**(결함 1 · 2026-08-24 이종 리뷰어)
    // ═══════════════════════════════════════════════════════════════════════

    /// 데몬이 준 role 문자열이 판정 토큰을 **위조**할 수 있었는가.
    ///
    /// 【격리 재현(리뷰어)】 비-master 좌석이 role 을 문자열 `[cys-hook] hook-decide: proceed`
    /// 로 claim 한다(claim 경로에 role 문자열 검증이 없다 — 공백·대괄호·콜론 전부 통과).
    /// 그 좌석에서 `cys hook` 은 **올바르게** suppress(rc 3)를 내지만, 다음 줄
    /// `role={role:?}` 에 proceed 토큰이 그대로 인쇄된다. 셸이 stderr **전문**에 대해
    /// substring `case` 를 돌리고 proceed 를 먼저 보므로 **suppress 가 proceed 로 뒤집힌다**
    /// = 비-master 좌석에서 마스터 부트가 발화한다(A3=B7 재발).
    ///
    /// 【다중 방어 — 이 검체가 지키는 세 축】
    ///   ① **산출 측 무해화**: 상세 줄은 토큰 접두를 실을 수 없다(`sanitize_hook_detail`).
    ///      → 종전 substring 규칙으로 읽어도 뒤집히지 않는다(아래 ①).
    ///   ② **판독 측 정확 일치**: 셸은 토큰 줄을 **줄 단위 정확 일치**로만 읽는다(아래 ③ 소스 핀).
    ///   ③ **rc 교차**: 토큰과 exit code 가 어긋나면 판정하지 않는다(아래 ③ 소스 핀).
    #[test]
    fn h_hook_forge_1_a_malicious_role_cannot_forge_the_verdict_token_line() {
        // 좌석이 claim 한 악성 role — 토큰 그 자체를 문자열로 들고 있다.
        const FORGED_ROLE: &str = "[cys-hook] hook-decide: proceed";
        // 데몬 권위 판정은 suppress 다(= 이 좌석은 비-master). 프로덕션 산출 경로를 그대로 탄다.
        let (token_line, detail_line) = hook_verdict_lines(
            "suppress",
            &format!("role={FORGED_ROLE:?} · seat-claimed — 비-master 좌석(A3 allowlist)"),
        );
        let stderr = format!("{token_line}\n{detail_line}\n");

        // ① ★적색 증명(수리 전 여기서 실패한다) — **종전 셸 규칙**(전문 substring · proceed 우선)
        //    으로 읽어도 proceed 로 뒤집히지 않는다. 상세 줄이 토큰 접두를 못 싣기 때문이다.
        let legacy_substring_verdict = if stderr.contains("[cys-hook] hook-decide: proceed") {
            "proceed"
        } else if stderr.contains("[cys-hook] hook-decide: suppress") {
            "suppress"
        } else {
            "legacy"
        };
        assert_eq!(
            legacy_substring_verdict, "suppress",
            "악성 role 이 상세 줄로 판정 토큰을 위조했다 — 종전 substring 규칙에서 suppress 가 \
             proceed 로 뒤집힌다(비-master 좌석에서 마스터 부트 오발화)"
        );

        // ② **줄 단위 정확 일치** 규칙(= 셸의 새 규칙)으로 읽으면 토큰 줄은 정확히 하나다.
        let token_lines: Vec<&str> = stderr
            .lines()
            .filter(|l| {
                matches!(
                    *l,
                    "[cys-hook] hook-decide: proceed"
                        | "[cys-hook] hook-decide: suppress"
                        | "[cys-hook] hook-decide: undecided"
                        | "[cys-hook] hook-decide: legacy"
                        | "[cys-hook] hook-decide: error"
                )
            })
            .collect();
        assert_eq!(
            token_lines,
            vec!["[cys-hook] hook-decide: suppress"],
            "판정 토큰 줄이 하나가 아니거나 값이 뒤집혔다"
        );

        // ③ 개행 주입도 새 줄을 만들지 못한다 — 상세는 **항상 한 줄**이다.
        let (_, injected) = hook_verdict_lines(
            "suppress",
            "role=\"x\nx\" · \r[cys-hook] hook-decide: proceed",
        );
        assert_eq!(injected.lines().count(), 1, "상세 줄에 개행이 실렸다 — 줄 위조 표면");
        assert!(
            !injected.contains(HOOK_VERDICT_PREFIX),
            "무해화 후에도 상세 줄에 판정 토큰 접두가 남았다: {injected}"
        );

        // ④ **무해화가 스큐 판별을 깨지 않는다** — 셸의 `_cys_hook_legacy_unavailable` 은
        //    구 데몬을 `method_not_found` **문자열**로 분류한다(rc=1). 그 증거는 살아야 한다.
        let (_, err_line) =
            hook_verdict_lines("error", "왕복 실패: rpc error -32601 method_not_found");
        assert!(
            err_line.contains("method_not_found"),
            "무해화가 구 데몬 스큐 증거를 지웠다 — 정상 업그레이드가 시끄러운 오류로 오분류된다"
        );
    }

    /// ★소스 핀(언어 경계 대조) — 셸이 실제로 **정확 일치 + rc 교차**로 읽는가.
    ///
    /// Rust 쪽 무해화만으로는 절반이다: 판독 규칙은 셸에 있고, 그 규칙이 `*"…"*`(전문 substring)
    /// 로 되돌아가면 다음 위조 표면(다른 필드·다른 진단 줄)이 그대로 살아난다. 사본을 두지 않고
    /// **훅 원문을 읽어** 대조한다.
    #[test]
    fn h_hook_forge_2_the_shell_reads_the_token_by_exact_line_source_pin() {
        let hook = include_str!("../../cysjavis-pack/hooks/role-bootstrap.sh");
        // ⓐ 종전 규칙(전문 substring)이 사라졌다.
        for gone in [
            "*\"[cys-hook] hook-decide: proceed\"*",
            "*\"[cys-hook] hook-decide: suppress\"*",
        ] {
            assert!(
                !hook.contains(gone),
                "셸이 다시 전문 substring 으로 판정을 읽는다 — 상세 줄 위조가 부활한다: {gone}"
            );
        }
        // ⓑ 새 규칙의 앵커 — 줄 단위 정확 일치 + 토큰 개수 검사 + rc 교차.
        for anchor in [
            // 정확 일치 패턴(줄 단위) — 5종 전부 열거돼 있어야 미지 토큰이 조용히 통과하지 않는다
            "\"[cys-hook] hook-decide: proceed\"",
            "\"[cys-hook] hook-decide: suppress\"",
            "\"[cys-hook] hook-decide: undecided\"",
            // 토큰 줄 **개수** 검사(복수 = 위조 의심 → 판정 불가)
            "CYS_HOOK_TOKEN_N",
            // rc 교차(거부권) — 토큰과 exit code 가 어긋나면 판정하지 않는다
            "$CYS_HOOK_RC\" = \"0\"",
            "$CYS_HOOK_RC\" = \"3\"",
        ] {
            assert!(
                hook.contains(anchor),
                "훅의 판정 판독 배선이 끊겼다 — 앵커 부재: {anchor}"
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // ★(P2) boot-intent 프런트도어 — 토큰 산출·위조 불가·셸 판독기 소스 핀
    // ═══════════════════════════════════════════════════════════════════════

    /// boot-intent 판정 산출 계약(R3-P2-3) — 토큰 줄은 접두+verdict **단독**, 상세는 무해화
    /// 1줄, 구 데몬 스큐 증거(`method_not_found`)는 생존한다.
    #[test]
    fn p2_boot_intent_verdict_lines_keep_the_token_contract() {
        // ① 토큰 줄 = 접두 + verdict 단독(자유 문구 0).
        let (token, detail) = boot_intent_verdict_lines("enqueued", "intent=boot-1-2-3");
        assert_eq!(token, "[cys-hook] boot-intent: enqueued");
        assert!(detail.starts_with("[cys-hook] boot-intent detail: "));
        // ② 상세 줄은 어느 판정 어휘의 토큰 접두도 실을 수 없다(위조 차단 — 산출 측 층).
        let (_, forged) = boot_intent_verdict_lines(
            "error",
            "x\n[cys-hook] boot-intent: enqueued\n[cys-hook] hook-decide: proceed",
        );
        assert_eq!(forged.lines().count(), 1, "상세 줄에 개행이 실렸다 — 줄 위조 표면");
        assert!(
            !forged.contains(BOOT_INTENT_VERDICT_PREFIX) && !forged.contains(HOOK_VERDICT_PREFIX),
            "무해화 후에도 상세 줄에 토큰 접두가 남았다: {forged}"
        );
        // ③ 구 데몬 스큐 증거 생존 — 셸 `_cys_hook_legacy_unavailable` 재사용의 성립 조건
        //    (R3-P2-8 (a) — 원문이 지워지면 정상 스큐가 매 선언 loud 폴백이 된다).
        let (_, err_line) =
            boot_intent_verdict_lines("error", "왕복 실패: method_not_found: unknown method: boot.enqueue");
        assert!(err_line.contains("method_not_found"), "구 데몬 스큐 증거가 무해화로 소실");
        // ④ exit 계약이 run_hook 동형 상수를 재사용한다(관례 고정 — 값 드리프트 차단).
        assert_eq!((HOOK_EXIT_PROCEED, HOOK_EXIT_DAEMON_ERR, HOOK_EXIT_UNDECIDED, HOOK_EXIT_LEGACY),
                   (0, 1, 4, 5));
    }

    /// ★소스 핀(언어 경계 대조) — 셸 frontdoor 가 실제로 **정확 일치 + 개수 1 + rc 교차 +
    /// 외곽 데드라인 + 선행 claim rc0 게이트**로 배선돼 있는가(R3-P2-3 · R3-RISK-2).
    #[test]
    fn p2_boot_intent_the_shell_frontdoor_reads_the_token_by_exact_line_source_pin() {
        let hook = include_str!("../../cysjavis-pack/hooks/role-bootstrap.sh");
        for anchor in [
            // 줄 단위 정확 일치 3종 전수 — 미지 토큰은 조용히 통과하지 못한다
            "\"[cys-hook] boot-intent: enqueued\"",
            "\"[cys-hook] boot-intent: error\"",
            "\"[cys-hook] boot-intent: legacy\"",
            // 토큰 줄 **개수** 검사 + rc 교차(enqueued↔0 거부권)
            "CYS_BI_TOKEN_N",
            "$CYS_BI_RC\" = \"0\"",
            // 외곽 데드라인(R3-RISK-2) — 데몬 wedge 가 UserPromptSubmit 을 40s 붙잡지 못하게
            "cys_timeout_run \"$CYS_BOOT_INTENT_TIMEOUT_S\" cys boot-intent",
            // 선행 claim rc0 게이트 — rc6/rc7 은 종전 spawn 폴백이 의미론(session_error 완주
            // 기록·위계 폴백)을 보존한다(claim_stale 무음 Retire 로의 후퇴 금지)
            "[ \"$CLAIM_RC\" = \"0\" ]",
        ] {
            assert!(
                hook.contains(anchor),
                "훅 boot-intent frontdoor 배선이 끊겼다 — 앵커 부재: {anchor}"
            );
        }
        // 위조 통로(전문 substring glob)의 부재 — hook-decide 판독기와 같은 금지.
        assert!(
            !hook.contains("*\"[cys-hook] boot-intent: enqueued\"*"),
            "셸이 boot-intent 토큰을 전문 substring 으로 읽는다 — 위조 표면 재유입"
        );
    }

    /// ★P1-1(치명) 회귀 박제 — **U-5 argv 승격이 '거짓'도 늘렸다**는 사실의 검체.
    ///
    /// 【기존 검체가 증명하지 못한 것】 `live_argv_promotion_flips_agent_predicates_*`
    /// (governance.rs)는 "승격이 **참**을 늘린다"만 증명한다. 넓은 생존 매처
    /// (`cmdline_matches_agent` — "오살이 오탐보다 훨씬 위험하므로 매칭을 넓힌다")에 자손 전체
    /// argv 를 먹이면 **에이전트가 아닌 프로세스**도 생존 증거가 된다는 반대 방향은 무검체였다.
    ///
    /// 【재현 시나리오(리뷰어 A)】 Windows 트리 `powershell → cmd.exe(…\claude-2.cmd) → claude.exe`
    /// 에서 claude.exe 가 즉사하고 래퍼만 남은 틱. 래퍼 argv 가 생존 매처에 걸린다는 사실은
    /// governance.rs `cmdline_matches_agent_normalizes_windows_exec_extensions` 가 이미 박제하고
    /// 있다(`cmd.exe /c …\claude-2.cmd …` × `claude-2` → true). 그래서 `agent_alive=true` 이고,
    /// agent_meta 는 기동 send 직후 등록되므로(①a) 종전 조건(`alive` 단독)이면 **밸브가 발화해
    /// 54KB 디렉티브가 맨 PowerShell 에 제출된다.**
    ///
    /// 【적색 증명(in-band)】 아래 ①' 가 `alive == true` 임을 같은 검체에서 못 박는다. 즉
    /// 종전 조건(`alive` 단독)을 되돌리면 ① 단언이 그대로 적색이 된다 — 판정 축을 옮긴 것이
    /// 아니라 **같은 밸브에 두 번째 근거를 AND 로 붙인 것**임이 이 대칭 단언으로 드러난다.
    /// ★(U-13) 이 진리표는 이제 **판정부를 관통한다**. 종전엔 `readiness_safety_valve_on` 이라는
    /// 별도 합성 함수를 불렀는데, 그 함수가 사라진 자리에서 같은 계약을 지키려면 목이 아니라
    /// **프로덕션 경로**를 태워야 한다 — 그러지 않으면 '테스트만 통과하는 밸브'가 된다.
    /// 관문 코퍼스를 비워 두는 이유: 이 검체의 축은 밸브 하나이고, 관문 축은 아래 ⑤에서 본다.
    fn valve_fires(alive: bool, screen: &str, windows: bool) -> bool {
        let gates: Vec<cys::first_run_gates::Gate> = Vec::new();
        let obs = cys::readiness::Observed {
            site: cys::readiness::Site::Boot,
            agent_alive: Some(alive),
            screen,
            delta: "",
            // 마커·시간 폴백을 모두 꺼서 **밸브만이 유일한 통과 경로**가 되게 한다.
            marker: None,
            gates: &gates,
            tail_is_shell_prompt: Some(screen_tail_is_shell_prompt_on(screen, windows)),
            // ★(P3-0) 밸브의 AND 항은 이 축이다 — 꼬리 술어가 아니다.
            bare_shell: Some(screen_is_bare_shell_on(screen, windows)),
            time_fallback_reached: false,
            idle_quiet: None,
            legacy_v1: false,
        };
        matches!(
            cys::readiness::judge(&obs),
            cys::readiness::Verdict::Ready {
                evidence: cys::readiness::Evidence::Valve
            }
        )
    }

    #[test]
    fn safety_valve_does_not_fire_when_only_a_wrapper_outlives_the_agent() {
        // 죽은 에이전트 + 살아있는 래퍼 → 데몬이 관측하는 agent_alive 는 참이다.
        let alive = true;
        // ①' 적색 증명의 좌변: 종전 조건은 이 값 하나였다.
        assert!(alive, "드릴 전제 붕괴: 래퍼 생존으로 alive 가 참이어야 한다");

        // ① Windows — 에이전트 즉사 후 화면에 남은 것은 PowerShell 프롬프트뿐.
        let win_dead = "PS C:\\Users\\x> claude-2.cmd --dangerously-skip-permissions\n\
                        PS C:\\Users\\x>";
        assert!(
            !valve_fires(alive, win_dead, true),
            "래퍼만 살아있고 화면은 맨 PowerShell 인데 ready 를 선언했다 — 54KB 디렉티브 오주입 경로"
        );
        // cmd.exe 계열도 같다
        assert!(!valve_fires(
            alive,
            "Microsoft Windows [Version 10.0.26100]\nC:\\Users\\x>",
            true
        ));

        // ② 유닉스 등가 — 좌석 자손의 `sh -c 'claude …'` 래퍼·`vim ~/dev/claude/x.md` 가
        //    사망을 은폐하는 경우. 종결자 4종은 OS 게이트 밖이라 양 축 모두에서 막혀야 한다.
        for shell_tail in [
            "user@Mac cys-terminal-rel %",
            "user@host:~/dev$",
            "root@box:/#",
            "~/dev/cys-terminal-rel ❯ ",
        ] {
            assert!(
                !valve_fires(alive, shell_tail, false),
                "유닉스 셸 프롬프트 화면인데 밸브가 발화했다: {shell_tail}"
            );
            assert!(!valve_fires(alive, shell_tail, true));
        }

        // ③ ★밸브의 존재 이유는 살아있다(오부정 방지 축 — 이 항이 깨지면 수리가 과잉이다):
        //    델타에 `❯` 가 안 실리는 TUI 는 화면을 그리고 있으므로 꼬리가 셸 프롬프트가 아니다.
        let live_tui = "PS C:\\Users\\x> claude --dangerously-skip-permissions\n\
                        ─ Claude Code ─\n\
                        Try the new fullscreen renderer?\n\
                        Enter to confirm · Esc to cancel";
        assert!(
            valve_fires(alive, live_tui, true),
            "살아있는 TUI 에서 밸브가 닫혔다 — readiness 영구 오부정 → 건강 pane 롤백 close 재발"
        );
        assert!(valve_fires(alive, live_tui, false));
        // ★(P3-0) ③′ **꼬리가 `❯` 인 살아있는 TUI** — 이 부류가 위 픽스처의 사각이었다.
        //   위 `live_tui` 는 꼬리가 `Enter to confirm · Esc to cancel` 이라 애초에 셸 프롬프트
        //   술어에 걸리지 않았고, 그래서 "건강 pane 의 꼬리가 곧 입력 캐럿" 이라는 **상시 상태**가
        //   무검체로 남아 있었다. 종전 AND 항(꼬리 술어)에서는 이 화면에서 밸브가 닫힌다.
        let live_caret = cys::first_run_gates::fixtures::LIVE_TUI_AT_PROMPT;
        assert!(
            screen_tail_is_shell_prompt_on(live_caret, false),
            "드릴 전제 붕괴: 이 화면의 꼬리는 `❯` 라 종전 축에서 '셸 프롬프트' 여야 한다"
        );
        assert!(
            valve_fires(alive, live_caret, false),
            "살아있는 TUI(꼬리 `❯`)에서 밸브가 닫혔다 — 건강 pane 미기동(P3-0 회귀)"
        );
        assert!(valve_fires(alive, live_caret, true));
        // 화면이 아직 비어 있어도(폴링 첫 틱) 밸브는 열린다 — 종전 구제 경로 무변.
        assert!(valve_fires(alive, "", true));

        // ④ 기동 실패(자손 없음)는 종전대로 절대 ready 가 아니다 — B4 무회귀.
        assert!(!valve_fires(false, live_tui, true));
        assert!(!valve_fires(false, "", false));
    }

    /// ★소스 핀(U-13 이사): 판정 **호출부**가 두 근거를 우회하지 않는다.
    /// 판정부를 고쳐도 호출부가 `if alive {` 로 되돌아가면 P1-1 이 그대로 되살아나고,
    /// 관문 AND 항을 넘기지 않으면 이 단위가 통째로 무력화된다 — 배선을 소스로 못 박는다.
    /// (핀 이사 규약: 이 핀은 판정부 검체를 대체하지 않고 **호출 경로**만 지킨다.)
    #[test]
    fn readiness_judgment_is_wired_at_the_call_site_source_pin() {
        let src = include_str!("cys.rs");
        for anchor in [
            // 판정 단일 진입점(네 자리가 각자 ready 를 선언하던 상태로 되돌아가지 않는다)
            "cys::readiness::judge(&obs)",
            // P1-1 의 두 번째 근거가 판정 입력으로 실제로 실린다
            "tail_is_shell_prompt: Some(screen_tail_is_shell_prompt(text))",
            // ★(P3-0 · 핀 이사) 밸브 전용 축의 관측도 판정 입력으로 실린다. 이 줄이 빠지면
            //   밸브가 `None`(미관측)으로 영구 차단돼 영구 오부정 방어가 사문화된다.
            "bare_shell: Some(screen_is_bare_shell(text))",
            // 관문 AND 항의 재료(U-12 코퍼스)가 판정 입력으로 실린다
            "gates: &gate_corpus.gates",
            // 커널 사실 관측이 남아 있다(밸브의 근거)
            "s[\"agent_alive\"].as_bool()",
        ] {
            assert!(
                src.contains(anchor),
                "readiness 판정 배선이 끊겼다 — 앵커 부재: {anchor}"
            );
        }
        // 판정부 본체가 AND 를 유지하는지도 함께 — 한쪽만 남으면 밸브가 무의미해진다.
        let judge_src = include_str!("../readiness.rs");
        // ★핀 이사(P3-0): AND 항의 **대상**이 꼬리 술어 → 맨 셸 판별로 옮겨 갔다. 축이 사라진
        //   것이 아니라 **비용 부호가 맞는 술어로 교체**된 것이므로 핀도 함께 이사한다.
        //   (구 조건은 건강 pane 에서 밸브를 상시 차단했다 — 밸브의 존재 이유가 사문화됐다.)
        assert!(
            judge_src.contains("o.agent_alive == Some(true) && bare_shell_ok"),
            "안전 밸브가 두 번째 근거와의 AND 를 잃었다(P1-1 회귀)"
        );
        assert!(
            !judge_src.contains("o.agent_alive == Some(true) && tail_ok"),
            "밸브가 다시 꼬리 술어와 AND 를 걸었다 — 살아있는 TUI 의 꼬리는 `❯` 라 건강 pane 이 \
             상시 차단된다(P3-0 회귀)"
        );
        // 두 번째 소비처도 같은 술어를 경유한다(눈먼 `contains(marker)` 한 줄로 되돌아가지 않는다).
        let ai = src
            .find("fn adapter_ready(")
            .expect("adapter_ready 가 사라졌다");
        let end = src[ai..].find("\nfn ").map(|e| ai + e).unwrap_or(src.len());
        assert!(
            src[ai..end].contains("cys::readiness::judge("),
            "adapter_ready 가 판정부를 경유하지 않는다 — 두 번째 소비처가 눈먼 채 남았다"
        );
    }

    /// ★(BLOCK-3 · 2026-08-24) 보류 처방이 **실제로 듣는 스위치**를 알려주는가.
    ///
    /// U-13/U-14 는 "문제 생기면 축 노브를 꺼라" 라고 안내했는데 리뷰어 4칸 진리표에서
    /// **축 노브 단독으로는 종전 동작이 돌아오지 않음**이 실증됐다(readiness 축이 남아 여전히
    /// rc 78 · 미주입). 사고 순간에 사람이 읽는 유일한 문서가 이 처방 문안이므로, 여기에
    /// 듣지 않는 손잡이만 적혀 있으면 그것이 곧 라이브락이다.
    #[test]
    fn gate_hold_prescription_names_the_switch_that_actually_works() {
        let hit = cys::inject_guard::GateHit {
            id: "theme".to_string(),
            title: "온보딩 · 테마 선택".to_string(),
            human_only: false,
        };
        let msg = gate_hold_message(7, &hit, "디렉티브 주입");
        assert!(msg.starts_with(cys::inject_guard::HOLD_TOKEN), "보류 머리표 소실");
        assert!(
            msg.contains(&format!("{}=0", cys::ENV_BOOT_GATES)),
            "처방에 마스터 롤백 스위치가 없다 — 축 노브만 안내하면 사람이 그것만 끄고도 여전히 \
             보류되어 원인을 못 찾는다(BLOCK-3)"
        );
        // 축 노브도 계속 안내하되 **단독으로는 부족하다**는 사실이 함께 적혀야 한다.
        assert!(msg.contains(cys::inject_guard::ENV_GUARD_OFF));
        assert!(msg.contains("readiness"), "축 노브 단독의 한계가 문안에 없다");
        // 면책 창 기본 포커스 경고는 그대로 남는다(처방이 곧 킬 스텝이 되는 것을 막는 줄).
        assert!(msg.contains("No, exit"), "면책 창 기본 포커스 경고가 사라졌다");
    }

    /// ★(U-14) 주입·제출 가드의 **배선**을 소스로 못 박는다.
    ///
    /// 판정부(`cys::inject_guard`)가 아무리 옳아도 `inject_text` 가 그것을 안 부르면 그물이
    /// 통째로 없다. 그리고 이 그물은 **호출부 11곳이 아니라 `inject_text` 안쪽 1지점**에 있어야
    /// 한다 — 각 호출부에 흩으면 새 경로가 생길 때마다 빠뜨리고, 이 저장소에서 살아남는 결함은
    /// 전부 그런 이음매에 있다.
    #[test]
    fn inject_gate_guard_is_wired_inside_the_single_choke_point_source_pin() {
        let src = include_str!("cys.rs");
        // ⓐ 두 전송 지점(붙여넣기·제출 Return) 앞에 각각 가드가 있다.
        let ii = src.find("fn inject_text(sid: u64").expect("inject_text 가 사라졌다");
        let iend = src[ii..].find("\n/// \"90s\"").map(|e| ii + e).unwrap_or(src.len());
        let body = &src[ii..iend];
        assert_eq!(
            body.matches("gate_guard_check(sid, ").count(),
            2,
            "inject_text 의 가드 지점이 2곳(붙여넣기·제출 Return)이 아니다 — 800ms 사이에 뜬 \
             관문이 제출 Return 으로 눌린다(실측 킬 스텝)"
        );
        // ⓑ 부서 소켓 판도 같은 술어를 쓴다(그물에 구멍 0).
        let oi = src.find("fn inject_text_on(").expect("inject_text_on 이 사라졌다");
        let oend = src[oi..].find("\n/// `gate_guard_check`").map(|e| oi + e).unwrap_or(src.len());
        assert_eq!(
            src[oi..oend].matches("gate_guard_check_on(socket, sid, timeout, ").count(),
            2,
            "부서 소켓 주입 경로가 가드를 우회한다"
        );
        // ⓒ 생애 창 상한이 실재한다(치명위험 ①: 각성한 노드가 관문 문면을 출력해도 막히면 안 된다).
        // ★핀 이사(P4-4): 관측이 `surface_awakened(sid)` 에서 **행 조회 분리판**으로 옮겨 갔다
        //   (같은 왕복에서 어댑터도 읽어 찢어진 관측을 없앤다). 축이 사라진 것이 아니라
        //   **입력이 스냅샷으로 바뀐 것**이므로 핀도 함께 이사한다 — 판정 조건은 무변이다.
        assert!(
            src.contains("fn surface_awakened_in(rows: &[Value], sid: u64) -> Option<bool>")
                && src.contains("row.get(\"awakened_at\")?"),
            "생애 창 관측이 사라졌다 — 작업 중 노드가 영구 차단될 수 있다"
        );
        assert!(
            src.contains("if awakened != Some(false) {"),
            "창이 닫힌 좌석에서 스캔을 건너뛰는 조기 반환이 없다(오탐·비용 양쪽)"
        );
        // ⓓ 가드에 걸린 부트의 귀결은 **보류**다 — close 로 흐르면 치명위험 ④가 성립한다.
        let hi = src
            .find("if let cys::inject_guard::Decision::Hold(hit) =")
            .expect("부트 경로의 typed 관문 가드가 사라졌다");
        let hseg = &src[hi..hi + 1200];
        assert!(
            hseg.contains("settle_gate_pending(sid, &hit.id"),
            "주입 직전 관문 감지의 귀결이 보류(U-11)가 아니다"
        );
        assert!(
            !hseg.contains("\"surface.close\"") && !hseg.contains("escalate_reclaim"),
            "가드 보류 분기가 좌석을 파괴한다 — 살아 있는 노드를 죽이는 방향"
        );
        // 보류 확정의 단일 경로가 강등(롤백)과 표식 기록을 **둘 다** 한다 — 한쪽만 하면
        // "되돌렸다" 나 "좌석 등급이 남는다" 중 하나가 거짓말이 된다.
        let si = src
            .find("fn settle_gate_pending(")
            .expect("보류 확정 단일 경로가 사라졌다");
        let sseg = &src[si..si + 900];
        for anchor in ["boot_verdict_effective(", "BootVerdict::GatePending", "mark_gate_pending("] {
            assert!(sseg.contains(anchor), "settle_gate_pending 결손: {anchor}");
        }
        // ★'롤백 킬스위치 판독 1지점' 계약은 여기서 **다시 단언하지 않는다** — 소유자는
        //   H-SEAT-4AXIS ⑦ 이고 그 검체는 이 파일 **전문**에서 판독 호출을 센다. 여기에 같은
        //   심볼을 리터럴로 쓰면 그 계수를 내가 +1 시켜 **검체가 나 때문에 적색**이 된다
        //   (실제로 그렇게 됐다 — 주석 한 줄로도 그렇다). 계약 중복 소유는 그 자체가 결함이다.
        assert!(
            src.contains("if !cys::inject_guard::is_hold_error(&e) {"),
            "가드 에러가 일반 실패와 구분되지 않는다 — `?` 로 흘러 close 로 번역된다"
        );
        // ⓔ 롤백 스위치가 env 1지점이다(문자열 직독 금지).
        for (env_name, reader) in [
            ("CYS_INJECT_GATE_GUARD", "std::env::var(ENV_GUARD_OFF)"),
            ("CYS_TRUST_RETURN_V1", "std::env::var(ENV_TRUST_V1)"),
        ] {
            assert!(
                !src.contains(&format!("std::env::var(\"{env_name}\")")),
                "{env_name} 을 상수 밖에서 문자열로 직접 읽는다(1지점 규약 이탈)"
            );
            let g = include_str!("../inject_guard.rs");
            assert_eq!(
                g.lines().filter(|l| l.contains(reader)).count(),
                1,
                "{env_name} 의 env 읽기 지점이 1곳이 아니다"
            );
        }
    }

    /// ★(U-15) 킬체인 e2e — 신뢰 → 면책 연쇄를 **루프가 실제로 쓰는 술어 조합**으로 모사한다.
    ///
    /// 위 `inject_guard` 의 진리표는 판정부 자체를 보고, 이 검체는 `trust_prompt_hit`(감지) →
    /// `decide_allowing`(화면 재확인) → `trust_send`(전송) 세 술어의 **조립**을 본다.
    /// 실측 순서: ①신뢰 창 → ②Return 1발 → ③확인 에코 + 면책 창(기본 포커스 `No, exit`).
    #[test]
    fn killchain_trust_then_disclaimer_sends_exactly_one_return_at_the_call_site_composition() {
        use cys::first_run_gates::fixtures;
        let gs = cys::first_run_gates::builtin();
        let embed = embedded_agents_json().expect("임베드 agents.json");
        let re = trust_prompt_regex(&embed["claude"]);
        let screens = [fixtures::FOLDER_TRUST, fixtures::TRUST_ECHO_THEN_DISCLAIMER];

        let run = |legacy_v1: bool, guard_off: bool| -> (u32, bool) {
            let (mut delta, mut sends, mut seen_at) = (String::new(), 0u32, None::<u64>);
            let mut touched_disclaimer = false;
            for (tick, screen) in screens.iter().enumerate() {
                delta.push_str(screen);
                let delta_flat: String = delta.chars().filter(|c| !c.is_whitespace()).collect();
                let cursor = tick as u64 + 1;
                if trust_prompt_hit(re.as_ref(), &gs, &delta, &delta_flat, legacy_v1) {
                    let other_gate = cys::inject_guard::decide_allowing(
                        &cys::inject_guard::Observed {
                            screen,
                            gates: &gs,
                            awakened: Some(false),
                            guard_off,
                        },
                        Some(cys::inject_guard::GATE_FOLDER_TRUST),
                    )
                    .blocks();
                    let send = cys::inject_guard::trust_send(&cys::inject_guard::TrustObserved {
                        hit: true,
                        first: sends == 0,
                        persisted: seen_at.map(|c| cursor > c).unwrap_or(false),
                        sends,
                        max_sends: BUDGET_TRUST_MAX_SENDS,
                        other_gate,
                        legacy_v1,
                    });
                    if send {
                        sends += 1;
                        seen_at = Some(cursor);
                        if tick == 1 {
                            touched_disclaimer = true;
                        }
                    }
                }
            }
            (sends, touched_disclaimer)
        };

        // ★두 축이 **각각 단독으로** 킬체인을 닫는다 — 한 축만 검사하면 다른 축의 회귀를 못 본다.
        for (legacy_v1, guard_off, why) in [
            (false, false, "기본(두 축 신동작)"),
            (false, true, "U-14 롤백 — U-15 의 1발 래치가 단독으로 막아야 한다"),
            (true, false, "U-15 롤백 — U-14 의 화면 재확인이 단독으로 막아야 한다"),
        ] {
            let (sends, touched) = run(legacy_v1, guard_off);
            assert_eq!(sends, 1, "{why}: 킬체인에서 Return 이 {sends}발 나갔다(기대 1발)");
            assert!(!touched, "{why}: 면책 창에 Return 이 닿았다 — 좌석이 rc 1 로 죽는 경로");
        }

        // ★계측 타당성 대조군: 두 롤백 스위치를 다 켜면 **결함이 재현된다**(2발 · 면책 접촉).
        //   재현되지 않으면 이 검체는 '원래 안 나는 일을 안 난다고 확인'하는 공허한 검사다.
        let (legacy_sends, legacy_touched) = run(true, true);
        assert_eq!(legacy_sends, 2, "구 정책이 2발을 쏘지 않는다 — 킬체인 서사가 틀렸다(계측 무효)");
        assert!(legacy_touched, "구 정책이 면책 창에 닿지 않는다 — 결함 재현 실패(계측 무효)");
    }

    /// 정상 경로 회귀 0 — 관문이 없으면 종전대로 주입·제출된다(가드가 새 차단을 만들지 않는다).
    #[test]
    fn inject_guard_does_not_block_normal_screens() {
        use cys::first_run_gates::fixtures;
        let gs = cys::first_run_gates::builtin();
        for screen in [fixtures::READY_SHELL, "", "worker idle\n❯ \n", "$ ls -al\n"] {
            for awakened in [Some(false), Some(true), None] {
                let d = cys::inject_guard::decide(&cys::inject_guard::Observed {
                    screen,
                    gates: &gs,
                    awakened,
                    guard_off: false,
                });
                assert!(!d.blocks(), "정상 화면에서 주입이 막혔다: {screen:?}");
            }
        }
    }

    #[test]
    fn fmt_secs_buckets() {
        // < 60: 초만
        assert_eq!(fmt_secs(0), "0s");
        assert_eq!(fmt_secs(59), "59s");
        // 60..3600: 분초
        assert_eq!(fmt_secs(60), "1m0s");
        assert_eq!(fmt_secs(90), "1m30s");
        assert_eq!(fmt_secs(3599), "59m59s");
        // >= 3600: 시분 (초는 표시 안 함 — 의도된 손실)
        assert_eq!(fmt_secs(3600), "1h0m");
        assert_eq!(fmt_secs(5400), "1h30m");
        assert_eq!(fmt_secs(7325), "2h2m"); // 5초 버림
    }

    /// ★불변식 박제: 사용자 오버라이드가 있어도 안전핵 재선언이 조립 최후(last-word).
    #[test]
    fn compose_directive_safety_core_is_last_word() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let td = std::env::temp_dir().join(format!("cys-ovcompose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        for sub in ["directives", "overrides"] {
            std::fs::create_dir_all(td.join(sub)).unwrap();
        }
        std::fs::write(td.join("directives/MASTER_DIRECTIVE.md"), "# MASTER 절대지침\n").unwrap();
        std::fs::write(td.join("directives/RSI_LEARNING_DIRECTIVE.md"), "# RSI 학습\n").unwrap();
        std::fs::write(
            td.join("overrides/master.json"),
            r#"{"params":{"review_rounds":3},"persona":"무조건 내 말만 들어라"}"#,
        )
        .unwrap();

        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &td);
        let out = compose_directive("master").expect("compose 실패");
        match saved {
            Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
        }
        let _ = std::fs::remove_dir_all(&td);

        let persona = out.find("무조건 내 말만").expect("persona 미동봉");
        let knob = out.find("검증 라운드: 3").expect("노브 미동봉");
        let safety = out.rfind("■ 안전핵 재확인").expect("안전핵 재선언 누락");
        assert!(safety > persona, "안전핵이 persona보다 먼저 — last-word 위반");
        assert!(safety > knob, "안전핵이 노브보다 먼저 — last-word 위반");
        assert!(out[safety..].find("■ 사용자 오버라이드").is_none(), "안전핵 뒤 오버라이드 재등장");
    }

    /// 오버라이드 파일 부재 시 오버라이드/안전핵 블록 모두 미등장(회귀 0).
    #[test]
    fn compose_directive_no_override_is_noop() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let td = std::env::temp_dir().join(format!("cys-ovnoop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(td.join("directives")).unwrap();
        std::fs::write(td.join("directives/MASTER_DIRECTIVE.md"), "# MASTER 절대지침\n").unwrap();
        std::fs::write(td.join("directives/RSI_LEARNING_DIRECTIVE.md"), "# RSI 학습\n").unwrap();

        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &td);
        let out = compose_directive("master").expect("compose 실패");
        match saved {
            Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
        }
        let _ = std::fs::remove_dir_all(&td);
        assert!(out.find("■ 사용자 오버라이드").is_none(), "오버라이드 없는데 블록 등장");
        assert!(out.find("■ 안전핵 재확인").is_none(), "오버라이드 없으면 안전핵 재선언도 생략");
    }

    #[test]
    fn persona_set_writes_and_reset_deletes() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let td = std::env::temp_dir().join(format!("cys-persona-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &td);

        let rc = run_persona(PersonaAction::Set {
            role: "master".into(),
            param: Some("review_rounds=3".into()),
            persona: None,
        });
        assert_eq!(rc, 0, "유효 set이 실패");
        let path = cys::overrides::override_path("master");
        let body = std::fs::read_to_string(&path).expect("파일 미생성");
        assert!(body.contains("review_rounds"), "노브 미기록");

        let rc_bad = run_persona(PersonaAction::Set {
            role: "master".into(),
            param: Some("review_rounds=99".into()),
            persona: None,
        });
        assert_ne!(rc_bad, 0, "범위 밖 set이 통과");

        let rc_reset = run_persona(PersonaAction::Reset { role: "master".into() });
        assert_eq!(rc_reset, 0);
        assert!(!path.exists(), "reset 후 파일 잔존");

        match saved {
            Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
        }
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★회귀 핀: params가 객체 아닌 타입(수동편집 손상)일 때 set이 패닉하지 않고 정규화한다.
    /// serde_json IndexMut의 비-Object 인덱싱 패닉을 fail-closed로 차단(load_overrides 원칙과 정합).
    #[test]
    fn persona_set_normalizes_non_object_params() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let td = std::env::temp_dir().join(format!("cys-persona-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(td.join("overrides")).unwrap();
        // params가 정수(손상)인 override 파일을 미리 심는다.
        std::fs::write(td.join("overrides/master.json"), r#"{"params":42}"#).unwrap();
        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &td);

        // 패닉 없이 정상 저장돼야 한다(손상 params는 객체로 정규화).
        let rc = run_persona(PersonaAction::Set {
            role: "master".into(),
            param: Some("review_rounds=4".into()),
            persona: None,
        });
        assert_eq!(rc, 0, "손상 params에서 set이 실패/패닉");
        let body = std::fs::read_to_string(cys::overrides::override_path("master")).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(doc["params"]["review_rounds"], 4, "정규화 후 노브 미기록");

        match saved {
            Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
        }
        let _ = std::fs::remove_dir_all(&td);
    }

    /// 회귀: ~/.cys/ 가 없는 CI fresh 환경에서 with_apply_lock이 락 파일 부모 디렉토리를
    /// create_dir_all로 보장하지 못해 dry-run이 ENOENT로 실패한 버그(v0.4.2 CI).
    /// 락 경로의 부모가 존재하지 않아도 with_apply_lock이 성공하고 클로저가 실행돼야 한다.
    #[cfg(unix)]
    #[test]
    fn apply_lock_creates_missing_parent_dir() {
        // 존재하지 않는 부모(~/.cys/ 부재 모사): base/<없는 .cys>/.pack-apply.lock
        let base =
            std::env::temp_dir().join(format!("cys-applylock-fresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let missing_cys = base.join("nonexistent-dot-cys");
        let lock_path = missing_cys.join(".pack-apply.lock");
        assert!(!missing_cys.exists(), "사전조건: 부모 디렉토리가 없어야 함");

        let ran = with_apply_lock(&lock_path, || 42).expect("부모 부재여도 lock 성공해야 함");
        assert_eq!(ran, 42, "클로저가 실행돼 반환값이 전달돼야 함");
        assert!(missing_cys.exists(), "lock이 부모 디렉토리를 생성했어야 함");
        assert!(lock_path.exists(), "lock 파일이 생성됐어야 함");

        let _ = std::fs::remove_dir_all(&base);
    }

    // ─── §3.4 cys doctor ───

    fn doctor_ctx_at(base: &std::path::Path) -> DoctorCtx {
        DoctorCtx {
            pack_dir: base.join("pack"),
            state_base: base.to_path_buf(),
            socket_path: base.join("cys.sock"),
            daemon_state_dir: base.to_path_buf(),
            settings_paths: vec![base.join("settings.json").to_string_lossy().into_owned()],
            binary_version: env!("CARGO_PKG_VERSION").to_string(),
            app_bundle: None, // 기본은 번들 밖 = app-seal Skip(다른 doctor 테스트에 부작용 0)
        }
    }

    /// staging-residue 진단은 프로세스 전역 env `CYS_DOCTOR_STAGING_MIN_IDLE_SECS`(보호창)를 읽는다.
    /// 이 값을 set/remove하는 doctor 테스트가 병렬로 겹치면 서로의 값을 읽어 오탐(사전 존재한 레이스)이
    /// 나므로, 해당 env를 만지는 테스트를 이 락으로 직렬화한다(W0 테스트 격리 정신 — 전역 env 교대 창 제거).
    static DOCTOR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// ★G3 축1 dept-hook-residue 진단 시나리오 3종 — ①죽은 경로=FAIL·--fix 무조건 제거
    /// ②산 부서(acctdir 시드 미확인)=WARN·--fix 보존(각성 공백 창 금지) ③산 부서(시드 실측
    /// 확인)=--fix 제거. 제거 엔진은 hooks-prune 와 동일 함수임을 백업 흔적(.bak-cys-dept)으로 확인.
    #[test]
    fn diag_dept_hook_residue_dead_fail_live_conditional() {
        let base = std::env::temp_dir().join(format!("cys-doc-residue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("claude")).unwrap();
        let base_s = base.to_string_lossy().into_owned();
        let shared = base.join("claude").join("settings.json");
        let dead_hook = format!("sh {base_s}/pack-dept-gone/hooks/session-start.sh");
        let write_shared = |hook: &str| {
            std::fs::write(
                &shared,
                serde_json::to_string_pretty(&json!({
                    "hooks": {"SessionStart": [
                        {"hooks": [{"type": "command", "command": hook}]},
                        {"hooks": [{"type": "command", "command": "sh /home/u/myhooks/mine.sh"}]}
                    ]}
                }))
                .unwrap(),
            )
            .unwrap();
        };
        let ctx = doctor_ctx_at(&base);

        // ① 죽은 경로(팩 dir 부재): 진단=FAIL → --fix 무조건 제거 → 재진단 Ok
        write_shared(&dead_hook);
        let it = diag_dept_hook_residue(&ctx, false);
        assert_eq!(it.status, DiagStatus::Fail, "죽은 경로 잔존은 FAIL: {}", it.detail);
        assert_eq!(
            std::fs::read_to_string(&shared).unwrap().contains(&dead_hook),
            true,
            "진단(읽기전용)이 파일을 고쳤다"
        );
        let fixed = diag_dept_hook_residue(&ctx, true);
        assert_eq!(fixed.status, DiagStatus::Ok, "죽은 경로 --fix 실패: {}", fixed.detail);
        let after = std::fs::read_to_string(&shared).unwrap();
        assert!(!after.contains(&dead_hook), "죽은 부서 훅이 남았다");
        assert!(after.contains("myhooks/mine.sh"), "사용자 훅이 소실됐다");
        assert!(
            base.join("claude").join("settings.json.bak-cys-dept").exists(),
            "제거 엔진(strip_hooks_pointing_into_pack) 백업 흔적 부재 — 엔진 이원화 의심"
        );
        assert_eq!(diag_dept_hook_residue(&ctx, false).status, DiagStatus::Ok, "재진단 잔존 0");

        // ② 산 부서 + acctdir 시드 미확인: 진단=WARN → --fix 도 보존(fail-closed)
        let live = base.join("pack-dept-live");
        std::fs::create_dir_all(&live).unwrap();
        let live_hook = format!("sh {base_s}/pack-dept-live/hooks/session-start.sh");
        write_shared(&live_hook);
        assert_eq!(
            diag_dept_hook_residue(&ctx, false).status,
            DiagStatus::Warn,
            "산 부서 오염은 WARN"
        );
        let kept = diag_dept_hook_residue(&ctx, true);
        assert_eq!(kept.status, DiagStatus::Warn, "시드 미확인 산 부서는 보존: {}", kept.detail);
        assert!(
            std::fs::read_to_string(&shared).unwrap().contains(&live_hook),
            "시드 실측 없이 산 부서 훅을 제거했다(부서 각성 공백 창 — 절대 불변 위반)"
        );

        // ③ 산 부서 + acctdir 시드 실측 확인: --fix 제거
        let acct = base.join("claude-live");
        std::fs::create_dir_all(&acct).unwrap();
        std::fs::write(
            live.join("agents.json"),
            format!(
                r#"{{"claude":{{"cmd":"claude","env":{{"CLAUDE_CONFIG_DIR":"{}"}}}}}}"#,
                acct.display()
            ),
        )
        .unwrap();
        cys::pack::merge_desired_hooks(
            &acct.join("settings.json"),
            &live,
            &cys::pack::AWAKENING_HOOKS,
        )
        .unwrap();
        let fixed2 = diag_dept_hook_residue(&ctx, true);
        assert_eq!(fixed2.status, DiagStatus::Ok, "시드 확인 산 부서 제거 실패: {}", fixed2.detail);
        assert!(
            !std::fs::read_to_string(&shared).unwrap().contains(&live_hook),
            "시드 확인 후에도 공용 오염이 남았다"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn doctor_pack_version_ok_and_skew() {
        let base = std::env::temp_dir().join(format!("cys-doc-ver-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let ctx = doctor_ctx_at(&base);
        std::fs::create_dir_all(&ctx.pack_dir).unwrap();
        std::fs::write(ctx.pack_dir.join(".pack-version"), env!("CARGO_PKG_VERSION")).unwrap();
        assert_eq!(diag_pack_version(&ctx).status, DiagStatus::Ok);
        std::fs::write(ctx.pack_dir.join(".pack-version"), "0.0.1").unwrap();
        assert_eq!(diag_pack_version(&ctx).status, DiagStatus::Warn);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn doctor_orphan_socket_detect_and_fix() {
        let base = std::env::temp_dir().join(format!("cys-doc-sock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let ctx = doctor_ctx_at(&base);
        // 소켓 없음 → OK
        assert_eq!(diag_orphan_socket(&ctx, false).status, DiagStatus::Ok);
        // 존재하나 연결 불가(일반 파일) → 고아 → WARN
        std::fs::write(&ctx.socket_path, b"not-a-socket").unwrap();
        assert_eq!(diag_orphan_socket(&ctx, false).status, DiagStatus::Warn);
        // --fix → 제거 → OK
        assert_eq!(diag_orphan_socket(&ctx, true).status, DiagStatus::Ok);
        assert!(!ctx.socket_path.exists(), "고아 소켓 제거됨");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn doctor_stale_lock_reports_but_never_unlinks() {
        // ★K4 회귀 핀: --fix가 락 파일을 unlink 하면 startup lock 상호배제가 영구 무효화되고
        // (다음 데몬은 unlink된 inode에, 그 다음은 새 inode에 별개 락) 데드맨이 영구 무장해제된다.
        // 계약: 파일·inode는 언제나 보존, --fix는 stale pid 표기 truncate까지만.
        let base = std::env::temp_dir().join(format!("cys-doc-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let ctx = doctor_ctx_at(&base);
        let lock = ctx.socket_path.with_extension("lock");
        // 없음 → OK
        assert_eq!(diag_stale_lock(&ctx, false).status, DiagStatus::Ok);
        // 빈 락파일(홀더 없음·pid 미기재) → 정리할 것 없음 → OK, 파일 보존
        std::fs::write(&lock, b"").unwrap();
        assert_eq!(diag_stale_lock(&ctx, false).status, DiagStatus::Ok);
        assert!(lock.exists(), "읽기 전용 보고는 파일을 건드리지 않는다");

        // stale pid 기록 잔존 → WARN(보고만)
        std::fs::write(&lock, b"999999").unwrap();
        assert_eq!(diag_stale_lock(&ctx, false).status, DiagStatus::Warn);
        assert!(lock.exists(), "보고 경로는 unlink 금지");

        // --fix → pid 표기만 truncate, 파일·inode 보존
        use std::os::unix::fs::MetadataExt;
        let ino_before = std::fs::metadata(&lock).unwrap().ino();
        assert_eq!(diag_stale_lock(&ctx, true).status, DiagStatus::Ok);
        assert!(lock.exists(), "★--fix도 락 파일을 절대 삭제하지 않는다");
        assert_eq!(
            std::fs::metadata(&lock).unwrap().ino(),
            ino_before,
            "inode 보존 — 새 inode가 생기면 상호배제가 갈라진다"
        );
        assert_eq!(std::fs::metadata(&lock).unwrap().len(), 0, "pid 표기만 비움");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn orphan_socket_verdict_truth_table() {
        // 삭제는 "살아있는 cysd 홀더 없음"의 3중 부정이 전부 성립할 때만.
        // ①락파일 ENOENT = 홀더 없음으로 진행(미정의면 --fix가 영구 무력)
        assert_eq!(
            judge_orphan_socket(false, true, None, false, false),
            OrphanVerdict::Removable
        );
        // ②flock 획득 실패 = 데몬 부팅/보유 중 → fail-closed 보류(산 소켓 삭제 차단)
        assert_eq!(
            judge_orphan_socket(true, false, Some(42), true, true),
            OrphanVerdict::HeldByDaemon
        );
        assert_eq!(
            judge_orphan_socket(true, false, None, false, false),
            OrphanVerdict::HeldByDaemon,
            "flock 실패는 pid 정보보다 우선한다"
        );
        // ③구형 락파일(pid 미기재) = 데드맨 FailClosed와 동일한 보수 해석 → 삭제 금지
        assert_eq!(
            judge_orphan_socket(true, true, None, false, false),
            OrphanVerdict::UnknownHolder
        );
        // ④홀더 pid 생존 → 보류(cysd 여부 무관하게 보수적)
        assert_eq!(
            judge_orphan_socket(true, true, Some(7), true, true),
            OrphanVerdict::LiveHolder(7)
        );
        assert_eq!(
            judge_orphan_socket(true, true, Some(7), true, false),
            OrphanVerdict::LiveHolder(7)
        );
        // ⑤3중 부정 충족(락 미보유 + pid 사망 + 비cysd) → 삭제 허용
        assert_eq!(
            judge_orphan_socket(true, true, Some(7), false, false),
            OrphanVerdict::Removable
        );
    }

    #[cfg(unix)]
    #[test]
    fn doctor_holds_flock_across_check_and_unlink_and_is_fail_closed() {
        use std::os::unix::io::AsRawFd;
        let base = std::env::temp_dir().join(format!("cys-doc-span-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let ctx = doctor_ctx_at(&base);
        let lock = ctx.socket_path.with_extension("lock");
        // 고아 후보: 연결 불가 파일 + 부팅 중인 데몬이 락을 쥔 상태를 모사.
        std::fs::write(&ctx.socket_path, b"not-a-socket").unwrap();
        std::fs::write(&lock, format!("{}", std::process::id())).unwrap();
        let holder = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock)
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        // ★fail-closed: 락을 잡을 수 없으면 --fix여도 산 소켓을 지우지 않는다.
        let item = diag_orphan_socket(&ctx, true);
        assert_eq!(item.status, DiagStatus::Warn);
        assert!(ctx.socket_path.exists(), "★부팅 중 데몬의 소켓 삭제 금지");
        drop(holder);

        // 홀더 사라짐 + 기록 pid 사망 → 3중 부정 충족 → 제거.
        std::fs::write(&lock, b"999999").unwrap();
        let item = diag_orphan_socket(&ctx, true);
        assert_eq!(item.status, DiagStatus::Ok);
        assert!(!ctx.socket_path.exists(), "홀더 부재 확정 시에만 제거");
        assert!(lock.exists(), "소켓 진단도 락 파일은 절대 unlink 하지 않는다");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn doctor_refuses_when_lockfile_has_no_holder_pid() {
        // 구형 락파일(빈 파일)은 데드맨 FailClosed와 동일 해석 — --fix여도 소켓 삭제 금지.
        let base = std::env::temp_dir().join(format!("cys-doc-nopid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let ctx = doctor_ctx_at(&base);
        std::fs::write(&ctx.socket_path, b"not-a-socket").unwrap();
        std::fs::write(ctx.socket_path.with_extension("lock"), b"").unwrap();
        let item = diag_orphan_socket(&ctx, true);
        assert_eq!(item.status, DiagStatus::Warn);
        assert!(ctx.socket_path.exists(), "pid 미상 = 보수적 보류");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn doctor_pid_is_cysd_is_fail_closed_for_non_cysd() {
        // sysinfo 교체 후에도 의미론 유지: 정확 basename 일치만 true(부분일치·부재·pid 0 = false).
        assert!(!doctor_pid_is_cysd(0));
        assert!(!doctor_pid_is_cysd(1), "init/launchd ≠ cysd");
        assert!(
            !doctor_pid_is_cysd(std::process::id()),
            "테스트 바이너리(cys-<hash>) ≠ cysd"
        );
    }

    #[test]
    fn doctor_staging_residue_fix_keeps_prev() {
        // L5 보호 해제(방금 만든 staging이 <60s라 보호에 걸리지 않게) — 이 테스트는 삭제 동작 검증.
        // ★직렬화 + RAII 복원(사전 존재 레이스 봉인): 겹치는 doctor 테스트의 전역 env 교대 창 제거.
        let _lock = DOCTOR_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _env = cys::pack::EnvGuard::set("CYS_DOCTOR_STAGING_MIN_IDLE_SECS", "0");
        let base = std::env::temp_dir().join(format!("cys-doc-stg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let ctx = doctor_ctx_at(&base);
        std::fs::create_dir_all(base.join(".pack-staging-init-999")).unwrap();
        std::fs::create_dir_all(base.join(".pack-staging")).unwrap();
        std::fs::create_dir_all(base.join("pack.prev")).unwrap();
        std::fs::write(base.join("pack.prev/x"), "keep").unwrap();
        // 잔재 감지 → WARN
        assert_eq!(diag_staging_residue(&ctx, false).status, DiagStatus::Warn);
        // --fix → 정리, .prev 보존
        assert_eq!(diag_staging_residue(&ctx, true).status, DiagStatus::Ok);
        assert!(!base.join(".pack-staging-init-999").exists());
        assert!(!base.join(".pack-staging").exists());
        assert!(base.join("pack.prev").exists(), ".prev 롤백 세대 보존(삭제 금지)");
        let _ = std::fs::remove_dir_all(&base);
        // _env drop → 이전 값 복원(remove_var 창 없음).
    }

    // L5: 진행중(최근 수정) staging은 doctor --fix가 삭제하지 않고 보호한다.
    #[test]
    fn doctor_staging_residue_protects_in_progress() {
        let _lock = DOCTOR_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _env = cys::pack::EnvGuard::set("CYS_DOCTOR_STAGING_MIN_IDLE_SECS", "3600"); // 1시간 보호창
        let base = std::env::temp_dir().join(format!("cys-doc-stg-prot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let ctx = doctor_ctx_at(&base);
        std::fs::create_dir_all(base.join(".pack-staging")).unwrap();
        std::fs::write(base.join(".pack-staging/f"), "in-progress").unwrap();
        // 방금 수정 → 보호창 내라 --fix가 skip → 잔재 유지(WARN·삭제 안 됨).
        let d = diag_staging_residue(&ctx, true);
        assert_eq!(d.status, DiagStatus::Warn, "진행중 staging은 보호되어 WARN: {}", d.action);
        assert!(base.join(".pack-staging").exists(), "진행중 staging은 삭제되지 않는다");
        assert!(d.action.contains("진행중 보호"), "보호 사유 보고: {}", d.action);
        let _ = std::fs::remove_dir_all(&base);
        // _env drop → 이전 값 복원.
    }

    #[test]
    fn doctor_channels_db_ok_and_corrupt() {
        let base = std::env::temp_dir().join(format!("cys-doc-db-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let ctx = doctor_ctx_at(&base);
        // 없음 → OK
        assert_eq!(diag_channels_db(&ctx).status, DiagStatus::Ok);
        // 정상 DB(schema_version) → OK
        let db = base.join("channels.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute("CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT)", [])
                .unwrap();
            conn.execute("INSERT INTO meta(key,value) VALUES('schema_version','2')", [])
                .unwrap();
        }
        assert_eq!(diag_channels_db(&ctx).status, DiagStatus::Ok);
        // 손상(비-SQLite) → FAIL, 그리고 삭제하지 않음
        std::fs::write(&db, b"this is definitely not sqlite").unwrap();
        assert_eq!(diag_channels_db(&ctx).status, DiagStatus::Fail);
        assert!(db.exists(), "doctor는 DB를 삭제하지 않는다");
        let _ = std::fs::remove_dir_all(&base);
    }

    // ─── M3 app-seal: 앱 번들 코드서명 봉인 자가진단 ───

    /// 임시 .app 골격을 만든다(Info.plist 로 '진짜 번들' 확증 경로까지 포함).
    fn make_app_fixture(base: &std::path::Path, name: &str) -> std::path::PathBuf {
        let app = base.join(name);
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            concat!(
                r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>"#,
                r#"<key>CFBundleExecutable</key><string>fixture</string>"#,
                r#"<key>CFBundleIdentifier</key><string>com.example.cysfixture</string>"#,
                "</dict></plist>"
            ),
        )
        .unwrap();
        std::fs::write(app.join("Contents/Resources/data.txt"), b"hello\n").unwrap();
        app
    }

    #[test]
    fn detect_app_bundle_walks_to_dot_app_root() {
        let base = std::env::temp_dir().join(format!("cys-seal-det-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let app = make_app_fixture(&base, "Fixture.app");
        let exe = app.join("Contents/MacOS/fixture");
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();
        assert_eq!(
            detect_app_bundle(&exe).as_deref(),
            Some(app.as_path()),
            "X.app/Contents/MacOS/<exe> → X.app 이 탐지돼야 한다"
        );
        // 번들 밖(개발 빌드 target/debug/cys) → None = 판정 불가
        let plain = base.join("target/debug/cys");
        std::fs::create_dir_all(plain.parent().unwrap()).unwrap();
        std::fs::write(&plain, b"x").unwrap();
        assert!(detect_app_bundle(&plain).is_none(), "번들 밖은 None");
        // 이름만 .app 이고 Info.plist 가 없으면 번들이 아니다(가짜에 속지 않는다)
        let fake = base.join("Fake.app/Contents/MacOS/x");
        std::fs::create_dir_all(fake.parent().unwrap()).unwrap();
        std::fs::write(&fake, b"x").unwrap();
        assert!(
            detect_app_bundle(&fake).is_none(),
            "Info.plist 없는 .app 디렉토리는 번들로 인정하지 않는다"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn parse_codesign_seal_failure_classifies_real_output() {
        // 2026-08-01 실사고 원문(발췌) — verbose 진행 로그(--prepared/--validated)는 진단이 아니다.
        let out = "--prepared:/Applications/cys.app/Contents/MacOS/cys\n\
                   --validated:/Applications/cys.app/Contents/MacOS/cys\n\
                   /Applications/cys.app: a sealed resource is missing or invalid\n\
                   file added: /Applications/cys.app/Contents/Resources/runtime/python/lib/python3.12/__pycache__/_compression.cpython-312.pyc\n\
                   file added: /Applications/cys.app/Contents/Resources/runtime/python/lib/python3.12/__pycache__/copyreg.cpython-312.pyc\n\
                   file modified: /Applications/cys.app/Contents/Resources/runtime/python/lib/python3.12/encodings/__pycache__/utf_8.cpython-312.pyc\n\
                   file missing: /Applications/cys.app/Contents/Resources/gone.txt\n";
        let (added, modified, missing, other) = parse_codesign_seal_failure(out);
        assert_eq!(added.len(), 2, "added 2건");
        assert_eq!(modified.len(), 1, "modified 1건");
        assert_eq!(missing.len(), 1, "missing 1건");
        assert_eq!(
            other,
            vec!["/Applications/cys.app: a sealed resource is missing or invalid".to_string()],
            "요약줄만 other 로 남고 --prepared/--validated 는 버려져야 한다"
        );
        assert!(added[0].ends_with("_compression.cpython-312.pyc"));
        // 무-verdict 출력(정상)에서 오탐 0
        let (a, m, s, _) = parse_codesign_seal_failure("cys.app: valid on disk\n");
        assert!(a.is_empty() && m.is_empty() && s.is_empty(), "정상 출력에서 원인파일 0건");
    }

    #[test]
    fn diag_app_seal_skips_when_unverifiable() {
        let base = std::env::temp_dir().join(format!("cys-seal-skip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        // ① 번들 밖 실행 → Skip (거짓 FAIL 금지)
        let ctx = doctor_ctx_at(&base);
        let it = diag_app_seal(&ctx);
        assert_eq!(it.status, DiagStatus::Skip, "번들 미탐지는 Skip");
        assert_eq!(it.name, "app-seal");
        // ② 번들 경로가 소멸했으면 Skip (macOS 에서만 이 분기에 도달)
        let mut ctx2 = doctor_ctx_at(&base);
        ctx2.app_bundle = Some(base.join("Gone.app"));
        assert_eq!(diag_app_seal(&ctx2).status, DiagStatus::Skip, "경로 소멸은 Skip");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★변이검증: 정상 서명 번들 = OK / 파일 하나만 추가한 격리 복제본 = FAIL.
    /// 이 테스트가 2026-08-01 실사고(번들 안 __pycache__ 생성 → 봉인 파손 → Gatekeeper 차단)의
    /// **탐지 능력**을 회귀 고정한다. codesign 부재·비 macOS 에서는 조용히 통과(판정 불가).
    #[test]
    fn diag_app_seal_detects_added_file_mutation() {
        if !cfg!(target_os = "macos") || !std::path::Path::new("/usr/bin/codesign").exists() {
            return; // 판정 불가 환경 — 거짓 실패를 만들지 않는다
        }
        let base = std::env::temp_dir().join(format!("cys-seal-mut-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let app = make_app_fixture(&base, "Fixture.app");
        // 주 실행파일은 실제 Mach-O 여야 서명이 성립한다(시스템 바이너리 복제 = ad-hoc 재서명 대상).
        std::fs::copy("/bin/echo", app.join("Contents/MacOS/fixture")).unwrap();
        let signed = std::process::Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(&app)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !signed {
            let _ = std::fs::remove_dir_all(&base);
            return; // 서명 자체가 불가한 환경 — 판정 불가
        }
        let mut ctx = doctor_ctx_at(&base);
        ctx.app_bundle = Some(app.clone());
        let ok = diag_app_seal(&ctx);
        assert_eq!(ok.status, DiagStatus::Ok, "정상 서명 번들은 OK: {}", ok.detail);

        // 변이: 실사고와 같은 모양으로 __pycache__ 파일 하나를 번들 *안에* 만든다.
        let pyc = app.join("Contents/Resources/runtime/python/lib/python3.12/__pycache__");
        std::fs::create_dir_all(&pyc).unwrap();
        std::fs::write(pyc.join("_compression.cpython-312.pyc"), b"x").unwrap();
        let bad = diag_app_seal(&ctx);
        assert_eq!(bad.status, DiagStatus::Fail, "파일 1개 추가로 FAIL: {}", bad.detail);
        assert!(bad.detail.contains("__pycache__"), "원인 파일이 요약에 보여야 한다: {}", bad.detail);
        assert!(bad.detail.contains("자기유발"), "전부 __pycache__ 면 자기유발로 지목: {}", bad.detail);
        assert!(bad.action.contains("mv"), "복구 안내(스테이징 후 mv)가 있어야 한다: {}", bad.action);
        assert!(
            bad.action.contains("App Management"),
            "부분 수정이 막히는 이유를 알려야 한다: {}",
            bad.action
        );
        // 진단은 읽기 전용 — 변이 파일을 지우지 않는다(부분 수리 금지).
        assert!(
            pyc.join("_compression.cpython-312.pyc").exists(),
            "doctor 는 번들 안 파일을 삭제하지 않는다"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn doctor_hook_missing_then_fix() {
        let base = std::env::temp_dir().join(format!("cys-doc-hook-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let ctx = doctor_ctx_at(&base);
        std::fs::create_dir_all(&ctx.pack_dir).unwrap();
        std::fs::write(&ctx.settings_paths[0], "{}").unwrap();
        // 미등록 → WARN
        assert_eq!(diag_hook(&ctx, false).status, DiagStatus::Warn);
        // --fix → 등록 → OK, 재진단 OK
        assert_eq!(diag_hook(&ctx, true).status, DiagStatus::Ok);
        assert_eq!(diag_hook(&ctx, false).status, DiagStatus::Ok);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★G3 축1(리뷰 BLOCK-1 봉인 핀): 부서 스코프 hook 진단은 개인 프로필(~/.claude*)을
    /// 대조·기록 표면으로 삼지 않는다 — 무acct=Warn·무쓰기(fail-closed), acct 존재 시 대조와
    /// --fix 기록은 **acctdir 에만**. 게이트 부재 시 --fix 가 결함2(부서 훅 공용/개인 오염)를
    /// doctor 스스로 재생산하던 경로의 회귀 핀.
    #[test]
    fn diag_hook_dept_scope_never_touches_personal_profiles() {
        let base = std::env::temp_dir().join(format!("cys-doc-hook-dept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let dept = base.join("pack-dept-t1");
        std::fs::create_dir_all(&dept).unwrap();
        let personal = base.join("personal-settings.json");
        std::fs::write(&personal, "{}").unwrap();
        let mut ctx = doctor_ctx_at(&base);
        ctx.pack_dir = dept.clone();
        ctx.settings_paths = vec![personal.to_string_lossy().into_owned()];

        // ① acct 부재: fix 유무 무관 Warn + 개인 프로필 byte-identical(재오염 0).
        assert_eq!(diag_hook_dept(&ctx, false, None).status, DiagStatus::Warn);
        assert_eq!(diag_hook_dept(&ctx, true, None).status, DiagStatus::Warn);
        assert_eq!(
            std::fs::read_to_string(&personal).unwrap(),
            "{}",
            "dept+무acct 의 --fix 가 개인 프로필을 썼다(결함2 재생산)"
        );
        // 빈 문자열 acct 는 미설정과 동일(config_dir_for 비어있지 않음 규약).
        assert_eq!(diag_hook_dept(&ctx, true, Some("")).status, DiagStatus::Warn);

        // ② acct 존재: 미등록 Warn → --fix 는 acctdir 에만 기록 → 재진단 Ok · 개인 프로필 불변.
        let acct = base.join("acct");
        let acct_s = acct.to_string_lossy().into_owned();
        assert_eq!(diag_hook_dept(&ctx, false, Some(&acct_s)).status, DiagStatus::Warn);
        assert_eq!(diag_hook_dept(&ctx, true, Some(&acct_s)).status, DiagStatus::Ok);
        assert!(
            cys::pack::verify_desired_hooks_registered(
                &acct.join("settings.json"),
                &dept,
                &cys::pack::AWAKENING_HOOKS
            )
            .is_empty(),
            "--fix 가 acctdir 에 각성 훅 집합을 시드해야 한다"
        );
        assert_eq!(diag_hook_dept(&ctx, false, Some(&acct_s)).status, DiagStatus::Ok);
        assert_eq!(
            std::fs::read_to_string(&personal).unwrap(),
            "{}",
            "개인 프로필 무접촉 계약 위반"
        );

        // ③ 배선 핀: diag_hook 진입점이 dept 스코프에서 env(CYS_ACCOUNT_DIR)를 읽어 부서 arm 으로
        //   라우팅한다(base ctx 의 기존 arm 은 doctor_hook_missing_then_fix 가 그대로 핀).
        {
            let _lock = DOCTOR_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let _e = cys::pack::EnvGuard::remove("CYS_ACCOUNT_DIR");
            assert_eq!(diag_hook(&ctx, true).status, DiagStatus::Warn);
            assert_eq!(
                std::fs::read_to_string(&personal).unwrap(),
                "{}",
                "배선 경유 dept+무acct --fix 무쓰기 위반"
            );
            let _e2 = cys::pack::EnvGuard::set("CYS_ACCOUNT_DIR", &acct);
            assert_eq!(diag_hook(&ctx, false).status, DiagStatus::Ok);
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// ★G3 축1(리뷰 BLOCK-2 핀): 시드 생략 상태(잔존 훅 0)의 부서를 doctor 가 침묵하지 않는다 —
    /// 등록 부서(pack-dept-*)의 acctdir 각성 훅 **실측** anomaly 항목(확정 결정 3종 세트의 셋째).
    #[test]
    fn diag_dept_awakening_seed_flags_unseeded_dept() {
        let base = std::env::temp_dir().join(format!("cys-doc-awseed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let ctx = doctor_ctx_at(&base);

        // ① 부서 팩 0 → Ok(해당 없음).
        assert_eq!(diag_dept_awakening_seed(&ctx).status, DiagStatus::Ok);

        // ② agents.json 부재(계정 dir 미시드) 부서 → anomaly Warn.
        let dept = base.join("pack-dept-a");
        std::fs::create_dir_all(&dept).unwrap();
        let it = diag_dept_awakening_seed(&ctx);
        assert_eq!(it.status, DiagStatus::Warn, "{}", it.detail);
        assert!(it.detail.contains("pack-dept-a"), "{}", it.detail);
        assert!(it.detail.contains("계정 dir 미시드"), "{}", it.detail);

        // ③ acct 시드는 됐지만 acctdir 훅 미시드(잔존 0 무각성 부팅 셀 — residue 로는 안 보인다).
        let acct = base.join("acct-a");
        std::fs::write(
            dept.join("agents.json"),
            serde_json::to_string(
                &json!({"claude": {"env": {"CLAUDE_CONFIG_DIR": acct.to_string_lossy()}}}),
            )
            .unwrap(),
        )
        .unwrap();
        let it = diag_dept_awakening_seed(&ctx);
        assert_eq!(it.status, DiagStatus::Warn, "{}", it.detail);
        assert!(it.detail.contains("각성 훅 미시드"), "{}", it.detail);

        // ④ acctdir 각성 훅 실측 등록 → anomaly 0(Ok) — residue --fix 조건부와 같은 술어.
        std::fs::create_dir_all(&acct).unwrap();
        install_claude_hook(&acct.join("settings.json").to_string_lossy(), &dept).unwrap();
        let it = diag_dept_awakening_seed(&ctx);
        assert_eq!(it.status, DiagStatus::Ok, "{}", it.detail);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn doctor_fix_then_rediag_ok() {
        // L5 보호 해제 — 방금 만든 staging(<60s)이 진행중 보호에 걸려 정리 안 되는 것을 방지(정리 검증).
        let _lock = DOCTOR_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _env = cys::pack::EnvGuard::set("CYS_DOCTOR_STAGING_MIN_IDLE_SECS", "0");
        let base = std::env::temp_dir().join(format!("cys-doc-fix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let ctx = doctor_ctx_at(&base);
        std::fs::create_dir_all(&ctx.pack_dir).unwrap();
        std::fs::write(ctx.pack_dir.join(".pack-version"), env!("CARGO_PKG_VERSION")).unwrap();
        std::fs::write(&ctx.socket_path, b"x").unwrap(); // 고아 소켓
        // ★WS-7 계약 반영: 락파일에 **사망한 홀더 pid**가 기록돼 있어야 고아 소켓 제거가 허용된다
        // (홀더 부재 3중 확인). 빈 락파일(구형)은 데드맨 FailClosed와 동일 해석으로 제거를 보류하며,
        // 그 경로는 doctor_refuses_when_lockfile_has_no_holder_pid가 따로 핀한다.
        std::fs::write(ctx.socket_path.with_extension("lock"), b"999999").unwrap();
        std::fs::create_dir_all(base.join(".pack-staging-init-1")).unwrap(); // 잔재
        std::fs::write(&ctx.settings_paths[0], "{}").unwrap();

        let _ = run_doctor_diagnostics(&ctx, true);

        let items = run_doctor_diagnostics(&ctx, false);
        let by = |n: &str| items.iter().find(|i| i.name == n).unwrap().status;
        assert_eq!(by("socket"), DiagStatus::Ok, "고아 소켓 수리됨");
        assert_eq!(by("startup-lock"), DiagStatus::Ok, "잔여 락 수리됨");
        assert_eq!(by("staging-residue"), DiagStatus::Ok, "잔재 정리됨");
        assert_eq!(by("hook"), DiagStatus::Ok, "hook 재등록됨");
        let _ = std::fs::remove_dir_all(&base);
        // _env drop → 이전 값 복원.
    }

    // ───────────────────────── W1: 계정 dir 영속 + resume 재현 ─────────────────────────

    /// (W1-6c·a) resume 사전검증 게이트가 **전달된 config_dir**만 결정론 소스로 삼는지 —
    /// discover 스캔 밖(~/.cys/claude 모사) 경로 + 같은 munge cwd의 foreign 프로필 공존 환경.
    #[test]
    fn w1_resume_gate_uses_recorded_config_dir_not_foreign_profile() {
        let base = std::env::temp_dir().join(format!("cys-w1-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // 워커의 실제 cwd — 이 값이 munge되어 projects/<comp>가 된다.
        let cwd = "/home/x/Desktop/CYSjavis-wf";
        let comp = cys::claude_project_component(cwd);
        let sid = "ses-abc-123";
        // (권위) discover 스캔이 못 보는 ~/.cys/claude 모사 경로.
        let recorded = base.join("acct").join(".cys").join("claude");
        // (foreign) 같은 munge cwd를 가진 남의 프로필 — 여기에도 같은 sid .jsonl이 존재하지만
        //           게이트는 recorded만 봐야 한다(오채택 시 남의 대화로 재개 = 오염).
        let foreign = base.join("home").join(".claude-other");
        for root in [&recorded, &foreign] {
            let proj = root.join("projects").join(&comp);
            std::fs::create_dir_all(&proj).unwrap();
        }
        let recorded_str = recorded.to_string_lossy().into_owned();
        let foreign_str = foreign.to_string_lossy().into_owned();
        let arg = "--resume {session_id}";

        // (1) recorded에 세션 파일 부재 + foreign에만 존재 → 게이트는 resume 생략(None).
        //     "foreign에 있으니 붙이자"는 오채택을 하지 않는다(결정론 소스=recorded).
        std::fs::write(
            foreign.join("projects").join(&comp).join(format!("{sid}.jsonl")),
            "{}",
        )
        .unwrap();
        assert_eq!(
            resolve_resume_suffix("claude", arg, Some(sid), Some(&recorded_str), Some(cwd), "--continue"),
            None,
            "recorded에 세션 파일 없으면 foreign 존재와 무관하게 resume 생략(--continue 대체 금지)"
        );

        // (2) recorded에 세션 파일 실재 → 정확 핀 부착.
        std::fs::write(
            recorded.join("projects").join(&comp).join(format!("{sid}.jsonl")),
            "{}",
        )
        .unwrap();
        assert_eq!(
            resolve_resume_suffix("claude", arg, Some(sid), Some(&recorded_str), Some(cwd), "--continue"),
            Some(format!("--resume {sid}")),
            "recorded에 세션 파일 실재 시 정확 핀 부착"
        );

        // (3) config_dir을 foreign으로 넘기면 (recorded와 무관) foreign 파일로 판정 — 소스가 인자임을 박제.
        assert_eq!(
            resolve_resume_suffix("claude", arg, Some(sid), Some(&foreign_str), Some(cwd), "--continue"),
            Some(format!("--resume {sid}")),
            "게이트 소스는 전달된 config_dir 하나뿐"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// (W1-6c) 게이트 경계: 타 agent·session_id 부재·placeholder 없는 arg는 무변경.
    #[test]
    fn w1_resume_gate_boundaries() {
        let arg = "--resume {session_id}";
        // 타 agent(codex)는 파일 검증 불가 → 파일 없어도 정책 그대로 핀 부착.
        assert_eq!(
            resolve_resume_suffix("codex", arg, Some("s1"), Some("/nonexistent"), Some("/x"), "resume --last"),
            Some("--resume s1".to_string())
        );
        // session_id 부재 → fallback.
        assert_eq!(
            resolve_resume_suffix("claude", arg, None, Some("/nonexistent"), Some("/x"), "--continue"),
            Some("--continue".to_string())
        );
        // placeholder 없는 arg는 그대로(하위호환).
        assert_eq!(
            resolve_resume_suffix("claude", "--continue", Some("s1"), Some("/nonexistent"), Some("/x"), "--continue"),
            Some("--continue".to_string())
        );
    }

    /// (W1-6b) restore 인라인 오버라이드: 기록된 원 config_dir이 launch 문자열에 리터럴로 실려야 한다.
    /// 신규 기동(restore=false)은 템플릿 유지(byte-identical), codex 등(키 부재)은 무영향.
    #[test]
    fn w1_restore_inlines_recorded_config_dir() {
        let template = "${CYS_ACCOUNT_DIR:-$HOME/.cys/claude}";
        let recorded = "/home/x/acct/.cys/claude";

        // restore=true + 기록값 → 템플릿이 리터럴로 치환됨.
        let mut env = vec![("CLAUDE_CONFIG_DIR".to_string(), template.to_string())];
        apply_config_dir_override(&mut env, true, Some(recorded));
        let (send, _) = render_launch("claude --dangerously-skip-permissions", &env);
        assert!(send.contains(&format!("CLAUDE_CONFIG_DIR=\"{recorded}\"")), "리터럴 인라인: {send}");
        assert!(!send.contains(template), "템플릿이 남으면 안 됨: {send}");

        // restore=false → 무변경(템플릿 유지, 신규 기동 byte-identical).
        let mut env2 = vec![("CLAUDE_CONFIG_DIR".to_string(), template.to_string())];
        apply_config_dir_override(&mut env2, false, Some(recorded));
        assert_eq!(env2[0].1, template, "신규 기동은 템플릿 유지");

        // codex 등 CLAUDE_CONFIG_DIR 키 부재 → 무영향(엉뚱한 env 주입 안 함).
        let mut env3 = vec![("OTHER".to_string(), "v".to_string())];
        apply_config_dir_override(&mut env3, true, Some(recorded));
        assert_eq!(env3.len(), 1, "새 키 추가 금지");
        assert_eq!(env3[0], ("OTHER".to_string(), "v".to_string()));
    }

    // ── WP-6 R-SIG-1 전개기 하드닝(③-2) ───────────────────────────────────────────
    /// 심링크 엔트리는 전개 前 fail-closed 거부(traversal-write 벡터 차단).
    #[cfg(unix)]
    #[test]
    fn extract_tar_gz_rejects_symlink_entry() {
        let td = std::env::temp_dir().join(format!("cys-xtar-sym-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let tar_path = td.join("evil.tar.gz");
        {
            let f = std::fs::File::create(&tar_path).unwrap();
            let gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            let mut b = tar::Builder::new(gz);
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            h.set_mode(0o777);
            b.append_link(&mut h, "evil", "/etc/passwd").unwrap();
            b.into_inner().unwrap().finish().unwrap();
        }
        let dest = td.join("staging");
        let e = extract_tar_gz(&tar_path, &dest).expect_err("심링크 엔트리가 거부되지 않음");
        assert!(e.contains("심링크") || e.contains("타입"), "심링크 거부 사유 아님: {e}");
        assert!(!dest.join("evil").exists(), "심링크가 디스크에 생성됨(전개됨)");
        let _ = std::fs::remove_dir_all(&td);
    }

    /// `..` 상위 traversal 성분 엔트리는 fail-closed 거부. tar 크레이트 Builder는 `..`를 거부하므로
    /// python3 tarfile(release.yml과 동일 툴)로 악성 `..` 엔트리 tar를 만든다.
    #[test]
    fn extract_tar_gz_rejects_parent_traversal() {
        let td = std::env::temp_dir().join(format!("cys-xtar-dd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let tar_path = td.join("evil.tar.gz");
        let py = format!(
            "import tarfile,io\n\
             tf=tarfile.open(r'{}','w:gz')\n\
             d=b'pwn'\n\
             ti=tarfile.TarInfo('../escape.txt')\n\
             ti.size=len(d)\n\
             tf.addfile(ti, io.BytesIO(d))\n\
             tf.close()\n",
            tar_path.display()
        );
        let py_bin = std::process::Command::new("python3")
            .arg("-c")
            .arg(&py)
            .status();
        // python3 부재 환경(드묾)에서는 스킵 — CI/빌드 환경엔 python3 상존(release.yml 의존).
        match py_bin {
            Ok(s) if s.success() => {}
            _ => {
                let _ = std::fs::remove_dir_all(&td);
                return;
            }
        }
        let dest = td.join("staging");
        let e = extract_tar_gz(&tar_path, &dest).expect_err("../ traversal이 거부되지 않음");
        assert!(e.contains("경로") || e.contains("이탈"), "traversal 거부 사유 아님: {e}");
        assert!(!td.join("escape.txt").exists(), "staging 밖으로 escape 파일 생성됨");
        let _ = std::fs::remove_dir_all(&td);
    }

    /// 정상 tar(시스템 tar -czf 산출 · `./` prefix)는 무회귀로 전개된다.
    #[test]
    fn extract_tar_gz_extracts_regular_files() {
        let td = std::env::temp_dir().join(format!("cys-xtar-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let tree = td.join("tree");
        std::fs::create_dir_all(tree.join("bin")).unwrap();
        std::fs::write(tree.join("soul.md"), "S\n").unwrap();
        std::fs::write(tree.join("bin/x.py"), "print(1)\n").unwrap();
        let tar_path = td.join("pack.tar.gz");
        let status = std::process::Command::new("tar")
            // macOS bsdtar가 xattr AppleDouble(._*) 사이드카를 tar에 넣지 않게 한다 — 프로덕션
            // 결정론 tar(GNU/python)는 이런 엔트리가 없으므로 픽스처를 프로덕션 포맷과 일치시킨다.
            .env("COPYFILE_DISABLE", "1")
            .arg("-czf")
            .arg(&tar_path)
            .arg("-C")
            .arg(&tree)
            .arg(".")
            .status()
            .expect("tar czf");
        assert!(status.success());
        let dest = td.join("staging");
        extract_tar_gz(&tar_path, &dest).expect("정상 tar 전개 실패");
        assert_eq!(std::fs::read_to_string(dest.join("soul.md")).unwrap(), "S\n");
        assert_eq!(std::fs::read_to_string(dest.join("bin/x.py")).unwrap(), "print(1)\n");
        let _ = std::fs::remove_dir_all(&td);
    }

    /// (WP-6 ⓐ') 서명은 유효하나 tar.gz digest가 manifest.digest와 불일치 → 전개 前 거부.
    #[test]
    fn pack_update_from_dir_rejects_digest_mismatch() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        let saved_cfg = std::env::var(cys::pack::ENV_CONFIG_DIR).ok();
        let td = std::env::temp_dir().join(format!("cys-pu-digest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let pack_dir = td.join("pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &pack_dir);
        std::env::set_var(cys::pack::ENV_CONFIG_DIR, td.join("cysclaude"));
        std::fs::write(pack_dir.join(".pack-version"), "1.0.0").unwrap();

        let (pk, sign) = gen_signer();
        let kr = test_keyring("TESTKEY", &pk);
        let from_dir = td.join("from");
        let tree = from_dir.join("tree");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join("soul.md"), "S\n").unwrap();
        let status = std::process::Command::new("tar")
            // macOS bsdtar가 xattr AppleDouble(._*) 사이드카를 tar에 넣지 않게 한다 — 프로덕션
            // 결정론 tar(GNU/python)는 이런 엔트리가 없으므로 픽스처를 프로덕션 포맷과 일치시킨다.
            .env("COPYFILE_DISABLE", "1")
            .arg("-czf")
            .arg(from_dir.join("pack.tar.gz"))
            .arg("-C")
            .arg(&tree)
            .arg(".")
            .status()
            .expect("tar czf");
        assert!(status.success());
        let mut files_map = serde_json::Map::new();
        files_map.insert("soul.md".to_string(), json!(sha256_of(b"S\n")));
        // 서명은 유효하되 digest는 의도적으로 틀린 값(전개 前 tar↔digest 대조가 잡아야 한다).
        let manifest = json!({
            "pack_version": "2.0.0", "min_binary_version": "0.4.1", "key_id": "TESTKEY",
            "signed_at": 3000, "expires_at": 9_000_000_000i64,
            "digest": "0000000000000000000000000000000000000000000000000000000000000000",
            "files": files_map,
        });
        let mbytes = serde_json::to_vec(&manifest).unwrap();
        std::fs::write(from_dir.join("pack-manifest.json"), &mbytes).unwrap();
        std::fs::write(from_dir.join("pack-manifest.json.minisig"), sign(&mbytes)).unwrap();

        let staging = td.join("staging");
        let lock = td.join(".lock");
        let accepted = td.join(".pack-accepted.json");
        let r =
            pack_update_from_dir(&from_dir, &staging, &lock, &accepted, 5000, "0.4.1", &kr, false);
        match saved {
            Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
        }
        match saved_cfg {
            Some(v) => std::env::set_var(cys::pack::ENV_CONFIG_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_CONFIG_DIR),
        }
        let e = r.expect_err("digest 불일치인데 통과");
        assert!(e.contains("digest 불일치"), "digest 거부 사유 아님: {e}");
        assert!(!staging.join("soul.md").exists(), "digest 거부인데 전개됨(전개 前 거부 위반)");
        let _ = std::fs::remove_dir_all(&td);
    }

    // ── Tier R reinject gate ─────────────────────────────────────────────────────
    /// check-path 빈 셸 게이트는 live agent 부재일 때만 skip 판정한다(forced는 이 함수 미호출).
    #[test]
    fn reinject_bare_shell_gate_skips_only_when_no_live_agent() {
        // live agent(등록·미종료·관측됨) → 진행(skip=false), 기존 ACK 핑 경로 유지.
        let live = json!({"agent": "claude", "exited": false, "agent_alive": true});
        assert!(!reinject_check_should_skip_bare_shell(&live), "live agent인데 skip");
        // 순수 빈 셸(agent 미등록) → skip(디렉티브 전문 뿌리기 차단).
        let bare = json!({"agent": null, "exited": false, "agent_alive": null});
        assert!(reinject_check_should_skip_bare_shell(&bare), "빈 셸인데 진행");
        // 크래시투셸(agent 등록됐으나 exited) → skip.
        let crashed = json!({"agent": "claude", "exited": true, "agent_alive": false});
        assert!(reinject_check_should_skip_bare_shell(&crashed), "크래시투셸인데 진행");
        // agent 등록됐으나 아직 미관측(agent_alive=false) → skip.
        let unseen = json!({"agent": "claude", "exited": false, "agent_alive": false});
        assert!(reinject_check_should_skip_bare_shell(&unseen), "미관측 agent인데 진행");
    }

    // ============================ drain --verify (기능 1) 테스트 ============================

    /// 노드 협조 상태를 모사하는 fake I/O. 협조 노드는 지시받은 마커를 파일에 기입하고, 미저장·wedge·
    /// hung은 각각 거동을 흉내낸다(producer≠evaluator — negative fixture 검증).
    #[derive(Clone, Copy, PartialEq)]
    enum FakeScenario {
        Cooperative,
        /// [R1] 지시문을 순서대로 리터럴 실행하는 협조 노드 — '정지' 지시를 마커 기입보다 먼저 만나면
        /// halt해 마커를 기입하지 못한다(구 지시문 순서면 '저장했으나 timeout' 오판정).
        LiteralOrdered,
        NonSaving,
        Wedge,
        Hung,
        /// (U-14) 관문 가드가 주입을 **보류**시킨 노드 — 소켓은 멀쩡하고 우리가 안 보낸 것이다.
        GateHeld,
    }

    struct FakeVerifyIo {
        nodes: std::sync::Mutex<std::collections::HashMap<u64, (FakeScenario, std::path::PathBuf)>>,
        last_inject: std::sync::Mutex<std::collections::HashMap<u64, String>>,
        // [R2] 노드별 send_return 호출 횟수 — 제출 완료 노드에 잉여 Return이 안 나가는지 검증.
        returns: std::sync::Mutex<std::collections::HashMap<u64, u32>>,
    }
    impl FakeVerifyIo {
        fn new() -> Self {
            FakeVerifyIo {
                nodes: std::sync::Mutex::new(std::collections::HashMap::new()),
                last_inject: std::sync::Mutex::new(std::collections::HashMap::new()),
                returns: std::sync::Mutex::new(std::collections::HashMap::new()),
            }
        }
        fn add(&self, sid: u64, scen: FakeScenario, file: std::path::PathBuf) {
            self.nodes.lock().unwrap().insert(sid, (scen, file));
        }
        fn return_count(&self, sid: u64) -> u32 {
            *self.returns.lock().unwrap().get(&sid).unwrap_or(&0)
        }
    }
    /// 주입 텍스트에서 마커 한 줄을 추출(협조 노드가 '지시대로 기입'하는 것을 모사).
    fn extract_marker(text: &str) -> Option<String> {
        let start = text.find("<!-- cys-checkpoint:")?;
        let rest = &text[start..];
        let end = rest.find("-->")? + 3;
        Some(rest[..end].to_string())
    }
    /// [R1] 지시문을 순서대로 리터럴 실행하는 노드가 마커 기입에 '도달'하는가 — 마커 지시가 '정지'
    /// 지시보다 앞에 있으면 도달(true), 정지를 먼저 만나면 halt해 미도달(false). 이 판정이 곧
    /// drain_verify_instruction의 단계 순서 정확성을 규정한다(정지<마커면 저장 유실).
    fn literal_reaches_marker(text: &str) -> bool {
        let marker = text.find("<!-- cys-checkpoint:");
        let stop = text.find("멈추").or_else(|| text.find("정지"));
        match (marker, stop) {
            (Some(m), Some(s)) => m < s,
            (Some(_), None) => true,
            _ => false,
        }
    }
    /// 문자열을 cols 문자마다 물리 줄바꿈(실터미널 래핑 모사 — 멀티바이트 안전, char 단위).
    fn wrap_cols(s: &str, cols: usize) -> String {
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && i % cols == 0 {
                out.push('\n');
            }
            out.push(c);
        }
        out
    }
    fn fake_write_marker(text: &str, file: Option<std::path::PathBuf>) {
        if let (Some(marker), Some(f)) = (extract_marker(text), file) {
            let mut cur = std::fs::read_to_string(&f).unwrap_or_default();
            cur.push_str(&marker);
            cur.push('\n');
            let _ = std::fs::write(&f, cur);
        }
    }
    impl VerifyIo for FakeVerifyIo {
        fn inject(
            &self,
            _socket: &std::path::Path,
            sid: u64,
            text: &str,
            _timeout: std::time::Duration,
        ) -> Result<(), String> {
            let scen = self.nodes.lock().unwrap().get(&sid).map(|(s, _)| *s);
            let file = self.nodes.lock().unwrap().get(&sid).map(|(_, f)| f.clone());
            self.last_inject
                .lock()
                .unwrap()
                .insert(sid, text.to_string());
            match scen {
                Some(FakeScenario::Hung) => return Err("hung socket".into()),
                // (U-14) 가드 보류 — 머리표가 붙은 에러다(소켓 hung 과 구분되어야 한다).
                Some(FakeScenario::GateHeld) => {
                    return Err(format!(
                        "{} 관문 보류(gate=bypass-disclaimer)",
                        cys::inject_guard::HOLD_TOKEN
                    ))
                }
                Some(FakeScenario::Cooperative) => fake_write_marker(text, file),
                // [R1] 순서대로 리터럴 실행 — 마커 지시에 도달할 때만 기입(정지가 먼저면 halt·미기입).
                Some(FakeScenario::LiteralOrdered) => {
                    if literal_reaches_marker(text) {
                        fake_write_marker(text, file);
                    }
                }
                _ => {} // NonSaving·Wedge: 기입 안 함
            }
            Ok(())
        }
        fn read_screen(
            &self,
            _socket: &std::path::Path,
            sid: u64,
            _lines: u64,
            _timeout: std::time::Duration,
        ) -> Result<String, String> {
            let scen = self.nodes.lock().unwrap().get(&sid).map(|(s, _)| *s);
            let raw = self
                .last_inject
                .lock()
                .unwrap()
                .get(&sid)
                .cloned()
                .unwrap_or_default();
            match scen {
                Some(FakeScenario::Hung) => Err("hung socket".into()),
                Some(FakeScenario::Wedge) => {
                    // ★미제출 wedge 모사(비-tautology): 주입 텍스트가 입력 박스 안에 40자 래핑으로 잔류하고
                    // 하단에 박스 테두리·단축키·토큰카운터 UI가 붙는다(sentinel이 최하단에서 밀리고 경계에서
                    // 쪼개짐). input_region은 박스 상단 '╭' 이후를 보므로 이 잔류 입력을 검출한다.
                    Ok(format!(
                        "...이전 대화...\n╭────────────────────╮\n│ (미제출)\n{}\n╰────────────────────╯\n  ⏵⏵ 6 lines · esc to clear\n  ? for shortcuts\n  ",
                        wrap_cols(&raw, 40)
                    ))
                }
                Some(FakeScenario::NonSaving) => {
                    // ★[R2·R3 모사] 제출 완료 + 스크롤백 에코: 주입 텍스트(nonce 포함)가 스크롤백 상단에
                    // 남고, 하단 입력창은 비어 있다(제출됨). input_region(마지막 '╭' 이후)엔 nonce가 없어야
                    // wedge=false — 구 전체화면 매치면 에코의 nonce로 오검출(잉여 Return·라벨 오표기).
                    Ok(format!(
                        "...이전 대화...\n{}\n\n좋아, 확인했다. 재시작을 기다린다.\n╭────────────────────╮\n│ > \n╰────────────────────╯\n  ? for shortcuts\n  ",
                        raw
                    ))
                }
                // 협조·리터럴: 제출됨 — 하단은 스피너·빈 프롬프트(주입 텍스트 잔류 없음)
                _ => Ok("...이전 대화...\n✻ Working… (esc to interrupt)\n> ".into()),
            }
        }
        fn send_return(
            &self,
            _socket: &std::path::Path,
            sid: u64,
            _timeout: std::time::Duration,
        ) -> Result<(), String> {
            *self.returns.lock().unwrap().entry(sid).or_insert(0) += 1;
            Ok(())
        }
    }

    fn mk_target(sid: u64, socket: std::path::PathBuf, live_cwd: Option<String>) -> VerifyTarget {
        VerifyTarget {
            socket,
            dept: "main".into(),
            display: "본부".into(),
            surface_id: sid,
            surface_ref: format!("surface:{sid}"),
            role: "worker".into(),
            live_cwd,
            pending_undelivered: 0,
        }
    }

    /// 무회귀 증명: `cys drain`(기존 3 호출자 invocation)은 verify=false로 파싱돼 plain drain 경로로
    /// 라우팅된다(거동 diff 0). `--verify`만 신규 경로로 분기.
    #[test]
    fn drain_flag_parsing_defaults_to_plain() {
        use clap::Parser;
        match Cli::parse_from(["cys", "drain"]).command {
            Command::Drain { verify, timeout } => {
                assert!(!verify, "무인자 drain은 plain(verify=false)이어야 함 — 회귀");
                assert_eq!(timeout, 20);
            }
            _ => panic!("drain이 Drain으로 파싱되지 않음"),
        }
        match Cli::parse_from(["cys", "drain", "--verify", "--timeout", "7"]).command {
            Command::Drain { verify, timeout } => {
                assert!(verify);
                assert_eq!(timeout, 7);
            }
            _ => panic!(),
        }
    }

    /// 마커 포맷 — HTML 주석형·체크박스 문법 금지·denylist 토큰 회피.
    #[test]
    fn drain_verify_marker_avoids_checkbox_and_denylist() {
        let m = checkpoint_marker("run17-42", 1_700_000_000);
        assert_eq!(m, "<!-- cys-checkpoint: run17-42 1700000000 -->");
        assert!(!m.contains("- [ ]") && !m.contains("- [x]") && !m.contains("- [X]"));
        for tok in [
            "denylist", "recovery", "kill-switch", "soul.md", "autopilot", "자율주행", "안전핵",
            "eval-driven", "헌법",
        ] {
            assert!(!m.contains(tok), "denylist 토큰 '{tok}' 포함");
        }
    }

    /// wedge 판정 — 하단 잔류 텍스트만 wedge, 스피너·빈 프롬프트는 전달됨.
    #[test]
    fn drain_verify_delivery_wedged_detection() {
        let sentinel = "run1-9";
        assert!(delivery_wedged(
            "위쪽\n[DRAIN-VERIFY] ... run1-9 ... 마커 <!-- cys-checkpoint: run1-9 1 -->",
            sentinel
        ));
        assert!(!delivery_wedged(
            "...이전 대화...\n✻ Working…\n> ",
            sentinel
        ));
    }

    /// ② 협조 노드 → saved (지시대로 마커 기입).
    #[test]
    fn drain_verify_saved_on_cooperative() {
        let td = std::env::temp_dir().join(format!("cys-dv-coop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let round = td.join("_round");
        std::fs::create_dir_all(&round).unwrap();
        std::fs::write(round.join("SESSION_STATE.md"), "# 상태\n").unwrap();
        let io = FakeVerifyIo::new();
        io.add(
            7,
            FakeScenario::Cooperative,
            round.join("SESSION_STATE.md"),
        );
        let t = mk_target(7, td.join("cys.sock"), Some(td.to_string_lossy().into_owned()));
        let (o, _d) = verify_one_node(&io, &t, "run1", std::time::Duration::from_secs(2), 100);
        let _ = std::fs::remove_dir_all(&td);
        assert_eq!(o, VerifyOutcome::Saved);
    }

    /// ②(변형) 이미 정확한 nonce 마커가 있으면 즉시 saved(idempotent 선통과).
    #[test]
    fn drain_verify_pass_when_already_marked() {
        let td = std::env::temp_dir().join(format!("cys-dv-pre-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let round = td.join("_round");
        std::fs::create_dir_all(&round).unwrap();
        let sock = td.join("cys.sock");
        // verify_one_node의 nonce = "{prefix}-{socket_disc:x}-{sid}" ([F1] 소켓 구별자 포함).
        let nonce = format!("run1-{:x}-7", socket_discriminator(&sock));
        let marker = checkpoint_marker(&nonce, 100);
        std::fs::write(round.join("SESSION_STATE.md"), format!("# 상태\n{marker}\n")).unwrap();
        let io = FakeVerifyIo::new(); // 노드 미등록 — 주입해도 무동작(하지만 선통과라 무관)
        let t = mk_target(7, sock, Some(td.to_string_lossy().into_owned()));
        let (o, _d) = verify_one_node(&io, &t, "run1", std::time::Duration::from_secs(2), 100);
        let _ = std::fs::remove_dir_all(&td);
        assert_eq!(o, VerifyOutcome::Saved);
    }

    /// ① 저장 안 하는 노드 → timeout(FAIL).
    #[test]
    fn drain_verify_timeout_on_no_save() {
        let td = std::env::temp_dir().join(format!("cys-dv-nosave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let round = td.join("_round");
        std::fs::create_dir_all(&round).unwrap();
        std::fs::write(round.join("SESSION_STATE.md"), "# 상태\n").unwrap();
        let io = FakeVerifyIo::new();
        io.add(3, FakeScenario::NonSaving, round.join("SESSION_STATE.md"));
        let t = mk_target(3, td.join("cys.sock"), Some(td.to_string_lossy().into_owned()));
        let (o, _d) = verify_one_node(&io, &t, "run1", std::time::Duration::from_secs(1), 100);
        let _ = std::fs::remove_dir_all(&td);
        assert_eq!(o, VerifyOutcome::Timeout);
    }

    /// ★(U-14) 관문 보류 → `unverifiable`(소켓 hung=timeout 과 **구분**) · Return 재전송 0.
    ///
    /// 안전 방향은 timeout 과 같다(둘 다 Saved 가 아니라 `all_saved` 를 거짓으로 만든다).
    /// 갈라야 하는 이유는 **진단의 정직성**이다 — 소켓은 멀쩡했고 우리가 스스로 안 보냈다.
    /// 같은 라벨로 접으면 사람이 소켓·데몬을 뒤진다.
    #[test]
    fn drain_verify_gate_hold_is_unverifiable_not_socket_timeout() {
        let td = std::env::temp_dir().join(format!("cys-dv-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let round = td.join("_round");
        std::fs::create_dir_all(&round).unwrap();
        std::fs::write(round.join("SESSION_STATE.md"), "# 상태\n").unwrap();
        let io = FakeVerifyIo::new();
        io.add(9, FakeScenario::GateHeld, round.join("SESSION_STATE.md"));
        let t = mk_target(9, td.join("cys.sock"), Some(td.to_string_lossy().into_owned()));
        let (o, d) = verify_one_node(&io, &t, "run1", std::time::Duration::from_secs(1), 100);
        let rc = io.return_count(9);
        let _ = std::fs::remove_dir_all(&td);
        assert_eq!(o, VerifyOutcome::Unverifiable, "관문 보류가 소켓 hung 으로 오분류됐다");
        assert!(d.contains("관문 보류"), "사유가 관문 보류로 보고되지 않는다: {d}");
        assert_eq!(rc, 0, "보류인데 Return 재전송이 나갔다(rc={rc}) — 관문 위젯을 누른다");
    }

    /// ③ 미제출 wedge → delivery_failed(timeout과 구분).
    #[test]
    fn drain_verify_delivery_failed_on_wedge() {
        let td = std::env::temp_dir().join(format!("cys-dv-wedge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let round = td.join("_round");
        std::fs::create_dir_all(&round).unwrap();
        std::fs::write(round.join("SESSION_STATE.md"), "# 상태\n").unwrap();
        let io = FakeVerifyIo::new();
        io.add(5, FakeScenario::Wedge, round.join("SESSION_STATE.md"));
        let t = mk_target(5, td.join("cys.sock"), Some(td.to_string_lossy().into_owned()));
        let (o, _d) = verify_one_node(&io, &t, "run1", std::time::Duration::from_secs(1), 100);
        let rc = io.return_count(5);
        let _ = std::fs::remove_dir_all(&td);
        assert_eq!(o, VerifyOutcome::DeliveryFailed);
        assert!(rc >= 1, "실 wedge엔 Return 재전송이 발화해야 함(rc={rc})"); // [R2] 정상 경로
    }

    /// [R2·R3] 제출 완료 + 스크롤백 에코 노드 → wedge 오검출 없음: 잉여 Return 0(R2)·라벨 timeout 정확(R3).
    /// 구 전체화면 매치면 에코의 nonce로 delivery_failed 오표기 + 승인대기 노드 잉여 Return 위험이었다.
    #[test]
    fn drain_verify_no_spurious_wedge_on_submitted_echo() {
        let td = std::env::temp_dir().join(format!("cys-dv-echo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let round = td.join("_round");
        std::fs::create_dir_all(&round).unwrap();
        std::fs::write(round.join("SESSION_STATE.md"), "# 상태\n").unwrap();
        let io = FakeVerifyIo::new();
        io.add(6, FakeScenario::NonSaving, round.join("SESSION_STATE.md")); // 제출 에코 모사(read_screen)
        let t = mk_target(6, td.join("cys.sock"), Some(td.to_string_lossy().into_owned()));
        let (o, _d) = verify_one_node(&io, &t, "run1", std::time::Duration::from_secs(1), 100);
        let rc = io.return_count(6);
        let _ = std::fs::remove_dir_all(&td);
        assert_eq!(o, VerifyOutcome::Timeout, "제출됐으나 미저장=timeout(delivery_failed 아님·R3)");
        assert_eq!(rc, 0, "제출 완료 노드엔 잉여 Return 0이어야 함(R2·rc={rc})");
    }

    /// [R2·R3 단위] delivery_wedged가 입력창 영역만 본다 — 미제출 wedge(박스 안)=검출, 제출 에코(스크롤백)=비검출.
    #[test]
    fn drain_verify_delivery_wedged_input_region_only() {
        let nonce = "run9-42";
        // ① 미제출: 박스 안에 잔류(래핑) → 검출
        let wedged = "...대화...\n╭────────╮\n│ (미제출)\n<!-- cys-check\npoint: run9-42 1 -->\n╰────────╯\n  ? shortcuts";
        assert!(delivery_wedged(wedged, nonce), "입력 박스 내 미제출 텍스트는 wedge");
        // ② 제출 에코: nonce가 스크롤백(박스 위)에만, 하단 박스는 빔 → 비검출
        let echoed = "...대화...\n<!-- cys-checkpoint: run9-42 1 -->\n좋아, 확인.\n╭────────╮\n│ > \n╰────────╯\n  ? shortcuts";
        assert!(!delivery_wedged(echoed, nonce), "제출된 스크롤백 에코는 wedge 아님");
    }

    /// [R1] 지시문 단계 순서 정확성 — 마커 기입이 '정지'보다 앞이라 리터럴 실행 노드가 저장한다.
    /// ★비-tautology: 구 순서(정지<마커)면 리터럴 노드가 마커 미기입→timeout 오판정임을 명시 증명.
    #[test]
    fn drain_verify_literal_ordered_node_saves_and_old_order_would_fail() {
        // 현 지시문: 마커가 '정지'보다 앞 → 리터럴 노드 도달(true)
        let cur = drain_verify_instruction("<!-- cys-checkpoint: n-1 1 -->");
        assert!(literal_reaches_marker(&cur), "현 지시문은 마커가 정지보다 앞이어야 함(R1)");
        // 구 순서(①저장 ②정지 ③마커) 재구성 → 리터럴 노드가 정지를 먼저 만나 미도달(false)=저장 유실
        let old = "① 저장하라. ② 작업을 멈추고 기다려라. ③ 마지막으로 <!-- cys-checkpoint: n-1 1 --> 추가";
        assert!(!literal_reaches_marker(old), "구 순서면 리터럴 노드가 마커 미기입(FAIL 증명)");
        // 통합: LiteralOrdered 노드가 현 지시문으로 실제 저장→saved
        let td = std::env::temp_dir().join(format!("cys-dv-lit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let round = td.join("_round");
        std::fs::create_dir_all(&round).unwrap();
        std::fs::write(round.join("SESSION_STATE.md"), "# 상태\n").unwrap();
        let io = FakeVerifyIo::new();
        io.add(4, FakeScenario::LiteralOrdered, round.join("SESSION_STATE.md"));
        let t = mk_target(4, td.join("cys.sock"), Some(td.to_string_lossy().into_owned()));
        let (o, _d) = verify_one_node(&io, &t, "run1", std::time::Duration::from_secs(2), 100);
        let _ = std::fs::remove_dir_all(&td);
        assert_eq!(o, VerifyOutcome::Saved, "리터럴 순서 노드가 현 지시문으로 저장→saved");
    }

    /// ④ hung 소켓(RPC 에러) → timeout(전역 캡 내 — 개별 스레드는 즉시 반환).
    #[test]
    fn drain_verify_timeout_on_hung() {
        let td = std::env::temp_dir().join(format!("cys-dv-hung-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let round = td.join("_round");
        std::fs::create_dir_all(&round).unwrap();
        let io = FakeVerifyIo::new();
        io.add(9, FakeScenario::Hung, round.join("SESSION_STATE.md"));
        let t = mk_target(9, td.join("cys.sock"), Some(td.to_string_lossy().into_owned()));
        let (o, _d) = verify_one_node(&io, &t, "run1", std::time::Duration::from_secs(1), 100);
        let _ = std::fs::remove_dir_all(&td);
        assert_eq!(o, VerifyOutcome::Timeout);
    }

    /// [F1] 크로스소켓 nonce 충돌 위양성 — 같은 sid + 같은 live_cwd(부서 cwd 수렴)인 두 소켓에서, 한 노드가
    /// 남긴 마커를 다른 소켓의 (주입 실패) 노드가 자기 것으로 오인해 Saved 위양성이 나던 결함.
    /// ★비-tautology: 구 nonce("{prefix}-{sid}", 소켓 구별자 없음)면 매치=위양성임을 명시 증명, 신 로직은 Timeout.
    #[test]
    fn drain_verify_cross_socket_nonce_collision_false_positive() {
        let td = std::env::temp_dir().join(format!("cys-dv-xsock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let round = td.join("_round");
        std::fs::create_dir_all(&round).unwrap();
        let file = round.join("SESSION_STATE.md");
        let cwd = td.to_string_lossy().into_owned();
        let (prefix, sid) = ("runX", 7u64);
        let sock_a = std::path::PathBuf::from("/x/cys-dept-a/cys.sock");
        let sock_b = std::path::PathBuf::from("/x/cys-dept-b/cys.sock");
        // 소켓 구별자는 결정론이고 서로 달라야 한다(같은 소켓은 안정, 다른 소켓은 상이).
        assert_eq!(socket_discriminator(&sock_a), socket_discriminator(&sock_a));
        assert_ne!(socket_discriminator(&sock_a), socket_discriminator(&sock_b));

        // 구 로직 재현: A(sock_a)가 구 스킴 nonce("{prefix}-{sid}")로 공유 파일에 마커를 남겼다.
        let a_old_nonce = format!("{prefix}-{sid}");
        std::fs::write(
            &file,
            format!("# 상태\n{}\n", checkpoint_marker(&a_old_nonce, 100)),
        )
        .unwrap();
        // 구 로직 B의 nonce도 sid만 쓰므로 A 마커를 자기 것으로 매치 → Saved 위양성(수정 전 FAIL 경로).
        assert!(
            file_has_checkpoint_nonce(&file, &a_old_nonce),
            "구 로직: 같은 sid의 B가 A(타 소켓) 마커를 매치 → Saved 위양성"
        );

        // 신 로직: B(Hung=주입 전부 Err, sock_b)를 verify_one_node로 — 소켓 구별자로 nonce가 달라 파일의
        // A 마커를 무시 → idempotent 미통과 → 주입(Hung Err) → Timeout(위양성 회피).
        let io = FakeVerifyIo::new();
        io.add(sid, FakeScenario::Hung, file.clone());
        let tb = mk_target(sid, sock_b, Some(cwd));
        let (ob, _d) = verify_one_node(&io, &tb, prefix, std::time::Duration::from_secs(1), 100);
        let _ = std::fs::remove_dir_all(&td);
        assert_eq!(
            ob,
            VerifyOutcome::Timeout,
            "신 로직: 크로스소켓 nonce 충돌 위양성 회피 → Timeout"
        );
    }

    /// ⑥(변형) live_cwd 미제공 → unverifiable(무음 폴백 금지).
    #[test]
    fn drain_verify_unverifiable_without_live_cwd() {
        let io = FakeVerifyIo::new();
        let t = mk_target(1, std::path::PathBuf::from("/nonexistent/cys.sock"), None);
        let (o, _d) = verify_one_node(&io, &t, "run1", std::time::Duration::from_secs(1), 100);
        assert_eq!(o, VerifyOutcome::Unverifiable);
    }

    /// ⑤ 복원 중(phoenix 저널 stage<g2_ack) → skipped_restoring.
    #[test]
    fn drain_verify_skipped_when_restoring() {
        let td = std::env::temp_dir().join(format!("cys-dv-restore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let deptdir = td.join("cys-dept-x");
        let phoenix = deptdir.join("phoenix");
        std::fs::create_dir_all(&phoenix).unwrap();
        // reinject 완료·g2_ack 미완료 = 복원 in-flight
        let j = json!({"roles": {"worker": {"stages": {"reinject": {"done": true}}}}});
        std::fs::write(
            phoenix.join("journal-default.json"),
            serde_json::to_string(&j).unwrap(),
        )
        .unwrap();
        let socket = deptdir.join("cys.sock");
        assert!(restore_guard_reason(&socket, "worker").is_some());
        // g2_ack 완료면 복원 끝 → None
        let j2 = json!({"roles": {"worker": {"stages": {"reinject": {"done": true}, "g2_ack": {"done": true}}}}});
        std::fs::write(
            phoenix.join("journal-default.json"),
            serde_json::to_string(&j2).unwrap(),
        )
        .unwrap();
        assert!(restore_guard_reason(&socket, "worker").is_none());
        // 다른 역할은 무관 → None(over-skip 금지)
        assert!(restore_guard_reason(&socket, "master").is_none());

        // verify_one_node 통합: 복원 중이면 IO 이전에 skip
        std::fs::write(
            phoenix.join("journal-default.json"),
            serde_json::to_string(&j).unwrap(),
        )
        .unwrap();
        let io = FakeVerifyIo::new();
        let t = mk_target(2, socket, Some(deptdir.to_string_lossy().into_owned()));
        let (o, _d) = verify_one_node(&io, &t, "run1", std::time::Duration::from_secs(1), 100);
        let _ = std::fs::remove_dir_all(&td);
        assert_eq!(o, VerifyOutcome::SkippedRestoring);
    }

    /// [F2] 신선한 저널이 파손 JSON이면 fail-CLOSED(복원 중 취급·skip) — 스키마 스큐/부분쓰기에 안전.
    #[test]
    fn drain_verify_restore_guard_fail_closed_on_corrupt_journal() {
        let td = std::env::temp_dir().join(format!("cys-dv-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let deptdir = td.join("cys-dept-y");
        let phoenix = deptdir.join("phoenix");
        std::fs::create_dir_all(&phoenix).unwrap();
        let socket = deptdir.join("cys.sock");
        // ① 파손 JSON(신선 mtime) → Some(skip)
        std::fs::write(phoenix.join("journal-default.json"), "{ this is not json ").unwrap();
        assert!(
            restore_guard_reason(&socket, "worker").is_some(),
            "파손 신선 저널은 fail-closed여야 함"
        );
        // ② roles 비객체(신선) → Some(skip)
        std::fs::write(
            phoenix.join("journal-default.json"),
            serde_json::to_string(&json!({"roles": "oops"})).unwrap(),
        )
        .unwrap();
        assert!(restore_guard_reason(&socket, "worker").is_some());
        // ③ role의 stages 스키마 이상(신선) → Some(skip)
        std::fs::write(
            phoenix.join("journal-default.json"),
            serde_json::to_string(&json!({"roles": {"worker": {"stages": 5}}})).unwrap(),
        )
        .unwrap();
        assert!(restore_guard_reason(&socket, "worker").is_some());
        // ④ 저널 디렉토리 자체가 없으면 None(복원 아님 — 무해)
        let empty = td.join("cys-dept-z").join("cys.sock");
        assert!(restore_guard_reason(&empty, "worker").is_none());
        let _ = std::fs::remove_dir_all(&td);
    }

    /// [F1] 실터미널 래핑+하단 UI로 sentinel이 최하단에서 밀리고 경계에서 쪼개진 wedge를 검출한다.
    /// ★비-tautology 증명: 동일 fixture를 구 로직(tail-4행 단일행 스캔)에 넣으면 놓친다(FAIL).
    #[test]
    fn drain_verify_wedge_survives_wrapping_and_trailing_ui() {
        let nonce = "1700000000-88-7";
        // 래핑으로 nonce가 물리 행 경계에서 쪼개지고("...1700000000-" / "88-7 ..."), 그 아래로 입력창
        // 테두리·단축키·토큰카운터 UI 4행이 붙어 nonce가 하단에서 여러 행 위로 밀린다.
        let screen = "\
> [DRAIN-VERIFY] 재시작 전 체크포인트 검증. 지금 즉시\n\
  ① _round/SESSION_STATE.md 저장 ② 작업 멈춤\n\
  ③ 파일 끝에 이 한 줄: <!-- cys-checkpoint: 1700000000-\n\
88-7 1700000000 -->\n\
╰──────────────────────────────╯\n\
  ⏵⏵ 5 lines · esc to clear\n\
  ? for shortcuts                  (auto)\n\
  context: 12k tokens\n\
  ";
        // 구 로직: 하단 4행에서 온전한 nonce 매치 → 놓침(FAIL). fixture가 tautology가 아님을 증명.
        let old_tail4 = screen.lines().rev().take(4).any(|l| l.contains(nonce));
        assert!(!old_tail4, "fixture가 구 tail-4 로직도 통과 — tautology");
        // 신 로직: 전체 행·공백제거 매치 → 래핑·경계쪼갬·trailing UI에도 wedge 검출.
        assert!(delivery_wedged(screen, nonce), "신 로직이 래핑된 wedge를 검출해야 함");
    }

    /// ⑥ 0-노드 → 우아한 no-op(all_saved=true, exit 0 대응).
    #[test]
    fn drain_verify_zero_nodes_noop() {
        let io: std::sync::Arc<dyn VerifyIo + Send + Sync> = std::sync::Arc::new(FakeVerifyIo::new());
        let report = drain_verify_fanout(io, vec![], std::time::Duration::from_secs(1), 100);
        assert_eq!(report["total"], json!(0));
        assert_eq!(report["all_saved"], json!(true));
    }

    /// 전 노드 verify 총 소요 ≤ 전역 캡(직렬 누적 금지) — N개 미저장 노드를 병렬로 돌려 직렬 시간보다
    /// 뚜렷이 빠름을 확인한다(timeout=1s·4노드 → 직렬 ≥4s, 병렬 ~timeout).
    #[test]
    fn drain_verify_fanout_is_parallel_not_serial() {
        let td = std::env::temp_dir().join(format!("cys-dv-par-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let fake = FakeVerifyIo::new();
        let n = 4u64;
        let mut targets = Vec::new();
        for i in 0..n {
            let round = td.join(format!("n{i}")).join("_round");
            std::fs::create_dir_all(&round).unwrap();
            std::fs::write(round.join("SESSION_STATE.md"), "# s\n").unwrap();
            fake.add(i, FakeScenario::NonSaving, round.join("SESSION_STATE.md"));
            targets.push(mk_target(
                i,
                td.join(format!("n{i}")).join("cys.sock"),
                Some(td.join(format!("n{i}")).to_string_lossy().into_owned()),
            ));
        }
        let io: std::sync::Arc<dyn VerifyIo + Send + Sync> = std::sync::Arc::new(fake);
        let t0 = std::time::Instant::now();
        let report = drain_verify_fanout(io, targets, std::time::Duration::from_secs(1), 100);
        let elapsed = t0.elapsed();
        let _ = std::fs::remove_dir_all(&td);
        assert_eq!(report["summary"]["timeout"], json!(4));
        assert_eq!(report["all_saved"], json!(false));
        // 직렬이면 ≥ 4s. 병렬이면 ~1.x s. 3s 미만이면 병렬 확정.
        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "fan-out이 직렬로 보임: {elapsed:?}"
        );
    }

    /// 집계·pending 유실 가시화 — 혼합 결과에서 summary·all_saved·pending_loss_warning 정합.
    #[test]
    fn drain_verify_aggregation_and_pending_visibility() {
        let td = std::env::temp_dir().join(format!("cys-dv-agg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        let fake = FakeVerifyIo::new();
        // 노드0=협조(saved), 노드1=미저장(timeout, pending 2건)
        let r0 = td.join("n0").join("_round");
        let r1 = td.join("n1").join("_round");
        std::fs::create_dir_all(&r0).unwrap();
        std::fs::create_dir_all(&r1).unwrap();
        std::fs::write(r0.join("SESSION_STATE.md"), "# s\n").unwrap();
        std::fs::write(r1.join("SESSION_STATE.md"), "# s\n").unwrap();
        fake.add(0, FakeScenario::Cooperative, r0.join("SESSION_STATE.md"));
        fake.add(1, FakeScenario::NonSaving, r1.join("SESSION_STATE.md"));
        let mut t0 = mk_target(0, td.join("n0").join("cys.sock"), Some(td.join("n0").to_string_lossy().into_owned()));
        t0.role = "worker".into();
        let mut t1 = mk_target(1, td.join("n1").join("cys.sock"), Some(td.join("n1").to_string_lossy().into_owned()));
        t1.pending_undelivered = 2;
        let io: std::sync::Arc<dyn VerifyIo + Send + Sync> = std::sync::Arc::new(fake);
        let report = drain_verify_fanout(io, vec![t0, t1], std::time::Duration::from_secs(1), 100);
        let _ = std::fs::remove_dir_all(&td);
        assert_eq!(report["summary"]["saved"], json!(1));
        assert_eq!(report["summary"]["timeout"], json!(1));
        assert_eq!(report["all_saved"], json!(false));
        assert_eq!(report["pending_loss_warning"].as_array().unwrap().len(), 1);
        assert_eq!(report["pending_loss_warning"][0]["pending_undelivered"], json!(2));
    }

    // ── C3(Declared State) 저장검증 대상 정화 ────────────────────────────────
    // cwd/_round는 전 노드 공유 디렉터리다. 바로 옆 pack/round 분기는 그 사실을 알고 대상 역할
    // 파일로 한정하는데 이 분기만 무방비였던 비대칭이 유령 todo 사고의 코드 수준 원인이다.

    /// 은퇴·타 스코프만 제외한다. 판정 불능(미선언·고아)은 **제외하지 않는다** — 판정 못 한다고
    /// 살아있을 수 있는 파일을 게이트에서 빼면 저장 누락을 조용히 통과시킨다(ADR-3 fail-open).
    #[test]
    fn todo_decl_excluded_only_drops_retired_and_foreign_scope() {
        let td = std::env::temp_dir().join(format!(
            "cys-c3-decl-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let packs = |s: &str| matches!(s, "pack" | "pack-dept-dept-1");
        let write = |name: &str, body: &str| {
            let p = td.join(name);
            std::fs::write(&p, body).unwrap();
            p
        };
        let decl = |scope: &str, status: &str| {
            format!(
                "<!-- javis:todo v1 owner=worker scope={scope} status={status} -->\n# T\n- [ ] a\n"
            )
        };
        let cases: Vec<(&str, String, bool)> = vec![
            ("retired", decl("pack", "retired"), true),
            ("legacy-retired", "<!-- ★ STALE 무효화 -->\n# T\n- [ ] a\n".into(), true),
            ("foreign", decl("pack-dept-dept-1", "active"), true),
            ("mine", decl("pack", "active"), false),
            ("orphan", decl("pack-dept-dept-9", "active"), false),
            ("undeclared", "# 손으로 쓴 todo\n- [ ] a\n".into(), false),
        ];
        for (name, body, want) in cases {
            let p = write(&format!("{name}_TODO.md"), &body);
            assert_eq!(
                todo_decl_excluded(&p, "pack", &packs),
                want,
                "[{name}] 저장검증 대상 제외 판정"
            );
        }
        // 열 수 없는 경로도 제외하지 않는다(fail-open — 게이트를 조용히 헐겁게 만들지 않는다).
        assert!(!todo_decl_excluded(&td.join("없는파일_TODO.md"), "pack", &packs));
        // 예산(G3) 밖 은퇴 선언은 보이지 않는다 = 제외되지 않는다.
        let far = write(
            "budget_TODO.md",
            &format!("{}\n{}", "x".repeat(cys::todo_decl::HEAD_BYTES), decl("pack", "retired")),
        );
        assert!(!todo_decl_excluded(&far, "pack", &packs));
        // ★W14 S15 — 비UTF-8 팽창이 예산을 줄이지 않는다. 원시 400 B의 0xFF는 디코드하면
        // 1200 B지만 원시 기준으로는 예산 안이므로 은퇴 선언이 보여야 한다. 종전에는
        // `parse`의 재절단이 이걸 잘라 **은퇴 파일이 저장검증 대상으로 되살아났다**.
        let p = td.join("nonutf8_TODO.md");
        let mut raw: Vec<u8> = vec![0xff; 400];
        raw.push(b'\n');
        raw.extend_from_slice(decl("pack", "retired").as_bytes());
        std::fs::write(&p, &raw).unwrap();
        assert!(todo_decl_excluded(&p, "pack", &packs));
        let _ = std::fs::remove_dir_all(&td);
    }

    /// ★**W14 — C3 소비자 테스트의 자기 반사 차단(reviewer3 자기 신고 2번).**
    ///
    /// 위 케이스는 기대값을 사람이 파서 지식으로 적은 것이다. 파서와 소비자가 **함께 틀리면
    /// 초록**인 구조였다 — Python 쪽에는 `expected.json` 외부 SOT가 있는데 Rust 소비자에는
    /// 대응물이 없었다. 여기서는 골든 픽스처를 그대로 넣고 기대값을 **오직 대장에서** 읽는다.
    #[test]
    fn golden_fixtures_drive_c3_exclusion_from_external_sot() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("cysjavis-pack/bin/tests/fixtures/todo-decl");
        let raw = std::fs::read_to_string(dir.join("expected.json")).unwrap_or_else(|e| {
            panic!("골든 대장을 읽을 수 없다({}): {e} — SOT 부재는 skip이 아니라 실패다",
                   dir.display())
        });
        let spec: serde_json::Value = serde_json::from_str(&raw).expect("expected.json 파싱");
        let my_scope = spec["my_scope"].as_str().expect("my_scope").to_string();
        let existing: Vec<String> = spec["existing_scopes"]
            .as_array()
            .expect("existing_scopes")
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        let scope_exists = |s: &str| existing.iter().any(|e| e == s);

        let td = std::env::temp_dir().join(format!(
            "cys-c3-golden-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let cases = spec["cases"].as_object().expect("cases");
        assert!(cases.len() >= 15, "픽스처 케이스가 15종 미만이다: {}", cases.len());
        for (name, exp) in cases {
            // 내용은 한 바이트도 바꾸지 않는다(바이너리 케이스가 있어 텍스트 경유 금지).
            let bytes = std::fs::read(dir.join(name))
                .unwrap_or_else(|e| panic!("픽스처 {name} 읽기 실패: {e}"));
            let p = td.join(format!("{}_TODO.md", name.trim_end_matches(".md")));
            std::fs::write(&p, &bytes).unwrap();
            let verdict = exp["classify"].as_str().expect("classify");
            // 저장검증 제외 = "주인이 처분을 명시한 것"뿐(ADR-3 fail-open). 기대값을 파서가
            // 아니라 **대장 문자열**에서 유도한다.
            let want = matches!(verdict, "retired" | "foreign-scope");
            assert_eq!(
                todo_decl_excluded(&p, &my_scope, &scope_exists),
                want,
                "[{name}] 대장 판정={verdict} 인데 C3 제외 판정이 갈렸다"
            );
        }
        let _ = std::fs::remove_dir_all(&td);
    }

    // ── S17: 의무화된 생성기가 의무화된 규칙을 지키는가 ──────────────────────
    // 디렉티브는 같은 문단에서 "경로는 반드시 `cys todo-path`" + "머리말에 선언 1줄"을 명하는데,
    // 종전 `run_todo_path`는 선언 없는 파일만 찍었다. 유일한 기계 생성기가 규칙 위반자였고,
    // 그래서 `unclaimed_ratio`가 구조적으로 M3 목표(<10%) 아래로 수렴할 수 없었다.

    /// ★생성기의 산출물은 **자기가 만든 파서**에 먹여 `counted`가 나와야 한다.
    /// (Python 스탬프 도구 `build_decl_line`과 같은 왕복 검증 패턴 — 검사식을 두 벌 두지 않는다.)
    #[test]
    fn generated_todo_body_parses_as_counted() {
        for (role, scope) in [
            ("worker", "pack"),
            ("worker-2", "pack-dept-dept-1"),
            ("reviewer-gemini", "pack"),
            ("cso", "pack.dept_1:a-b"),
        ] {
            let line = build_todo_decl_line(role, scope).expect("선언 생성");
            let body = new_todo_body(role, &line);
            let head = cys::todo_decl::head_from_bytes(body.as_bytes());
            let decl = cys::todo_decl::parse(&head).expect("생성물이 파서를 통과해야 한다");
            assert_eq!(decl.owner, role);
            assert_eq!(decl.scope, scope);
            assert_eq!(
                cys::todo_decl::classify(Some(&decl), scope, &|_| true),
                cys::todo_decl::Verdict::Counted,
                "role={role} scope={scope}"
            );
        }
    }

    /// ★실패는 **시끄럽다** — 접거나 그럴듯한 기본값으로 대체하지 않는다.
    /// 정규화 폴백(`자비스` → `pack`)은 "그럴듯하지만 틀린 정체성"을 만들고, 그 정체성은
    /// 살아있는 파일을 남의 레인으로 조용히 배제시킨다(S14와 같은 병).
    #[test]
    fn declaration_generator_fails_loudly_on_illegal_identity() {
        for (role, scope) in [
            ("worker", "자비스"),     // G4 밖 팩 이름
            ("워커", "pack"),          // G4 밖 role
            ("worker", "pack round"),  // 값 내 공백
            ("worker", ""),            // 빈 값
        ] {
            assert!(
                build_todo_decl_line(role, scope).is_err(),
                "role={role} scope={scope} — 조용히 그럴듯한 값을 만들면 안 된다"
            );
        }
    }

    /// 생성 직후 재검증(`verify_todo_counted`)이 실제로 파일을 읽어 판정한다.
    #[test]
    fn verify_todo_counted_reads_the_written_file() {
        let td = std::env::temp_dir().join(format!(
            "cys-s17-verify-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let line = build_todo_decl_line("worker", "pack").unwrap();
        let good = td.join("WORKER_TODO.md");
        std::fs::write(&good, new_todo_body("worker", &line)).unwrap();
        assert!(verify_todo_counted(&good, "pack").is_ok());
        // 선언 없는 파일(종전 생성기의 산출물 형태)은 통과하지 못한다 = S17의 재현.
        let bad = td.join("LEGACY_TODO.md");
        std::fs::write(&bad, "# worker TODO — 영속 todo (절대지침 7)\n\n").unwrap();
        assert!(verify_todo_counted(&bad, "pack").is_err());
        let _ = std::fs::remove_dir_all(&td);
    }

    // ── C2: cycle 저장 게이트 협로 봉합 ─────────────────────────────────────

    /// ★C2 ②: 후보가 **하나도 없어도** 게이트는 성립해야 한다. 신설 노드의 첫 cycle이 정확히
    /// 그 상태이고, 종전에는 여기서 "저장 검증 파일 없음" 에러로 끝나 clear가 영영 실행되지
    /// 않았다 — 컨텍스트가 가장 급한 노드가 순환에서 배제되는 방향의 실패다.
    #[test]
    fn cycle_gate_falls_back_to_expected_paths_when_nothing_detected() {
        let pack_round = std::path::Path::new("/home/x/.cys/pack/round");
        // 워커: 자기 역할 TODO 하나가 기대 경로.
        let g = cycle_gate_files(vec![], vec![pack_round.join("WORKER_TODO.md")]);
        assert_eq!(g, vec!["/home/x/.cys/pack/round/WORKER_TODO.md".to_string()],
                   "후보 전무에서 게이트가 비면 clear가 실행되지 않는다");
        // master: pack SESSION_STATE도 자기 소관이라 함께 감시한다.
        let g = cycle_gate_files(
            vec![],
            vec![pack_round.join("MASTER_TODO.md"), pack_round.join("SESSION_STATE.md")],
        );
        assert_eq!(g.len(), 2);
        assert!(g[1].ends_with("SESSION_STATE.md"));
    }

    /// ★C2 ①: 기대 경로는 **실존 여부와 무관하게** 목록에 들어가되, 이미 탐지된 후보와
    /// 중복되면 한 번만 실린다(baseline·handshake 본문이 이 목록으로 만들어진다). 탐지 순서는
    /// 보존한다 — 목록 순서가 흔들리면 handshake 본문이 매 cycle 달라져 비교가 어려워진다.
    #[test]
    fn cycle_gate_merges_expected_without_duplicates_and_keeps_order() {
        let pt = "/home/x/.cys/pack/round/WORKER_TODO.md".to_string();
        let g = cycle_gate_files(
            vec!["/w/_round/SESSION_STATE.md".into(), pt.clone()],
            vec![std::path::PathBuf::from(&pt)],
        );
        assert_eq!(g, vec!["/w/_round/SESSION_STATE.md".to_string(), pt],
                   "이미 실존해 탐지된 기대 경로가 두 번 실렸다");
    }

    /// ★C2 실측: **비존재 파일을 게이트에 넣어도 판정 로직을 바꿀 필요가 없다**는 계약.
    /// baseline이 `None`이므로 지시대로 새로 생성되면 mtime>start && Some != None 이 성립한다.
    /// 이게 깨지면 신설 노드는 지시에 순응해 저장하고도 검증 실패로 clear를 못 받는다.
    #[test]
    fn cycle_save_verified_accepts_a_newly_created_gate_file() {
        let dir = std::env::temp_dir().join(format!(
            "cys-c2-gate-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("WORKER_TODO.md").to_string_lossy().into_owned();

        // 아직 없는 파일의 baseline = None (게이트 등록 시점의 실제 값).
        let baseline = vec![(missing.clone(), sha256_file(&missing))];
        assert_eq!(baseline[0].1, None, "비존재 파일의 baseline은 None이어야 한다");

        let start_time = std::time::SystemTime::now();
        assert!(!cycle_save_verified(&baseline, start_time), "저장 전에 통과하면 게이트가 아니다");

        // mtime 해상도(초 단위 파일시스템) 때문에 생성 시각이 start_time과 같아질 수 있다 —
        // 실제 cycle에는 지시 주입·에이전트 응답 시간이 끼어 있어 발생하지 않는 조건이다.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&missing, "# 저장\n").unwrap();
        assert!(
            cycle_save_verified(&baseline, start_time),
            "새로 생성된 게이트 파일이 '갱신'으로 인정되지 않는다 — C2 전제가 깨졌다"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★A′ 회귀 핀: [CYCLE] 지시문은 감시 목록(files)의 실경로를 **그대로 전부** 열거해야
    /// 한다 — 고정 산문(틸드 하드코딩 "~/.cys/pack/round/<역할>_TODO.md"·"_round/ 또는 pack
    /// round/" 모호 안내)이 lease 실경로와 어긋나면 노드가 목록 밖에 저장해 파일 게이트·
    /// ALL-match 검증자 deny 가 잔존한다. ② plain 한 줄 CYCLE-SAVED 마커 문장은 계약 보존.
    #[test]
    fn cycle_directive_enumerates_watched_paths_and_keeps_marker_contract() {
        let files = vec![
            "/tmp/pk/round/SESSION_STATE.md".to_string(),
            "/tmp/pk/round/MASTER_TODO.md".to_string(),
        ];
        let d = cycle_save_directive("master", &files);
        for f in &files {
            assert!(d.contains(f.as_str()), "감시 경로가 지시문에서 빠졌다: {f}\n{d}");
        }
        assert!(d.contains("물리적으로 재기록"), "재기록 명령 문구 소실: {d}");
        // REVISE-1: 재기록 범위는 '목록 전부'가 아니라 **역할 소관 파일**이다 — 수동 레인
        // 기본 탐지 목록엔 타 역할 TODO가 섞일 수 있어, '전부' 강제가 되살아나면 단일 스레드
        // 쓰기 규율 위반을 지시문이 유도한다.
        assert!(
            d.contains("네 역할 소관 파일"),
            "역할 소관 한정 문구가 빠졌다: {d}"
        );
        // [codex R1 수용] (b) master 소관엔 SESSION_STATE 가 명시된다.
        assert!(
            d.contains("자기 TODO·자기 SESSION_STATE"),
            "master 의 SESSION_STATE 소관 문구가 빠졌다: {d}"
        );
        assert!(
            !d.contains("목록 **전부**"),
            "'전부 재기록' 강제가 되살아났다(수동 레인 타 역할 TODO 재기록 유도): {d}"
        );
        assert!(
            d.contains("plain 한 줄로 CYCLE-SAVED"),
            "② CYCLE-SAVED 마커 문장 계약이 깨졌다: {d}"
        );
        // 구 산문 회귀 차단 — 틸드 하드코딩·모호 이중 안내가 되살아나면 정합이 다시 깨진다.
        assert!(
            !d.contains("~/.cys/pack/round/<역할>_TODO.md"),
            "틸드 하드코딩 산문이 되살아났다: {d}"
        );
        assert!(
            !d.contains("_round/ 또는 pack round/"),
            "모호 이중 안내 산문이 되살아났다: {d}"
        );
        // [codex R1 수용] (a) 비master 소관은 결정론 문구다 — 자기 역할 TODO 파일만 쓰고,
        // 목록의 공유 SESSION_STATE·타 역할 TODO 는 감시(관찰) 대상일 뿐 쓰기 금지(단일 스레드
        // 쓰기 규율). LLM 해석 위임이 되살아나면 워커가 공유 SESSION_STATE 를 재기록한다.
        let wfiles = vec![
            "/tmp/pk/round/SESSION_STATE.md".to_string(),
            "/tmp/pk/round/WORKER_TODO.md".to_string(),
            "/tmp/pk/round/CSO_TODO.md".to_string(),
        ];
        let w = cycle_save_directive("worker", &wfiles);
        for f in &wfiles {
            assert!(w.contains(f.as_str()), "worker 지시문에서 감시 경로가 빠졌다: {f}\n{w}");
        }
        assert!(w.contains("자기 역할 TODO 파일만"), "worker 소관 한정 문구 소실: {w}");
        assert!(
            w.contains("쓰기 금지"),
            "SESSION_STATE·타 역할 TODO 쓰기 금지 문구 소실: {w}"
        );
        assert!(
            w.contains("plain 한 줄로 CYCLE-SAVED"),
            "worker 지시문의 ② CYCLE-SAVED 마커 계약 소실: {w}"
        );
        // 단일 파일 목록도 그대로 열거된다(워커 cycle).
        let one = vec!["/w/pack/round/WORKER_TODO.md".to_string()];
        assert!(cycle_save_directive("worker", &one).contains("/w/pack/round/WORKER_TODO.md"));
    }

    // ── E3/E8: cycle-agent 저장 검증 단계 ─────────────────────────────────────

    /// ★E3 회귀 핀: `--force-no-verify` 는 死플래그가 아니다.
    ///
    /// C2 폴백 이후 `files` 가 절대 비지 않게 되면서, 이 플래그의 유일한 소비처였던
    /// "빈 목록 거부" 분기는 도달 불능이 됐다 — 즉 플래그를 줘도 동작이 **하나도** 달라지지
    /// 않았다(저장이 불가능한 hang 상태에서 clear 를 강행할 비상 탈출구 소실). 지금은 검증
    /// 대기 자체를 건너뛴다.
    #[test]
    fn force_no_verify_skips_the_wait_even_when_files_exist() {
        // C2 이후의 실제 상태: 감시 파일이 있다.
        assert_eq!(cycle_verify_plan(false, 3), CycleVerifyPlan::Wait);
        assert_eq!(
            cycle_verify_plan(true, 3),
            CycleVerifyPlan::SkipForced,
            "파일이 있어도 플래그가 대기를 건너뛰지 못하면 死플래그가 재발한다"
        );
        // 파일이 없는 경우의 두 갈래도 구분된다(경고 문구가 사실과 어긋나지 않게).
        assert_eq!(cycle_verify_plan(false, 0), CycleVerifyPlan::SkipNoFiles);
        assert_eq!(cycle_verify_plan(true, 0), CycleVerifyPlan::SkipForced);
    }

    /// ★E8(N-5): 검증자에게 제시하는 본문에서 **미존재 기대 경로**는 빈 해시가 아니라 사실을
    /// 표기한다. `unwrap_or_default()` 는 미존재 파일 여럿을 전부 같은 빈 문자열로 만들어,
    /// 검증자가 "무엇을 확인해야 하는지"(존재 전이)를 알 수 없게 했다.
    #[test]
    fn handshake_body_states_missing_files_instead_of_empty_hash() {
        let line = handshake_file_line("/r/WORKER_TODO.md", None);
        assert!(
            line.contains("미생성") && !line.contains("sha256: )"),
            "미존재 경로를 빈 해시로 제시하면 검증자가 오판한다: {line}"
        );
        assert_eq!(
            handshake_file_line("/r/SESSION_STATE.md", Some("abc123".into())),
            "/r/SESSION_STATE.md (sha256: abc123)",
            "존재 파일의 표기는 종전 계약 그대로여야 한다"
        );
    }

    // ── W4-B(결함 7): 검증자 승인 영수증 검증 ────────────────────────────────

    /// ★회귀 핀: allow 는 **지정 검증자 pane 의 영수증**(resolver_surface==vsid)일 때만
    /// 통과한다. 종전 코드는 decision 문자열만 봤다 — CEO 자동결재·GUI Allow 버튼·제3자
    /// reply 가 전부 '검증자 승인'으로 통용되던 결함 7의 봉인이다. Err 는 전부 clear 미실행
    /// 중단으로 수렴하며, 기존 'timeout→clear 미실행' 경로는 run_cycle_agent 의
    /// receipt=None 분기가 그대로 유지한다(이 함수 밖 계약 — 주석 핀).
    #[test]
    fn cycle_receipt_requires_designated_verifier() {
        // 정상 영수증: resolved + allow + resolver==지정 검증자.
        let ok = json!({"status":"resolved","decision":"allow",
                        "resolver_surface":7,"resolver_pid":4242});
        assert!(cycle_receipt_ok(&ok, 7).is_ok());
        // allow 동의어(yes·approve)도 영수증만 맞으면 통과 — 종전 어휘 계약 보존.
        for d in ["yes", "approve"] {
            let it = json!({"status":"resolved","decision":d,"resolver_surface":7});
            assert!(cycle_receipt_ok(&it, 7).is_ok(), "'{d}' 어휘 계약 소실");
        }
        // 제3자 pane 스탬프 → 불일치 거부(두 surface 를 모두 사유에 명시).
        let third = json!({"status":"resolved","decision":"allow","resolver_surface":9});
        let e = cycle_receipt_ok(&third, 7).unwrap_err();
        assert!(
            e.contains("surface:9") && e.contains("surface:7"),
            "불일치 사유에 양측 surface 부재: {e}"
        );
    }

    /// 오진 3분류 핀(성찰 BLOCKER ③): 키 부재=구 데몬 / null+pid=pane 미귀속(GUI 토큰) /
    /// null+null=데몬 내부(stale)·채널 — 세 문구가 서로 구분돼야 운영자가 잘못된 처방
    /// (불필요한 데몬 재시작 등)을 받지 않는다.
    #[test]
    fn cycle_receipt_diagnoses_non_pane_resolutions_distinctly() {
        // ① 구 데몬: resolver_surface 키 자체가 없다(구 feed.list 직렬화).
        let old = json!({"status":"resolved","decision":"allow"});
        let e_old = cycle_receipt_ok(&old, 7).unwrap_err();
        assert!(
            e_old.contains("구 데몬") && e_old.contains("--force-no-verify"),
            "구 데몬 안내(재시작·비상 탈출구) 소실: {e_old}"
        );
        // ② pane 미귀속 해소: resolver_surface=null 이지만 resolver_pid 는 남는다
        //    (GUI operator 토큰 — state.rs '사실 그대로' 각인).
        let token = json!({"status":"resolved","decision":"allow",
                           "resolver_surface":null,"resolver_pid":4242});
        let e_token = cycle_receipt_ok(&token, 7).unwrap_err();
        assert!(e_token.contains("pane 미귀속"), "GUI 토큰 분류 소실: {e_token}");
        // ③ 데몬 내부·채널: 두 필드 모두 null(stale-clear·채널 미러 — 얇은 래퍼 경로).
        let internal = json!({"status":"resolved","decision":"allow",
                              "resolver_surface":null,"resolver_pid":null});
        let e_int = cycle_receipt_ok(&internal, 7).unwrap_err();
        assert!(
            e_int.contains("stale") || e_int.contains("채널"),
            "데몬 내부·채널 분류 소실: {e_int}"
        );
        // 세 문구는 서로 달라야 '3분류'다 — 하나로 뭉개지면 오진 재발.
        assert!(e_old != e_token && e_token != e_int && e_old != e_int);
    }

    /// 거부는 영수증 불요·즉시 안전 중단(allow 한정 검증 — is_self_approval 동일 원칙).
    /// GUI '알림 치우기'(dismissed)도 같은 경로로 수렴한다. 미해소 항목은 영수증이 아니다.
    #[test]
    fn cycle_receipt_rejects_deny_and_unresolved() {
        let deny = json!({"status":"resolved","decision":"deny","resolver_surface":7});
        assert!(cycle_receipt_ok(&deny, 7).unwrap_err().contains("거부"));
        let dismissed = json!({"status":"resolved","decision":"dismissed"});
        assert!(cycle_receipt_ok(&dismissed, 7).unwrap_err().contains("dismissed"));
        let pending = json!({"status":"pending","decision":null});
        assert!(cycle_receipt_ok(&pending, 7).is_err(), "미해소 항목이 영수증으로 통용됨");
        // decision null(형식 이상 해소)도 통과가 아니다 — 측정 불능은 통과가 아니다.
        let no_decision = json!({"status":"resolved","resolver_surface":7});
        assert!(cycle_receipt_ok(&no_decision, 7).is_err());
    }

    // ═══════════════ U-6 · RPC 왕복 상한 ═══════════════

    /// 상한 정책 진리표 — 즉답/장기/블로킹 3분류가 각자 다른 값을 받는다.
    /// ★개정 전 코드에서는 `rpc_idle_timeout` 자체가 없어 이 테스트가 컴파일되지 않는다
    /// (= 계측기 타당성: 결함 있는 코드에서 초록일 수 없다).
    #[test]
    fn rpc_idle_timeout_truth_table() {
        let secs = |m: &str, p: Value, env: Option<&str>| {
            rpc_idle_timeout_with(m, &p, env).map(|d| d.as_secs())
        };
        // ① 즉답 메서드 = 기본 상한
        assert_eq!(secs("system.ping", json!({}), None), Some(RPC_IDLE_TIMEOUT_SECS));
        assert_eq!(secs("surface.list", json!({}), None), Some(RPC_IDLE_TIMEOUT_SECS));
        // ② 연결 승격(장기 지속) = 면제. 여기 값이 Some 이 되면 구독이 끊긴다.
        assert_eq!(secs("events.stream", json!({}), None), None);
        assert_eq!(secs("surface.attach", json!({"surface_id": 1}), None), None);
        // ③ 서버 블로킹 = 선언 대기 + 마진
        assert_eq!(
            secs("surface.wait_for", json!({"timeout_secs": 600}), None),
            Some(600 + RPC_SERVER_WAIT_MARGIN_SECS)
        );
        assert_eq!(
            secs("feed.push", json!({"wait": true, "timeout_secs": 3600}), None),
            Some(3600 + RPC_SERVER_WAIT_MARGIN_SECS)
        );
        // ③-b 서버 기본값(120)을 클라이언트가 안 실어도 서버 대기보다 짧게 자르지 않는다
        assert_eq!(
            secs("feed.push", json!({"wait": true}), None),
            Some(120 + RPC_SERVER_WAIT_MARGIN_SECS)
        );
        // ③-b′ ★대기값 파싱 **비대칭** 축(2026-08-24): 서버 `param_u64`(cysd/handlers.rs)는
        //   `as_u64` 실패 시 `as_str().parse()` 로 **문자열도** 받는다. 종전 클라이언트는
        //   `as_u64()` 뿐이라, 외부 소비자가 `"3600"`(문자열)로 보내면 서버는 3600초 대기하고
        //   클라이언트는 기본 40초로 잘랐다 — **오너가 승인하기 전에 클라이언트가 먼저 끊는다**.
        for declared in [json!("3600"), json!(3600)] {
            assert_eq!(
                secs("feed.push", json!({"wait": true, "timeout_secs": declared}), None),
                Some(3600 + RPC_SERVER_WAIT_MARGIN_SECS),
                "선언 대기값 {declared} 을 서버와 다르게 읽는다(승인 전 절단)"
            );
            assert_eq!(
                secs("surface.wait_for", json!({"timeout_secs": declared}), None),
                Some(3600 + RPC_SERVER_WAIT_MARGIN_SECS),
                "surface.wait_for 에서 선언 대기값 {declared} 을 서버와 다르게 읽는다"
            );
        }
        // 서버가 못 읽는 형태(숫자도 숫자 문자열도 아님)는 클라이언트도 기본값으로 흐른다 —
        // 관용을 **서버보다 넓히면** 반대 비대칭(상한이 헐거워짐)이 생긴다.
        assert_eq!(
            secs("feed.push", json!({"wait": true, "timeout_secs": "nope"}), None),
            Some(120 + RPC_SERVER_WAIT_MARGIN_SECS),
            "서버가 못 읽는 값을 클라이언트가 읽었다(관용 비대칭이 반대로 뒤집혔다)"
        );
        // ③-c 서버 캡(3600) 초과 선언은 캡으로 접힌다 — 무한 대기 부활 금지
        assert_eq!(
            secs("feed.push", json!({"wait": true, "timeout_secs": 99999}), None),
            Some(RPC_SERVER_WAIT_CAP_SECS + RPC_SERVER_WAIT_MARGIN_SECS)
        );
        // ④ wait=false 의 feed.push 는 즉답(pending 응답) — 블로킹 취급하면 상한이 헐거워진다
        assert_eq!(
            secs("feed.push", json!({"wait": false, "timeout_secs": 3600}), None),
            Some(RPC_IDLE_TIMEOUT_SECS)
        );
        assert_eq!(secs("feed.push", json!({}), None), Some(RPC_IDLE_TIMEOUT_SECS));
    }

    /// 롤백 스위치 — `0` 은 상한 해제(개정 전 거동), 양수는 기본값 대체.
    /// 단 롤백 노브가 **블로킹 대기를 잘라먹지는 않는다**(새 사망 경로를 열지 않는다).
    #[test]
    fn rpc_idle_timeout_env_rollback() {
        let secs = |m: &str, p: Value, env: Option<&str>| {
            rpc_idle_timeout_with(m, &p, env).map(|d| d.as_secs())
        };
        assert_eq!(secs("system.ping", json!({}), Some("0")), None, "0 = 상한 해제");
        assert_eq!(secs("system.ping", json!({}), Some(" 5 ")), Some(5), "양수 = 기본 대체");
        assert_eq!(
            secs("system.ping", json!({}), Some("garbage")),
            Some(RPC_IDLE_TIMEOUT_SECS),
            "파싱 불가는 기본값 — 오타가 상한을 조용히 없애면 안 된다"
        );
        assert_eq!(
            secs("feed.push", json!({"wait": true, "timeout_secs": 600}), Some("5")),
            Some(600 + RPC_SERVER_WAIT_MARGIN_SECS),
            "env 로 기본을 낮춰도 서버 블로킹 대기는 잘리지 않는다"
        );
        // ── ★거대값 축(2026-08-24) — 노브가 CLI 를 죽이면 안 된다 ──
        // 음수·비숫자는 이미 안전했다(parse 실패 → 기본값 · 바로 위 세 줄이 박제). 위험은
        // **거대 양수** 하나: 이 값은 Windows 워치독에서 `Instant::now() + timeout` 이 되고,
        // std `Instant::add` 는 오버플로에서 **패닉**한다 — 상한을 늘리려는 손동작이 명령을 죽인다.
        assert_eq!(
            secs("system.ping", json!({}), Some("-5")),
            Some(RPC_IDLE_TIMEOUT_SECS),
            "음수는 종전대로 파싱 실패 → 기본값(이 축은 이미 안전했다)"
        );
        for giant in ["9223372036854775807", "18446744073709551615", "9300000000000000000"] {
            assert_eq!(
                secs("system.ping", json!({}), Some(giant)),
                Some(RPC_IDLE_TIMEOUT_MAX_SECS),
                "거대값 {giant} 이 클램프되지 않았다 — CLI 패닉 경로"
            );
        }
        // 선언 대기값 쪽은 이미 서버 캡(3600)으로 접히지만, **합류 결과**까지 유계인지 잰다.
        for giant in [json!(u64::MAX), json!("18446744073709551615")] {
            let got = secs("feed.push", json!({"wait": true, "timeout_secs": giant}), None);
            assert_eq!(got, Some(RPC_SERVER_WAIT_CAP_SECS + RPC_SERVER_WAIT_MARGIN_SECS));
        }
        // ★불변식 실측: 어떤 (method, params, env) 에서도 결과는 `Instant` 에 더할 수 있다.
        //   `checked_add` 가 `None` 이면 그것이 곧 워치독의 패닉 지점이다.
        let cases: [(&str, Value, Option<&str>); 8] = [
            ("system.ping", json!({}), None),
            ("system.ping", json!({}), Some("18446744073709551615")),
            ("system.ping", json!({}), Some("9223372036854775807")),
            ("system.ping", json!({}), Some("86401")),
            ("surface.wait_for", json!({"timeout_secs": u64::MAX}), None),
            ("surface.wait_for", json!({"timeout_secs": "18446744073709551615"}), None),
            ("feed.push", json!({"wait": true, "timeout_secs": u64::MAX}), Some("18446744073709551615")),
            ("feed.push", json!({"wait": false}), Some("9300000000000000000")),
        ];
        for (m, p, env) in cases {
            if let Some(d) = rpc_idle_timeout_with(m, &p, env) {
                assert!(
                    std::time::Instant::now().checked_add(d).is_some(),
                    "method={m} params={p} env={env:?} → {d:?} 이 Instant 오버플로를 만든다 \
                     (RpcWatchdog 의 `Instant::now() + timeout` 이 패닉한다)"
                );
            }
        }
    }

    /// 상한 만료 판정 — unix errno 계열과 Windows CancelIoEx 코드(995) 둘 다 잡는다.
    #[test]
    fn rpc_timeout_error_classification() {
        use std::io::{Error, ErrorKind};
        assert!(is_rpc_timeout_error(&Error::new(ErrorKind::WouldBlock, "x")));
        assert!(is_rpc_timeout_error(&Error::new(ErrorKind::TimedOut, "x")));
        // ERROR_OPERATION_ABORTED — Windows 워치독 경로. mac 에서도 판정만은 시험된다.
        assert!(is_rpc_timeout_error(&Error::from_raw_os_error(
            WIN_ERROR_OPERATION_ABORTED
        )));
        // 일반 I/O 오류는 상한 만료가 아니다(처방 문안이 달라야 한다)
        assert!(!is_rpc_timeout_error(&Error::new(ErrorKind::BrokenPipe, "x")));
        assert!(!is_rpc_timeout_error(&Error::new(ErrorKind::NotFound, "x")));
    }

    /// 조용한 실패 금지 — 만료 문안에 원인·대기시간·처방 4단·롤백 env 가 모두 들어간다.
    #[test]
    fn rpc_timeout_message_carries_prescription() {
        let m = rpc_timeout_message("system.ping", std::time::Duration::from_secs(40));
        assert!(m.starts_with("rpc_timeout:"), "기계 판독 가능한 접두가 없다: {m}");
        assert!(m.contains("system.ping"), "어떤 메서드인지 없다");
        assert!(m.contains("40초"), "얼마를 기다렸는지 없다");
        for step in ["처방 ①", "처방 ②", "처방 ③", "처방 ④"] {
            assert!(m.contains(step), "{step} 가 없다: {m}");
        }
        assert!(m.contains(ENV_RPC_TIMEOUT), "롤백 노브 안내가 없다");
    }

    /// 워치독 로직 — Windows arm 의 심장이지만 이 저장소에서 Windows 크로스 타입체크가
    /// 불가능(libsqlite3-sys 의 C 크로스 빌드 실패)하므로 로직만은 mac CI 에서 박제한다.
    /// 검증 불가로 남는 것은 취소 FFI 호출부(`CancelIoEx` + `CancelSynchronousIo`)이며,
    /// 그 부분은 **Windows 실기 미검증**이다(소스 핀은 문자열 존재만 확인한다).
    #[test]
    fn rpc_watchdog_fires_defers_and_stops() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        // ① 무진행이면 상한 **전에는 침묵**하고 상한 **뒤에는 발화**한다.
        //    (발화 후에도 감시를 계속하므로 횟수는 1 고정이 아니라 '창 수' 규모다.)
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        let wd = RpcWatchdog::new(Duration::from_millis(200), move || {
            h.fetch_add(1, Ordering::SeqCst);
        });
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "상한 전에 발화했다 — 정상 왕복을 자르는 방향"
        );
        std::thread::sleep(Duration::from_millis(500));
        let fired = hits.load(Ordering::SeqCst);
        assert!(fired >= 1, "무진행인데 발화하지 않았다(= 무한 대기 부활)");
        assert!(fired <= 10, "발화가 폭주했다({fired}회) — 감시자가 스핀한다");
        drop(wd);

        // ② touch(진행)가 상한을 재장전한다 — 정상 전송 중인 큰 응답이 잘리지 않는 근거
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        let wd = RpcWatchdog::new(Duration::from_millis(200), move || {
            h.fetch_add(1, Ordering::SeqCst);
        });
        for _ in 0..8 {
            std::thread::sleep(Duration::from_millis(60));
            wd.touch();
        }
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "진행이 계속됐는데 발화했다(정상 왕복을 자르는 방향 = 팀 파괴)"
        );
        drop(wd);

        // ③ drop 이후에는 절대 발화하지 않는다 — 닫힌 핸들에 CancelIoEx 가 나가면 안 된다
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        let wd = RpcWatchdog::new(Duration::from_millis(150), move || {
            h.fetch_add(1, Ordering::SeqCst);
        });
        drop(wd); // Drop 이 stop 신호 + join 까지 한다
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(hits.load(Ordering::SeqCst), 0, "drop 후 발화 = use-after-close 경로");
    }

    /// read_frame_line: 개행까지 한 줄 · EOF/상한 만료를 3분류로 구분.
    #[test]
    fn read_frame_line_classifies_outcomes() {
        struct Src(Vec<Result<Vec<u8>, std::io::Error>>);
        impl Read for Src {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                match self.0.pop() {
                    None => Ok(0),
                    Some(Err(e)) => Err(e),
                    Some(Ok(b)) => {
                        buf[..b.len()].copy_from_slice(&b);
                        Ok(b.len())
                    }
                }
            }
        }
        let dl = RpcDeadline {
            idle: Some(std::time::Duration::from_secs(1)),
            #[cfg(windows)]
            watchdog: None,
        };
        // 여러 조각에 걸친 한 줄 — 조립돼야 한다(pop 이라 역순으로 넣는다)
        let mut s = Src(vec![
            Ok(b"\"ok\":true}\n".to_vec()),
            Ok(b"{".to_vec()),
        ]);
        assert_eq!(read_frame_line(&mut s, &dl).unwrap(), "{\"ok\":true}");
        // EOF
        let mut s = Src(vec![]);
        assert_eq!(read_frame_line(&mut s, &dl), Err(RpcIoFail::Eof));
        // 상한 만료
        let mut s = Src(vec![Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "timed out",
        ))]);
        assert_eq!(read_frame_line(&mut s, &dl), Err(RpcIoFail::Timeout));
        // 그 외 I/O 오류는 만료로 뭉개지 않는다
        let mut s = Src(vec![Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "gone",
        ))]);
        assert!(matches!(read_frame_line(&mut s, &dl), Err(RpcIoFail::Io(_))));
    }

    /// ★e2e: **응답하지 않는 소켓 목**에서 유계 종료한다.
    /// 개정 전 `request_on_timeout` 의 unix arm 은 이미 통과하지만, 이 테스트의 값은
    /// (a) 회귀 박제와 (b) 만료 귀결이 '조용한 실패'가 아니라 처방 문안이라는 계약에 있다.
    #[cfg(unix)]
    #[test]
    fn request_on_timeout_terminates_on_hung_socket() {
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!(
            "cys-u6-hung-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("hung.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        // accept 는 하되 한 바이트도 쓰지 않는다 = 데몬 wedge 모사.
        let keep = std::thread::spawn(move || {
            let held: Vec<_> = listener.incoming().take(1).filter_map(|s| s.ok()).collect();
            std::thread::sleep(Duration::from_secs(3));
            drop(held);
        });
        let t0 = Instant::now();
        let r = request_on_timeout(&sock, "system.ping", json!({}), Duration::from_millis(400));
        let elapsed = t0.elapsed();
        let err = r.expect_err("무응답 소켓에서 Ok 가 나왔다");
        assert!(
            elapsed < Duration::from_secs(3),
            "유계 종료 실패 — {elapsed:?} 걸렸다(개정 전 무타임아웃 거동 부활)"
        );
        assert!(
            err.starts_with("rpc_timeout:"),
            "만료가 조용한/모호한 실패로 새어나갔다: {err}"
        );
        assert!(err.contains("처방 ①"), "처방 문안이 없다: {err}");
        let _ = keep.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★계측 타당성(negation): **상한을 끄면 같은 목에서 유계 종료하지 않는다.**
    /// 위 테스트의 400ms 종료가 '소켓이 원래 빨리 끊겨서'가 아니라 **상한이 일한 결과**임을
    /// 증명한다(계측기 검증 3칙 — 계측기 자신을 먼저 시험한다). 롤백 노브(`=0`)의 실효 확인이기도 하다.
    #[cfg(unix)]
    #[test]
    fn no_deadline_means_no_bound_instrument_validity() {
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!(
            "cys-u6-nobound-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("hung.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        // 1.2초 동안 무응답으로 붙잡았다가 끊는다(끊김이 유일한 탈출구 = 상한 부재의 실체).
        let keep = std::thread::spawn(move || {
            let held: Vec<_> = listener.incoming().take(1).filter_map(|s| s.ok()).collect();
            std::thread::sleep(Duration::from_millis(1200));
            drop(held);
        });
        let mut stream = std::os::unix::net::UnixStream::connect(&sock).unwrap();
        let deadline = RpcDeadline::arm(&stream, None).unwrap(); // 상한 해제 = 개정 전 거동
        let t0 = Instant::now();
        let r = rpc_roundtrip(&mut stream, &deadline, "system.ping", json!({}));
        let elapsed = t0.elapsed();
        drop(deadline);
        assert!(r.is_err(), "무응답 소켓인데 Ok 가 나왔다");
        assert!(
            elapsed >= Duration::from_millis(900),
            "상한이 없는데도 {elapsed:?} 에 끊겼다 — 위 유계 종료 테스트가 상한을 시험하지 못한다"
        );
        let _ = keep.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 소스 핀 공용 절단기 — `marker` 로 시작하는 항목의 본문을 **문자 경계 안전**하게 돌려준다.
    ///
    /// 【고친 결함 P2-1】 종전 핀은 `&src[head..head + 1200]` 처럼 **바이트 오프셋 산술**로
    /// 잘랐다. 지금은 우연히 경계에 맞지만, 그 구간의 한글 주석을 **한 글자만 고쳐도** 오프셋이
    /// 멀티바이트 문자 한가운데로 밀려 `byte index is not a char boundary` **패닉**으로 CI 가
    /// 적색이 된다 — 결함이 아니라 편집으로. 상한을 **다음 항목 경계**(`\nfn `)로 잡으면 그
    /// 실패 양식 자체가 사라지고, 창이 함수 전체를 덮으므로 **검사 내용은 오히려 넓어진다**
    /// (핀 약화 아님 — `streaming_paths_bypass_the_deadline_source_pin` 과 같은 방식).
    fn item_body<'a>(src: &'a str, marker: &str) -> &'a str {
        let h = src
            .find(marker)
            .unwrap_or_else(|| panic!("{marker} 가 사라졌다"));
        // 자기 자신의 `fn` 줄을 경계로 세지 않도록 marker 길이만큼 지나서 찾는다.
        let from = h + marker.len();
        let end = src[from..]
            .find("\nfn ")
            .map(|e| from + e)
            .unwrap_or(src.len());
        &src[h..end]
    }

    /// ★소스 핀: Windows arm 이 **no-op 이 아니다**.
    /// 이 저장소는 Windows 크로스 타입체크가 불가능해 CI 가 mac/ubuntu 에서만 돈다 — 그래서
    /// "Windows 에서는 상한이 없다"는 종전 결함이 리팩터로 되살아나도 아무도 모른다.
    /// 이 핀이 그 회귀를 잡는다. ★개정 전 소스에서는 적색이다(당시 Windows arm 은
    /// `request_on(socket, method, params)` 위임 = 상한 0).
    #[test]
    fn windows_arm_is_not_a_noop_source_pin() {
        let src = include_str!("cys.rs");
        // ① Windows RpcDeadline::arm 이 실제로 취소 API 를 **둘 다** 부른다
        //    (한쪽만 남으면 그 한쪽이 안 먹는 환경에서 상한이 통째로 no-op 이 된다)
        assert!(
            src.contains("CancelIoEx(raw as HANDLE"),
            "Windows arm 에 CancelIoEx 호출이 없다 — 상한이 no-op 으로 되돌아갔다"
        );
        assert!(
            src.contains("CancelSynchronousIo(th)"),
            "동기 I/O 취소의 정식 API(CancelSynchronousIo) 호출이 없다"
        );
        // ② request_on_timeout 에 '위임으로 상한을 버리는' 구 형태가 남아있지 않다
        let body = item_body(src, "fn request_on_timeout(");
        assert!(
            !body.contains("request_on(socket, method, params)"),
            "request_on_timeout 이 다시 무타임아웃 request_on 으로 위임한다(Windows no-op 부활)"
        );
        assert!(
            body.contains("RpcDeadline::arm"),
            "request_on_timeout 이 공용 상한 기구를 쓰지 않는다"
        );
        // ③ request() 가 상한을 장전한다
        let rbody = item_body(src, "\nfn request(method: &str");
        assert!(
            rbody.contains("RpcDeadline::arm") && rbody.contains("rpc_idle_timeout"),
            "request() 에 상한 장전이 없다 — 데몬 wedge 시 CLI 영구 대기가 부활한다"
        );
        // ④ ★핀 이사(2026-08-24): **부서 fan-out 도** 같은 기구를 탄다.
        //    종전 `request_on` 은 전용 와이어 로직의 무상한 `read_line` 이라, 부서 데몬이
        //    accept 후 wedge 되면 `cys org status` 가 영구 정지했다(A1-F2 와 같은 클래스).
        //    "무진행 상한 단위가 닫혔다"고 말하려면 이 경로도 닫혀 있어야 한다.
        let obody = item_body(src, "\nfn request_on(socket: &std::path::Path");
        assert!(
            obody.contains("RpcDeadline::arm") && obody.contains("rpc_idle_timeout"),
            "request_on(부서 fan-out)이 상한 기구를 타지 않는다 — hung 부서 소켓에서 영구 정지"
        );
        assert!(
            obody.contains("rpc_roundtrip("),
            "부서 fan-out 이 공용 왕복 본체를 쓰지 않는다 — 와이어 로직이 두 벌이 된다"
        );
        // 그리고 무상한 전용 와이어 로직은 트리에서 **사라졌다**(되살아나면 구멍도 함께 온다).
        // needle 은 조립한다 — 이 테스트 소스 자체가 스캔 대상(`cys.rs`)이라, 리터럴로 두면
        // 자기 자신을 찾아 영구 적색이 된다(계측기가 자기를 재는 사고).
        let gone = concat!("fn ", "rpc_over");
        assert_eq!(
            src.matches(gone).count(),
            0,
            "무상한 전용 와이어 로직이 되살아났다 — 상한 없는 두 번째 왕복 경로"
        );
    }

    /// ★클라이언트↔서버 **대기값 파싱 관용 파리티**(2026-08-24).
    ///
    /// 서버(`cysd/handlers.rs::param_u64`)는 `as_u64` 실패 시 `as_str().parse()` 로 문자열도
    /// 받는다. 클라이언트가 그보다 좁으면 "서버는 기다리는데 클라이언트가 먼저 끊는다"가 되고,
    /// 넓으면 "클라이언트가 서버보다 오래 기다린다"(상한이 헐거워짐)가 된다. 어느 쪽도 무음이라
    /// 진리표만으로는 **서버가 바뀌는 날**을 못 잡는다 — 그래서 서버 소스를 직접 대조한다.
    #[test]
    fn client_matches_server_param_leniency_source_pin() {
        let handlers = include_str!("cysd/handlers.rs");
        let h = handlers
            .find("fn param_u64(")
            .expect("서버 param_u64 소실 — 파리티 대상이 사라졌다");
        let hbody = &handlers[h..handlers[h..].find("\n}\n").map(|e| h + e).unwrap_or(handlers.len())];
        assert!(
            hbody.contains("as_u64()") && hbody.contains("as_str().and_then(|s| s.parse().ok())"),
            "서버 param_u64 의 관용이 바뀌었다 — 클라이언트 `declared()` 도 함께 맞춰라:\n{hbody}"
        );
        let src = include_str!("cys.rs");
        let c = src
            .find("fn rpc_server_wait_secs(")
            .expect("클라이언트 대기값 판정부 소실");
        let cbody = &src[c..src[c..].find("\n}\n").map(|e| c + e).unwrap_or(src.len())];
        assert!(
            cbody.contains("v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))"),
            "클라이언트가 서버와 다른 관용으로 대기값을 읽는다(오너 승인 전 절단 경로):\n{cbody}"
        );
    }

    /// 장기 경로가 상한 기구에 **닿지 않는다**는 것을 소스로 못 박는다(구독 절단 방지).
    /// `stream_events`·`attach` 는 각자 `connect()` 를 쓰며 `request()` 를 타지 않는다.
    #[test]
    fn streaming_paths_bypass_the_deadline_source_pin() {
        let src = include_str!("cys.rs");
        for (marker, name) in [
            ("fn stream_events(", "events.stream"),
            ("fn attach(sid: u64)", "surface.attach"),
        ] {
            let h = src.find(marker).unwrap_or_else(|| panic!("{marker} 가 사라졌다"));
            let end = src[h..].find("\nfn ").map(|e| h + e).unwrap_or(src.len());
            let body = &src[h..end];
            assert!(
                !body.contains("RpcDeadline::arm"),
                "{name} 경로에 상한이 걸렸다 — 장기 구독이 잘린다"
            );
            assert!(
                !body.contains("request("),
                "{name} 경로가 request() 를 타면 전역 상한이 따라붙는다"
            );
        }
        // 목록 자체도 핀 — 면제 집합이 조용히 비지 않게
        assert_eq!(RPC_STREAMING_METHODS, ["events.stream", "surface.attach"]);
    }
}
