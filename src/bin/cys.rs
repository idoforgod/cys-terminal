//! cys — the CYSJavis terminal CLI client. 모든 pane 안의 AI가 이 CLI로 동등 노드가 된다.
//! 예: cys send --surface surface:31 "..." ; cys send-key --surface surface:31 Return

use clap::{Parser, Subcommand};
use cys::{key_to_bytes, parse_surface_ref, socket_path, surface_ref, ENV_SURFACE_ID};
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
    Drain,
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
    CloseSurface { surface: String },
    /// Subscribe to the daemon event stream (push; no polling)
    Events {
        #[arg(long)]
        after_seq: Option<u64>,
        #[arg(long = "name")]
        names: Vec<String>,
        #[arg(long = "category")]
        categories: Vec<String>,
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
    /// Boot the standard node set — 설치된 CLI만 자동 감지·기동·지침 주입 (worker claude + reviewer agy/codex/grok). CSO는 3단 정책상 띄우지 않는다
    Boot {
        /// Working directory for launched nodes
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Print (creating if absent) this surface's role-specific TODO file path — 복수 워커가 같은 파일을 공유하지 않도록 역할별 고유 경로를 결정론적으로 산출
    TodoPath,
    /// Print this surface's cysd-authoritative role (one word) — PreToolUse capability-gate hook용.
    /// CYS_SURFACE_ID로 자기 surface를 찾아 데몬 roles 맵의 role을 출력(미등록 시 빈 줄·exit 0).
    SurfaceRole,
    /// HMAC signed-prefix 승인 — 위험명령 prefix를 1회 서명하면 이후 자동 통과(guard.sh 연동)
    Approval {
        #[command(subcommand)]
        action: ApprovalAction,
    },
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
    },
    /// Drop all undelivered queued messages for a surface
    Clear { surface: String },
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

#[derive(Subcommand)]
enum SkillAction {
    /// Create a new skill from experience (SKILL.md, 4-칸 본문 템플릿)
    New {
        /// kebab-case skill name
        name: String,
        #[arg(long)]
        description: String,
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
    },
}

fn main() {
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

/// 지침·과업 텍스트의 표준 주입: bracketed paste → 0.8s → Return
fn inject_text(sid: u64, text: &str) -> Result<(), String> {
    let wrapped = format!("\x1b[200~{text}\x1b[201~");
    // authoritative: 디렉티브·과업 주입은 타이핑 가드를 면제한다 — 막 기동한 에이전트
    // pane에 사람 미완성 입력이 없고, GUI 활성 pane의 사람-입력 잔향이 주입을 영구
    // 차단하던 경로(human is typing 무한)를 끊는다. ACL은 데몬에서 그대로 집행된다.
    request(
        "surface.send_text",
        json!({"surface_id": sid, "text": wrapped, "quiet": true, "authoritative": true}),
    )?;
    std::thread::sleep(std::time::Duration::from_millis(800));
    request(
        "surface.send_key",
        json!({"surface_id": sid, "key": "Return", "authoritative": true}),
    )?;
    Ok(())
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

#[cfg(windows)]
fn connect_raw() -> Result<std::fs::File, String> {
    let path = socket_path();
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
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

/// 데몬을 분리 세션으로 기동 — CLI가 Ctrl-C로 죽어도 데몬은 살아남는다.
fn spawn_detached_daemon(path: &std::path::Path) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(path);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: 데몬에 콘솔 창을 붙이지 않는다(검은 빈 창·ConPTY 오염 방지).
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map(|_| ())
}

/// 온보딩④: 연결 실패 시 형제 cysd를 자동 기동 후 재시도 — 신규 머신 zero-setup.
/// 옵트아웃: CYS_NO_AUTOSTART=1. (데몬 중복 기동은 cysd 자체의 flock이 차단)
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
            let Some(daemon) = sibling_daemon_path() else {
                return Err(format!("{first} (no sibling cysd to autostart)"));
            };
            eprintln!("[cys] cysd not running — autostarting {}", daemon.display());
            if spawn_detached_daemon(&daemon).is_err() {
                return Err(first);
            }
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if let Ok(s) = connect_raw() {
                    return Ok(s);
                }
            }
            Err(format!("{first} (autostarted cysd did not come up within 4s)"))
        }
    }
}

#[cfg(unix)]
type ConnStream = std::os::unix::net::UnixStream;
#[cfg(windows)]
type ConnStream = std::fs::File;

