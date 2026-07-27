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
    List {
        /// 관측만 하고 데몬을 깨우지 않는다(env `CYS_NO_AUTOSTART=1`과 동일 경로).
        /// ★기본 동작은 변경하지 않는다 — phoenix 복원 폴백이 `cys list`의 자동 기동에 의존한다.
        #[arg(long)]
        no_autostart: bool,
    },
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
        /// 멱등 키(--queued 전용): 같은 (대상, 키) 대기 메시지를 제자리 교체한다.
        /// "이 주제의 최신 상태"를 보낼 때만 쓴다 — 키가 없으면 어떤 중복 억제도 하지 않는다
        /// (동일 문자열 재전송은 정당한 패턴이다). 밀려난 구 텍스트는 dead-letter에 남는다.
        #[arg(long = "idempotency-key")]
        idempotency_key: Option<String>,
        /// TTL 면제 선언(--queued 전용) — master/cso 계열 발신자만 허용.
        /// 지휘 ACTION처럼 시간으로 사라지면 안 되는 메시지에만 쓴다(남용 = 무한 적체).
        #[arg(long)]
        important: bool,
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
    /// 산출물을 GUI 뷰어 web pane으로 열기 (md/diff 렌더 · 허용 루트 한정 · cys pane 안에서만 —
    /// full-trust 역할 필요, reviewer/외부 셸은 거부. GUI 미기동 시 exit 3)
    View {
        /// 렌더할 파일 경로 (절대/상대 — 실재하는 파일이어야 함)
        path: String,
    },
    /// shared-default Browser 컨텍스트에서 URL을 연다. 수명주기는 cysd가 소유하며
    /// 결과는 GUI의 격리된 in-pane cast에서 관측한다.
    Browser {
        /// 열 URL (생략 시 about:blank)
        url: Option<String>,
    },
    /// Packaged Browser Runtime ensure adapter. Lifecycle decisions remain in cysd.
    #[command(name = "browser-runtime-ensure", hide = true)]
    BrowserRuntimeEnsure {
        #[arg(long)]
        headless: bool,
    },
    /// Proxy one worker Browser operation without exposing the private engine endpoint.
    #[command(name = "browser-runtime-operation", hide = true)]
    BrowserRuntimeOperation { request_json: String },
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
    /// 직전 설치 보존본(<pack>.prev)에서 파일 단위 복원 — 업데이트 직후 "잃었다" 순간의 원커맨드 되돌리기
    #[command(name = "pack-rollback")]
    PackRollback {
        /// 복원할 팩 상대경로. 생략 시 .prev 와 현재 팩의 차이 목록 표시
        #[arg(long)]
        file: Option<String>,
        /// 확인 프롬프트 없이 적용
        #[arg(long)]
        yes: bool,
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
        /// todo 대신 **팩 단위 파일**의 경로를 산출한다(생성·기록 없음 · 설계 CU-2C).
        /// `session-state`=`<팩>/round/SESSION_STATE.md` · `recovery`=`<팩>/round/RECOVERY.md`.
        /// 역할과 무관한 파일이므로 역할 미등록에서도 동작한다 — 팩 산출 권위는 todo 경로와 공유.
        #[arg(long, value_enum)]
        kind: Option<TodoKind>,
    },
    /// Print this surface's cysd-authoritative role (one word) — PreToolUse capability-gate hook용.
    /// CYS_SURFACE_ID로 자기 surface를 찾아 데몬 roles 맵의 role을 출력(미등록 시 빈 줄·exit 0).
    SurfaceRole,
    /// Judge whether a write target belongs to another scope's pack — pack-guard 훅의 판정 SOT.
    /// stdout JSON `{verdict: "ok"|"cross-scope", own_scope, target_scope, suggest, authority}` ·
    /// **exit 0 고정**(어떤 오류도 도구 실행을 막지 않는다 — 판정 불가는 verdict "ok"로 표현).
    /// `authority`는 자기 scope의 출처다: "daemon"(surface.list 봉투 = 권위) | "local"(로컬 env 산출).
    /// 소비자 계약 — authority가 "daemon"이 아니면 차단(deny)을 기록(log)으로 강등하라.
    PackScopeCheck {
        /// 검사할 경로. 아직 존재하지 않아도 된다(부모 기준 정규화 — 훅은 파일 생성 **전**에 불린다).
        #[arg(long)]
        path: String,
    },
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
}

/// ★CU-2C — `todo-path --kind`가 산출하는 **팩 단위 파일**의 종류.
///
/// 왜 todo-path에 얹는가: 디렉티브가 없애려는 병은 "`~/.cys/pack/round/...`를 손으로 적는 것"
/// 하나다. todo만 기계 산출로 바꾸고 SESSION_STATE·RECOVERY는 리터럴로 남기면, 부서 노드는
/// 여전히 본사 팩에 복원 파일을 쓴다 — 같은 병이 이름만 바꿔 남는다. 산출 권위(데몬 우선)를
/// 재구현하지 않고 **같은 경로를 공유**하는 것이 이 옵션의 요점이다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum TodoKind {
    /// 복원의 단일 진실 — 현재 위치·지시 대장·노드 상태·미해결 게이트·다음 액션.
    SessionState,
    /// 재부팅 프로토콜 — SESSION_STATE보다 먼저 읽히는 복원 순서서.
    Recovery,
}

impl TodoKind {
    /// 파일명 정본. 이 대응을 두 곳에 적으면 언젠가 갈리고, 그러면 **쓰는 곳과 읽는 곳이
    /// 다른 파일을 가리킨다** — 유령 todo 사고와 같은 형태의 실패다.
    fn file_name(self) -> &'static str {
        match self {
            TodoKind::SessionState => "SESSION_STATE.md",
            TodoKind::Recovery => "RECOVERY.md",
        }
    }
}