fn request(method: &str, params: Value) -> Result<Value, String> {
    let mut stream = connect()?;
    let req = json!({"id": 1, "method": method, "params": params});
    let mut line = serde_json::to_string(&req).unwrap();
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut resp_line = String::new();
    reader
        .read_line(&mut resp_line)
        .map_err(|e| e.to_string())?;
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

// ---------- commands ----------

fn run(command: Command) -> i32 {
    let result = match command {
        Command::Ping => request("system.ping", json!({})).map(|r| println!("{}", r.as_str().unwrap_or("pong"))),

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
                for sid in sids {
                    // T3-13 권위 전달: clear_first는 데몬이 원자적으로(Ctrl-U 선정리 → paste → CR)
                    // 집행한다. 클라측 C-u·150ms sleep·게이트는 제거 — 비원자 split·race를 없앤다.
                    // agent 등록 pane 게이트는 데몬 send_text가 집행(clear_first_unsupported).
                    let r = request(
                        "surface.send_text",
                        json!({"surface_id": sid, "text": text.join(" "), "from": from, "queued": queued, "clear_first": clear_first}),
                    )?;
                    let tag = if multi { format!(" → surface:{sid}") } else { String::new() };
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
                for sid in sids {
                    for key in &keys {
                        let r = request(
                            "surface.send_key",
                            json!({"surface_id": sid, "key": key, "queued": queued}),
                        )?;
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
                    if multi {
                        println!("OK → surface:{sid}");
                    }
                }
                if !multi && !queued {
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

        Command::Status { json: as_json } => return run_status(as_json),
        Command::Fleet { json: as_json } => return run_fleet(as_json),

        Command::Pause { reason } => request("system.pause", json!({"reason": reason}))
            .map(|_| println!("PAUSED — 큐 배달·스케줄 발화 동결 (이미 실행 중인 에이전트 행동은 계속된다; cys resume로 해제)")),

        Command::Resume => request("system.resume", json!({}))
            .map(|_| println!("RESUMED — 동결된 큐·스케줄 재개")),

        Command::Drain => {
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
                            let _ = inject_text(sid, "[DRAIN] 업데이트 재시작이 임박했다. 승인 프롬프트 대기 중이면 이 메시지는 무시하라. 아니면 지금 _round/SESSION_STATE.md와 자기 TODO를 저장하고 작업을 멈춰라. 작업 재개는 복원 후 master 지시를 기다린다.");
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
                QueueAction::List { surface } => parse_explicit_surface(&surface)
                    .and_then(|sid| request("queue.list", json!({"surface_id": sid})))
                    .map(|r| {
                        let entries = r["entries"].as_array().cloned().unwrap_or_default();
                        if entries.is_empty() {
                            println!("(queue empty)");
                        }
                        for e in entries {
                            println!(
                                "{}\t[{}]\t{}B\t{}",
                                e["surface_ref"].as_str().unwrap_or("?"),
                                e["index"],
                                e["bytes"],
                                e["preview"].as_str().unwrap_or(""),
                            );
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

        Command::PackManifest { key_id, signed_at, expires_at, min_binary_version } => {
            return run_pack_manifest(key_id, signed_at, expires_at, &min_binary_version);
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

        Command::ClaimRole { role, surface } => target_surface(&surface, &None).and_then(|sid| {
            request("system.claim_role", json!({"role": role, "surface_id": sid}))
                .map(|r| println!("registered: {} → surface:{}", r["role"].as_str().unwrap_or("?"), sid))
        }),

        Command::LaunchAgent { role, agent, cwd } => return run_launch_agent(&role, &agent, cwd),
        Command::Boot { cwd } => return run_boot(cwd),
        Command::TodoPath => return run_todo_path(),

        Command::SurfaceRole => return run_surface_role(),

        Command::Resize { surface, rows, cols } => target_surface(&surface, &None).and_then(|sid| {
            request("surface.resize", json!({"surface_id": sid, "rows": rows, "cols": cols}))
                .map(|_| println!("OK"))
        }),

        Command::CloseSurface { surface } => parse_surface_ref(&surface)
            .ok_or_else(|| format!("invalid surface ref: {surface}"))
            .and_then(|sid| {
                request("surface.close", json!({"surface_id": sid})).map(|r| {
                    println!("closed {} (descendants killed)", surface);
                    let _ = r;
                })
            }),

        Command::Events { after_seq, names, categories, reconnect, cursor_file } => {
            stream_events(after_seq, names, categories, reconnect, cursor_file)
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
        FeedAction::Push { kind, title, body, surface, request_id, wait, timeout_secs } => {
            parse_explicit_surface(&surface)
                .and_then(|explicit| {
                    let sid = explicit
                        .or_else(|| cys::env_compat(ENV_SURFACE_ID).and_then(|s| parse_surface_ref(&s)));
                    request(
                        "feed.push",
                        json!({"kind": kind, "title": title, "body": body, "surface_id": sid,
                               "request_id": request_id, "wait": wait, "timeout_secs": timeout_secs}),
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
        FeedAction::Reply { request_id, decision } => {
            request("feed.reply", json!({"request_id": request_id, "decision": decision}))
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
    reconnect: bool,
    cursor_file: Option<String>,
) -> Result<(), String> {
    // (3) 시드: --after_seq 미지정이면 cursor-file에서 읽는다(cmux Events.swift:25-27).
    let mut last_seq = after_seq.or_else(|| {
        cursor_file
            .as_ref()
            .and_then(|p| read_event_cursor(p).ok().flatten())
    });
    loop {
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
                if let Ok(v) = serde_json::from_str::<Value>(&l) {
                    match v["type"].as_str() {
                        Some("event") => {
                            if let Some(seq) = v["seq"].as_u64() {
                                last_seq = Some(seq);
                                if let Some(cf) = &cursor_file {
                                    write_event_cursor(cf, seq)?; // (3) 매 이벤트 원자적 갱신
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
                println!("{l}");
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
        SkillAction::New { name, description } => (|| {
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err("name must be kebab-case ascii (a-z0-9-)".into());
            }
            let dir = skills_dir.join(&name);
            let path = dir.join("SKILL.md");
            if path.exists() {
                return Err(format!("skill '{name}' already exists: {}", path.display()));
            }
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
            println!("created {}", path.display());
            println!("(4칸을 채우고, master 승인이 필요하면 feed push로 보고하라)");
            Ok(())
        })(),
        SkillAction::List => (|| {
            let entries = std::fs::read_dir(&skills_dir).map_err(|_| {
                format!(
                    "no skills dir: {} (run cys init-pack)",
                    skills_dir.display()
                )
            })?;
            let mut count = 0;
            for entry in entries.flatten() {
                let skill_md = entry.path().join("SKILL.md");
                let Ok(content) = std::fs::read_to_string(&skill_md) else {
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
                    println!("{name}\t{desc}");
                    count += 1;
                }
            }
            if count == 0 {
                println!("(no skills yet — `cys skill new <name> --description \"...\"`)");
            }
            Ok(())
        })(),
        SkillAction::Show { name } => (|| {
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err("name must be kebab-case ascii (a-z0-9-)".into());
            }
            let path = skills_dir.join(&name).join("SKILL.md");
            let content = std::fs::read_to_string(&path)
                .map_err(|_| format!("no skill '{name}' ({})", path.display()))?;
            println!("{content}");
            Ok(())
        })(),
        SkillAction::Run { name, ticket, agent, close_after } => (|| {
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err("name must be kebab-case ascii (a-z0-9-)".into());
            }
            if ticket.trim().is_empty() {
                return Err("ticket 비어 있음 — 무계약 실행 금지(task-prompt 경유 필수)".into());
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
                        Some(chrono::Local::now().timestamp() + secs as i64)
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
    let (written, kept) = match cys::pack::install(force) {
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

/// Claude Code 설정 파일 자동 탐색: $HOME 직하의 `.claude*` 디렉터리에 있는 settings.json 전부.
/// (멀티 프로필 환경 — 예: .claude / .claude-* — 을 한 번에 커버.)
/// 결정론: 존재하는 파일만, 사전순 정렬로 반환한다.
fn discover_claude_settings() -> Vec<String> {
    let Some(home) = dirs::home_dir() else {
        return vec![];
    };
    let Ok(entries) = std::fs::read_dir(&home) else {
        return vec![];
    };
    let mut found: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n == ".claude" || n.starts_with(".claude-"))
                .unwrap_or(false)
        })
        .map(|e| e.path().join("settings.json"))
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

/// SessionStart hook으로 등록할 명령 문자열을 OS별로 조립한다 (순수 함수 — 회귀 핀).
///
/// Unix: 기존과 동일하게 `sh <path>/session-start.sh`.
/// Windows: 바닐라 Windows 셸(cmd/PowerShell)은 `.sh`를 인터프리터 없이 실행하지 못하고
///   "open with" 대화상자를 띄운다(anthropics/claude-code #21847·#24097). Claude Code가
///   Windows에서 찾는 인터프리터는 Git Bash의 `bash`이므로, 바 셸이 해석할 수 있도록
///   `bash` 명령으로 명시 호출한다(맨 이름 `sh`는 Git Bash가 `bash.exe`만 보장하므로 회피).
///   `/clear` 후 SessionStart 자동 재주입(autopilot 축2)이 Windows에서도 발동하게 하는 핵심.
fn hook_command(pack_dir: &std::path::Path) -> String {
    // RC-2: 공용 함수로 위임 — 격리 config dir(pack.rs setup_isolated_config_dir)과 init-pack이
    // 동일 OS 분기를 쓰게 단일화(중복 제거·불일치 차단).
    cys::pack::session_start_hook_command(pack_dir)
}

/// Claude Code settings.json에 SessionStart hook을 등록한다 (백업 생성, 중복 등록 방지).
fn install_claude_hook(settings_path: &str, pack_dir: &std::path::Path) -> Result<String, String> {
    // symlink 거부 — 링크 너머 실파일을 덮어쓰는 TOCTOU 부류 차단(preflight와 동일 규약).
    if std::fs::symlink_metadata(settings_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(format!("{settings_path} is a symlink — refusing to write"));
    }
    let hook_cmd = hook_command(pack_dir);
    let mut root: Value = match std::fs::read_to_string(settings_path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("settings parse error: {e}"))?,
        // 파일 없음일 때만 빈 설정으로 시작 — 권한 등 다른 읽기 에러를 무시하면
        // 기존 settings.json이 hooks만 남은 JSON으로 대체될 수 있다
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(format!("settings read error: {e}")),
    };
    let hooks = root
        .as_object_mut()
        .ok_or("settings root is not an object")?
        .entry("hooks")
        .or_insert(json!({}));
    let session_start = hooks
        .as_object_mut()
        .ok_or("hooks is not an object")?
        .entry("SessionStart")
        .or_insert(json!([]));
    let arr = session_start
        .as_array_mut()
        .ok_or("SessionStart is not an array")?;
    let already = arr.iter().any(|m| {
        m["hooks"]
            .as_array()
            .map(|hs| {
                hs.iter()
                    .any(|h| h["command"].as_str() == Some(hook_cmd.as_str()))
            })
            .unwrap_or(false)
    });
    if already {
        return Ok("hook already installed (skipped)".into());
    }
    // backup — RC-1(D2 master 조건): 실제 write가 발생할 때만 백업한다. `already` 체크 앞에서
    // 백업하면 온보딩이 매 기동 init-pack을 호출할 때(멱등) 정상 상태 .bak-cys가 매번 클로버돼
    // "정상 백업"이 소실된다(적대검증 serious). already→skip 경로는 백업을 건드리지 않는다.
    if std::path::Path::new(settings_path).exists() {
        let backup = format!("{settings_path}.bak-cys");
        std::fs::copy(settings_path, &backup).map_err(|e| e.to_string())?;
    }
    arr.push(json!({"hooks": [{"type": "command", "command": hook_cmd}]}));
    std::fs::write(settings_path, serde_json::to_string_pretty(&root).unwrap())
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "SessionStart hook registered in {settings_path} (backup: .bak-cys)"
    ))
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
fn run_boot(cwd: Option<String>) -> i32 {
    // (역할, 에이전트) 표준 편성 — 4차 의무 4종 + 선택 grok. 순서: CSO 먼저(감독).
    const PLAN: &[(&str, &str)] = &[
        ("cso", "claude"),
        ("worker", "claude"),
        ("reviewer-gemini", "gemini"),
        ("reviewer-codex", "codex"),
        ("reviewer-grok", "grok"),
    ];
    let agents: Value = std::fs::read_to_string(cys::pack::pack_dir().join("agents.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    // 이미 가동 중인 역할은 중복 기동하지 않는다
    let live_roles: std::collections::HashSet<String> = request("surface.list", json!({}))
        .ok()
        .and_then(|r| r["surfaces"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter(|s| !s["exited"].as_bool().unwrap_or(true))
        .filter_map(|s| s["role"].as_str().map(|x| x.to_string()))
        .collect();
    let mut launched = 0;
    let mut failed = 0;
    println!("cys boot — LLM orchestrating 편성 점검 (CSO·worker·agy·codex 4종 의무 + grok 선택)");
    for (role, agent) in PLAN {
        let bin = agents
            .get(*agent)
            .and_then(|a| a["cmd"].as_str())
            // env-prefix를 건너뛰고 실제 바이너리 토큰을 찾는다 (extract_bin 단일 진실) — claude
            // cmd가 `CLAUDE_CONFIG_DIR="..." claude ...`처럼 env 대입으로 시작해 첫 토큰을 바이너리로
            // 오판('미설치')하던 회귀를 차단한다 (gemini/codex는 바이너리로 시작해 영향 없음).
            .map(|c| extract_bin(c, agent).to_string())
            .unwrap_or_else(|| agent.to_string());
        // 경로형 cmd('~/'·'/' 포함 — 예: agy 절대경로)는 which/where가 틸드를 확장하지
        // 않아 '미설치'로 오판한다 → 파일 존재로 판정 (실행은 셸 -lc 경유라 틸드 확장됨)
        let found = if bin.starts_with('~') || bin.contains('/') {
            expand_tilde(&bin).exists()
        } else {
            #[cfg(windows)]
            let ok = std::process::Command::new("where")
                .arg(&bin)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            #[cfg(not(windows))]
            let ok = std::process::Command::new("which")
                .arg(&bin)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            ok
        };
        if !found {
            println!("· {agent}: CLI '{bin}' 미설치 — 건너뜀");
            continue;
        }
        if live_roles.contains(*role) {
            println!("· {agent}: 역할 '{role}' 이미 가동 중 — 건너뜀");
            continue;
        }
        println!("· {agent}: 기동 시작 (role={role})…");
        if run_launch_agent(role, agent, cwd.clone()) == 0 {
            launched += 1;
        } else {
            failed += 1;
            println!("· {agent}: 기동 실패 — 나머지 노드는 계속 진행");
        }
    }
    println!(
        "boot 완료: 신규 기동 {launched} · 실패 {failed} · 현황은 `cys list`로 확인 (role 열)"
    );
    if failed > 0 {
        1
    } else {
        0
    }
}

/// agents.json에서 어댑터 스펙 로드
fn load_agent_spec(agent: &str) -> Result<Value, String> {
    let agents_path = cys::pack::pack_dir().join("agents.json");
    let agents: Value = std::fs::read_to_string(&agents_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .ok_or_else(|| {
            format!(
                "agents.json not found at {} — run `cys init-pack` first",
                agents_path.display()
            )
        })?;
    agents
        .get(agent)
        .cloned()
        .ok_or_else(|| format!("unknown agent '{agent}' (agents.json에 정의 필요)"))
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
    // 스킬 색인(표지) 동봉 — 본문은 필요 시 `cys skill show <name>`으로 점진 로드
    let mut index = String::new();
    if let Ok(entries) = std::fs::read_dir(dir.join("skills")) {
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
                    index.push_str(&format!("- {name}: {desc}\n"));
                }
            }
        }
    }
    if !index.is_empty() {
        directive.push_str("\n\n■ 보유 스킬 색인 (본문: `cys skill show <name>`)\n");
        directive.push_str(&index);
    }
    // 사용자 오버라이드(취향·운영 노브) — 스킬 색인 뒤. PACK 밖 파일이라 install 불가침·
    // 정식 directive 무동결. render_block이 SAFETY_CORE_REASSERT를 항상 최후에 둬(last-word)
    // 사용자 텍스트가 안전핵을 못 뒤집는다. 파일 부재 시 빈 문자열(회귀 0).
    let expert = std::env::var("CYS_OVERRIDE_EXPERT").map(|v| v == "1").unwrap_or(false);
    let ov = cys::overrides::load_overrides(role, expert);
    directive.push_str(&cys::overrides::render_block(&ov));
    // T4-3 ②: 런타임 카탈로그 플레이스holder 치환 — 정적 본문에 `$action_catalog`가 있으면
    // 실제 레지스트리(edit_kinds::EditKind)에서 파생한 카탈로그로 교체(하드코딩 미주입 = Max
    // 토큰효율 + 반드리프트). 플레이스홀더 부재 시 무변(회귀 0). 단건 상세는 on-demand
    // (`editor.action_info` RPC) — 전체 산문 미주입.
    let directive = cys::action_catalog::substitute_catalog(&directive);
    Ok(directive)
}

/// 화면 마지막 비공백 줄이 셸 프롬프트로 끝나는지 판정 — marker 없는 에이전트의 시간 폴백
/// 직전 검사다. TUI가 떴다면 끝줄이 셸 프롬프트일 수 없다; 셸 프롬프트가 남아 있으면
/// 에이전트가 조용히 즉시 종료(에러 문구 없이)한 것이므로 주입하면 zsh로 들어간다.
fn screen_tail_is_shell_prompt(text: &str) -> bool {
    let Some(last) = text.lines().rev().find(|l| !l.trim().is_empty()) else {
        return false; // 화면 비어 있음 — 판단 보류(시간 폴백 유지)
    };
    let t = last.trim_end();
    // zsh "...%" / bash·sh "...$" / root "#" / powerlevel10k·starship "❯" —
    // 끝문자 기준(프롬프트 커스텀의 공통 꼬리). 오탐 효과는 '대기 후 명시 Err'(안전측).
    t.ends_with('%') || t.ends_with('$') || t.ends_with('#') || t.ends_with('❯')
}

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

/// launch-agent(새 surface)와 node-recover(기존 surface 재기동)가 공유한다.
fn boot_agent_on_surface(
    sid: u64,
    role: &str,
    agent: &str,
    spec: &Value,
    resume: bool,
    session_id: Option<&str>,
) -> Result<(), String> {
    let mut cmd = spec["cmd"].as_str().ok_or("agent cmd missing")?.to_string();
    if resume {
        if let Some(arg) = spec["resume_arg"].as_str() {
            // T2-6 resume 어댑터: 대화 기억 복원 플래그 (예: claude --continue).
            // (4b) {session_id} placeholder 치환: id가 있으면 정확한 세션 핀, 없으면 fallback arg
            // (claude=--continue, codex=resume --last). placeholder 없는 arg는 그대로(하위호환).
            let resolved = if arg.contains("{session_id}") {
                match session_id {
                    Some(id) => arg.replace("{session_id}", id),
                    None => spec["resume_arg_fallback"]
                        .as_str()
                        .unwrap_or("--continue")
                        .to_string(),
                }
            } else {
                arg.to_string()
            };
            cmd.push(' ');
            cmd.push_str(&resolved);
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
    let (send, _send_env) = render_launch(&cmd, &agent_env_pairs(spec));
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
    let max_wait_secs = delay.max(30) * 2;
    let mut waited = 0u64;
    let mut ready = false;
    let mut last_screen = String::new();
    while waited < max_wait_secs {
        std::thread::sleep(std::time::Duration::from_millis(2500));
        waited += 2; // ~2.5s per tick (보수적 집계)
        let screen = request("surface.read_text", json!({"surface_id": sid}))?;
        let text = screen["text"].as_str().unwrap_or("");
        last_screen = text.to_string();
        let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if screen_shows_launch_failure(&flat) {
            return Err(format!(
                "agent '{agent}' failed to start (command error on screen) — check cmd in agents.json"
            ));
        }
        if flat.contains("trustthisfolder") || flat.contains("Doyoutrust") {
            eprintln!("[launch-agent] folder-trust prompt detected → confirming");
            request(
                "surface.send_key",
                json!({"surface_id": sid, "key": "Return", "authoritative": true}),
            )?;
            std::thread::sleep(std::time::Duration::from_secs(2));
            continue;
        }
        match &ready_marker {
            Some(m) if text.contains(m.as_str()) => {
                ready = true;
                break;
            }
            // marker 미정의 에이전트(codex 등)의 시간 폴백 — 단 화면 끝이 여전히
            // 셸 프롬프트(%·$)면 에이전트(TUI)가 안 뜬 것이다(조용한 즉시 종료 등):
            // 시간만 믿고 주입하면 디렉티브가 zsh로 들어간다(맹주입 잔존 경로 차단).
            None if waited >= delay => {
                if screen_tail_is_shell_prompt(text) {
                    continue; // 아직 셸 — max_wait까지 더 기다린다(못 뜨면 아래 Err)
                }
                ready = true;
                break;
            }
            _ => {}
        }
    }
    if !ready {
        // 준비 미확인 주입 금지: 에이전트가 안 떠 있으면 디렉티브가 맨 셸(zsh)로 들어가
        // 첫 단어가 명령으로 실행된다("zsh: command not found: 는" — 2026-06-12 실측).
        // 주의: launch 경로 호출자가 실패 surface를 정리(close)하므로, 진단 증거(화면 꼬리)는
        // 여기서 에러 본문에 동봉한다 — "read-screen으로 확인하라"는 안내는 close 후 거짓이 된다.
        let tail: Vec<&str> = last_screen
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let tail = tail
            .iter()
            .rev()
            .take(5)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "agent '{agent}' readiness not confirmed in {max_wait_secs}s — directive injection \
             aborted (셸 오주입 차단). 실패 surface는 정리된다. 마지막 화면 꼬리:\n{tail}\n\
             → agents.json의 cmd를 점검하고 `cys launch-agent --role <role> --agent {agent}`로 \
             재시도하라"
        ));
    }
    // marker 감지 직후 TUI 입력 활성화까지 약간의 여유
    std::thread::sleep(std::time::Duration::from_secs(2));

    // 3) 지침 주입 — bracketed paste로 감싸 단일 입력으로 전달
    inject_text(sid, &directive)?;

    // 4) 주입 확인: 화면에 지침 머리말이 나타났는지 검사 (실패 시 경고)
    std::thread::sleep(std::time::Duration::from_secs(3));
    let screen = request(
        "surface.read_text",
        json!({"surface_id": sid, "lines": 200}),
    )?;
    let flat: String = screen["text"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if flat.contains("ABSOLUTEDIRECTIVE") || flat.contains("절대지침") {
        eprintln!(
            "[launch-agent] directive injected & visible on screen ({} bytes)",
            directive.len()
        );
    } else {
        eprintln!("[launch-agent] warning: directive not visible on screen — verify with `cys read-screen --surface {}`", surface_ref(sid));
    }

    // 5) T2-5 에이전트 메타 등록은 ★Phase 5 ①a로 기동 직후(위)로 이동했다 — readiness 폴링/주입
    // 성공에 의존하지 않게. 여기서 재등록하면 set_meta가 agent_seen을 false로 리셋해, 이미 사망감지가
    // 관측한(agent_seen=true) 노드를 일시 허위 DEAD로 되돌리므로 재호출하지 않는다.
    Ok(())
}

/// 에이전트 기동 + 역할 지침 자동 주입 (어댑터: agents.json).
/// 워커 todo 경로 결정론 산출: 자기 surface의 (데몬 권위) 역할 → `<pack>/round/<ROLE>_TODO.md`.
/// 역할은 데몬 roles 맵(dedup된 worker-N 포함)에서 읽으므로 LLM 치환·env 스냅샷에 의존하지 않는다.
/// 복수 워커는 각자 distinct 역할 → distinct 파일 → 충돌 0. 파일이 없으면 골격을 만들어 둔다.
/// 자기 surface의 cysd-권위 역할 한 단어를 stdout으로 출력 (PreToolUse capability-gate hook 전용).
/// CYS_SURFACE_ID(데몬이 PTY에 주입·상속)로 자기 surface를 surface.list에서 찾아 데몬 roles 맵의
/// role을 출력한다. 역할 미등록·env 부재·데몬 미응답이면 빈 줄 + exit 0(hook이 deny측 안전 판정).
/// ★role은 self-declared가 아니라 데몬 권위 — claim_role/launch-agent가 신원검증 후 등록한 값.
fn run_surface_role() -> i32 {
    let Some(my_sid) = cys::env_compat(ENV_SURFACE_ID).and_then(|s| parse_surface_ref(&s)) else {
        println!();
        return 0;
    };
    let role = request("surface.list", json!({}))
        .ok()
        .and_then(|r| {
            r["surfaces"].as_array().and_then(|arr| {
                arr.iter()
                    .find(|s| s["surface_id"].as_u64() == Some(my_sid))
                    .and_then(|s| s["role"].as_str().map(|x| x.to_string()))
            })
        })
        .unwrap_or_default();
    println!("{role}");
    0
}

fn run_todo_path() -> i32 {
    let Some(sref) = cys::env_compat(ENV_SURFACE_ID) else {
        eprintln!("CYS_SURFACE_ID 없음 — 데몬이 띄운 pane 안에서만 동작한다");
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
    let pack = cys::env_compat("CYS_PACK_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cys/pack")))
        .unwrap_or_else(|| std::path::PathBuf::from(".cys/pack"));
    let round = pack.join("round");
    if let Err(e) = std::fs::create_dir_all(&round) {
        eprintln!("round 디렉터리 생성 실패: {e}");
        return 1;
    }
    let fname = format!("{}_TODO.md", role.to_uppercase().replace('-', "_"));
    let path = round.join(&fname);
    if !path.exists() {
        let _ = std::fs::write(&path, format!("# {role} TODO — 영속 todo (절대지침 7)\n\n"));
    }
    println!("{}", path.display());
    0
}

fn run_launch_agent(role: &str, agent: &str, cwd: Option<String>) -> i32 {
    run_launch_agent_opts(role, agent, cwd, false, None)
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

fn run_launch_agent_opts(
    role: &str,
    agent: &str,
    cwd: Option<String>,
    resume: bool,
    session_id: Option<String>,
) -> i32 {
    // 절대지침(앵커1-b): 워커는 워크플로우 폴더에서 산다 — cwd 미지정이면 호출 폴더가
    // 워크플로우 폴더다 (데몬 기본값 home에 맡기지 않는다. 명시 --cwd는 그대로 우선).
    // 빈 문자열은 None으로 정규화 — 구버전 topology의 "cwd": "" 가 PTY 생성을 깨거나
    // 잘못된 타이틀을 만드는 것을 차단(restore 경로 방어).
    let cwd = cwd.filter(|s| !s.is_empty()).or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    });
    // 기동 실패 시 정리용 — 만들어 둔 surface가 role을 점유한 채 남으면 재기동이 차단된다
    let mut created: Option<u64> = None;
    let result = (|| -> Result<(), String> {
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
        let (_, inject_env) = render_launch("", &agent_env_pairs(&spec));
        let env_obj: serde_json::Map<String, Value> = inject_env
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        let r = request(
            "surface.create",
            json!({"cwd": cwd, "title": workflow_title(role, agent, &cwd), "role": role,
                   "rows": 40, "cols": 140, "idempotency_key": idem, "env": env_obj}),
        )?;
        let sid = r["surface_id"].as_u64().ok_or("create returned no id")?;
        created = Some(sid);
        eprintln!("[launch-agent] {} created (role={role})", surface_ref(sid));
        boot_agent_on_surface(sid, role, agent, &spec, resume, session_id.as_deref())?;
        println!("{}", surface_ref(sid));
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            if let Some(sid) = created {
                // close 결과를 정직히 보고한다 — 실패를 'closed'로 거짓 보고하면 role이
                // 좀비 surface에 점유된 채 남아 재기동이 claim_denied로 막힌다(이번 회귀의 근원).
                match request("surface.close", json!({"surface_id": sid})) {
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
                    let out = std::process::Command::new("schtasks")
                        .args([
                            "/Create", "/TN", TASK, "/TR",
                            &format!("\"{}\"", daemon.display()),
                            "/SC", "ONLOGON", "/RL", "LIMITED", "/F",
                        ])
                        .output()
                        .map_err(|e| e.to_string())?;
                    if !out.status.success() {
                        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
                    }
                    println!("작업 스케줄러 등록 완료 (로그온 시 자동 기동). 사망 시 자동 재기동은 미지원 — CLI 자동기동이 보완한다.");
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
                    let alive = connect_raw().is_ok();
                    println!("registered={registered} socket_alive={alive}");
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

/// 지정 스트림에 단발 RPC(부서 fan-out 집계용 와이어 로직). request()와 동일 프로토콜.
fn rpc_over<S: std::io::Read + std::io::Write>(
    mut stream: S,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let req = json!({"id": 1, "method": method, "params": params});
    let mut line = serde_json::to_string(&req).unwrap();
    line.push('\n');
    stream.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    reader.read_line(&mut resp).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(resp.trim()).map_err(|e| e.to_string())?;
    if v["ok"].as_bool() == Some(true) {
        Ok(v["result"].clone())
    } else {
        Err(v["error"]["message"].as_str().unwrap_or("error").to_string())
    }
}

/// 지정 소켓에 단발 RPC — fan-out 집계용(부서 소켓 순회). autostart 안 함(부서 다운=정상 정보·도달불가 표기).
#[cfg(unix)]
fn request_on(socket: &std::path::Path, method: &str, params: Value) -> Result<Value, String> {
    let stream = std::os::unix::net::UnixStream::connect(socket)
        .map_err(|e| format!("connect {}: {e}", socket.display()))?;
    rpc_over(stream, method, params)
}
#[cfg(windows)]
fn request_on(socket: &std::path::Path, method: &str, params: Value) -> Result<Value, String> {
    let stream = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(socket)
        .map_err(|e| format!("open {}: {e}", socket.display()))?;
    rpc_over(stream, method, params)
}

/// Tasks Control Center(CLI) — depts.json을 읽어 본부+각 부서 소켓에 org.status를 순회 집계한다.
/// master 능동 모니터링: 모든 부서의 모든 노드가 지금 하는 업무를 1콜로 본다. 도달불가 부서는 표기.
fn run_fleet(as_json: bool) -> i32 {
    // RC-7: HOME 미설정(Windows) 함정 회피 — dirs 기반 공용 해소.
    let home = cys::home_dir().to_string_lossy().into_owned();
    let mut targets: Vec<(std::path::PathBuf, String)> =
        vec![(socket_path(), "본부 · CEO".to_string())];
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
                    targets.push((sock, disp));
                }
            }
        }
    }
    let mut out: Vec<Value> = Vec::new();
    for (sock, disp) in &targets {
        match request_on(sock, "org.status", json!({})) {
            Ok(r) => out.push(json!({"department": disp, "surfaces": r["surfaces"].clone()})),
            Err(e) => out.push(json!({"department": disp, "error": e, "surfaces": []})),
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
            if let Ok(entries) = std::fs::read_dir(&cwd_round) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.ends_with("_TODO.md") {
                        v.push(e.path().to_string_lossy().into_owned());
                    }
                }
            }
            let pack_round = cys::pack::pack_dir().join("round");
            let role_todo = format!(
                "{}_TODO.md",
                role_name.to_uppercase().replace('-', "_")
            );
            let pt = pack_round.join(&role_todo);
            if pt.exists() {
                v.push(pt.to_string_lossy().into_owned());
            }
            // SESSION_STATE(pack 정본)는 master 소관 — master cycle일 때만 게이트에 포함
            if role_name == "master" {
                let pss = pack_round.join("SESSION_STATE.md");
                if pss.exists() {
                    v.push(pss.to_string_lossy().into_owned());
                }
            }
            v
        };
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
        inject_text(sid, "[CYCLE] 컨텍스트 순환 절차 개시. 지금 즉시: ① 자기 TODO 파일(~/.cys/pack/round/<역할>_TODO.md)과 SESSION_STATE(_round/ 또는 pack round/ 정본)에 현재 작업 상태·미해결 게이트·다음 액션을 저장하라. ② 저장 완료 후 다른 출력 없이 plain 한 줄로 CYCLE-SAVED 를 출력하라.")?;

        // 2) 파일 변화 게이트 (화면 마커는 참고 신호일 뿐 — reward-hack·stale 마커 차단)
        if !baseline.is_empty() {
            eprintln!("[cycle 2/5] 저장 파일 검증 대기 (mtime+해시, 최대 {timeout}s)");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
            let mut verified = false;
            while std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_secs(2));
                for (f, base_hash) in &baseline {
                    let mtime_ok = std::fs::metadata(f)
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .map(|t| t > start_time)
                        .unwrap_or(false);
                    if mtime_ok && sha256_file(f) != *base_hash {
                        verified = true;
                        break;
                    }
                }
                if verified {
                    break;
                }
            }
            if !verified {
                return Err(format!(
                    "저장 검증 실패 — {timeout}s 내 파일 갱신 없음. cycle 중단 (clear 미실행)"
                ));
            }
            eprintln!("[cycle] 저장 검증 통과");
        } else {
            eprintln!("[cycle 2/5] ⚠ 파일 검증 생략 (--force-no-verify)");
        }

        // 3) 2-phase handshake — 검증자 부재 시 clear 금지 (soul 규칙)
        if let Some(v) = &verifier {
            eprintln!("[cycle 3/5] 검증자 '{v}' handshake");
            let vr = request("system.resolve_role", json!({"role": v}))
                .map_err(|e| format!("검증자 '{v}' 부재 — clear 금지 (self-clear 차단): {e}"))?;
            let vsid = vr["surface_id"].as_u64().ok_or("bad verifier resolve")?;
            let body: String = baseline
                .iter()
                .map(|(f, _)| format!("{f} (sha256: {})", sha256_file(f).unwrap_or_default()))
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
            let decision = loop {
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
                        break item["decision"].as_str().map(String::from);
                    }
                }
            };
            match decision.as_deref() {
                Some("allow") | Some("yes") | Some("approve") => {
                    eprintln!("[cycle] 검증자 승인 — clear 진행")
                }
                Some(d) => return Err(format!("검증자 거부({d}) — cycle 중단")),
                None => return Err("검증자 응답 없음 (timeout) — clear 중단".into()),
            }
        } else {
            eprintln!("[cycle 3/5] (검증자 미지정 — handshake 생략)");
        }

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
    let result = (|| -> Result<(), String> {
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
        boot_agent_on_surface(sid, &role_name, &agent, &spec, true, sess.as_deref())?;
        inject_text(sid, "[RECOVER] 너는 방금 재기동되었다. _round/SESSION_STATE.md와 자기 TODO 파일을 읽어 작업 기억을 복원한 뒤 master에게 복귀를 1줄 push로 보고하라. 작업 재개는 master 지시를 따른다.")?;
        println!("recovered surface:{sid} ({agent})");
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

/// T2-6 조직 복원: 토폴로지 스냅샷 기준으로 죽은 역할 일괄 재기동 (작업 재개는 master 판단)
fn run_restore(cwd: Option<String>, include_master: bool, no_resume: bool) -> i32 {
    let result = (|| -> Result<(usize, usize), String> {
        let topo = request("system.topology", json!({}))?;
        let live: std::collections::HashSet<String> = topo["live"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|e| e["role"].as_str().map(String::from))
            .collect();
        let saved = topo["saved"].as_array().cloned().unwrap_or_default();
        if saved.is_empty() {
            println!("(토폴로지 스냅샷 없음 — launch-agent로 역할을 기동하면 자동 기록된다)");
            return Ok((0, 0));
        }
        let (mut ok, mut fail) = (0usize, 0usize);
        for entry in saved {
            let Some(role) = entry["role"].as_str() else {
                continue;
            };
            if role == "master" && !include_master {
                println!("· {role}: 제외 (restore 실행자가 보통 master — --include-master로 포함)");
                continue;
            }
            if live.contains(role) {
                println!("· {role}: 이미 가동 중 — 건너뜀");
                continue;
            }
            let Some(agent) = entry["agent"].as_str() else {
                println!("· {role}: agent 미상 — 건너뜀 (claim-role로 등록된 pane)");
                continue;
            };
            let target_cwd = cwd
                .clone()
                .or_else(|| entry["cwd"].as_str().map(String::from));
            println!("· {role}: {agent} 재기동…");
            // (4b) saved entry의 session_id를 꺼내 정확한 세션 재개(없으면 fallback)
            let sess = entry["session_id"].as_str().map(String::from);
            if run_launch_agent_opts(role, agent, target_cwd, !no_resume, sess) == 0 {
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
                        let _ = inject_text(sid, "[RESTORE] 조직 복원 절차다. _round/SESSION_STATE.md와 자기 TODO를 읽고 상태를 복원하라. ★작업 재개는 하지 말고 master의 지시를 기다려라.");
                    }
                }
            } else {
                fail += 1;
                println!("· {role}: 기동 실패 — 나머지 역할 계속 진행");
            }
        }
        Ok((ok, fail))
    })();
    match result {
        Ok((ok, fail)) => {
            println!("restore 완료: 재기동 {ok} · 실패 {fail} · 현황 `cys status`");
            if fail > 0 {
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
) -> serde_json::Value {
    let mut files: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (rel, content) in cys::pack::PACK.iter().chain(cys::pack::PACK_SKILLS.iter()) {
        files.insert((*rel).to_string(), sha256_hex(content));
    }
    let mut obj = serde_json::Map::new();
    obj.insert("pack_version".into(), json!(env!("CARGO_PKG_VERSION")));
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
) -> i32 {
    let v = build_pack_manifest_value(key_id, signed_at, expires_at, min_binary_version);
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

/// 시스템 tar로 tar.gz를 dest에 푼다(§ 소스 해석 — 신규 crate 의존 회피, shell-out).
fn extract_tar_gz(tar_gz: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("staging 생성 실패 {}: {e}", dest.display()))?;
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(tar_gz)
        .arg("-C")
        .arg(dest)
        .status()
        .map_err(|e| format!("tar 실행 실패: {e}"))?;
    if !status.success() {
        return Err(format!("tar 압축해제 실패(code {:?}) {}", status.code(), tar_gz.display()));
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

    // staging: 깨끗이 비우고 tar 풀기.
    let _ = std::fs::remove_dir_all(staging);
    extract_tar_gz(&tar_path, staging)?;

    // ⓐ 서명·신선도·replay 검증(P2, fail-closed) — 실패 시 staging 폐기·반영 0.
    let manifest = match cys::packsig::verify_with_keyring(
        &manifest_bytes,
        &sig_bytes,
        now_unix,
        accepted_path,
        keyring,
    ) {
        Ok(m) => m,
        Err(e) => {
            let _ = std::fs::remove_dir_all(staging);
            return Err(format!("manifest 검증 실패: {e}"));
        }
    };

    // ⓑ 파일별 sha256 대조(P2 verify_files) — manifest.files → staging 전방 무결성.
    if let Err(e) = cys::packsig::verify_files(&manifest, staging) {
        let _ = std::fs::remove_dir_all(staging);
        return Err(format!("파일 무결성 검증 실패: {e}"));
    }

    // ⓑ' 역방향 커버리지(§7-①) — staging 트리의 전 파일이 서명 manifest.files에 등재돼야.
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
            cys::pack::apply_pack_transactional(&items, &pv, &new_state, || {
                cys::packsig::record_accepted(&acc_path, &manifest_acc)
            })
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
    match cys::pack::install(false) {
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
fn adapter_ready(agent: &Option<String>, idle: bool, idle_secs: u64, scrollback_tail: &str) -> bool {
    const QUIET_THRESHOLD_SECS: u64 = 8; // ACK timeout 근사 — turn-boundary 근사 quiet 창
    let marker = agent
        .as_ref()
        .and_then(|a| load_agent_spec(a).ok())
        .and_then(|spec| spec["ready_marker"].as_str().map(|s| s.to_string()));
    match marker {
        Some(m) if !m.is_empty() => scrollback_tail.contains(&m),
        _ => idle && idle_secs >= QUIET_THRESHOLD_SECS, // ready_marker 부재 → fallback
    }
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
        let status = std::process::Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&out)
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
    if command.is_empty() {
        return Err("no command given".into());
    }
    let sid = parse_explicit_surface(&surface)?
        .or_else(|| cys::env_compat(ENV_SURFACE_ID).and_then(|s| parse_surface_ref(&s)));

    let mut cmd = std::process::Command::new(&command[0]);
    cmd.args(&command[1..]);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
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

    // pack-update 통합테스트는 CYS_PACK_DIR/CYS_CONFIG_DIR 전역 env를 공유하므로 직렬화한다.
    static PACK_UPDATE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn sha256_of(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes))
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
        let v = build_pack_manifest_value(Some("39E60A702949D6C3".into()), Some(100), Some(200), "0.4.1");
        assert_eq!(v["pack_version"], json!(env!("CARGO_PKG_VERSION")));
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
        let v2 = build_pack_manifest_value(None, None, None, "");
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
        let _g = PACK_UPDATE_ENV_LOCK.lock().unwrap();
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
        let _g = PACK_UPDATE_ENV_LOCK.lock().unwrap();
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
        let _g = PACK_UPDATE_ENV_LOCK.lock().unwrap();
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
        let _g = PACK_UPDATE_ENV_LOCK.lock().unwrap();
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
        let _g = PACK_UPDATE_ENV_LOCK.lock().unwrap();
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

    /// compose_directive 테스트들은 전역 ENV_PACK_DIR를 변경하므로 직렬화한다(병렬 레이스 방지).
    static COMPOSE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// ★불변식 박제: compose_directive는 디렉티브 → soul.md → 장기메모리 색인 → 스킬 색인
    /// 순서로 조립한다. 메모리 색인 누락은 "리뷰어·워커 장기기억 0" 결함의 재발이므로
    /// 섹션 존재와 순서를 기계 검증한다 (launch/reinject/cycle 공용 경로).
    #[test]
    fn compose_directive_includes_memory_index_after_soul() {
        let _env = COMPOSE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _env = COMPOSE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let cmd = hook_command(std::path::Path::new("/pack"));
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
        let _env = COMPOSE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _env = COMPOSE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _env = COMPOSE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _env = COMPOSE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
}