/// `--kind` 경로 조립 — 팩 round 디렉터리 + 종류별 파일명. **순수 함수인 것 자체가 계약**이다:
/// 여기엔 `create_dir_all`도 `write`도 없으므로 "산출만 하고 만들지 않는다"가 코드 형태로 자명하다
/// (todo 경로는 생성기라서 이 성질이 없다 — 그래서 분기 자체를 분리했다).
fn kind_path(pack_round: &std::path::Path, kind: TodoKind) -> std::path::PathBuf {
    pack_round.join(kind.file_name())
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
    Clear {
        surface: String,
        /// 위임 clear: state_dir의 operator.token 값. 일치하면 **타 surface** 큐도 비울 수 있고,
        /// 제거되는 전 항목이 dead-letters.jsonl에 기록된다(무손실).
        #[arg(long = "operator-token")]
        operator_token: Option<String>,
    },
    /// 대상 노드에 **자기 큐를 점검·정리하라고 요청**한다(master·cso 전용).
    /// 남의 큐를 대신 비우지 않는다 — 판단·집행은 대상 노드 몫이라 ACL이 보존된다.
    RequestClear {
        #[arg(long)]
        surface: String,
    },
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

fn browser_worker_authority_params(operation: Option<Value>) -> Value {
    let task_id = std::env::var("CYS_TASK_ID")
        .ok()
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .unwrap_or_else(|| format!("browser-cli-{}", std::process::id()));
    let mut params = json!({
        "authority_kind":"authorized_worker_operation",
        "realm":"shared-default",
        "mode":"headless",
        "task_id":task_id,
    });
    if let Some(Value::Object(fields)) = operation {
        params.as_object_mut().unwrap().extend(fields);
    }
    params
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
    // 순수 프로브 명령은 자동 기동 금지 — "데몬이 떠 있는가"라는 질문의 답을 바꾸면 안 된다.
    // ★WS-10: `list --no-autostart`는 사용자가 명시적으로 같은 성질을 요구한 경우다(옵트인).
    // `status`·`doctor`는 여기 편입하지 않는다 — status는 관제 보드(INSTALL Step1·릴리스 스모크·
    // phoenix _daemon_identity exit 6 계약이 자동 기동을 전제), doctor는 raw connect만 써서
    // 이미 autostart 무접촉이다. 전환하면 순수 회귀가 된다.
    if matches!(
        cli.command,
        Command::Ping
            | Command::Daemon {
                action: DaemonAction::Status
            }
            | Command::List {
                no_autostart: true
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

/// ERROR_PIPE_BUSY(231) 한정 bounded 재시도로 named pipe 를 연다. 그 외 오류(파이프 부재
/// ERROR_FILE_NOT_FOUND = 데몬 다운 등)는 즉시 반환 — autostart 판단은 호출부 몫.
/// 231을 데몬 다운으로 오판하면 connect()의 sibling cysd autostart 까지 헛발동한다
/// (2026-07-10 Windows 실사고). 정책 상수는 GUI(cys-app)와 공용 단일 진실인 lib(cys::PIPE_BUSY_*)
/// — 근거·계약은 그 정의부 주석 참조. 비-Windows 테스트가 정책 불변을 박제한다.
#[cfg(windows)]
fn open_pipe_busy_retry(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let deadline = std::time::Instant::now() + cys::PIPE_BUSY_RETRY_DEADLINE;
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(path) {
            Err(e)
                if e.raw_os_error() == Some(cys::PIPE_BUSY_ERROR)
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(cys::PIPE_BUSY_RETRY_INTERVAL);
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

/// 형제 cysd 경로 해석의 **순수부**(테스트 가능 — `current_exe()`는 주입할 수 없다).
///
/// ★F7 CLI측 절반(WS-5 · 설계 §7-1): `current_exe()`는 심링크를 **해석하지 않은 채** 돌려줄 수 있다
/// (`/usr/local/bin/cys → /opt/cysjavis/bin/cys` 같은 심링크 설치, 개발자의 `ln -s` 배치). 그 부모에서
/// 형제 cysd를 찾으면 실제 설치 트리가 아니라 **심링크 디렉토리**를 데몬 트리로 오해석한다: 형제가
/// 없으면 autostart가 "no sibling cysd"로 죽고, 하필 다른 버전의 cysd가 있으면 스큐 데몬을 띄운다
/// (아래 autostart 경로의 crashloop 사고 이력과 결합되는 지점 — cys.rs autostart 주석 참조).
/// → `canonicalize()`로 실경로를 먼저 확정한 뒤 형제를 찾는다.
///
/// **폴백 필수**: 브로커 신원 검증(mod.rs)은 fail-closed지만 여기는 **기능 경로**다. 권한·레이스로
/// canonicalize가 실패했다고 `None`을 돌려주면 autostart·`daemon install`이 통째로 죽는다.
/// 실패 시 원경로로 폴백해 종전 동작(dev 빌드 `target/debug` 포함)을 그대로 보존한다.
fn sibling_daemon_from_exe(exe: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let resolved = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    resolved
        .parent()
        .map(|d| d.join(name))
        .filter(|p| p.exists())
}

fn sibling_daemon_path() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) { "cysd.exe" } else { "cysd" };
    sibling_daemon_from_exe(&std::env::current_exe().ok()?, name)
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

/// ★WS-6: 응답 프레임 디코딩 — `_pv` 마이너 스큐(구/신 데몬 혼재, 업데이트 직후 창)는 hard fail이
/// 아니라 **graceful downgrade**다. 길이 대칭검증만 건너뛰고 본문 JSON을 그대로 수용하며, 경고는
/// 실행당 1회만 낸다(다중 대상 루프에서 결과가 스크롤로 밀리는 것 방지).
///
/// `LenMismatch`는 **동일버전 트렁케이션**이므로 하드 실패를 유지한다 — 잘린 프레임을 수용하면
/// 부분 응답을 성공으로 오독한다. `parse_frame` 시그니처는 무접촉이다(wire.rs 계약 보존).
fn decode_response_frame(raw: &str) -> Result<Value, String> {
    match cys::wire::parse_frame(raw) {
        Ok(v) => Ok(v),
        Err(cys::wire::AbiError::VersionSkew { peer_pv, local_pv }) => {
            warn_skew_once(&format!(
                "[cys] warning: daemon ABI _pv={peer_pv} != cli _pv={local_pv} — 프레임 길이 검증을 건너뛰고 진행(graceful downgrade). 데몬·CLI 버전을 맞추면 사라진다."
            ));
            serde_json::from_str(raw)
                .map_err(|e| format!("abi: version-skew frame is not valid json: {e}"))
        }
        Err(e) => Err(format!("abi: {e:?}")),
    }
}

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
    let resp: Value = decode_response_frame(resp_line.trim())?;
    if resp["ok"].as_bool() == Some(true) {
        Ok(resp["result"].clone())
    } else {
        // ★C2: 오류에 구조화 후속 지시(error.data.directive)가 실려 오면 사람·에이전트가
        // 읽는 문자열에 그대로 이어붙인다. 지시가 stderr에 안 보이면 에이전트는 기본 행동
        // (재전송)으로 돌아가 폭주를 키운다 — 지시 전달이 거부의 절반이다.
        let mut msg = format!(
            "{}: {}",
            resp["error"]["code"].as_str().unwrap_or("error"),
            resp["error"]["message"].as_str().unwrap_or("unknown error")
        );
        if let Some(d) = resp["error"]["data"]["directive"].as_str() {
            msg.push_str(" — ");
            msg.push_str(d);
        }
        if let Some(p) = resp["error"]["data"]["dead_letter"].as_str() {
            msg.push_str(&format!(" (dead-letter: {p})"));
        }
        Err(msg)
    }
}

/// 버전 스큐 경고는 실행당 1회만 낸다 — 다중 대상(--to 글롭) 루프에서 같은 경고가
/// 대상 수만큼 반복되면 정작 읽어야 할 결과가 스크롤로 밀려난다.
fn warn_skew_once(msg: &str) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| eprintln!("{msg}"));
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

        Command::List { .. } => request("surface.list", json!({})).map(|r| {
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

        Command::Send { surface, to, queued, clear_first, idempotency_key, important, text } => {
            // 큐 전용 플래그를 직접 전송에 붙이면 조용히 무시된다 — 의도 오해를 즉시 끊는다.
            if !queued && (idempotency_key.is_some() || important) {
                Err("--idempotency-key/--important는 --queued 전용이다 (직접 전송은 대기열이 없다)"
                    .to_string())
            } else {
            resolve_targets(&surface, &to).and_then(|sids| {
                let from = cys::env_compat(ENV_SURFACE_ID).and_then(|s| parse_surface_ref(&s));
                let total = sids.len();
                let multi = total > 1;
                // ★A7⑦ 다중 대상 부분 실패: 종전엔 `?`로 **첫 실패에서 루프를 이탈**했다.
                // `--to 'reviewer-*'` 처럼 여러 노드에 뿌리는 호출에서 앞선 대상 하나가 죽어 있으면
                // 뒤의 멀쩡한 대상은 아예 시도조차 되지 않았고, 이미 성공한 발신은 보고되지 않았다
                // (부분 성공이 전량 실패처럼 보이는 오표시). 이제 전 대상을 시도하고, 대상별 오류를
                // 모아 말미에 요약한다. 단 하나라도 실패하면 종료코드는 비0이다(스크립트 게이트 보존).
                let mut failures: Vec<String> = Vec::new();
                let mut ok_n = 0usize;
                for sid in sids {
                    // T3-13 권위 전달: clear_first는 데몬이 원자적으로(Ctrl-U 선정리 → paste → CR)
                    // 집행한다. 클라측 C-u·150ms sleep·게이트는 제거 — 비원자 split·race를 없앤다.
                    // agent 등록 pane 게이트는 데몬 send_text가 집행(clear_first_unsupported).
                    let r = match request(
                        "surface.send_text",
                        json!({"surface_id": sid, "text": text.join(" "), "from": from,
                               "queued": queued, "clear_first": clear_first,
                               "idempotency_key": idempotency_key, "important": important}),
                    ) {
                        Ok(r) => r,
                        // 단일 대상은 종전 오류 문자열을 그대로 전달한다(계약 무변경).
                        Err(e) if !multi => return Err(e),
                        Err(e) => {
                            eprintln!("[send] surface:{sid} 실패: {e}");
                            failures.push(format!("surface:{sid}: {e}"));
                            continue;
                        }
                    };
                    let tag = if multi { format!(" → surface:{sid}") } else { String::new() };
                    if queued {
                        // ★C4 스큐 감지: 키를 보냈는데 응답에 merged가 없으면 구 데몬이다 —
                        // 병합이 일어난 것처럼 오표시하지 않고 결정론 경고를 낸다(send-key 선례).
                        if idempotency_key.is_some() && r["merged"].as_bool().is_none() {
                            warn_skew_once(
                                "[send] 경고: 데몬이 --idempotency-key를 지원하지 않아 \
                                 병합 없이 적재됐다(구버전 cysd — 재기동으로 갱신하라)",
                            );
                        }
                        let merged = if r["merged"].as_bool() == Some(true) { " MERGED" } else { "" };
                        println!("QUEUED (depth {}){merged}{tag}", r["depth"]);
                    } else {
                        println!("OK{tag}");
                    }
                    ok_n += 1;
                }
                if failures.is_empty() {
                    Ok(())
                } else {
                    Err(format!(
                        "{total} 대상 중 {ok_n} 성공 · {} 실패 — {}",
                        failures.len(),
                        failures.join(" | ")
                    ))
                }
            })
            }
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

        Command::View { path } => (|| -> Result<(), String> {
            // 실재 게이트: 없는 경로·디렉토리는 여기서 즉시 실패 — 오류가 pane 안으로 숨지 않게
            // (reveal_path의 metadata 게이트와 동형). canonicalize가 아닌 absolute를 쓴다:
            // Windows canonicalize는 \\?\ verbatim 경로를 반환해 사이드카(python realpath) 대조와
            // 미검증 결합면을 만든다. 심볼릭 링크 해소·허용 루트 판정은 사이드카(within_roots)가
            // 단일 소유 — allowlist·해소 로직 이중화는 드리프트 위험.
            let abs = std::path::absolute(&path)
                .map_err(|e| format!("경로 해석 실패: {path} — {e}"))?;
            let meta = std::fs::metadata(&abs)
                .map_err(|e| format!("경로 열기 실패: {path} — {e}"))?;
            if !meta.is_file() {
                return Err(format!("파일이 아님(디렉토리?): {}", abs.display()));
            }
            let reply = request("viewer.open", json!({"path": abs.to_string_lossy()}))?;
            let listeners = reply.get("listeners").and_then(|v| v.as_u64()).unwrap_or(0);
            if listeners == 0 {
                // 발행 성공 ≠ 표시 성공: 구독자 0 = GUI 포워더 부재 확정 → pane은 뜨지 않았다.
                eprintln!("발행됐으나 구독자 0 — GUI(cys.app) 미기동으로 표시되지 않음");
                std::process::exit(3);
            }
            println!("OK → viewer.open ({})", abs.display());
            Ok(())
        })(),

        Command::Browser { url } => (|| -> Result<(), String> {
            // Public compatibility surface. Python only translates the operation; cysd remains
            // the sole lifecycle authority and the private supervisor owns the engine process.
            let script = cys::pack::pack_dir().join("bin/javis_browser.py");
            if !script.exists() {
                return Err(format!("browser 스크립트 없음: {}", script.display()));
            }
            let target = url
                .filter(|u| !u.trim().is_empty())
                .unwrap_or_else(|| "about:blank".into());
            // 포그라운드 adapter 실행 — JSON·결정론 exit code를 그대로 전달한다.
            let mut cmd = std::process::Command::new("python3");
            // 동봉 Python adapter PATH만 보강한다. Browser engine/Bun/Chromium 선택은 cysd의
            // signed runtime authority가 담당하며 이 command는 PATH discovery로 engine을 고르지 않는다.
            if let Some(exe_dir) = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            {
                let cur = std::env::var("PATH").unwrap_or_default();
                if let Some(newp) = cys::runtime_prefixed_path(&exe_dir, &cur) {
                    cmd.env("PATH", newp);
                }
            }
            let status = cmd
                .arg(&script)
                .arg("open")
                .arg(&target)
                .status()
                .map_err(|e| format!("browser 실행 실패: {e}"))?;
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
            Ok(())
        })(),

        Command::BrowserRuntimeEnsure { headless } => (|| -> Result<(), String> {
            if !headless {
                return Err("shared Browser Runtime is fixed to headless mode".into());
            }
            let result = request(
                "browser.runtime.ensure",
                browser_worker_authority_params(None),
            )?;
            println!(
                "{}",
                serde_json::to_string(&result).map_err(|e| e.to_string())?
            );
            Ok(())
        })(),

        Command::BrowserRuntimeOperation { request_json } => (|| -> Result<(), String> {
            let operation: Value = serde_json::from_str(&request_json)
                .map_err(|error| format!("invalid Browser operation JSON: {error}"))?;
            let verb = operation
                .get("verb")
                .and_then(Value::as_str)
                .filter(|verb| !verb.is_empty() && verb.len() <= 64)
                .ok_or_else(|| "Browser operation verb is required".to_string())?;
            let args = operation
                .get("args")
                .filter(|args| args.is_object())
                .cloned()
                .ok_or_else(|| "Browser operation args must be an object".to_string())?;
            let result = request(
                "browser.runtime.operation",
                browser_worker_authority_params(Some(json!({"verb":verb,"args":args}))),
            )?;
            println!(
                "{}",
                serde_json::to_string(&result).map_err(|e| e.to_string())?
            );
            Ok(())
        })(),

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
                QueueAction::Clear { surface, operator_token } => parse_surface_ref(&surface)
                    .ok_or_else(|| format!("invalid surface ref: {surface}"))
                    .and_then(|sid| {
                        request(
                            "queue.clear",
                            json!({"surface_id": sid, "operator_token": operator_token}),
                        )
                    })
                    .map(|r| {
                        let by_op = if r["by_operator"].as_bool() == Some(true) {
                            " (operator — 전 항목 dead-letters.jsonl 기록됨)"
                        } else {
                            ""
                        };
                        println!("cleared {} queued message(s){by_op}", r["cleared"]);
                        0
                    })
                    .unwrap_or_else(|e| {
                        eprintln!("error: {e}");
                        1
                    }),
                QueueAction::RequestClear { surface } => parse_surface_ref(&surface)
                    .ok_or_else(|| format!("invalid surface ref: {surface}"))
                    .and_then(|sid| request("queue.request_clear", json!({"surface_id": sid})))
                    .map(|r| {
                        // `delivered` 가 정본, `notified` 는 구 데몬 호환 동의어.
                        let ok = r["delivered"].as_bool().or_else(|| r["notified"].as_bool());
                        if ok == Some(true) {
                            println!(
                                "request sent → {} (depth {} · oldest {}s)",
                                surface, r["depth"], r["oldest_age_secs"]
                            );
                        } else {
                            // ★A4: 미도달은 실패가 아니지만 **자동 재통지도 없다** — 사유와
                            // 재발행 가능 시점을 그대로 보여준다(종전 문언은 없는 재시도를 약속했다).
                            println!(
                                "request not injected (depth {} · oldest {}s · 사유={}): {}",
                                r["depth"],
                                r["oldest_age_secs"],
                                r["reason"].as_str().unwrap_or("unknown"),
                                r["hint"].as_str().unwrap_or("게이트 미통과 — 직접 재발행하라")
                            );
                        }
                        0
                    })
                    .unwrap_or_else(|e| {
                        // 구 데몬은 이 메서드를 모른다 — 결정론 안내로 스큐를 드러낸다.
                        if e.contains("unknown method") {
                            eprintln!(
                                "error: 데몬이 queue request-clear를 지원하지 않는다(구버전 cysd — \
                                 재기동으로 갱신하라). 임시로는 대상 노드에 직접 점검을 요청하라."
                            );
                        } else {
                            eprintln!("error: {e}");
                        }
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
        Command::PackPlan { force } => return run_pack_plan(force),
        Command::PackMerge { file, take_new, keep_mine, ai, to_local, propose, yes } => {
            return run_pack_merge(file, take_new, keep_mine, ai, to_local, propose, yes);
        }
        Command::PackOwnership { rel, quiet } => {
            // 결정론 조회 전용(쓰기 0) — 분류 SOT 는 pack::ownership() 한 곳(pack-guard hook 이 소비).
            // ★effective 등급: 치유·prune 은 임베드/매니페스트 파일에만 작용하므로, 임베드에 없는
            // 자작 신규 파일은 등급과 무관하게 불가침 — "custom" 으로 구분해 hook 오탐을 차단한다.
            let embedded = cys::pack::PACK_ALL.iter().any(|(r, _)| *r == rel.as_str());
            let name = if embedded { cys::pack::ownership_name(&rel) } else { "custom" };
            if quiet {
                println!("{name}");
            } else {
                let meaning = match name {
                    "custom" => "비출하 자작 파일 — 업데이트·치유·정리 전부 불가침(생존 보증 대상)",
                    "user" => "사용자 소유 — 업데이트가 절대 덮지 않음(vendor 전진은 .new 병치)",
                    "seed-once" => "런타임 상태 — 부재 시에만 시드, 존재하면 불가침",
                    _ => "vendor 소유 — 수정본은 다음 설치 스윕에 치유(수정 전 .user 보존). 자작은 새 파일로",
                };
                println!("{rel}: {name} — {meaning}");
            }
            return 0;
        }
        Command::PackRollback { file, yes } => {
            return run_pack_rollback(file, yes);
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

        Command::ClaimRole { role, surface, takeover_empty_seat } => target_surface(&surface, &None).and_then(|sid| {
            request("system.claim_role", json!({"role": role, "surface_id": sid,
                                                "takeover_empty_seat": takeover_empty_seat}))
                .map(|r| println!("registered: {} → surface:{}", r["role"].as_str().unwrap_or("?"), sid))
        }),

        Command::LaunchAgent { role, agent, cwd } => return run_launch_agent(&role, &agent, cwd),
        Command::Boot { cwd } => return run_boot(cwd),
        Command::TodoPath { role, emit_decl, kind } => return run_todo_path(role, emit_decl, kind),

        Command::SurfaceRole => return run_surface_role(),
        Command::PackScopeCheck { path } => return run_pack_scope_check(&path),

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
    let (written, kept) = match cys::pack::install_staged(force, Some(cys::pack::PackWriteAuth::production())) {
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
}

impl DiagStatus {
    fn as_str(&self) -> &'static str {
        match self {
            DiagStatus::Ok => "OK",
            DiagStatus::Warn => "WARN",
            DiagStatus::Fail => "FAIL",
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
}

/// settings.json 루트에 우리 SessionStart hook 명령이 등록돼 있는가.
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
    let hook_cmd = cys::pack::session_start_hook_command(&ctx.pack_dir);
    let is_registered = |path: &str| -> bool {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .map(|root| doctor_hook_present(&root, &hook_cmd))
            .unwrap_or(false)
    };
    if ctx.settings_paths.iter().any(|p| is_registered(p)) {
        return DiagItem {
            name: "hook",
            status: DiagStatus::Ok,
            detail: "SessionStart hook 등록됨".into(),
            action: String::new(),
        };
    }
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
            detail: "SessionStart hook 미등록 — 재등록 시도".into(),
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
            "SessionStart hook 미등록({}개 settings 확인)",
            ctx.settings_paths.len()
        ),
        action: "cys doctor --fix 또는 cys init-pack 로 등록".into(),
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

/// 고아 소켓 제거 판정(순수 함수 — 진리표 테스트 가능). 삭제는 **①flock 홀더 부재**(=락 획득 성공
/// 또는 락파일 ENOENT) **②기록된 홀더 pid가 사망**, 두 부정이 함께 성립할 때만이다.
///
/// ★정직성 정정(WS-8): 종전 주석·사용자 문구는 "3중 부정"을 표방했으나 **`pid_is_cysd`가 verdict에
/// 전혀 반영되지 않았다**(두 match arm이 같은 값을 반환). 진단 계약을 내세운 코드가 자기 사인을
/// 과장한 셈이다. 지금은 세 번째 신호를 **실제로 분기시킨다**: 살아있는 pid가 cysd가 아니면
/// `ForeignPid`(= pid 재사용 의심)로 갈라 사용자에게 다른 문구·다른 조치를 준다. 판정 자체는 종전과
/// 동일하게 보수적이다(둘 다 삭제 금지) — 바뀐 것은 **삭제/보류 경계가 아니라 진단의 해상도**다.
/// pid 사망이 확인된 경로에서는 `pid_is_cysd`를 애초에 조회하지 않는다(호출부에서 lazy — 락 스팬 안
/// sysinfo 스냅샷의 순수 비용 제거).
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum OrphanVerdict {
    /// 삭제 가능 — 락 미보유 + 기록된 홀더 pid 사망이 함께 확인됨.
    Removable,
    /// flock 획득 실패 = 데몬이 **부팅 중이거나 보유 중** — 판정 보류·삭제 금지(fail-closed).
    /// 이 분기를 미정의로 두면 산 소켓을 지우는 영구 장애 경로가 재현된다.
    HeldByDaemon,
    /// 구형 락파일(holder pid 미기재) — 데드맨 `FailClosed`와 동일한 보수 해석으로 삭제 금지.
    UnknownHolder,
    /// 기록된 홀더 pid가 살아있는 cysd — 삭제 금지.
    LiveHolder(u32),
    /// 기록된 홀더 pid가 살아있으나 **cysd가 아니다**(pid 재사용 의심) — 삭제 금지(보수적).
    /// LiveHolder와 판정 결과는 같지만 원인이 다르다: 전자는 "데몬이 산다", 후자는 "락파일이 낡았고
    /// 그 번호를 남이 쓰고 있다". 사용자 조치가 다르므로(전자=대기, 후자=수동 확인) 문구를 가른다.
    ForeignPid(u32),
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
        Some(pid) if pid_alive => OrphanVerdict::ForeignPid(pid), // 생존하나 cysd 아님 = pid 재사용 의심
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
    // ★lazy: sysinfo 스냅샷은 **pid가 살아있을 때만** 뜬다. 사망 pid에 대한 조회는 verdict에
    // 영향을 줄 수 없는데(진리표상 `Some(_) => Removable`) 락 스팬 안에서 비용만 지불했다 — WS-7이
    // 줄이려는 바로 그 보유 구간이다.
    let alive = holder_pid.map(doctor_pid_alive).unwrap_or(false);
    let is_cysd = alive && holder_pid.map(doctor_pid_is_cysd).unwrap_or(false);
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
            detail: format!("고아 후보 소켓 — 홀더 pid {pid}가 살아있는 cysd → 보류"),
            action: "삭제 금지(잠시 후 재실행)".into(),
        },
        OrphanVerdict::ForeignPid(pid) => DiagItem {
            name: "socket",
            status: DiagStatus::Warn,
            detail: format!("고아 후보 소켓 — 홀더 pid {pid} 생존하나 cysd 아님(pid 재사용 의심) → 보류"),
            action: "삭제 금지(수동 확인)".into(),
        },
        OrphanVerdict::Removable if fix => match std::fs::remove_file(sp) {
            Ok(()) => DiagItem {
                name: "socket",
                status: DiagStatus::Ok,
                // ★정직한 사인(WS-8): 실제로 확인한 것만 적는다 — 시작 락을 아무도 안 쥐었고,
                // 락파일에 적힌 홀더 pid가 죽었다. 종전 "3중 확인"은 검사하지 않은 신호를 셌다.
                detail: "고아 소켓 제거(시작 락 미보유 + 홀더 pid 사망 확인)".into(),
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
                // L3: windows는 브리지 pid 생존 프로브가 없어 status alive 항상 true·자가치유 미발현
                // (재스폰은 reaper 신호 의존) — doctor가 이 한계를 경고한다(WINFIX 트랙). unix는 정상.
                #[cfg(windows)]
                Ok(v) => DiagItem {
                    name: "channels-db",
                    status: DiagStatus::Warn,
                    detail: format!("채널 DB 정상·schema_version={v} · [WINFIX] windows는 pid 생존 프로브 부재로 status alive 항상 true·자가치유(죽은 브리지 재스폰) 미발현"),
                    action: "windows 채널 자가치유는 WINFIX 트랙 — 브리지 이상 시 수동 재기동".into(),
                },
                #[cfg(not(windows))]
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

fn run_doctor_diagnostics(ctx: &DoctorCtx, fix: bool) -> Vec<DiagItem> {
    vec![
        diag_pack_version(ctx),
        diag_pack_state(ctx),
        diag_install_manifest(ctx),
        diag_hook(ctx, fix),
        diag_orphan_socket(ctx, fix),
        diag_stale_lock(ctx, fix),
        diag_staging_residue(ctx, fix),
        diag_channels_db(ctx),
        diag_legacy_config(ctx),
    ]
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
    };
    let items = run_doctor_diagnostics(&ctx, fix);
    let fails = items.iter().filter(|i| i.status == DiagStatus::Fail).count();
    let warns = items.iter().filter(|i| i.status == DiagStatus::Warn).count();
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
                "summary": {"ok": items.len() - fails - warns, "warn": warns, "fail": fails},
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
            "요약: {} OK · {} WARN · {} FAIL",
            items.len() - fails - warns,
            warns,
            fails
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
/// 소켓별 boot 락 가드 — Acquired는 fd를 쥔 채 Drop에서 flock 자동 해제. 파일 열기 실패나
/// windows는 Acquired(None)로 락 없이 진행(직렬화만 포기·중복은 데몬 특권 가드가 대부분 흡수).
enum BootLock {
    Acquired(#[allow(dead_code)] Option<std::fs::File>),
    Busy,
}

/// 현재 소켓(CYS_SOCKET 상속)의 boot 락 파일을 비차단 flock 획득 시도.
fn acquire_boot_lock() -> BootLock {
    let lock_path = cys::socket_path()
        .parent()
        .map(|d| d.join("cys-boot.lock"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/cys-boot.lock"));
    if let Some(parent) = lock_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let f = match std::fs::OpenOptions::new().create(true).write(true).open(&lock_path) {
        Ok(f) => f,
        Err(_) => return BootLock::Acquired(None), // 락 못 열면 직렬화 없이 진행(보수적 허용)
    };
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            BootLock::Acquired(Some(f))
        } else {
            BootLock::Busy
        }
    }
    #[cfg(not(unix))]
    {
        BootLock::Acquired(Some(f))
    }
}

/// boot 기계판독 요약 줄의 접두 — GUI 파서와 공유하는 상수(오타 방어).
const BOOT_MACHINE_PREFIX: &str = "boot-machine:";

/// ★boot 판정 계약(결함 A 수리 2026-07-25): 사람용 요약 줄("boot 완료: 신규 기동 …")은 팀 상태를
/// 판정할 수 없다 — 선택 노드 grok이 미설치면 팀 5노드가 전부 살아 있어도 "신규 기동 0"+"미설치"가
/// 동시에 찍혀 GUI 휴리스틱이 오탐 배너를 띄웠다. 판정용 기계 줄을 별도로 1줄 찍는다.
/// **형식 단일 진실** — 소비자는 `src-tauri/src/main.rs::parse_boot_machine_line()`.
/// 형식 변경 시 양쪽을 같은 커밋에서 고쳐야 한다(키 추가는 소비자가 무시하므로 하위호환).
/// 순수 함수 — 회귀 핀.
fn boot_machine_line(launched: u32, failed: u32, mandatory_missing: u32) -> String {
    format!("{BOOT_MACHINE_PREFIX} launched={launched} failed={failed} mandatory_missing={mandatory_missing}")
}

/// 의무 편성 역할 = PLAN 5종 중 grok(reviewer-grok)을 뺀 4종. grok은 **선택 노드**라 미설치가
/// 정상이며 팀 결손이 아니다 — `mandatory_missing` 카운트에서 제외하는 유일한 기준.
/// 순수 함수 — 회귀 핀(PLAN과의 정합은 boot_plan_mandatory_matches_roster 테스트가 박제).
fn is_mandatory_boot_role(role: &str) -> bool {
    matches!(role, "cso" | "worker" | "reviewer-gemini" | "reviewer-codex")
}

fn run_boot(cwd: Option<String>) -> i32 {
    // ★이중 boot 직렬화(오너 2026-07-15 적대검증 D-7 + 아키텍트 성찰): 마스터 팀 스폰 트리거가
    // 겹칠 수 있다(고전 경로=UserPromptSubmit 훅이 javis_bootstrap.py ④ boot 발화 · 버튼 경로=GUI
    // spawn_orchestra_boot · 마스터 LLM이 스스로 boot). 두 boot가 겹치면 각자 "역할 미가동" 스냅샷을
    // 보고 리뷰어(데몬 특권 가드 없음)를 중복 스폰할 수 있다. 소켓별 boot 락을 비차단 획득 —
    // 이미 다른 boot가 진행 중이면 즉시 성공 반환(그 boot가 팀을 세운다·이 호출은 멱등 no-op).
    // (claim-role의 boot 부작용은 아키텍트 성찰로 제거 — 레지스트리 op가 프로세스 스폰하는 결합 차단.)
    let _boot_lock = match acquire_boot_lock() {
        BootLock::Acquired(g) => g,
        BootLock::Busy => {
            println!("cys boot — 다른 boot 진행 중(락 보유) — 중복 스폰 방지로 skip (그 boot가 팀을 세움)");
            return 0;
        }
    };
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
    let mut launched: u32 = 0;
    let mut failed: u32 = 0;
    // 의무 4종이 CLI 미발견으로 skip된 횟수 — 진짜 팀 결손 신호(선택 노드 grok은 세지 않는다).
    let mut mandatory_missing: u32 = 0;
    println!("cys boot — LLM orchestrating 편성 점검 (CSO·worker·agy·codex 4종 의무 + grok 선택)");
    for (role, agent) in PLAN {
        // ★검사 순서(결함 A 수리 2026-07-25): '이미 가동 중'을 CLI 존재 검사보다 **먼저** 본다.
        // 이미 살아 있는 역할은 그 CLI 바이너리가 지워졌거나 PATH에서 사라져도 팀 결손이 아니다
        // (프로세스는 이미 떠 있다) — 종전 순서는 이를 '미설치'로 세어 mandatory_missing을 부풀렸다.
        // 부수 효과로 which/where 프로브도 생략돼 no-op boot가 빨라진다.
        if live_roles.contains(*role) {
            println!("· {agent}: 역할 '{role}' 이미 가동 중 — 건너뜀");
            continue;
        }
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
            // 부트스트랩 무력화 사태(2026-07-09) UX 후속: worker·cso가 claude 미설치(또는 PATH 미발견)로
            // 조용히 skip되면 사용자는 "pane 0개"의 원인을 모른다 — 설치 힌트를 병기해 자가 진단 가능하게.
            let hint = match *agent {
                "claude" => {
                    if cfg!(windows) {
                        " (설치: PowerShell에서 `irm https://claude.ai/install.ps1 | iex` 후 자비스 재시작)"
                    } else {
                        " (설치: `curl -fsSL https://claude.ai/install.sh | bash` 후 새 탭)"
                    }
                }
                "codex" => " (설치: `npm i -g @openai/codex` — 선택 리뷰어)",
                "gemini" => " (Antigravity CLI `agy` — 선택 리뷰어)",
                _ => " (선택 노드 — 미설치면 건너뜀이 정상)",
            };
            if is_mandatory_boot_role(*role) {
                mandatory_missing += 1;
            }
            println!("· {agent}: CLI '{bin}' 미설치 — 건너뜀{hint}");
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
    // 기계 줄 먼저(GUI 판정용·형식 고정) → 사람용 요약 줄(문구 보존 — 기존 독자·문서 무회귀).
    println!("{}", boot_machine_line(launched, failed, mandatory_missing));
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
    if let Some(spec) = agents.get(agent) {
        return Ok(spec.clone());
    }
    // ★W-B 보완(성찰 2 적대검증 산물): user 승격의 대가 = 동결 — 사용자가 agents.json 을 수정해
    // 두면 vendor 가 **새 어댑터**를 출하해도 .new 병치만 되고 디스크 본엔 영영 안 들어와
    // "신규 CLI 지원했는데 안 됨"이 된다(schedule.json 은 데몬의 ensure_builtin_jobs 가 같은
    // 문제를 코드로 메우지만 agents.json 엔 그 보완이 없었다). 디스크에 없는 키만 **임베드
    // 어댑터로 폴백**해 '사용자 수정 보존'과 'vendor 신기능 즉시 사용'의 합집합을 만든다.
    // (덮어쓰기 0 — 디스크 정의가 있으면 항상 디스크가 이긴다.)
    let embedded_agents: Option<Value> = cys::pack::PACK_ALL
        .iter()
        .find(|(r, _)| *r == "agents.json")
        .and_then(|(_, c)| serde_json::from_str(c).ok());
    if let Some(spec) = embedded_agents.as_ref().and_then(|v| v.get(agent)) {
        eprintln!(
            "[agents] '{agent}' 은 내 agents.json 에 없어 **내장 정의로 폴백**했다 \
             (vendor 신규 어댑터 — 내 수정본은 그대로 보존됨). 편입하려면: cys pack-merge --file agents.json"
        );
        return Ok(spec.clone());
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
) -> Result<(), String> {
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
    let (send, _send_env) = render_launch(&cmd, &env_pairs);
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
    let max_wait_secs = if restore {
        (delay.max(30) * 2).min(20)
    } else {
        delay.max(30) * 2
    };
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

/// ★CU-3B(ADR-1) — **팩 경로의 권위는 데몬이고 클라이언트 env는 폴백이다.**
///
/// 왜 이 판정이 필요한가: `todo-path`는 역할(role)은 이미 데몬에 물어보면서 **경로만** 자기
/// env로 산출하는 비대칭이었다. 부서 pane이 `CYS_PACK_DIR`를 잃으면(rotate·상속 누락)
/// `pack_dir()`은 panic하지 않고 **조용히 홈 기본 `~/.cys/pack`으로 폴백**한다 — 부서 노드가
/// 본사 팩 round에 todo를 쓰는 교차-scope 사고의 발원지다. 조용한 오답보다 데몬 권위가 낫다.
///
/// 왜 순수 함수인가: 4분기(일치/불일치/봉투 키 부재/데몬 미조회)가 각각 다른 사고를 막는
/// 별개 결정이라, 통합 경로 테스트로는 분기를 고정할 수 없다. 경고 문구는 **반환만 하고
/// 출력하지 않는다** — 출력까지 여기서 하면 테스트가 화면을 읽어야 한다.
///
/// 반환 = (채택할 팩 경로, 호출자가 stderr로 낼 경고).
fn resolve_pack_authority(
    daemon_pack: Option<&str>,
    local: &std::path::Path,
) -> (std::path::PathBuf, Option<String>) {
    match daemon_pack.filter(|s| !s.is_empty()) {
        // 불일치 = env 유실(또는 오염) 의심. **데몬 값을 채택**하고 시끄럽게 알린다 —
        // 조용히 고치면 env가 깨진 채로 영영 산다.
        Some(d) if std::path::Path::new(d) != local => (
            std::path::PathBuf::from(d),
            Some(format!(
                "⚠ pack 불일치: 데몬={d} 로컬env={} — 데몬 값 채택(env 유실 의심, 점검: echo $CYS_PACK_DIR)",
                local.display()
            )),
        ),
        Some(d) => (std::path::PathBuf::from(d), None),
        // 봉투에 키가 없다 = 구데몬(CU-3A 이전). **조용한** 로컬 폴백이 옳다 — 버전 스큐는
        // 사용자 잘못이 아니고, 여기서 경고하면 업그레이드 전 전 노드가 매 호출 소음을 낸다.
        None => (local.to_path_buf(), None),
    }
}

/// 데몬에 **묻지 못한** 경로(비-pane·데몬 미기동·소켓 오류)의 고지 문구. 위 `None` 분기(구데몬)와
/// 달리 여기는 조용하면 안 된다 — 권위 없이 로컬 env를 믿고 있다는 사실 자체가 사용자가 알아야
/// 할 상태다(같은 로컬 폴백이라도 이유가 다르면 알림도 달라야 한다).
fn pack_authority_local_notice(local: &std::path::Path) -> String {
    format!(
        "ℹ 데몬 미조회 — 로컬 env 팩({})으로 산출한다(데몬 권위 없음)",
        local.display()
    )
}

fn run_todo_path(role_opt: Option<String>, emit_decl: bool, kind: Option<TodoKind>) -> i32 {
    // `--role` 지정 = 남의 역할 산출 = **파일 기록 금지**(설계 R7 신원 게이트 우회 방지).
    let foreign = role_opt.is_some();
    // ★CU-3B: 역할 조회와 **같은 응답의 봉투**에서 팩 권위를 함께 회수한다(추가 왕복 0).
    // `--role`(남의 역할) 경로도 같은 권위를 공유해야 한다 — 남의 경로를 내 깨진 env로
    // 산출하면 "누구 팩인지"가 호출자마다 갈린다. 그 경로엔 조회할 surface가 없으므로
    // 봉투만 받으러 1회 묻고, 실패하면 로컬 폴백 + 고지한다(데몬 없이도 계속 동작).
    let mut daemon_pack: Option<String> = None;
    let mut daemon_scope: Option<String> = None;
    let mut daemon_queried = false;
    let mut absorb_envelope = |r: &Value| {
        daemon_queried = true;
        daemon_pack = r["pack_dir"].as_str().map(|s| s.to_string());
        daemon_scope = r["scope"].as_str().map(|s| s.to_string());
    };
    // `--role`은 **남의 역할 경로 산출 전용**이다(파일 생성·기록 없음 · 설계 R7). 자기 역할은
    // 데몬이 정본이므로 surface.list로 조회한다 — 손으로 지어 부르는 것을 막는 것이 이 명령의 존재 이유다.
    //
    // ★CU-2C: 역할 해석 **실패를 즉시 치명으로 다루지 않는다**. `--kind`가 산출하는
    // SESSION_STATE·RECOVERY는 역할과 무관한 팩 단위 파일이라, 역할이 없다고 거부하면 정확히
    // 필요한 순간에 막힌다 — RECOVERY 경로는 데몬 roles 맵이 아직 비어 있는 **복원 초입**에
    // 읽히는 파일이다. 그래서 실패 사유를 들고만 있다가, 역할이 실제로 필요한 지점에서 낸다.
    let mut own_role: Option<String> = None;
    let mut role_error: Option<String> = None;
    match &role_opt {
        Some(_) => {
            // 봉투(팩 권위)만 받으러 1회 묻는다 — 남의 역할 경로도 같은 권위를 써야 한다.
            if let Ok(resp) = request("surface.list", json!({})) {
                absorb_envelope(&resp);
            }
        }
        None => match cys::env_compat(ENV_SURFACE_ID) {
            None => {
                role_error = Some(
                    "CYS_SURFACE_ID 없음 — 데몬이 띄운 pane 안에서만 동작한다(다른 역할은 --role)"
                        .into(),
                )
            }
            Some(sref) => match parse_surface_ref(&sref) {
                None => role_error = Some(format!("CYS_SURFACE_ID 파싱 실패: {sref}")),
                Some(my_sid) => match request("surface.list", json!({})) {
                    Ok(r) => {
                        absorb_envelope(&r);
                        own_role = r["surfaces"].as_array().and_then(|arr| {
                            arr.iter()
                                .find(|s| s["surface_id"].as_u64() == Some(my_sid))
                                .and_then(|s| s["role"].as_str().map(|x| x.to_string()))
                        });
                        if own_role.is_none() {
                            role_error = Some(
                                "이 surface에 역할 미등록 — todo-path는 역할 노드(claim-role/launch-agent) 전용"
                                    .into(),
                            );
                        }
                    }
                    Err(e) => role_error = Some(format!("surface.list 실패: {e}")),
                },
            },
        },
    }

    // ★S19 — 팩 경로는 `cys::pack::pack_dir()` 단일 구현을 경유한다. 종전에는
    // `env_compat("CYS_PACK_DIR")`(= CYS_/JAVIS_/AITERM_**PACK**_DIR)만 봐서 레거시 키
    // `AITERM_JARVIS_DIR`를 인식하지 못했다 — 그 환경에서는 **생성 위치와 스캔 위치가 갈려
    // 파일이 보고기에 영영 보이지 않는다**(env 목록이 두 벌이면 언젠가 갈린다는 실증).
    // ★CU-3B: 그 값은 이제 **폴백**이다 — 권위는 위에서 회수한 데몬 봉투다.
    let local_pack = cys::pack::pack_dir();
    let (pack, warn) = resolve_pack_authority(daemon_pack.as_deref(), &local_pack);
    if let Some(w) = warn {
        eprintln!("{w}");
    } else if !daemon_queried {
        eprintln!("{}", pack_authority_local_notice(&local_pack));
    }
    // scope도 같은 우선순위다. 데몬 봉투에 `scope`가 있으면(= `pack_dir`도 함께 온다) 그 값을
    // 쓰고, 없으면 로컬 `scope_id()`(= basename(local_pack))를 쓴다 — 두 키는 CU-3A에서 함께
    // 추가됐으므로 "pack만 있고 scope는 없는" 봉투는 존재하지 않는다. 여기서 basename을
    // 재구현하지 않는 것이 요점이다(pack.rs `scope_id_of` 단일 정본과의 drift 차단).
    let scope = daemon_scope
        .filter(|s| !s.is_empty())
        .unwrap_or_else(cys::pack::scope_id);
    let round = pack.join("round");

    // ★CU-2C `--kind` — 팩 단위 파일 경로 **산출 전용**(생성·기록 없음). 위에서 산출한 `pack`을
    // 그대로 쓴다(별도 산출 금지 — 두 벌이면 갈린다). 역할은 이 경로에 관여하지 않으므로
    // `role_error`가 있어도 성공한다: 복원 초입(역할 미등록)이 이 옵션의 주 사용처다.
    if let Some(k) = kind {
        if emit_decl {
            // 선언은 todo 파일의 귀속 표식이다 — 팩 단위 파일에는 의미가 없다. 조용히 무시하면
            // 호출자는 선언을 받은 줄 알고 파이프에 흘린다. 무시했다는 사실을 말한다.
            eprintln!("ℹ --emit-decl 무시 — 선언은 todo 파일 전용이다(--kind는 팩 단위 파일 경로 산출).");
        }
        println!("{}", kind_path(&round, k).display());
        return 0;
    }

    // 여기부터는 역할이 필수다(todo는 역할별 파일) — 미룬 실패를 지금 낸다.
    let role = match role_opt {
        Some(r) => r,
        None => match own_role {
            Some(r) => r,
            None => {
                eprintln!("{}", role_error.unwrap_or_else(|| "역할 해석 실패".into()));
                return 1;
            }
        },
    };

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

// ───────────────── CU-2A 판정 SOT — `cys pack-scope-check` ─────────────────
//
// pack-guard 훅(sh)이 "이 쓰기가 남의 팩인가"를 **재구현하지 않게** 하는 것이 이 명령의 존재
// 이유다. sh에서 realpath·basename·scope 비교를 다시 짜면 판정식이 2벌이 되고, 그중 하나는
// 반드시 뒤처져 훅과 CLI가 다른 답을 낸다(팩 env 목록이 두 벌이라 갈렸던 S19와 같은 병).

/// 대상 경로가 **어느 팩 scope에 속하는가**. 판정 범위를 `$HOME/.cys/` 직하의 `pack*` 디렉터리로
/// 좁히는 것이 계약의 핵심이다 — 레포 워크트리 소스·문서·임시 파일은 애초에 팩이 아니므로 훅이
/// 개입하면 안 된다(개발 무간섭. 이 경계가 없으면 전 노드의 Write/Edit 핫패스가 오탐으로 막힌다).
/// `pack`(본사)·`pack-dept-<n>`(부서)·`pack.prev`(교체 잔재)를 한 규칙으로 덮는다.
fn target_pack_scope(canon: &std::path::Path, home: &std::path::Path) -> Option<String> {
    let rel = canon.strip_prefix(home.join(".cys")).ok()?;
    match rel.components().next()? {
        std::path::Component::Normal(n) => {
            let name = n.to_string_lossy().into_owned();
            name.starts_with("pack").then_some(name)
        }
        // `..` 등 비정상 컴포넌트는 팩 지목으로 보지 않는다(정규화 후엔 나올 수 없지만,
        // 판정 불가를 cross-scope로 오인하지 않는다는 안전 방향을 코드로 남긴다).
        _ => None,
    }
}

/// 교차-scope 판정. **자기 scope를 모르면 언제나 `ok`다** — 권위 없이 차단하지 않는 것이 이
/// 명령의 안전 계약이다(SIM-3: 로컬 env 산출로 자기 scope를 base로 오인해 **자기 부서 팩 쓰기를
/// 오차단**하는 자해가 실증됐다. 2호 방어가 3호 피해자를 공격하면 안 된다).
fn pack_scope_verdict(own_scope: &str, target_scope: Option<&str>) -> &'static str {
    match target_scope {
        Some(t) if !own_scope.is_empty() && t != own_scope => "cross-scope",
        _ => "ok",
    }
}

/// 심링크 우회 차단 — 실경로로 정규화한다. 아직 없는 파일(신규 Write)은 `canonicalize`가 실패하므로
/// **부모를 정규화하고 파일명을 다시 붙인다**: 훅은 파일 생성 **전**에 호출되므로 이 폴백이 없으면
/// 신규 파일 쓰기는 전부 판정 불가가 된다(= 교차-scope 신규 생성이 무조건 통과).
fn canonicalize_for_check(p: &std::path::Path) -> std::path::PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    match (p.parent(), p.file_name()) {
        (Some(parent), Some(name)) => std::fs::canonicalize(parent)
            .map(|c| c.join(name))
            .unwrap_or_else(|_| p.to_path_buf()),
        _ => p.to_path_buf(),
    }
}

/// 교차-scope일 때의 교정 안내 — 훅이 사용자에게 그대로 보여 준다. "쓰지 마라"로 끝내면
/// 사용자는 우회로를 찾는다. 올바른 경로를 **산출하는 명령**을 함께 준다.
fn pack_scope_suggest(own_scope: &str, target_scope: &str) -> String {
    format!(
        "이 경로는 다른 scope의 팩({target_scope})이다 — 내 scope는 {own_scope}. \
         팩 경로는 손으로 적지 말고 `cys todo-path`(필요하면 `--role <역할>`)가 산출한 경로를 쓰라."
    )
}

/// `cys pack-scope-check --path <p>` — pack-guard 훅 전용 판정기. **exit는 항상 0**이다(ADR-3:
/// "비정상 종료 = 차단"류의 우연 의존을 원천 제거. 차단은 훅의 JSON 결정 채널이 하고, 이 명령은
/// 사실만 낸다).
fn run_pack_scope_check(path: &str) -> i32 {
    // own_scope는 **데몬 권위 우선**(SIM-3 교정). 로컬 env 산출만 믿으면 env 유실 pane이 자기
    // 부서 팩을 base로 오인한다. 그래서 출처(`authority`)를 함께 실어 보내, 권위가 로컬일 때는
    // 소비자가 차단을 기록으로 강등할 수 있게 한다 — 불확실한 권위로 막지 않는다.
    //
    // ★`request`(autostart 경유)를 **쓰지 않는다**: 이 명령은 pack-guard 훅에서 모든 Write/Edit
    // 마다 불린다. 데몬이 죽어 있을 때 매 편집이 launchd kickstart + 4초 대기 + sibling spawn을
    // 유발하면, 방어 장치가 곧 개발 정지 장치가 된다. 여기선 **한 번 시도하고 빠르게 포기**하고
    // (authority=local), 소비자가 차단을 기록으로 강등한다 — 그게 이 필드의 존재 이유다.
    let daemon_scope = cys::env_compat(ENV_SURFACE_ID)
        .and_then(|_| {
            request_on_timeout(
                &cys::socket_path(),
                "surface.list",
                json!({}),
                std::time::Duration::from_secs(2),
            )
            .ok()
        })
        .and_then(|r| r["scope"].as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());
    let (own_scope, authority) = match daemon_scope {
        Some(s) => (s, "daemon"),
        None => (cys::pack::scope_id(), "local"),
    };

    let canon = canonicalize_for_check(std::path::Path::new(path));
    let target_scope = target_pack_scope(&canon, &cys::home_dir());
    let verdict = pack_scope_verdict(&own_scope, target_scope.as_deref());
    let suggest = target_scope
        .as_deref()
        .filter(|_| verdict == "cross-scope")
        .map(|t| pack_scope_suggest(&own_scope, t));

    println!(
        "{}",
        json!({
            "verdict": verdict,
            "own_scope": own_scope,
            "target_scope": target_scope,
            "suggest": suggest,
            "authority": authority,
        })
    );
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
        boot_agent_on_surface(
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
    // busy-retry: 부서 fan-out 도 ERROR_PIPE_BUSY(231)를 다운으로 오판하지 않는다(connect_raw 대칭).
    let stream = open_pipe_busy_retry(socket)
        .map_err(|e| format!("open {}: {e}", socket.display()))?;
    rpc_over(stream, method, params)
}

/// request_on의 타임아웃판 — connect 후 read/write 타임아웃을 강제한다. drain --verify fan-out은
/// hung 소켓(데몬이 accept 후 무응답)에서 request_on의 무타임아웃 read가 영구 정지[A1-F2]하므로 필수.
#[cfg(unix)]
fn request_on_timeout(
    socket: &std::path::Path,
    method: &str,
    params: Value,
    timeout: std::time::Duration,
) -> Result<Value, String> {
    let stream = std::os::unix::net::UnixStream::connect(socket)
        .map_err(|e| format!("connect {}: {e}", socket.display()))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    rpc_over(stream, method, params)
}
#[cfg(windows)]
fn request_on_timeout(
    socket: &std::path::Path,
    method: &str,
    params: Value,
    _timeout: std::time::Duration,
) -> Result<Value, String> {
    // Windows named pipe read 타임아웃은 별도 API(SetCommTimeouts/OVERLAPPED)라 현 1차 플랫폼(darwin/unix)
    // 밖. busy-retry 경로(request_on)로 위임한다 — hung 방어는 unix에서 완전(범위 한정).
    request_on(socket, method, params)
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
    let wrapped = format!("\x1b[200~{text}\x1b[201~");
    request_on_timeout(
        socket,
        "surface.send_text",
        json!({"surface_id": sid, "text": wrapped, "quiet": true, "authoritative": true}),
        timeout,
    )?;
    std::thread::sleep(std::time::Duration::from_millis(800));
    request_on_timeout(
        socket,
        "surface.send_key",
        json!({"surface_id": sid, "key": "Return", "authoritative": true}),
        timeout,
    )?;
    Ok(())
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
    if io.inject(&t.socket, t.surface_id, &instr, io_to).is_err() {
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
            // 같은 규칙을 쓴다 — 표 둘이 같은 값을 다르게 그리면 그게 곧 드리프트다.
            let ctx = ctx_cell(&s);
            let task = s["status"]["task"]
                .as_str()
                .filter(|t| !t.is_empty())
                .or_else(|| s["title"].as_str())
                .unwrap_or("(업무 미보고)");
            println!(
                "   {:<14} {:<9} {:>5}  {}",
                role,
                state,
                ctx,
                task.chars().take(60).collect::<String>()
            );
        }
    }
    0
}

/// ★CU-6A — status 표의 CTX 열: **관측 우선 + 출처 표식**.
///
/// 왜 표식을 붙이나: 관측(데몬이 트랜스크립트를 읽어 산출)과 자기보고(에이전트가 말한 값)는
/// 신뢰도가 다른데 종전 표는 둘을 구분 없이 `NN%`로 그려, 보는 사람이 "이 값이 어디서 왔는지"를
/// 알 수 없었다. `ᵒ`=observed · `ˢ`=self-reported · `-`=값 없음.
///
/// 왜 모호·무효 관측을 버리나: 귀속이 확정되지 않은 관측은 **남의 컨텍스트**일 수 있다. 그 값을
/// 이 노드의 것으로 그리면 사람이 엉뚱한 노드를 순환시킨다 — 관측이 없는 것보다 나쁘다.
/// 그때는 자기보고로 물러선다(완전 실명이 아니라 신뢰도 강등이다).
///
/// `--json`은 건드리지 않는다(필드만 추가됨) — 표시 규칙과 기계 소비는 다른 계약이다.
fn ctx_cell(s: &Value) -> String {
    let trusted = !matches!(
        s["usage"]["attribution"].as_str(),
        Some("ambiguous") | Some("evicted")
    );
    if trusted {
        if let Some(p) = s["usage"]["ctx_pct"].as_u64() {
            return format!("{p}%ᵒ");
        }
    }
    match s["status"]["context_pct"].as_u64() {
        Some(p) => format!("{p}%ˢ"),
        None => "-".into(),
    }
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
        "{:<14} {:<12} {:<8} {:<9} {:>5} {:>7} {:>5}  {}",
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
        let ctx = ctx_cell(&s);
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
            "{:<14} {:<12} {:<8} {:<9} {:>5} {:>7} {:>5}  {}",
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

/// ★CU-2B — cycle 저장 지시문. 종전 리터럴 `~/.cys/pack/round/<역할>_TODO.md`가 무엇을 깨뜨렸나:
/// 부서 노드(팩=`~/.cys/pack-dept-N`)가 이 지시를 그대로 따르면 **본사 팩에 저장**한다. 그건
/// ①교차-scope 쓰기(2호 방어 대상)이면서 ②저장 게이트가 검사하는 파일(`pack_round/<역할>_TODO.md`,
/// 이 노드의 팩)과 다른 파일이라 **저장해도 게이트가 통과하지 않는** 이중 실패다.
/// 지시와 검사는 같은 값에서 나와야 한다 — 그래서 게이트가 쓰는 그 값을 그대로 포맷에 넣는다.
///
/// 순수 함수인 이유: 이 문자열의 유일한 계약은 "본사/부서 각각에서 자기 팩 경로를 말한다"이고,
/// 그건 인젝션 없이 값만으로 단정할 수 있다.
fn build_cycle_save_instruction(pack_round: &std::path::Path, role_todo: &str) -> String {
    format!(
        "[CYCLE] 컨텍스트 순환 절차 개시. 지금 즉시: ① 자기 TODO 파일({})과 \
         SESSION_STATE({} 또는 <cwd>/_round/SESSION_STATE.md)에 현재 작업 상태·미해결 게이트·\
         다음 액션을 저장하라. ② 저장 완료 후 다른 출력 없이 plain 한 줄로 CYCLE-SAVED 를 출력하라.",
        pack_round.join(role_todo).display(),
        pack_round.join("SESSION_STATE.md").display()
    )
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
        // ★CU-2B: 팩 round와 대상 역할의 todo 파일명은 **저장 지시문과 저장 게이트가 함께 쓰는
        // 한 값**이다. 종전에는 게이트만 이 값을 계산하고 지시문은 `~/.cys/pack/round/...`를
        // 리터럴로 박아, `--save-file` 지정 시에는 아예 계산되지도 않았다. 지시와 검사가 다른
        // 파일을 가리키면 저장 검증은 구조적으로 통과할 수 없다 — 그래서 분기 밖으로 올린다.
        let pack_round = cys::pack::pack_dir().join("round");
        let role_todo = format!("{}_TODO.md", role_name.to_uppercase().replace('-', "_"));
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
        inject_text(sid, &build_cycle_save_instruction(&pack_round, &role_todo))?;

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
        // (W1) 같은 pane 재기동(restore=false → 인라인 없음)이나 resume 게이트엔 기록된 config_dir·cwd를 쓴다.
        let rec_cwd = entry["cwd"].as_str().map(String::from);
        let rec_cfg = entry["claude_config_dir"].as_str().map(String::from);
        boot_agent_on_surface(
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
    let result = (|| -> Result<(usize, usize), String> {
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
            return Ok((0, 0));
        }
        let (mut ok, mut fail) = (0usize, 0usize);
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
                    Ok(()) => {
                        ok += 1;
                        let directive = restore_directive(role);
                        let _ = inject_text(sid, directive);
                        continue;
                    }
                    Err(e) => {
                        // in-seat 실패는 치명이 아니다 — fresh 폴백이 가용성을 지킨다(정직히 알린다).
                        println!("· {role}: 좌석 내 재연결 실패({e}) — fresh 기동으로 폴백");
                    }
                }
            }
            println!("· {role}: {agent} 재기동…");
            if run_launch_agent_opts(role, agent, target_cwd, !no_resume, sess, true, cfg) == 0 {
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
                        let _ = inject_text(sid, restore_directive(role));
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
fn run_pack_merge(
    file: Option<String>,
    take_new: bool,
    keep_mine: bool,
    ai: bool,
    to_local: bool,
    propose: bool,
    yes: bool,
) -> i32 {
    let dir = cys::pack::pack_dir();
    let mut pending = cys::pack::load_merge_pending(&dir);
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
                if confirm(&format!("'{rel}' 을 vendor 신버전으로 교체(내 수정 폐기)?")) {
                    if let Err(e) = cys::pack::write_atomic(&target, theirs.as_bytes()) {
                        eprintln!("쓰기 실패: {e}");
                        return 1;
                    }
                    advance_manifest_base(&theirs);
                    resolve(&mut pending, ".new");
                    println!("✅ {rel} ← vendor 신버전 채택");
                }
                return 0;
            }
            if keep_mine {
                advance_manifest_base(&theirs); // 이번 신버전은 '본 것'으로 — vendor 재전진 시에만 재병치
                resolve(&mut pending, ".new");
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
                if confirm(&format!("'{rel}' 보존본({rel}.user) 정리(vendor 본 유지 확정)?")) {
                    resolve(&mut pending, ".user");
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
fn run_pack_rollback(file: Option<String>, yes: bool) -> i32 {
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
                    let own = cys::pack::ownership_name(&rel_s);
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
    let own = cys::pack::ownership_name(&rel);
    if own == "seed-once" {
        eprintln!(
            "⛔ '{rel}' 은 런타임 상태(seed-once) — 롤백이 업데이트 후 쌓인 기억·상태를 지우는 \
             역방향 소실을 만들므로 파일 단위 복원 대상에서 제외합니다."
        );
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
    match std::process::Command::new("python3")
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
        let _g = ENV_LOCK.lock().unwrap();
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

    /// ★불변식 박제(결함 A 수리 2026-07-25): 기계판독 요약 줄의 **형식 계약**.
    /// 소비자는 src-tauri/src/main.rs::parse_boot_machine_line() — 이 문자열이 바뀌면 GUI 배너
    /// 판정이 조용히 구 휴리스틱으로 폴백(오탐 재발)하므로 형식을 테스트로 못 박는다.
    #[test]
    fn boot_machine_line_format_is_the_gui_contract() {
        assert_eq!(
            boot_machine_line(0, 0, 0),
            "boot-machine: launched=0 failed=0 mandatory_missing=0"
        );
        assert_eq!(
            boot_machine_line(3, 1, 2),
            "boot-machine: launched=3 failed=1 mandatory_missing=2"
        );
        // 접두는 상수와 일치(파서가 접두로 줄을 찾는다).
        assert!(boot_machine_line(0, 0, 0).starts_with(BOOT_MACHINE_PREFIX));
    }

    /// ★불변식 박제(결함 A 수리 2026-07-25): 의무 4종 ↔ 선택 grok 판정.
    /// grok(reviewer-grok)이 의무로 잘못 분류되면 grok 미설치만으로 팀 정상인데도 경고가 뜬다
    /// (원 오탐의 근본 원인). PLAN 로스터와 동기화 여부도 함께 박제한다.
    #[test]
    fn boot_plan_mandatory_matches_roster() {
        for role in ["cso", "worker", "reviewer-gemini", "reviewer-codex"] {
            assert!(is_mandatory_boot_role(role), "{role}은 의무 4종이어야 한다");
        }
        assert!(
            !is_mandatory_boot_role("reviewer-grok"),
            "grok은 선택 노드 — 미설치가 정상이므로 결손으로 세면 안 된다"
        );
        assert!(!is_mandatory_boot_role("master"), "master는 boot PLAN 대상이 아니다");
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
        }
    }

    /// staging-residue 진단은 프로세스 전역 env `CYS_DOCTOR_STAGING_MIN_IDLE_SECS`(보호창)를 읽는다.
    /// 이 값을 set/remove하는 doctor 테스트가 병렬로 겹치면 서로의 값을 읽어 오탐(사전 존재한 레이스)이
    /// 나므로, 해당 env를 만지는 테스트를 이 락으로 직렬화한다(W0 테스트 격리 정신 — 전역 env 교대 창 제거).
    static DOCTOR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    #[test]
    fn list_no_autostart_is_opt_in_and_default_stays_autostarting() {
        // ★WS-10: 기본 동작 무변경이 계약이다 — phoenix 복원 폴백이 `cys list`의 자동 기동에 의존한다.
        use clap::Parser;
        let plain = Cli::try_parse_from(["cys", "list"]).expect("cys list");
        assert!(
            matches!(
                plain.command,
                Command::List {
                    no_autostart: false
                }
            ),
            "기본은 autostart 유지"
        );
        let opted = Cli::try_parse_from(["cys", "list", "--no-autostart"]).expect("플래그 파싱");
        assert!(matches!(
            opted.command,
            Command::List {
                no_autostart: true
            }
        ));
        // `status`·`doctor`는 전환 대상이 아니다(INSTALL Step1·릴리스 스모크·phoenix _daemon_identity
        // exit 6 계약이 자동 기동을 전제) — 플래그가 붙지 않았음을 핀으로 고정한다.
        assert!(
            Cli::try_parse_from(["cys", "status", "--no-autostart"]).is_err(),
            "status에는 --no-autostart를 붙이지 않는다(계약 파손 방지)"
        );
    }

    #[test]
    fn version_skew_frame_is_accepted_but_truncation_is_not() {
        // ★WS-6: 마이너 스큐 = graceful downgrade(본문 수용) / 동일버전 길이 불일치 = 하드 실패.
        let sound = cys::wire::frame_response(&json!({"ok": true, "result": {"a": 1}})).unwrap();
        assert_eq!(
            decode_response_frame(sound.trim()).unwrap()["result"]["a"],
            json!(1),
            "정상 프레임은 그대로 통과"
        );

        // 구 데몬 모사: _pv가 다르고 _flen도 어긋난다 → VersionSkew → 본문 수용.
        let skewed = json!({
            "ok": true,
            "result": {"surfaces": []},
            "_flen": 999_999,
            "_pv": cys::wire::PROTO_PV as u64 + 1,
        })
        .to_string();
        let decoded = decode_response_frame(&skewed).expect("스큐는 graceful 수용");
        assert_eq!(decoded["ok"], json!(true));
        assert!(decoded["result"]["surfaces"].is_array(), "본문 보존");

        // 동일버전 + 길이 불일치 = 트렁케이션 → 하드 실패 유지(부분 응답을 성공으로 오독 금지).
        let truncated = json!({
            "ok": true,
            "result": {"surfaces": []},
            "_flen": 999_999,
            "_pv": cys::wire::PROTO_PV as u64,
        })
        .to_string();
        let err = decode_response_frame(&truncated).expect_err("트렁케이션은 거부");
        assert!(err.contains("LenMismatch"), "사유가 드러나야 한다: {err}");

        // 메타 없는 legacy peer 프레임은 무검증 수용(기존 계약).
        assert_eq!(
            decode_response_frame(r#"{"ok":true,"result":"pong"}"#).unwrap()["result"],
            json!("pong")
        );
    }

    #[cfg(unix)]
    #[test]
    fn sibling_daemon_resolves_through_symlinked_launcher() {
        // ★F7 CLI측(WS-5): 심링크로 설치된 cys에서 형제 cysd를 찾을 때, 심링크 **디렉토리**가 아니라
        // 실제 설치 트리를 봐야 한다. 이 픽스처가 없으면 canonicalize 삭제가 무증상으로 통과한다.
        let base = std::env::temp_dir().join(format!(
            "cys-sibling-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let real = base.join("real/bin"); // 실제 설치 트리: cys + cysd 동봉
        let linkdir = base.join("shim"); // 심링크만 있는 디렉토리(cysd 없음)
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(&linkdir).unwrap();
        std::fs::write(real.join("cys"), b"#!/bin/sh\n").unwrap();
        std::fs::write(real.join("cysd"), b"#!/bin/sh\n").unwrap();
        let link = linkdir.join("cys");
        std::os::unix::fs::symlink(real.join("cys"), &link).unwrap();

        // ① 심링크 런처: 링크 디렉토리에는 cysd가 없다 → canonicalize 없이는 None(=autostart 사망).
        assert!(
            !linkdir.join("cysd").exists(),
            "픽스처 전제: 심링크 디렉토리에는 형제 cysd가 없다"
        );
        let got = sibling_daemon_from_exe(&link, "cysd").expect("심링크를 해석해 실트리를 찾아야 한다");
        assert_eq!(
            got,
            real.canonicalize().unwrap().join("cysd"),
            "심링크 디렉토리가 아니라 실제 설치 트리의 cysd여야 한다"
        );

        // ② 비심링크(직접 실행) 경로는 종전과 동일하게 동작한다(회귀 방지).
        assert_eq!(
            sibling_daemon_from_exe(&real.join("cys"), "cysd").unwrap(),
            real.canonicalize().unwrap().join("cysd")
        );

        // ③ 형제가 없으면 None — 폴백이 존재하지 않는 경로를 반환하지 않는다.
        let lone = base.join("lone");
        std::fs::create_dir_all(&lone).unwrap();
        std::fs::write(lone.join("cys"), b"#!/bin/sh\n").unwrap();
        assert_eq!(sibling_daemon_from_exe(&lone.join("cys"), "cysd"), None);

        // ④ canonicalize 실패(비존재 경로)여도 패닉·None-즉사 없이 원경로 폴백으로 진행한다.
        assert_eq!(
            sibling_daemon_from_exe(&base.join("ghost/cys"), "cysd"),
            None,
            "폴백해도 형제 부재는 None(존재 필터는 유지)"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[cfg(unix)]
    #[test]
    fn orphan_socket_verdict_truth_table() {
        // 삭제는 [락 미보유 + 홀더 pid 사망] 두 부정이 함께 성립할 때만. `pid_is_cysd`는 삭제 경계를
        // 바꾸지 않지만 **생존 pid의 성격을 갈라** 사용자 문구·조치를 바꾼다(WS-8 정직성 정정).
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
        // ④홀더 pid 생존 → 보류. **pid_is_cysd가 verdict를 실제로 가른다**(종전에는 두 arm이 같은
        //   값을 반환해 인자가 죽어 있었다 — 그러면서 사용자에겐 "3중 확인"이라 출력했다).
        assert_eq!(
            judge_orphan_socket(true, true, Some(7), true, true),
            OrphanVerdict::LiveHolder(7),
            "살아있는 cysd = 데몬 보유"
        );
        assert_eq!(
            judge_orphan_socket(true, true, Some(7), true, false),
            OrphanVerdict::ForeignPid(7),
            "살아있으나 cysd 아님 = pid 재사용 의심(문구·조치가 다르다)"
        );
        assert_ne!(
            judge_orphan_socket(true, true, Some(7), true, true),
            judge_orphan_socket(true, true, Some(7), true, false),
            "pid_is_cysd는 verdict에 반드시 반영된다 — 인자를 무시하는 회귀를 여기서 죽인다"
        );
        // ⑤삭제 조건 충족(락 미보유 + pid 사망) → 삭제 허용. 사망 pid에서는 is_cysd를 조회조차
        //   하지 않으므로(호출부 lazy) 두 입력 모두 같은 결론이어야 한다.
        assert_eq!(
            judge_orphan_socket(true, true, Some(7), false, false),
            OrphanVerdict::Removable
        );
        assert_eq!(
            judge_orphan_socket(true, true, Some(7), false, true),
            OrphanVerdict::Removable,
            "사망 pid는 is_cysd 값과 무관하게 삭제 가능"
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

    // ── CU-3B: 팩 경로 권위 4분기 ────────────────────────────────────────────
    // 막는 사고: 부서 pane이 `CYS_PACK_DIR`를 잃으면 `pack_dir()`은 panic이 아니라 **조용히**
    // 홈 기본(`~/.cys/pack`)으로 폴백한다 — 부서 노드가 본사 팩 round에 todo를 쓴다.
    // 네 분기는 각각 다른 실패를 막으므로 하나씩 못박는다.

    /// ①불일치 = env 유실 의심. **데몬 값을 채택**하고 경고한다(조용히 고치면 env가 깨진 채로 산다).
    #[test]
    fn pack_authority_prefers_daemon_and_warns_on_mismatch() {
        let local = std::path::Path::new("/home/x/.cys/pack");
        let (pack, warn) =
            resolve_pack_authority(Some("/home/x/.cys/pack-dept-dept-2"), local);
        assert_eq!(pack, std::path::PathBuf::from("/home/x/.cys/pack-dept-dept-2"));
        let w = warn.expect("불일치는 반드시 시끄러워야 한다 — 조용한 교정은 env 파손을 영속시킨다");
        assert!(w.contains("pack-dept-dept-2") && w.contains("/home/x/.cys/pack"),
                "경고가 두 값을 모두 보여야 사용자가 어디가 깨졌는지 안다: {w}");
    }

    /// ②일치 = 정상. 경고 없음(정상 경로에서 소음을 내면 진짜 경고가 묻힌다).
    #[test]
    fn pack_authority_silent_when_daemon_agrees_with_env() {
        let local = std::path::Path::new("/home/x/.cys/pack-dept-dept-2");
        let (pack, warn) =
            resolve_pack_authority(Some("/home/x/.cys/pack-dept-dept-2"), local);
        assert_eq!(pack, local);
        assert!(warn.is_none(), "일치인데 경고가 났다: {warn:?}");
    }

    /// ③봉투에 키 없음 = 구데몬(CU-3A 이전). **조용한** 로컬 폴백 — 버전 스큐로 전 노드가
    /// 매 호출 소음을 내면 안 된다. 빈 문자열도 '값 없음'과 같게 다룬다(직렬화 사고 방어).
    #[test]
    fn pack_authority_falls_back_silently_on_old_daemon() {
        let local = std::path::Path::new("/home/x/.cys/pack");
        for daemon in [None, Some("")] {
            let (pack, warn) = resolve_pack_authority(daemon, local);
            assert_eq!(pack, local, "구데몬 봉투({daemon:?})는 로컬 폴백이어야 한다");
            assert!(warn.is_none(), "스큐 폴백은 조용해야 한다: {warn:?}");
        }
    }

    /// ④비-pane(데몬 미조회) = 같은 로컬 폴백이지만 **이유가 다르므로 알린다**.
    /// 권위 없이 로컬 env를 믿고 있다는 사실 자체가 사용자가 알아야 할 상태다.
    #[test]
    fn pack_authority_notice_names_the_local_pack_when_daemon_unreachable() {
        let n = pack_authority_local_notice(std::path::Path::new("/home/x/.cys/pack"));
        assert!(n.contains("/home/x/.cys/pack") && n.contains("데몬"),
                "고지가 팩 경로와 사유를 모두 말해야 한다: {n}");
    }

    // ── CU-2B: cycle 저장 지시문이 자기 팩을 말하는가 ────────────────────────

    /// 리터럴 `~/.cys/pack/round/...`가 깨뜨린 것: 부서 노드가 그대로 따르면 **본사 팩에 저장**한다
    /// (교차-scope 쓰기) — 게다가 저장 게이트가 검사하는 파일은 자기 팩이라 **저장해도 통과 불가**인
    /// 이중 실패였다. 지시와 검사가 같은 값에서 나오는지 본사·부서 2케이스로 못박는다.
    #[test]
    fn cycle_instruction_points_at_this_nodes_own_pack() {
        for (pack, role_todo, foreign) in [
            ("/home/x/.cys/pack/round", "WORKER_TODO.md", "pack-dept-dept-2"),
            ("/home/x/.cys/pack-dept-dept-2/round", "WORKER_2_TODO.md", "/.cys/pack/round"),
        ] {
            let s = build_cycle_save_instruction(std::path::Path::new(pack), role_todo);
            assert!(s.contains(&format!("{pack}/{role_todo}")),
                    "지시문이 게이트가 검사하는 todo 경로를 말하지 않는다: {s}");
            assert!(s.contains(&format!("{pack}/SESSION_STATE.md")),
                    "지시문이 자기 팩의 SESSION_STATE를 말하지 않는다: {s}");
            assert!(!s.contains(foreign),
                    "지시문에 남의 scope 경로가 남아 있다({foreign}): {s}");
            // 게이트 마커 계약(CYCLE-SAVED)은 지시문의 기능적 핵심 — 문구를 다듬다 잃으면
            // 2단계 검증이 영원히 타임아웃한다.
            assert!(s.contains("CYCLE-SAVED"));
        }
    }

    // ── pack-scope-check: 훅 판정 SOT ───────────────────────────────────────

    /// 판정 범위는 `$HOME/.cys/pack*`뿐이다 — 레포 워크트리·문서·임시 파일까지 팩으로 보면
    /// 전 노드 Write/Edit 핫패스가 오탐으로 막힌다(개발 무간섭 계약 · SIM-3 실증 항목).
    #[test]
    fn target_pack_scope_only_matches_pack_family_under_dot_cys() {
        let home = std::path::Path::new("/home/x");
        for (p, want) in [
            ("/home/x/.cys/pack/round/MASTER_TODO.md", Some("pack")),
            ("/home/x/.cys/pack-dept-dept-2/round/W.md", Some("pack-dept-dept-2")),
            ("/home/x/.cys/pack.prev/soul.md", Some("pack.prev")),
            ("/home/x/.cys/pack", Some("pack")),
            // 팩이 아닌 .cys 하위(상태·소켓)는 비대상
            ("/home/x/.cys/claude-default/settings.json", None),
            // 레포 워크트리 소스 — 절대 건드리지 않는다
            ("/home/x/Desktop/CYSjavis/src/bin/cys.rs", None),
            // 이름이 비슷한 홈 하위 디렉터리도 .cys 밖이면 비대상
            ("/home/x/pack/round/A.md", None),
        ] {
            assert_eq!(
                target_pack_scope(std::path::Path::new(p), home).as_deref(),
                want,
                "경로 {p}"
            );
        }
    }

    /// **권위 없이 차단하지 않는다**(SIM-3 자해 재현 방지): own_scope가 비면 언제나 ok.
    /// 자기 팩 쓰기도 ok. 남의 팩일 때만 cross-scope.
    #[test]
    fn pack_scope_verdict_blocks_only_a_known_foreign_pack() {
        assert_eq!(pack_scope_verdict("pack-dept-dept-2", Some("pack")), "cross-scope");
        assert_eq!(pack_scope_verdict("pack", Some("pack")), "ok");
        assert_eq!(pack_scope_verdict("pack", None), "ok");
        // own_scope 미상(데몬 무응답 + 로컬 산출 실패) — 모르면 막지 않는다.
        assert_eq!(pack_scope_verdict("", Some("pack")), "ok");
    }

    /// 심링크 우회 차단 + **미존재 파일**(훅은 생성 전에 불린다) 부모 정규화 폴백.
    /// 이 폴백이 없으면 "교차-scope 신규 파일 생성"은 전부 무판정 통과다.
    #[test]
    fn canonicalize_for_check_resolves_symlinks_and_uncreated_files() {
        let td = std::env::temp_dir().join(format!(
            "cys-scopechk-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&td);
        let real = td.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let f = real.join("t.md");
        std::fs::write(&f, "x").unwrap();
        let real_canon = std::fs::canonicalize(&real).unwrap();

        // ① 실재 파일: 실경로로 정규화된다
        assert_eq!(canonicalize_for_check(&f), real_canon.join("t.md"));

        // ② 심링크 경유: 링크가 아니라 **대상**으로 판정된다(우회 차단)
        #[cfg(unix)]
        {
            let link = td.join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert_eq!(canonicalize_for_check(&link.join("t.md")), real_canon.join("t.md"));
            // ②' 아직 없는 파일 + 심링크 부모 = 부모만 정규화하고 이름을 다시 붙인다
            assert_eq!(
                canonicalize_for_check(&link.join("new.md")),
                real_canon.join("new.md")
            );
        }

        // ③ 부모도 없는 경로: 판정 불가 — 입력을 그대로 돌려주고(= 팩 밖) 조용히 ok가 된다
        let ghost = td.join("nope").join("deep").join("x.md");
        assert_eq!(canonicalize_for_check(&ghost), ghost);
        let _ = std::fs::remove_dir_all(&td);
    }

    /// 훅 계약: 교차-scope 안내는 "쓰지 마라"로 끝내지 않고 **올바른 산출 명령**을 준다
    /// (막기만 하면 사용자는 우회로를 찾는다).
    #[test]
    fn pack_scope_suggest_names_the_generator_command() {
        let s = pack_scope_suggest("pack-dept-dept-2", "pack");
        assert!(s.contains("cys todo-path"), "교정 경로 안내가 없다: {s}");
        assert!(s.contains("pack-dept-dept-2") && s.contains("pack"));
    }

    // ── CU-2C: todo-path --kind (팩 단위 파일 경로 산출) ─────────────────────
    //
    // 이 옵션이 없으면 디렉티브(MASTER 34·225행)가 **존재하지 않는 옵션을 지시**한다 —
    // 지시를 따르려는 노드는 `unexpected argument '--kind'`를 받고 리터럴 경로로 되돌아간다.
    // 그게 정확히 이 작업이 없애려는 병(손으로 적은 `~/.cys/pack/round/...`)이다.

    /// kind 2종의 경로 산출 + **파일을 만들지 않는다**는 성질을 함께 못박는다.
    /// 산출만 하는 명령이 조용히 디렉터리를 만들면, 부서 노드가 본사 팩에 빈 round를 남긴다.
    #[test]
    fn kind_path_derives_pack_scoped_files_without_creating_anything() {
        let td = std::env::temp_dir().join(format!(
            "cys-kind-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&td);
        let round = td.join("round");
        assert_eq!(
            kind_path(&round, TodoKind::SessionState),
            round.join("SESSION_STATE.md")
        );
        assert_eq!(kind_path(&round, TodoKind::Recovery), round.join("RECOVERY.md"));
        assert!(!td.exists(), "경로 산출이 디렉터리를 만들었다 — 산출 전용 계약 위반");
        assert!(!round.exists());
    }

    /// **데몬 권위 공유**: kind 경로는 todo 경로와 같은 `resolve_pack_authority` 결과에서 파생된다.
    /// 별도 산출을 두면 env 유실 pane에서 todo는 부서 팩, SESSION_STATE는 본사 팩으로 갈린다 —
    /// 한 노드의 복원 파일과 작업 파일이 다른 팩에 사는 상태가 된다.
    #[test]
    fn kind_path_shares_daemon_pack_authority_with_todo_path() {
        let local = std::path::Path::new("/home/x/.cys/pack"); // env 유실 → 홈 폴백(본사)
        let (pack, warn) = resolve_pack_authority(Some("/home/x/.cys/pack-dept-dept-2"), local);
        assert!(warn.is_some(), "불일치는 경고와 함께 데몬 값을 채택한다");
        let round = pack.join("round");
        assert_eq!(
            kind_path(&round, TodoKind::SessionState),
            std::path::PathBuf::from("/home/x/.cys/pack-dept-dept-2/round/SESSION_STATE.md"),
            "kind 경로가 데몬 권위를 따르지 않으면 복원 파일이 남의 팩에 산다"
        );
        // 구데몬(봉투 키 없음)에서는 todo와 **같은** 로컬 폴백을 쓴다 — 둘이 갈리지 않는다.
        let (pack2, _) = resolve_pack_authority(None, local);
        assert_eq!(
            kind_path(&pack2.join("round"), TodoKind::Recovery),
            std::path::PathBuf::from("/home/x/.cys/pack/round/RECOVERY.md")
        );
    }

    /// clap 계약: 값 enum 검증(오타는 파싱 단계에서 거부)·kebab 값·`--role`과 독립 조합.
    /// 디렉티브가 적는 문자열이 그대로 계약이므로 형태를 박제한다.
    #[test]
    fn todo_path_kind_cli_shape() {
        use clap::Parser;
        match Cli::parse_from(["cys", "todo-path", "--kind", "session-state"]).command {
            Command::TodoPath { kind, role, emit_decl } => {
                assert_eq!(kind, Some(TodoKind::SessionState));
                assert!(role.is_none() && !emit_decl);
            }
            _ => panic!("todo-path가 TodoPath로 파싱되지 않음"),
        }
        match Cli::parse_from(["cys", "todo-path", "--kind", "recovery"]).command {
            Command::TodoPath { kind, .. } => assert_eq!(kind, Some(TodoKind::Recovery)),
            _ => panic!(),
        }
        // 무인자 = 종전 todo 경로(회귀 0 — 기존 호출자 전부가 이 형태다)
        match Cli::parse_from(["cys", "todo-path"]).command {
            Command::TodoPath { kind, .. } => assert!(kind.is_none()),
            _ => panic!(),
        }
        // --role과 독립 조합 가능(배타 아님)
        match Cli::parse_from(["cys", "todo-path", "--role", "worker", "--kind", "recovery"]).command
        {
            Command::TodoPath { kind, role, .. } => {
                assert_eq!(kind, Some(TodoKind::Recovery));
                assert_eq!(role.as_deref(), Some("worker"));
            }
            _ => panic!(),
        }
        // 오타·미지 값은 **파싱에서** 거부한다 — 런타임에 조용히 폴백하면 엉뚱한 파일을 가리킨다.
        for bad in ["session_state", "sessionstate", "SESSION-STATE", "todo", ""] {
            assert!(
                Cli::try_parse_from(["cys", "todo-path", "--kind", bad]).is_err(),
                "미지 kind 값 {bad:?}가 통과했다"
            );
        }
    }

    // ── CU-6A: status 표의 CTX 열 ───────────────────────────────────────────

    /// 관측 우선·출처 표식·모호 강등. **모호한 관측을 그대로 그리면** 사람이 남의 컨텍스트를
    /// 보고 엉뚱한 노드를 순환시킨다 — 그때는 자기보고로 물러서는 것이 옳다.
    #[test]
    fn ctx_cell_prefers_trustworthy_observation_and_marks_its_source() {
        // ① 신뢰 가능한 관측 = 관측값 + ᵒ (자기보고가 있어도 관측이 이긴다)
        let s = json!({"usage": {"ctx_pct": 61, "attribution": "confident"},
                       "status": {"context_pct": 40}});
        assert_eq!(ctx_cell(&s), "61%ᵒ");
        // ①' 판정이 없는 구경로(비클로드 등)도 관측을 쓴다 — 회귀 0
        let s = json!({"usage": {"ctx_pct": 61}, "status": {"context_pct": 40}});
        assert_eq!(ctx_cell(&s), "61%ᵒ");
        // ② 모호·무효 관측 = 버리고 자기보고 + ˢ
        for attr in ["ambiguous", "evicted"] {
            let s = json!({"usage": {"ctx_pct": 89, "attribution": attr},
                           "status": {"context_pct": 40}});
            assert_eq!(ctx_cell(&s), "40%ˢ", "attribution={attr}");
        }
        // ③ 둘 다 없으면 `-` (모호 관측만 있고 자기보고가 없는 경우 포함 — 지어내지 않는다)
        assert_eq!(ctx_cell(&json!({})), "-");
        assert_eq!(
            ctx_cell(&json!({"usage": {"ctx_pct": 89, "attribution": "ambiguous"}})),
            "-"
        );
        // ④ 관측 없음 + 자기보고 있음 = 자기보고
        assert_eq!(ctx_cell(&json!({"status": {"context_pct": 7}})), "7%ˢ");
    }

    /// clap 계약: `--path`는 필수이고 서브커맨드 이름은 kebab(`pack-scope-check`)이다.
    /// 훅(sh)이 이 문자열을 호출하므로 이름·인자 형태가 계약이다.
    #[test]
    fn pack_scope_check_cli_shape() {
        use clap::Parser;
        match Cli::parse_from(["cys", "pack-scope-check", "--path", "/a/b"]).command {
            Command::PackScopeCheck { path } => assert_eq!(path, "/a/b"),
            _ => panic!("pack-scope-check가 PackScopeCheck로 파싱되지 않음"),
        }
        assert!(
            Cli::try_parse_from(["cys", "pack-scope-check"]).is_err(),
            "--path 없이 통과하면 훅이 빈 판정을 받는다"
        );
    }
}
