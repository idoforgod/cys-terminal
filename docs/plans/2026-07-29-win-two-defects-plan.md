# 기획서 — Windows 0.14.4 결함 5건: 팩 형제 import · 노드 기동 실패 · 고아 좌석

> **★2026-07-29 저녁 갱신**: 최초 2건 조사가 **실측으로 5건 확정**으로 확장됐다.
> master "surface exit"의 정체가 규명됐다 — **죽은 것이 아니라 태어나다 만 것**이었다(§3.4 인과 사슬).

> **문서 성격**: 설계안(design) 작성을 위한 **자기완결 기획서**. 이 문서 하나만 읽고 설계에 착수할 수 있도록,
> 배경·증거·코드 지도·채택안·철회안·파급·위험·역할배치·검증절차를 모두 담는다.
> **작성 시점**: 2026-07-29 · **기준 버전**: cys **v0.14.4** (`Cargo.toml`·`tauri.conf.json`·`ui/package.json` 3곳 일치, 태그 `v0.14.4`, HEAD `37e2f2e`)
> **측정 환경**: macOS(개발기, 데몬 pid 10445 · v0.14.2 가동 중) + Windows 11 게스트(설치본 0.14.4, 데몬 pid 2856, `\\.\pipe\cys`)
> **범위 밖(명시적 제외)**: 빈 surface 잔존 건 — 오너 결정으로 **종결**(오너·임시알바 좌석으로 확정). 재론하지 않는다.
>
> **인접 문서(PATH 영역 — 본 기획서와 별개 트랙이나 같은 도메인)**
> - `docs/plans/2026-06-29-cli-path-install.md` — macOS에서 `cys`·`cysd`를 `/usr/local/bin`에 노출하는 명시동의 메뉴(`install_cli_to_path`). **본 기획서의 보호 대상 ①②와는 다른 기능**이지만 PATH 도메인을 공유하므로, PATH 관련 변경 시 함께 검토할 것.
> - `docs/plans/2026-07-06-phoenix-hardening-design.md` — 부활(phoenix) 계층 설계. §5.1 좌석(seat) 개념의 상위 맥락.

---

## 0. 한 문장 요약

Windows 신규 설치에서 **claude 노드를 하나도 띄울 수 없다.** 폴더 신뢰 프롬프트 자동확인이 claude를 종료시키고(D2), 실패한 창을 되돌리는 롤백이 권한 규칙에 막혀(D3) **역할만 쥔 고아 좌석**이 남는다. 사용자에겐 60초간 백지 창으로 보이고(D5), 그 좌석은 사망 감지·부활 대상에서 모두 빠진다. 별개로 팩 파이썬이 형제 모듈을 못 찾아 이벤트 발행이 죽어 있다(D1).

## 0.1 확정 결함 6건 (2026-07-29 Windows 실측)

| # | 결함 | 범위 | 상태 |
|---|---|---|---|
| **D1** | 팩 `bin/` 형제 모듈 import 실패 (embeddable python) | Windows 전원 | ✅ 확정 §3.1 |
| **D2** | ★**기계가 보낸 `Return`(`\r`)이 claude 폴더-신뢰 창을 종료시킨다** (사람 Enter는 통과) | Windows 신규 설치 전원 | ✅ **실측 확정** §3.4 |
| **D3** | ★**launch-agent 실패 롤백이 `close_denied`로 막혀 고아 좌석 생성** | **전 OS** | ✅ 확정 §3.5 |
| **D4** | `screen_tail_is_shell_prompt`에 PowerShell `>` 누락 | Windows 전원 | ✅ 코드 확정 §3.6 |
| **D5** | 기동 실패가 60초간 백지 + 진단이 토스트뿐 + 유령 탭 | 전 OS (UX) | ✅ 확정 §3.7 |
| **D6** | `launch-agent`의 기본 작업폴더가 **설치폴더**로 잡힘(`--cwd` 미지정 시) | Windows | ✅ 확정 §3.8 |

**사고의 최종 인과: D2가 방아쇠, D3가 잔해를 남긴 범인.**
오너가 "master가 surface exit 했다"고 관측한 것의 정체는 **죽은 노드가 아니라, D2로 기동이 무산되고 D3로 정리가 실패해 남은 좌석 잔해**였다. D6는 D2의 발현 확률을 높이는 증폭 요인이다(설치폴더 = 신뢰 미등록 폴더 → 신뢰창이 반드시 뜬다).

---

## 1. 배경 — 두 사건

### 사건 A. 독자(Windows 사용자) 제보 — 2026-07-28
> cys 번들 파이썬이 embeddable 형태(`python312._pth` 고정)라, 스크립트를 서브프로세스로 실행할 때 스크립트 폴더를 `sys.path`에 넣지 않고 `cwd`·`PYTHONPATH`도 무시한다. 이 때문에 `bin/`의 형제 모듈 bare import가 조용히 ImportError로 실패한다. 2026-07-28 heal이 유저 가드본을 되돌려 현재 발현 중.

### 사건 B. 오너 실측 — 2026-07-29 (0.14.4 Windows 설치본)
부트스트랩 진행 중 **master pane의 claude가 저절로 종료**됐다(오너 조작 없음). 화면은 PowerShell 프롬프트로 돌아갔고, 그 프롬프트에 기계 텍스트 조각이 타이핑돼 있었다(`PS> ─────`, `PS> TX 7%…`). **워커·CSO·리뷰어 2명은 정상 생존**하며 하트비트에 응답 중이었다. 좌우 사이드바 경고 배지 8건.

**두 사건의 공통점**: Windows 전용 · 증상이 조용함 · 자동 회복 없음 · macOS 개발환경에서 검출 불가.

---

## 2. 확정 사실 (측정 근거 포함)

> 아래는 전부 이 기계에서 실행·확인한 것이다. 설계자는 §14의 명령으로 재현할 수 있다.

### 2.1 팩 형제 import 현황
- `cysjavis-pack/bin/` 파이썬 **76개**. 형제 `javis_*` 모듈을 import하는 것 **15개**.
- 그중 **경로 가드가 없는 것 7개**:

| 파일:라인 | import 대상 | 보호 | Windows 실패 시 영향 |
|---|---|---|---|
| `javis_event.py:19` | `javis_scrub` | **최상위 bare · try 없음** | 🔴 스크립트 즉사 — 이벤트 발행 전면 중단 |
| `javis_wakeup.py:33` | `javis_scrub` | **최상위 bare · try 없음** | 🔴 스크립트 즉사 — 웨이크업 큐 중단 |
| `javis_memory.py:38` | `javis_skillscan` | try/except (LOUD 경고) | 🟠 포이즌 스캐너 다운 |
| `javis_memory_inject.py:126` | `javis_skillscan` | try/except (fail-safe) | 🟠 기억 주입 **전량 억제** |
| `javis_orchestra.py:202, 627` | `javis_boot_node`, `javis_todo_decl` | try/except | 🟡 노드 관측·todo 파서 저하 |
| `javis_task.py:466` | `javis_boot_node` | try/except | 🟡 관측불가 = fail-closed |
| `javis_phoenix_win_smoke.py:281` | `javis_state_snapshot` | 없음 | 🟡 Windows 스모크 자체 실패 |

- **가드를 가진 선례 8개**: `javis_phoenix.py:374`, `javis_briefing.py:30`, `javis_idempotency.py:150`, `javis_replay.py`, `javis_compete.py`, `javis_todo_stamp.py:64-65`, `javis_purge_verify.py:40-43`, **`javis_report.py:33-34`(← `append` 사용)**, 그리고 훅 `hooks/inject_gate.py:22`(팩 디렉터리 기준 해석 — 레인 정확).
- **표준 라이브러리 이름 충돌 0건** (bin/ 76개 모듈명 ∩ stdlib = 공집합). 비-`javis_` 파일 5개: `caption_shape`, `check_entity_registry`, `check_manifest`, `check_timeline`, `grill_gate`.
- **호출 형태**: `subprocess.run([sys.executable, script, …])` — 예 `javis_report_gate.py:514`(event), `:527`(wakeup).

### 2.2 번들 파이썬 구성
- 빌드 워크플로가 `python-3.12.10-embed-amd64.zip`을 풀고 다음을 쓴다
  (`.github/workflows/windows-build.yml:57`, `release.yml:271`):
  ```
  python312.zip
  .
  import site
  ```
  주석은 *"스크립트 dir(.) + site 활성화(형제 import·표준 경로 계산 존중)"*.
- **확정 오류**: `._pth`의 상대 경로는 **`._pth` 파일이 있는 디렉터리(=python.exe 폴더)** 기준으로 해석된다. `.` = `<install>\runtime\python`이며 **팩 `bin`이 아니다.** 주석은 사실과 다르다.
- `import site` 줄은 **pip·site-packages를 살리는 줄**이다(워크플로 59-61행이 get-pip 부트스트랩). **삭제 금지**.

### 2.3 Windows CI 초록불은 반증이 아니다
- `windows-build.yml` T5가 **동봉 embeddable 파이썬으로** `javis_phoenix_win_smoke.py`를 직접 실행한다.
- 그 파일 281행은 무방비 형제 import인데 통과한다. 이유는 **호출 순서**:
  ```
  case① _PH._win_state_dir_for_socket()
        → javis_phoenix.py:387 → _snap_mod():369-375
          sys.path.insert(0, <bin>)      ← 가드 보유 모듈이 전역 경로를 먼저 고침
  case④ 281행 import javis_state_snapshot ← 그래서 성공
  ```
- 스모크는 phoenix 자체를 `importlib.util.spec_from_file_location`(절대경로)로 로드한다(`:613-614`) — 즉 **작성자도 sys.path를 신뢰하지 않았다.**
- **결론: 현행 CI는 이 결함을 검출할 능력이 없다.**

### 2.4 팩 파일 소유권 — 현장 수리가 원복되는 이유
- `src/pack.rs`의 3등급(`ownership()` 단일 SOT, `:577-587`): **User**(영구 보존) / **SeedOnce**(부재 시에만 시드) / **System**(강제 치유).
- 실측 분포: 전체 **527** 중 User **12** · SeedOnce **4** · **System 511 (97%)**.
- `bin/*.py`와 **`acl.json`은 System**이다(회귀 테스트 `pack.rs:2286-2290`이 박제). → **사용자가 로컬에서 고쳐도 `init-pack` 스윕에 원복된다.** 제보의 "heal이 되돌렸다"는 정확한 진단.
- User 등급: `soul.md`, `directives/*.md`, `CLAUDE.md`, `schedule.json`, `agents.json`.

### 2.5 master 좌석 — "반쪽 등록"의 정확한 정의
- 스크린샷의 master pane 제목은 **`C:\Users\x\`(자동 경로 제목)**. 다른 노드는 `cso-claude · x` 형식 = `launch-agent` 산물. → **master는 launch-agent가 만든 노드가 아니다.**
- 부트스트랩 설계상 그렇게 된다: `javis_bootstrap.py:7-8` 단계 = **① preflight --fix ② cys ping ③ `cys claim-role master` ④ `cys boot`**. ③은 **새 노드를 만드는 게 아니라 "지금 이 pane"에 역할을 선언**한다.
- 발동 조건도 그렇다: `hooks/role-bootstrap.sh:38-42`는 `cys surface-role`이 `worker|cso|reviewer-*`면 즉시 종료 → **미claim(빈) pane 또는 master pane에서만 발동**.
- `claim-role`이 남기지 **않는** 두 흔적:

| 흔적 | 무엇 | 어디서 기록 |
|---|---|---|
| `agent_meta` | (에이전트명, 바이너리) | **`surface.set_meta` RPC만이 기록**(`handlers.rs:3511-3560`), 그리고 **그 호출자는 `launch-agent` 단 하나**(`cys.rs:4613-4616`) |
| `env_injected` | 생성 시 호출자 env 주입 여부 | `state.rs:1803` `env_injected: !env.is_empty()` |

- **그 결과 안전망 4겹이 통째로 빠진다**:

| 안전망 | 코드 | 반쪽 좌석에서는 |
|---|---|---|
| 에이전트 사망 감지 | `governance.rs:291` `let Some((agent,bin)) = s.agent_meta … else { continue }` | **건너뜀** |
| 생존 보고 `agent_alive` | `handlers.rs:1186` | 항상 `null` |
| 부활 명단 등재 | `cys.rs:6774` *"agent 미상 — 건너뜀(claim-role로 등록된 pane)"* | **제외** |
| 좌석 내 재연결(in-seat) | `cys.rs:6805` `let safe = cfg!(unix) \|\| env_injected;` | Windows + env 없음 → **거부** |

### 2.6 관측 침묵의 구조 (왜 아무도 신고하지 않는가)
| 항목 | 측정값 |
|---|---|
| 훅 `exit 0`(fail-open 종결) | **51곳** |
| `ui/src/main.ts` 빈 catch | 63곳 |
| Rust `let _ = …` | 613곳 |
| Rust `unwrap_or(false/default)` | 290곳 |
| 팩 `except …: pass` | 132곳 |

대부분은 **의도된 설계**(철학 §3-⑥: 관측 훅은 fail-open). 부작용으로 **결함이 증상 없이 산다.**
- 게다가 이 결함을 잡으라고 만든 부트 게이트 `javis_preflight.py:3426-3432`(C60(b))는 **검사 직전 `sys.path.insert(0, BIN)`을 스스로 넣고** 프로브한다 → **실전과 다른 조건에서 검사** → 항상 초록불.

### 2.7 검출 구조의 플랫폼 비대칭
| 항목 | 값 |
|---|---|
| 팩 파이썬 테스트 | **34개 — 전부 macOS 잡**(`ci-branch.yml`에 `(macOS)` 라벨) |
| 그중 unix 전제 포함 | **9개**(`/tmp`·`/bin/sh`·`.local/state`·`posix`) |
| 코어 플랫폼 분기 | Rust **106곳** + 팩 **38곳** |
| 실행 파이썬 3종 | 개발기 **3.9.6** / mac 배포 **3.12.13**(python-build-standalone) / **Windows 동봉 3.12.10 embeddable** |
| 현장 결함 이력 | `git log --grep="현장 결함"` → **1호·2호 모두 Windows** |

- **mac 실측**: 일반 파이썬은 스크립트 폴더를 항상 `sys.path[0]`에 넣는다 → **이 결함은 mac에서 원리적으로 검출 불가**.

### 2.8 보호 대상 — 절대 훼손하면 안 되는 두 기능
| 기능 | 코드 | 내용 |
|---|---|---|
| **① 앱 안에서 CLI 설치** | `src/lib.rs:103-108` `runtime_bin_dirs` | 동봉 `runtime/{python, git/cmd, git/usr/bin, node}`를 pane PATH **선두**에 올림 → pane 안에서 `npm i -g`·`curl \| bash` 동작 |
| **② claude 설치 후 PATH 오류 자동수리** | `src/lib.rs:153-184` `runtime_prefixed_path` | **Windows**: 데몬이 `WM_SETTINGCHANGE`를 못 받아 PATH가 stale → **spawn마다 레지스트리에서 재합성**<br>**unix**: claude 설치기가 rc를 안 고침이 실측돼 `~/.local/bin` **무조건 append** |

- 이 함수의 주석에 **본 기획서가 채택할 교리**가 이미 명문화되어 있다:
  > *"append 인 이유는 MAJ#1과 동일: **발견이 목적 — 기존 항목의 precedence를 강등하지 않는다**."*

---

## 3. 미확정 사실 — 설계 전 반드시 판별할 것

| # | 명제 | 상태 | 근거 |
|---|---|---|---|
| **U1** | 번들 파이썬이 형제 모듈을 못 찾는다 | ✅ **참으로 확정** | §3.1 |
| **U2** | master의 claude가 **왜** 종료됐는가 | ✅ **규명 완료 — "죽은 것이 아니라 태어나다 만 것"** | §3.4·3.5 |

### 3.1 U1 확정 증거 (2026-07-29 · Windows 0.14.4 실측)
```
> & "<install>\runtime\python\python3.exe" "%USERPROFILE%\.cys\pack\bin\javis_event.py" --help
Traceback (most recent call last):
  File "C:\Users\x\.cys\pack\bin\javis_event.py", line 19, in <module>
    import javis_scrub  # ★G2: 기록·전파 직전 비밀 마스킹(같은 폴더 형제 모듈 — 부재 시 즉시 실패=fail-closed)
ModuleNotFoundError: No module named 'javis_scrub'
```
→ **제보 확정.** 예측한 파일·예측한 행(19)·예측한 모듈에서 정확히 재현.
→ **§6.2 티켓A의 선행 게이트 통과.** 착수 조건 충족.

### 3.2 반쪽 좌석 확정 증거 (같은 세션 실측)
```
surface_ref : surface:33
role        : master
agent       :            ← 없음
agent_alive :            ← 없음
seat        : empty
exited      : False      ← 셸은 살아 있음
title       : surface 33 ← 자동 번호 제목 = launch-agent 산물 아님
```
→ **§2.5 진단이 실측으로 확정.** role=master를 쥔 채 에이전트만 사라진 좌석이며,
`agent_meta` 부재로 **데몬은 이 죽음을 감지조차 하지 못했다**(= `agent.exited` 이벤트 없음).
→ **함의**: (a) 원인 규명 시 **"이벤트 로그에 없음"을 사건 부재의 근거로 쓸 수 없다**(§C 판정표 보조 항목).

### 3.3 (a) 관련 추가 실측 — 2026-07-29 화면 확인
- **claude는 에러 한 줄 없이 사라졌다.** scrollback 경계:
  ```
  ✛ Drizzling…  (작업 중 스피너)
      □ todo 선언 scope … / check 로스터 … / 5노드 age … / REVIEWER_VERDICT …
  ──────────────
  >                       ← claude 입력 프롬프트(마지막 렌더)
  PS C:\Users\...>        ← 여기서부터 PowerShell (claude 소멸)
  PS C:\Users\...>   ×5줄
  ```
  → 종료 사유·traceback·쿼터 경고 **없음**. H3(claude 자체 종료·사유 기록)는 **약화**, H1(외부 종료)·H5(조용한 크래시) 쪽이 남는다. **확정은 아직 불가.**
- **★정정**: "아무도 몰랐다"는 **절반만 맞다**. `agent.exited`(사망 감지)는 없었으나
  **`master.deadman` 경보는 발화했다** — `governance.rs:537-557` `check_master_deadman`
  (master surface 무출력 ≥ `CYS_MASTER_DEADMAN_SECS`, 기본 **900초**) → UI 토스트
  *"master 무응답(deadman) surface:20 master silent"* 실측.
  → **설계 시사**: 사망 감지가 막혀도 **idle 기반 deadman은 살아 있다.** 반쪽 좌석 문제의
  보완 신호로 이미 존재하는 장치이며, 새로 만들 필요가 없다(철회안 P5의 대체 후보).
- **환경 특성(수집 방해 요인)**: Windows 게스트에서 `Desktop`에 파일 쓰기가
  `UnauthorizedAccessException / FileOpenFailure`로 거부된다(실측). Parallels
  `SharedProfile.UseDesktop=1` 설정에도 **Mac↔Windows 파일 전달이 양방향 모두 실패**.
  → **증거 회수 채널은 화면 캡처뿐**임을 전제로 런북을 설계할 것.

### 3.4 D2 확정 — folder-trust 자동확인이 claude를 종료시킨다

**결정적 실측 (2026-07-29 · `cys launch-agent --role master --agent claude` 직접 실행)**
```
[launch-agent] surface:42 created (role=master)
[launch-agent] claude starting… (polling readiness, max 60s)
[launch-agent] folder-trust prompt detected → confirming     ← claude 기동 성공·프롬프트 감지 성공
error: agent 'claude' readiness not confirmed in 60s — directive injection aborted (셸 오주입 차단).
마지막 화면 꼬리:
PS C:\Users\x>                                      ← claude 소멸·셸 복귀
```

**해당 코드** (`src/bin/cys.rs:4648-4655`):
```rust
if flat.contains("trustthisfolder") || flat.contains("Doyoutrust") {
    eprintln!("[launch-agent] folder-trust prompt detected → confirming");
    request("surface.send_key", json!({"surface_id": sid, "key": "Return", "authoritative": true}))?;
    std::thread::sleep(Duration::from_secs(2));
    continue;   // ★프롬프트가 화면에 남아 있으면 2.5초마다 Return 재전송
}
```

**★대조 실험으로 확정 (2026-07-29 19:00 실측)**

| 실험 | 결과 |
|---|---|
| **사람이 직접** 신뢰창에서 Enter | ✅ **정상 통과** — claude 메인 UI 진입 (`Welcome back futurist! / Opus 5 (1M context) / bypass permissions on`) 확인 |
| **cys가 `send_key Return`** 전송 | ❌ **claude 종료** → 화면이 PowerShell 프롬프트로 복귀 |

두 번 재현(`▶CEO` 1회 + `cys launch-agent --cwd $HOME` 1회) 모두 동일.

**전송 바이트는 정상이다**: `src/lib.rs:498-499` `"Return" | "Enter" => b"\r"` (0x0D).
그럼에도 Windows claude TUI는 이를 **확인이 아닌 취소/종료**로 처리한다.
신뢰창의 안내가 `Enter to confirm · Esc to cancel` 두 갈래인 점과 정합.

**확정**: 폴더-신뢰 프롬프트에 대한 **기계 자동확인 경로가 Windows에서 작동하지 않는다.**
**미확정(설계 시 규명)**: 왜 `\r`이 취소로 처리되는지 —
(a) ConPTY 경로에서 `\r`이 Ink TUI 입력 핸들러에 도달하기 전 소실/변환
(b) TUI 입력 핸들러 부착 **이전**에 도착(타이밍) — 첫 폴링이 2.5초라 이른 편
(c) 반복 전송(2.5초 주기)이 후속 화면에서 원치 않는 선택을 누름
→ **설계 시 반드시 함께 검토**: 단발 전송·전송 후 프롬프트 소멸 확인·재전송 상한·`\r\n` 대안·
   그리고 **애초에 신뢰창이 뜨지 않게 하는 선행 조치**(§6.5 D-티켓).

**왜 mac에서 안 터졌나**: 이미 신뢰된 폴더는 프롬프트가 뜨지 않는다. **신규 Windows 설치는 반드시 뜬다.**
**왜 cso·worker는 살아있나**: 그 노드들이 먼저 떠서 신뢰를 기록했거나, 다른 시점/폴더에서 기동됐기 때문.

**왜 mac에서 안 터졌나**: 이미 신뢰된 폴더는 프롬프트가 뜨지 않는다. **신규 Windows 설치는 반드시 뜬다.**
**왜 cso·worker는 살아있나**: 그 노드들이 먼저 떠서 신뢰를 기록했거나, 다른 계정 디렉터리를 썼기 때문(§3.4.1).

#### 3.4.1 계정 디렉터리 불일치 (D2의 증폭 요인)
- `agents.json`(Windows 실측): `"cmd": "claude-2 --dangerously-skip-permissions"`,
  `"env": {"CLAUDE_CONFIG_DIR": "${CYS_ACCOUNT_DIR:-$HOME/.claude-2}"}`
- `claude-2`는 **실행파일이 아니라 PowerShell 함수**(`Get-Command claude-2` → `CommandType: Function`)이며
  **자체적으로 계정 디렉터리를 설정할 수 있다.**
- 즉 **cys가 주입한 `CLAUDE_CONFIG_DIR`와 함수가 쓰는 디렉터리가 다를 수 있다** → 신뢰 기록이 서로 안 보임
  → 수동 실행 시엔 프롬프트가 안 뜨는데 launch-agent 경로에선 뜬다(실측 일치).
- **설계 시사**: agents.json의 `env` 주입과 사용자 래퍼(함수/스크립트)의 계정 설정이 **충돌하지 않는지**
  점검하는 진단이 필요하다(§6.5 D-티켓).

### 3.5 ★D3 확정 — 실패 롤백이 막혀 고아 좌석이 남는다 (전 OS)

**결정적 실측 (같은 실행의 마지막 줄)**
```
[launch-agent] failed surface surface:42 close 실패:
  close_denied: surface.close denied: caller (surface 41) may only close its own surface, not surface 42
  — `cys close-surface surface:42`로 수동 정리 필요(role 점유 잔존 가능)
```

**충돌하는 두 코드**
| 위치 | 내용 |
|---|---|
| `src/bin/cys.rs:5017-5031` | launch 실패 시 `surface.close{cause:"reap"}`로 **자기가 만든 surface를 되돌린다**(롤백) |
| `src/bin/cysd/handlers.rs:1646-1663` | `surface.close`는 **발신 pane이 자기 surface만** 닫게 한다(권한 게이트). 익명(pane 밖) 발신은 통과 |

→ **`cys launch-agent`를 pane 안에서 실행하면 롤백이 구조적으로 불가능하다.**
`cys boot`·`▶CEO`·부트스트라핑·master의 노드 재기동은 **전부 pane 안에서 실행**되므로 이 경로를 탄다.

**결과 — 고아 좌석 생성**
```
기동 실패 → 롤백 close_denied → surface가 role을 쥔 채 에이전트 없이 잔존
   → §2.5 "반쪽 좌석"과 동일 상태 (agent_meta 없음/agent 미기동)
   → 사망 감지 스킵 · 부활 명단 제외 · Windows in-seat 거부
   → 사용자는 백지 창을 보고 "마스터가 죽었다"고 인식
```

**★이것이 오너가 최초 관측한 현상의 정체다.** surface:33(role=master·agent 없음·seat empty)은
"죽은 master"가 아니라 **기동 실패 + 롤백 실패의 잔해**였다.

**설계 방향 후보 (택1 필요)**
1. 롤백 전용 예외: `cause="reap"` + **자기가 방금 만든 surface**임을 증명하는 토큰(예: create가 반환한 nonce)으로 한정 허용
2. 데몬이 롤백을 수행: `surface.create`에 **readiness 실패 시 자동 회수** 계약 추가(생성자=데몬이므로 권한 문제 없음)
3. 익명 경로 사용: launch-agent가 close만 pane 밖 프로세스로 위임(취약·비권장)
→ **1안이 권장.** 권한 모델을 넓히지 않고 "생성자 자신의 롤백"만 정확히 연다.

**즉시 우회(사용자)**: 고아 좌석 **자기 창 안에서** `cys close-surface surface:<N> --reap` 실행.
`--reap` 없이 닫으면 묘비가 생겨 그 역할이 자동 부활 대상에서 영구 제외된다(`governance.rs:1916-1919`).

### 3.6 D4 확정 — 셸 프롬프트 판정에 PowerShell `>`가 없다

`src/bin/cys.rs:4399-4407`
```rust
fn screen_tail_is_shell_prompt(text: &str) -> bool {
    ...
    t.ends_with('%') || t.ends_with('$') || t.ends_with('#') || t.ends_with('❯')
}
```
주석은 zsh·bash·root·starship만 열거한다. **Windows PowerShell 프롬프트 `PS C:\...>`의 `>`가 빠져 있다.**

**영향**: 이 술어는 *"에이전트가 안 떴는데 역할 지침을 셸에 주입하는 것"* 을 막는 안전장치다
(주석의 원 사고: `zsh: command not found: 는` — 2026-06-12 실측).
`ready_marker`가 없는 에이전트(codex 등)의 시간 폴백 경로에서 Windows는 **가드가 무력화**된다
→ 지침이 PowerShell 명령으로 실행된다.

**최초 사고 화면의 잔해와 정합**:
```
PS C:\Users\x> ─────
PS C:\Users\x> TX 7%…
PS C:\Users\x> n · …
```
지침·상태줄 조각이 PowerShell에 제출된 형태다.

> **주의**: claude는 `ready_marker: "❯"`가 정의돼 있어 이 폴백 경로를 타지 않는다.
> D4는 **marker 없는 에이전트**(codex·grok 등)에서 발현한다. claude 실패의 원인은 D2다.

### 3.7 D5 확정 — 실패가 사용자에게 도달하지 않는다 (UX)

시스템은 **정확히 진단하고 있었다**. `cys.rs:4675-4695`의 에러는 화면 꼬리까지 붙여
*"agents.json의 cmd를 점검하고 재시도하라"* 고 지목한다. 문제는 **도달 경로**다.

| 구간 | 사용자가 보는 것 |
|---|---|
| 0~60초 | **백지 창** — 아무 안내 없음 |
| 60초 | 진단문이 **토스트 1회**로만 표시(놓치면 소실) |
| 이후 | 데몬에서 사라진 창의 **탭이 UI에 유령으로 잔존**(실측: `cys list`엔 없는데 탭은 보임) |

→ 오너가 "그냥 죽어버렸다"고 판단한 것이 정확히 이 구간이다.

### 3.8 D6 확정 — launch-agent 기본 작업폴더가 설치폴더로 잡힌다 (Windows)

**실측**: `▶CEO`(= `cys launch-agent --role master --agent claude`, `--cwd` 미지정)로 생성된 pane의 셸 위치가
```
PS C:\Users\x\AppData\Local\cys>      ← 설치 폴더
```
`--cwd`를 명시하면 정상:
```
cys launch-agent --role master --agent claude --cwd "$env:USERPROFILE"
→ [launch-agent] surface:45 created (role=master) · pane 위치 = C:\Users\x   ✅
```

**기대 동작과의 괴리**: `state.rs`의 pane 생성은 `cwd.unwrap_or_else(|| dirs::home_dir()…)`로
**cwd 미지정 시 홈**이어야 한다. Windows 경로에서 그 기본값이 적용되지 않고 **데몬/앱의 현재 작업폴더가 상속**되는 것으로 보인다.
`src-tauri/src/main.rs:2124-2130`의 `start_master`도 `--cwd`를 넘기지 않는다.

**영향(D2 증폭)**: 설치폴더는 사용자가 신뢰한 적 없는 폴더다 →
**claude 폴더-신뢰 프롬프트가 반드시 뜬다** → D2가 반드시 발현한다.
즉 D6는 단독으로도 결함이지만, **D2의 발생 확률을 100%로 만드는 증폭기**다.

**설계 시 확인할 것**: ①`start_master`/`spawn_orchestra_boot`이 `--cwd`를 명시하도록 할지
②데몬 측 기본값이 Windows에서 왜 홈으로 해소되지 않는지(근원 수리) — **②가 정도(正道)**.

> **원칙: 미확정 위에 코드를 얹지 않는다.** U1이 거짓이면 티켓A는 아무 문제도 해결하지 않는 7파일 수정이 되며, 그 자체가 품질 저하다.

---

## 4. 원인 분석

### 4.1 결함 ① — 형제 import 실패 (가설 U1 전제)
```
embeddable python + python312._pth
   └ 표준 경로 계산 우회 → 스크립트 폴더가 sys.path에 없음
        └ bin/ 스크립트가 subprocess로 실행됨 (sys.executable script)
             └ 형제 모듈 bare import 실패
                  ├ javis_event / javis_wakeup : 최상위 bare → 프로세스 즉사
                  └ 나머지 5개 : try/except → 기능 저하(스캐너·기억주입·관측)
                       └ 훅 fail-open(exit 0) → 화면에 아무 표시 없음
                            └ 부트 게이트 C60(b)는 경로를 스스로 고쳐 검사 → 초록불
                                 └ 사용자·CI 어느 쪽도 발견 못 함
```

### 4.2 결함 ② — master 좌석 조용한 사망
두 층으로 **반드시 분리**한다.

| 층 | 질문 | 상태 |
|---|---|---|
| **(a)** | claude가 **왜** 종료됐나 | **미상 (U2)** — 오너 1순위 |
| **(b)** | **왜 아무도 몰랐고 왜 회복 안 됐나** | **규명 완료** — §2.5 반쪽 좌석 |

```
부트스트랩 ③ claim-role master  (설계된 동작)
   └ agent_meta 없음 · env_injected 없음
        ├ 사망 감지 스킵 → agent.exited 이벤트 없음
        ├ 부활 명단 제외 → cys restore 대상 아님
        └ Windows in-seat 거부 → 그 자리에서 되살릴 수도 없음
             └ 죽은 좌석에 기계 텍스트가 계속 타이핑됨(PS 프롬프트에 제출)
```

**중요**: (b)는 (a)의 규명을 방해한다 — 정식 노드가 아니어서 **다음에 죽어도 증거가 안 남는다.**

---

## 5. 설계자가 알아야 할 시스템 개념 (코드 지도)

### 5.1 좌석(seat) — "등록"과 "실재"의 분리
- 정의: `governance.rs:1365-1392` `SeatState { Unknown=0, Occupied=1, Empty=2 }` — **커널 사실**(자손 프로세스 유무).
- 판정: `seat_state()` = `collect_descendants(sys, s.pid).is_empty()` (`governance.rs:1405-1420`, `collect_descendants` `:1554`).
- 갱신: **단일 writer = watchdog 틱**, 주기 **5초**(`WATCHDOG_INTERVAL_SECS`, `:13`). 초기값 Unknown(`state.rs:1825`).
- 노출: `surface.list`(`handlers.rs:1195-1218`)·`org.status`·`system.topology`에 `seat` 문자열.
- 도입 경위(중요): `cys.rs:6721` 주석 — *"★SEAT(2026-07-17 실사고 수리): '역할이 등록됨'과 '그 좌석에 누가 앉아 있음'을 구분한다. 종전 live 집합은 role 등록만 보고 skip 해서, role=master를 쥔 빈 셸이 있으면 master를 영영 부활시키지 못했다."*
- **이 수리가 적용된 곳은 부활 경로 한 곳뿐이다** — 사망 감지·배달 게이트·관측에는 옮겨지지 않았다.

### 5.2 사망 감지 상태 기계 (`governance.rs:276-360`) — 함부로 건드리면 안 되는 이유
```
agent_meta(에이전트명·bin) 필수(:291)
  → collect_descendants 중 cmdline_matches_agent(bin_base) = 의미적 판정(:294-298)
     ├ alive  → agent_seen=true, 복귀 시 agent.recovered 발행(:299-310)
     └ dead   → agent_seen 래치 확인(:312) → exit_notified 단일통지(:315)
                 → agent.exited 발행(:319-326)
                 → auto_restart는 opt-in(CYS_AGENT_AUTORESTART, 기본 off)(:327)
                 → 401/token_expired 최근 알림이면 재기동 차단 + poison(:331-347)
                 → 3회 상한 서킷브레이커 + Poisoned 마킹(:349-351)
```
**핵심**: `agent_meta`는 식별자가 아니라 **판정 재료**(bin_base)다. `seat`(존재 판정)와 **질문이 다르다.**

### 5.3 배달 게이트 (`governance.rs:2060-2125`)
- pause / queue_paused / busy(출력 중) / **최근 사람 입력(3초)** / **SEAT 게이트** 순으로 보류.
- SEAT 게이트: `role.is_some() && seat == Empty` → 보류. **`Unknown`이면 배달**(현행 유지 명시). **role 없는 맨 셸은 종전대로 통과**(주석 명시).
- 이 게이트가 막으려던 사고: *"빈 셸이 role을 쥔 동안 리뷰어 verdict·워커 보고가 프롬프트에 문자로 타이핑돼 보고가 증발"*.

### 5.4 launch-agent vs claim-role 산출물 차이 (결함②의 뿌리)
| | `cys launch-agent` (=`▶CEO`/`▶부서장` 버튼) | `cys claim-role` (=부트스트랩 ③) |
|---|---|---|
| surface | **새로 생성** | 기존 pane 재사용 |
| 제목 | `<role>-<agent> · <host>` | 자동 경로 제목 |
| `agent_meta` | **기록**(`set_meta`) | 없음 |
| `env_injected` | Windows에서 **true**(create에 env 주입) | false |
| `takeover_empty_seat` | **항상 true 요청**(`cys.rs:4990`) | 옵션 |
| 지침 주입 | **자동** | 없음 |
| 계정 격리 | `CLAUDE_CONFIG_DIR` 실림 | 기본 계정 |

- 좌석 승계 판정: `seat_claimable()` = Empty **AND** `agent_meta` 부재 **AND** 최근 사람 입력 없음(`governance.rs:1428-1441`). 소비처: `handlers.rs:996-1006`(surface.create), `:1793-1806`(claim_role).
- **따라서 `▶CEO` 버튼은 죽은 반쪽 좌석의 role을 회수하고 정식 노드를 세울 수 있다.**

### 5.5 OS별 기동 렌더 (`cys.rs:4477-4492` `render_launch`)
```
unix    → `KEY="val" cmd` 인라인 (env가 명령줄에 실림 → in-seat도 안전)
windows → (순수 cmd, env맵)     (env는 surface.create 시점에만 주입 가능)
```
- 그래서 `cys.rs:6805`의 `cfg!(unix) || env_injected`는 **플랫폼 차별이 아니라 계정격리 계약**이다.
- 회귀 테스트가 출력 문자열을 박제 중: `render_launch_os_aware_unix_byte_identical`(`cys.rs:10467`), `render_launch_no_env_agent_unchanged`(`:10496`).
- Windows pane 셸 기본 `powershell.exe`, **`CYS_SHELL`로 `cmd.exe` 오버라이드 지원**(`state.rs:2437-2459`) → 셸 문법 가정 금지.

### 5.6 pane env 주입 (`state.rs:1735-1758`)
모든 pane: `CYS_PACK_DIR`, `CYS_SURFACE_ID`, `CYS_SURFACE_REF`(+데몬에 있으면 `CYS_ACCOUNT_DIR`).
**role 있는 pane만**: `CYS_ROLE`. 호출자 지정 env는 마지막에 주입(`:1755-1758`).

---

## 6. 채택안

### 6.1 [무코드] 부트 절차 — ⚠**Windows에서는 D2 수리 전까지 무효**(2026-07-29 저녁 정정)

> **이 절은 낮에 작성된 처방이며, 저녁 실측으로 Windows 적용 불가가 확인됐다. 삭제하지 않고 정정 이력으로 남긴다.**

```
권장:  ▶CEO 버튼 → 그 pane에 "너는 마스터다" → launch-agent 산물(정식 좌석) → 훅이 팀 부팅
```
- 근거: `▶CEO` → `start_master`(`src-tauri/src/main.rs:2124-2130`) → `cys launch-agent --role master --agent claude`.
- 훅은 master pane에서도 발동한다(`role-bootstrap.sh:41`은 worker/cso/reviewer만 차단) → 팀 부팅 유지.
- `launch-agent`가 `takeover_empty_seat: true`를 보내므로 **죽은 좌석의 role 회수**도 함께 일어난다.
- 얻는 것: `agent_meta`·`env_injected`가 기록된 **정식 좌석** → 사망 감지·부활 명단·in-seat 재연결이 모두 살아난다.

**⚠ 플랫폼별 유효성 (2026-07-29 실측)**

| OS | 상태 |
|---|---|
| **macOS** | ✅ 유효 — 신뢰창이 뜨지 않아 자동확인이 필요 없다 |
| **Windows** | ❌ **무효** — D6로 설치폴더에서 기동 → D2로 신뢰창 자동확인이 claude를 종료 → D3로 잔해까지 남음. **▶CEO 2회 시도 모두 실패 실측** |

→ Windows에서는 **§6.6 수동 우회**를 쓰고, **T-D6 → T-D3 → T-D2a 수리 후에야 이 절이 유효**해진다.

### 6.2 [티켓A · 워커] 형제 import 가드 7곳 + 회귀 테스트
**선행 게이트: U1 검증 통과(§14-A). 미통과 시 착수 금지.**

- 대상 7파일(§2.1 표).
- **형태 — `insert(0)`가 아니라 `append` + 중복 가드**:
  ```python
  import os, sys
  _SELF_DIR = os.path.dirname(os.path.abspath(__file__))
  if _SELF_DIR not in sys.path:
      sys.path.append(_SELF_DIR)      # noqa: E402
  ```
  - 근거1: 목적은 **발견**이지 우선순위 강등이 아니다 — 코드베이스 명문 교리 MAJ#1(§2.8).
  - 근거2: 미래에 `bin/secrets.py` 같은 파일이 추가돼도 **stdlib를 가리지 않는다**(오늘 충돌 0건이나 `insert(0)`는 잠재 함정).
  - 근거3: 선례 `javis_report.py:33-34`가 이미 이 형태.
  - 레인 교차 포획 우려 검토 완료: 서브프로세스는 `sys.path`가 매번 새로 시작하므로 타 레인 `bin`이 선점할 경로 없음.
- **배치 규칙(수용 기준)**: 가드는 **첫 형제 import보다 위**. 아래에 붙으면 무효.
- **회귀 테스트(동봉 필수)**: "형제 `javis_*` import가 있는데 경로 가드가 없는 파일이 있으면 실패".
  - **부트 게이트(preflight)가 아니라 테스트로** 넣는다 — 릴리스 중 부팅 실패 위험 0.
  - 선례: P0-7 수리 시 도입한 **소스 grep 회귀 테스트**(`cysd/main.rs` 부트 호출 2회 강제, `main.rs:2306-2318`).
  - 이유: 수정만 하면 **다음 스크립트가 또 잊는다.** 철학 §3-①(산문 계약 → 코드 불변식).

### 6.3 [티켓B · 워커] `._pth` 주석 정정 — **내용 동결**
- 대상: `.github/workflows/windows-build.yml:56-57`, `.github/workflows/release.yml:270-271`.
- **주석만 수정.** 파일 내용(`python312.zip` / `.` / `import site`) **절대 변경 금지** — `import site` 제거 시 pip·site-packages가 죽어 **보호 기능 ①이 깨진다**.
- 티켓에 이 금지선을 명문으로 박는다.

### 6.4 [기록 · 릴리스 후] 후속 3건
| # | 내용 | 이유 |
|---|---|---|
| F1 | 팩 파이썬 테스트를 Windows 잡에서도 실행 | 이 클래스의 유일한 구조적 방지책. **단 34개 중 9개가 unix 전제** → 부분집합 25개 + skip 마커 분류 **선행 필요**(한 스텝 아님) |
| F2 | fail-closed 지점(`javis_event`·`javis_wakeup`) 실패를 승인 Feed로 승격 | 가드를 넣어도 **"조용한 죽음" 구조는 그대로**다 |
| F3 | Windows in-seat 재연결 지원(셸 인지 렌더) | §7 철회안의 올바른 버전. 별도 함수 + 테스트 필요 |

### 6.5 D2~D6 티켓 (신규 · 우선순위 순)

> 실행 주체는 §10 역할 배치를 따른다 — **구현은 워커, master는 직접 구현 금지.**

| 티켓 | 대상 | 내용 | 우선순위 | 비고 |
|---|---|---|---|---|
| **T-D6** | `state.rs` pane 생성 / `main.rs:2124` | cwd 미지정 시 **홈으로 해소**되도록 근원 수리(+`start_master`가 `--cwd` 명시하는 보강) | **최상** | D2 발현률을 100%→낮춤. 단독으로도 정합성 결함 |
| **T-D2a** | `cys.rs:4648-4655` | ★**"자동 시도 → 실패 시 사람 위임"으로 재설계**(§6.7 오너 결정 A안). 단발 전송 + 소멸 확인 + 재전송 상한 → 미해소 시 **사람에게 넘기고 대기** | **최상** | 설계 세부는 §6.7 |
| **T-D2b** | 온보딩/preflight | **선행 신뢰 등록**: 최초 기동 전 대상 폴더 신뢰를 사람이 1회 승인하도록 안내하거나, 신뢰창을 띄우지 않는 실행 형태 채택 | 상 | 자동확인에 의존하지 않는 근본 회피 |
| **T-D3** | `cys.rs:5017` + `handlers.rs:1646` | **롤백 전용 예외** — `cause="reap"` + 생성 시 발급한 nonce 소지자에 한해 자기 생성 surface close 허용(§3.5 1안) | **최상** | 전 OS. 고아 좌석의 유일한 발생원 |
| **T-D4** | `cys.rs:4399-4407` | 셸 프롬프트 판정에 `>` 추가(+Windows 프롬프트 형태 테스트) | 중 | marker 없는 에이전트(codex·grok)에 영향 |
| **T-D5** | UI(`ui/src/main.ts`) | ①기동 중 "에이전트 기동 중…(최대 60초)" 표시 ②실패 진단을 **그 창 안에** 남김 ③롤백된 창의 유령 탭 정리 | 중 | 진단은 이미 정확 — **도달 경로만** 고침 |

### 6.6 사용자 즉시 우회(코드 변경 없음)

**증상**: Windows에서 `▶CEO`·`cys boot`로 노드가 뜨지 않고 백지 창만 남는다.

1. `+ New`로 새 창을 연다(홈 폴더에서 열린다).
2. 에이전트를 **손으로** 실행한다 — 예: `claude-2 --dangerously-skip-permissions`
   신뢰창이 뜨면 **사람이 직접 Enter**(이 경로는 정상 통과).
3. 그 창에서 `cys claim-role master`로 역할을 선언한다.
4. `너는 마스터다`를 입력해 팀을 부팅한다.

⚠ 이렇게 만든 좌석은 §2.5의 **반쪽 좌석**이다(사망 감지·부활 대상 제외). **임시 방편임을 문서에 명시할 것.**

**고아 좌석 정리**: 그 좌석 **자기 창 안에서** `cys close-surface surface:<N> --reap`.
`--reap` 없이 닫으면 묘비가 생겨 그 역할이 자동 부활에서 영구 제외된다 → 되돌리려면 `cys tombstone <role> --remove`.

---

### 6.7 ★오너 결정 (2026-07-29) — A안: 버튼 유지 + 사람 위임

#### 검토된 안
오너 제안: *"`▶CEO`·`▶부서장` 버튼을 제거하고, 워크스페이스/부서를 연 뒤 사용자가 수동으로 클로드를 연결하고 '너는 마스터다'를 입력하는 레거시 방식으로 회귀"*

| 안 | 내용 | 판정 |
|---|---|---|
| **A** | **버튼 유지 + "자동 시도 → 실패 시 사람 위임" + D3·D5·D6 수리** | ✅ **채택(오너 결정)** |
| B | 버튼 유지, Windows에서만 사람 위임 | 미채택(플랫폼 분기 증식) |
| C | 버튼을 설정으로 끌 수 있게(기본 켬) | 미채택(선택지만 늘고 결함은 잔존) |
| D | 버튼 완전 제거(레거시 회귀) | ❌ **기각** — 아래 근거 |

#### D안(버튼 제거)을 기각한 근거 — 설계 시 재론 방지용

1. **버튼은 5개 좌석 중 1개만 담당한다.** "너는 마스터다" → `role-bootstrap.sh` 훅 → `javis_bootstrap.py` ④ → **`cys boot`가 CSO·워커·리뷰어2를 `launch-agent`로 기동**한다.
   실측 증거: 생존 노드 제목이 `cso-claude · x`·`worker-claude · x` = `workflow_title(role,agent,cwd)` 형식 = **launch-agent 산물**.
   → 버튼을 없애도 **나머지 4명은 그대로 D2 경로를 탄다.**
2. **부활(phoenix)이 죽는다.** `cys restore`·`node-recover`도 동일하게 `launch-agent`를 쓴다.
   D2 미수리 시 **Windows에서 자동 부활이 영구 불능**이며, 이는 **조용히** 실패한다(§2.6).
3. **레거시 경로의 산물이 곧 "반쪽 좌석"이다.** `claim-role`은 `agent_meta`를 남기지 않는다(§2.5)
   → 사망 감지 스킵 · 부활 명단 제외 · Windows in-seat 거부.
   회귀하면 **모든 master가 영구히 감시 사각지대**에 놓인다. 오늘 사고의 절반이 이것이었다.
4. 버튼은 2026-07-15 실사고(*"dept-master가 '부서장 스코프=단독 대기'를 환각해 `cys boot`를 건너뜀"*)의 처방으로 도입된 결정론 장치다(`src-tauri/src/main.rs` `spawn_orchestra_boot` 주석).
   *(단, 2026-07-16 도입된 `role-bootstrap.sh` 훅이 레거시 경로에도 결정론을 부여하므로, 이 근거는 4개 중 가장 약하다 — 정직히 기록한다.)*

#### 오너 직관 중 **채택된 것**
> *"자동인 척하다 실패하고, 잔해만 남기고, 사용자는 영문을 모른다"* — 이 판단은 정확하며 **A안의 설계 목표로 승격**한다.
> 다만 그 원인은 버튼이 아니라 **D3(잔해)·D5(침묵)** 이므로, 그 둘을 고쳐 해소한다.

#### A안 설계 요구사항 (설계 시 이대로 구현)

| 요구 | 내용 |
|---|---|
| **R1** | 신뢰창 감지 시 **자동확인을 1회만** 시도한다(현행 2.5초 주기 반복 금지) |
| **R2** | 전송 후 **프롬프트 소멸을 확인**한다. 소멸했으면 진행 |
| **R3** | 소멸하지 않으면 **사람에게 위임**한다 — pane과 UI에 명시 안내:<br>`폴더 신뢰 확인이 필요합니다 — 이 창에서 직접 Enter를 눌러 주세요 (N초 대기 중…)` |
| **R4** | 대기 중에는 **claude를 죽이지 않는다.** 타임아웃 시에도 **잔해를 남기지 않고**(D3 수리 선행) 정직하게 실패 보고 |
| **R5** | 실패 진단은 **그 창 안에 남긴다**(토스트만으로 끝내지 않음 — D5) |
| **R6** | 기동 중에는 창에 **"에이전트 기동 중…(최대 60초)"** 를 표시한다(백지 금지 — D5) |
| **R7** | `--cwd` 미지정 시 **홈으로 해소**(D6) — 신뢰창 자체가 덜 뜨게 만드는 선행 완화 |

**의존 순서**: D6 → D3 → D2a(R1~R4) → D5(R5·R6).
D3를 먼저 고쳐야 R4의 "잔해 없이 실패"가 성립한다.

**검증 기준(수용 조건)**
- 신규 Windows 설치에서 `▶CEO` 1회로 master가 뜬다(사람이 Enter 1회 눌러도 성공으로 인정).
- 실패 시 `cys list`에 **role을 쥔 고아 좌석이 0개**다.
- 실패 원인이 **그 창 화면에 남아 있다**(토스트를 놓쳐도 확인 가능).

---

## 7. 철회한 대안과 이유 (재제안 방지)

| 대안 | 철회 이유 |
|---|---|
| **사망 감지를 `seat` 기반으로 전환** | `agent_meta`는 *의미적* 판정(그 바이너리가 사는가), `seat`는 *존재* 판정. 교체 시 **정밀도 하락**(자식 프로세스만 남으면 사망 놓침) + `agent_seen` 래치·401 차단·3회 서킷브레이커·poison 원장 **4겹 상실**. 치명 위험 ①폭주 ②무clear ③자가치유 전멸에 직결. → 대안: **교체가 아니라 추가** — `role.seat_vacant` 저심각도 이벤트만 발행(자동 조치 없음) |
| **`cfg!(unix) \|\| env_injected` 제거·교체** | 그 줄은 플랫폼 차별이 아니라 **계정격리 계약**(§5.5). 제거 시 Windows에서 `CLAUDE_CONFIG_DIR` 없이 in-seat 기동 → **멀티계정 격리 붕괴**(치명 위험 ④). → 올바른 방향은 F3 |
| **`._pth`에 팩 bin 경로 추가 / `.pth` 파일 배치** | 팩 레인이 가변(`CYS_PACK_DIR`, 부서 팩)인데 정적 경로는 **한 레인에 고착** → **부서 격리 위반** |
| **preflight에 회귀 잠금 추가** | 방향은 옳으나 부트 게이트라 릴리스 중 **온보딩 실패 위험** → 같은 목적을 **테스트**로(6.2) |
| **빈 surface 수정안 3종** | 오너 결정으로 **종결**(범위 밖) |

---

## 8. 파급 분석 (Dependency / Coupling / Ripple)

### 티켓A (형제 import 가드)
| 축 | 분석 |
|---|---|
| 직접 의존 | `bin/` 7파일. 순수 추가 3줄, 기존 심볼·시그니처 무변경 |
| 호출 관계 | 호출자는 `[sys.executable, script, …]` — **인터페이스 무변경** |
| 구조 관계 | 상속·합성 없음. `javis_briefing`이 `javis_event`를 모듈로 import → 가드가 import 시 1회 실행(무해) |
| 데이터·스키마 | 없음 |
| 테스트 | 기존 34종 무영향. **신규 회귀 테스트 1개 추가** |
| 설정·빌드 | 없음(팩 임베드는 기존 파이프라인) |
| 문서 | 없음 |
| 보호기능 ①② | **무관** — `sys.path`(파이썬 모듈 경로) vs `PATH`(실행파일 경로). 기전이 다르고 교차점 없음 |
| 잠재 결합 | `insert(0)` 선택 시 **stdlib shadowing 잠재 위험** → `append`로 회피(채택) |

### 티켓B (`._pth` 주석)
| 축 | 분석 |
|---|---|
| 직접 의존 | 워크플로 주석 2줄 |
| 리스크 | **주석 작업이 내용 변경으로 번지면 보호기능 ① 붕괴** → 금지선 명문화로 통제 |

### 무코드 처방 (`▶CEO`)
| 축 | 분석 |
|---|---|
| 코드 변경 | **0** |
| 상태 변경 | 죽은 좌석의 role 회수 + 새 surface 1개 생성 |
| 리스크 | `seat_claimable` 판정이 거짓이면(최근 사람 입력·자손 존재) `claim_denied` → **비파괴 거부**. 안전 |

---

## 9. 부트 체인 치명 위험 4종 대조

| 위험 | 채택안 영향 | 판정 |
|---|---|---|
| ① 폭주(에이전트 큐 남발) | 티켓A/B: 무관. 무코드: 노드 수 불변 | 🟢 |
| ② 무clear 게이트(컨텍스트 100% 초과 지속) | 컨텍스트 사이클 경로 무접촉 | 🟢 |
| ③ 주기 자가치유 전멸 | 사망 감지·복구 로직 **무변경**(철회안이 건드리려던 곳) | 🟢 |
| ④ 전 pane 에이전트 사망 | 계정격리 계약 **무변경**. 티켓B 금지선으로 런타임 보호 | 🟢 |

> **철회한 두 대안은 ①②③④ 전부에 닿아 있었다.** 채택안은 어느 것도 닿지 않는다.

---

## 10. LLM Orchestration 앵커 준수

> CEO·master는 기획·전략·의사결정·분담·관리감독만. **직접 구현 금지.**
> worker=실행, reviewer1=적대검증, reviewer2=감사.

| 단계 | 담당 | 비고 |
|---|---|---|
| 0. 좌석 복구(`▶CEO`) | **오너** | UI 조작 |
| 1. 증거 수집(§13) | **CSO** | 자원·노드 진단은 CSO 직무(`CSO_DIRECTIVE §1`) |
| 2. U1 검증(§14-A) | **CSO** | 같은 패스 |
| 3. 판정·분해·위임 | **master** | **직접 구현 금지 — 티켓에 명문 박기** |
| 4. 구현(티켓A·B) | **worker** | 7파일은 master가 "간단하니 내가" 하기 쉬운 크기 → 금지 문구 필수 |
| 5. 적대 검증 | **reviewer 1** | 가드 위치·shadowing·레인 격리·U1 재확인 |
| 6. 감사 | **reviewer 2** | §11 체크리스트 |

---

## 11. 보호 기능 무손상 검증 체크리스트 (reviewer 2용)

- [ ] `src/lib.rs`의 `runtime_bin_dirs` / `runtime_prefixed_path` / `compose_pane_path` / `compose_unix_pane_path` **무변경**
- [ ] `python312._pth` 파일 **내용 무변경**(3줄 그대로, `import site` 유지)
- [ ] 워크플로의 get-pip 부트스트랩 단계 무변경
- [ ] pane 안에서 `npm -v` · `git --version` · `python3 -V` 정상(동봉 런타임 발견)
- [ ] claude 사후 설치 후 새 pane에서 `claude --version` 발견(레지스트리 재합성 동작)
- [ ] `cys.rs:6805` 및 `render_launch` **무변경**(계정격리 계약 보존)
- [ ] `governance.rs` 사망 감지 블록 **무변경**

---

## 12. 실행 순서와 게이트

```
[0] 오너: ▶CEO로 master 복구            ← 지금 상태는 스스로 낫지 않음
     │
[1] CSO: 증거 수집(§13) + U1 검증(§14)   ← 기계 재시작 금지·읽기 위주
     │
     ├─ U1 거짓 → 티켓A 폐기, 제보자에 회신
     └─ U1 참  → [2]
     │
[2] master: 판정 → 워커 위임(오너 승인)
     │
[3] worker: 티켓A(가드+회귀테스트) · 티켓B(주석만)
     │
[4] reviewer1 적대검증 → reviewer2 감사(§11)
     │
[5] 팩 재임베드·서명·릴리스(무중단 팩 채널)
     │
[후] F1(Windows CI) · F2(Feed 승격) · F3(in-seat 셸 인지 렌더)
```

**게이트**: U1 미검증 상태에서 티켓A 착수 금지 / 증거 확보 전 재현 시도 금지 / 수정은 오너 승인 후.

---

## 13. (a) 원인 규명 설계 — 가설↔판별증거

> 증거를 모으고 해석을 논쟁하지 않는다. **어떤 증거가 어떤 가설을 죽이는지 먼저 고정한다.**

| # | 가설 | 확정 증거 | 기각 증거 | 수집 명령 |
|---|---|---|---|---|
| H1 | watchdog 중복 프로세스 자동 kill | `watchdog.duplicates_killed`에 master pid | 이벤트 부재 또는 `CYS_AUTOKILL_DUP` 미설정 | `cys events --after-seq <N>` · 환경변수 확인 |
| H2 | 자원 게이트 차단 | `javis_resource_gate` 차단 판정 기록(`javis_bootstrap.py:455` 경유) | 통과 기록 | `~/.cys/state/boot-last.json` |
| H3 | claude 자체 종료(내부 오류·쿼터) | claude 세션 로그의 종료 사유 | 로그 무흔적 | `CLAUDE_CONFIG_DIR` 하위 세션 파일 |
| H4 | 외부 텍스트 주입으로 종료 | scrollback에 오너가 안 친 입력 | 없음 | `cys read-screen --surface <master>` |
| H5 | OS 수준 크래시/OOM | Windows 이벤트 로그 항목 | 없음 | 이벤트 뷰어 |

**수집 우선순위**: scrollback → claude 로그 → 이벤트 되감기 → `boot-last.json` → OS 로그.
**주의(플래그 실측 확인 2026-07-29)**: 과거 재생은 **`cys events --after-seq <N>`** 이다(`--since` 아님).
`--reconnect`는 재접속 옵션이지 과거 재생이 아니다. `--cursor-file`로 커서 영속 가능.
`cys read-screen`은 `--surface` · `--lines` · `--max-lines`(기본 2000) · `--since`(라인 커서 델타)를 받는다.

---

## 14. 재현·검증 명령 모음

### A. U1 판별 (Windows · 10초) — **가장 중요**
```powershell
<설치폴더>\runtime\python\python3.exe <팩>\bin\javis_event.py --help
```
- `ModuleNotFoundError: javis_scrub` → **U1 참**(제보 확정)
- 정상 출력 → **U1 거짓**(티켓A 폐기)

보조:
```powershell
<설치폴더>\runtime\python\python3.exe -c "import sys; print(sys.path)"
```

### B. 좌석 상태 확인 (Windows)
```powershell
cys status --json      # master의 role·agent·agent_alive·seat 확인
cys list
```
판정 기준: `role=master` + `agent=null` + `agent_alive=null` → **반쪽 좌석 확정**.

### C. mac에서 재현 가능한 측정 (설계 검증용)
```bash
# 미가드 파일 목록
cd cysjavis-pack/bin && python3 -c "
import re,glob
imp=re.compile(r'^([ \t]*)(?:import|from)\s+(javis_\w+)',re.M)
pf=re.compile(r'sys\.path\.(insert|append)\s*\([^)]*(__file__|_SELF_DIR|_bindir)',re.M|re.S)
print([f for f in sorted(glob.glob('*.py'))
       if imp.findall(open(f).read()) and not pf.search(open(f).read())])"

# stdlib 이름 충돌 확인
# 팩 소유권 분포 · 훅 exit 0 개수 등은 본문 §2 참조
```

---

## 15. 남은 리스크 · 열린 질문

| # | 리스크 | 대응 |
|---|---|---|
| R1 | (a) 원인이 **Claude CLI 자체**로 밝혀지면 우리가 못 고침 | 오너 결정: **우회책을 찾아 산다** |
| R2 | U1이 거짓이면 티켓A는 무의미한 변경 | 그래서 U1이 **선행 게이트** |
| R3 | 무코드 처방은 **사람 습관 의존** — 잊으면 반쪽 좌석 재발 | 결정론화는 부트스트랩 경로 변경이 필요(현 시점 미채택). **다음 판에 재검토** |
| R4 | F1(Windows CI)을 안 하면 같은 클래스가 계속 유입 | 릴리스 후 최우선 후속 |
| R5 | 가드를 넣어도 **"조용한 죽음" 구조는 유지** | F2로 별도 처리 |

**열린 질문(설계 시 결정할 것)**
1. 회귀 테스트를 팩 테스트(python)로 둘 것인가, Rust 소스 grep 테스트로 둘 것인가? (P0-7 선례는 후자)
2. `role.seat_vacant` 저심각도 이벤트(철회안의 안전한 대체)를 이번에 넣을 것인가, F 계열로 미룰 것인가?
3. F1의 테스트 분류 기준(어떤 9개를 skip 처리할 것인가)

---

## 16. 부록 — 코드 색인

| 주제 | 위치 |
|---|---|
| 미가드 7파일 | §2.1 표 |
| 가드 선례(append) | `cysjavis-pack/bin/javis_report.py:33-34` |
| 가드 선례(레인 기준) | `cysjavis-pack/hooks/inject_gate.py:22` |
| 호출 형태 | `cysjavis-pack/bin/javis_report_gate.py:514, 527` |
| `._pth` 생성 | `.github/workflows/windows-build.yml:57` · `release.yml:271` |
| Windows T5 스모크 | `.github/workflows/windows-build.yml:380-402` |
| 스모크의 phoenix 로드 | `cysjavis-pack/bin/javis_phoenix_win_smoke.py:613-614` |
| phoenix 내부 가드 | `cysjavis-pack/bin/javis_phoenix.py:369-375, 387` |
| 팩 소유권 SOT | `src/pack.rs:562, 573, 577-587` (테스트 `:2279-2291`) |
| 사망 감지 | `src/bin/cysd/governance.rs:276-360` |
| 좌석 정의·갱신 | `src/bin/cysd/governance.rs:13, 1365-1392, 1405-1420, 1554` |
| 좌석 승계 판정 | `src/bin/cysd/governance.rs:1428-1450` |
| 배달 SEAT 게이트 | `src/bin/cysd/governance.rs:2060-2125` |
| `set_meta`(agent_meta 유일 기록) | `src/bin/cysd/handlers.rs:3511-3560` · 호출자 `src/bin/cys.rs:4613-4616` |
| `claim_role` | `src/bin/cysd/handlers.rs:1743-1790` |
| 좌석 승계 소비처 | `src/bin/cysd/handlers.rs:996-1006, 1793-1806` |
| topology(live=role만) | `src/bin/cysd/handlers.rs:3754-3776` |
| `run_restore` in-seat | `src/bin/cys.rs:6718-6880`(가드 `:6805`, agent 미상 skip `:6774`) |
| `launch-agent` takeover | `src/bin/cys.rs:4990` |
| `render_launch` | `src/bin/cys.rs:4477-4492` (테스트 `:10467, :10496`) |
| pane env 주입 | `src/bin/cysd/state.rs:1735-1758, 1803` |
| Windows 셸 선택 | `src/bin/cysd/state.rs:2437-2459` |
| **보호기능 ①②** | `src/lib.rs:103-108, 153-184`(+`compose_unix_pane_path`) |
| `▶CEO` 배선 | `src-tauri/src/main.rs:2124-2130` |
| 부트스트랩 단계 | `cysjavis-pack/bin/javis_bootstrap.py:7-8, 63, 455, 565-583` |
| 부트 훅 role 게이트 | `cysjavis-pack/hooks/role-bootstrap.sh:38-42` |
| preflight C60(b) | `cysjavis-pack/bin/javis_preflight.py:3426-3432` |
| P0-7 소스 grep 테스트 선례 | `src/bin/cysd/main.rs:2306-2318` |

---

## 17. 이 기획서의 판단 원칙 (설계 시 승계할 것)

1. **미검증 전제 위에 코드를 얹지 않는다.** 확정과 추정을 같은 표에 넣지 않는다.
2. **"근본 수리"는 결합도를 세어본 뒤에만 쓴다.** 상태 기계·격리 계약을 건드리는 변경은 근본이 아니라 위험이다.
3. **증거를 얻을 수 있으면 이론보다 증거를 먼저 요청한다.**
4. **수정에는 잊히지 않게 하는 장치를 같이 넣는다.** 없으면 고친 게 아니라 미룬 것이다.
5. **발견이 목적이면 append. 기존 우선순위를 강등하지 않는다.**(MAJ#1)
6. **master는 구현하지 않는다.** 티켓에 실행 주체를 반드시 명시한다.

---

# 18. 세션 인계 (HANDOFF) — 2026-07-29 저녁

> **다음 세션은 이 절부터 읽어라.** 컨텍스트 초기화 후 이어가기 위한 상태 기록이다.
> 이 문서 하나로 배경·근거·결정·다음 행동이 전부 복원된다(§0~§17 참조).

## 18.1 지금까지 완료된 것

| # | 완료 항목 | 산출물 |
|---|---|---|
| 1 | Windows 0.14.4 결함 **6건 실측 확정** | 본 문서 §0.1·§3 |
| 2 | 증거 수집 런북 | `docs/plans/2026-07-29-win-evidence-runbook.md` |
| 3 | 오너 결정 **A안**(버튼 유지 + 사람 위임) 기록 | §6.7 |
| 4 | **티켓A 구현 완료**(D1 수리) | 아래 §18.2 |
| 5 | 리뷰어2(감사) 검증 통과 | 아래 §18.3 |

## 18.2 미커밋 변경 목록 (작업 트리 상태)

```
 M .github/workflows/ci-branch.yml          (+2/-1)  test_import_guard 등재 + 주석 7종
 M .github/workflows/pack-release.yml       (+1/-1)  동上
 M .github/workflows/release.yml            (+2/-2)  동上(2개 잡)
 M cysjavis-pack/bin/javis_event.py         (+11)    sys.path append 가드
 M cysjavis-pack/bin/javis_wakeup.py        (+11)    동上
 M cysjavis-pack/bin/javis_memory.py        (+11)    동上
 M cysjavis-pack/bin/javis_memory_inject.py (+11)    동上
 M cysjavis-pack/bin/javis_orchestra.py     (+11)    동上
 M cysjavis-pack/bin/javis_task.py          (+11)    동上
 M cysjavis-pack/bin/javis_phoenix_win_smoke.py (+11) 동上
 A  cysjavis-pack/bin/tests/test_import_guard.py     회귀 잠금(스테이징 완료)
 ?? docs/plans/2026-07-29-win-two-defects-plan.md    (이 문서)
 ?? docs/plans/2026-07-29-win-evidence-runbook.md
```

**Rust 코드 변경 0줄.** 커밋은 **아직 안 했다**(오너 판단 대기).

가드 형태(7개 파일 공통·동일 11줄 블록):
```python
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)      # ★insert(0) 아님 — MAJ#1 교리
```

## 18.3 검증 상태 (전부 실행 확인)

| 검증 | 결과 |
|---|---|
| 7개 파일 문법(`py_compile`) | 전건 OK |
| 가드가 첫 형제 import보다 앞 | 7/7 (삽입 시 프로그램으로 강제) |
| **실패 조건 재현**(`runpy`·스크립트 폴더 미등재) | 형제 import 통과 |
| **음성 시험**(가드 제거) | Windows와 동일한 `No module named 'javis_scrub'`로 FAIL → **잠금 작동 확인** |
| 신규 테스트 | **33/33 PASS** (exit 0) |
| 기존 팩 테스트 6종 회귀 | 전건 PASS |
| 레인 대조 게이트(3워크플로 목록) | 통과 · 비대칭 0 |
| **리뷰어2(감사)** | ✅ 보호 대상 ①② **무손상 확정** · 침해 0건 |
| **리뷰어1(적대적 검증)** | ✅ 완료 — **BLOCK 0건** · REVISE 4건(2건 즉시 반영, 2건 이월) |

리뷰어2가 잡아낸 블로킹 1건(신규 테스트 git 미추적 → CI 3종 즉시 실패)은 **`git add`로 처리 완료**.

### 18.3.1 리뷰어1(적대적 검증) 결과 — BLOCK 0건 · REVISE 4건

**즉시 반영 완료 2건**

| # | 지적 | 조치 |
|---|---|---|
| **R3** | `windows-build.yml:56` 주석 *"스크립트 dir(.)"* 이 **거짓** — `._pth` 의 `.` 은 python 루트 기준. 이 거짓 주석이 남으면 다음 사람이 "가드는 중복"이라 판단해 **되돌린다**(2026-07-28 heal 사고의 재발 경로) | ✅ 두 워크플로 주석 정정 + **내용 3줄 동결 경고** 명기. `printf` 줄 바이트 불변 확인 |
| **R4** | 7파일 가드 주석의 선례 인용 오류 — `hooks/inject_gate.py:22` 는 `insert(0)`+env 경로라 **append 선례가 아니다** | ✅ 7파일 주석 정정(`javis_report.py:33-34` 만 선례로 인용) |

**이월 2건 → ✅ 2026-07-29 후속 세션에서 반영 완료 (상세 §18.8)**

| # | 지적 | 근거·처방 |
|---|---|---|
| **R1** | ★**회귀 테스트가 4가지 우회를 통과한다**(리뷰어가 실제 파일을 심어 37/37 PASS 실증). 특히 `_GUARD_VAR` 정규식의 `[\s\S]*?` 때문에 **`append` 가 import 뒤 500줄에 있어도 "가드가 앞"으로 오판**한다 — 사람이 가장 실수하기 쉬운 형태다. `importlib.import_module`·`import os, javis_x`(콤마)·`if False:` 죽은 코드·주석 속 문자열도 우회 | **처방: 정규식 → `ast.parse` 전환.** `ast.Import/ImportFrom` 의 `lineno` vs `sys.path.append/insert` **Call 노드**의 `lineno` 비교. 주석·문자열·죽은 코드·콤마결합이 한 번에 해소. `importlib`/`__import__` 는 별도 룰<br>⚠**단 현 테스트에 이빨은 있다** — 가드 제거 뮤턴트로 30/32 정상 FAIL 확인 |
| **R2** | ★**Windows 결함의 회귀 테스트가 Windows에서 0회 실행**된다. 등재 4곳이 전부 macOS/ubuntu 잡이며, 실제 문제 런타임인 **번들 embeddable(`python312._pth`)로는 한 번도 안 돈다**. `③`은 `runpy` 로 조건을 **모사**할 뿐. 게다가 그 embeddable 을 만드는 `windows-build.yml` 은 **레인 대조 게이트의 `LANES` 에도 없다**(ci-branch.yml:75-79 · 게이트가 스스로 적어둔 "한계 6") | **처방: F1(§6.4)과 통합.** Windows 잡에서 동봉 `runtime\python\python3.exe` 로 이 테스트를 1회 실행 + `windows-build.yml` 을 `LANES` 에 등재 |

**ACCEPT(공격 실패) — 재검증 불요**
- 가드 배치·인코딩: 7파일 `import os`/`sys` 가 컬럼0에서 가드보다 앞(줄번호 확인) · BOM 0 · CR 0 · `py_compile` 전건 OK
- **`append` 선택**: `insert(0)` 대비 실패 시나리오 **재현 실패**. 근거 — 서브프로세스는 `sys.path[0]`=자기 레인 폴더, Windows `._pth` 는 `PYTHONPATH`·cwd 무시(저장소에 `PYTHONPATH` 설정 코드 0건) → **타 레인 bin 이 먼저 들어올 경로가 없다**
- 부작용: `_SELF_DIR` 충돌 0 · 소비자(팩 테스트 3종·`inject_gate`) 무영향 · 레인 등재 7종 로컬 전건 PASS
- 놓친 파일: 팩 158개 `.py` 전수 스캔 — **가드 없이 형제 bare import 하는 파일 0건**
- 워크플로: YAML 파싱·`bash -n`·루프 확장·레인 게이트 전건 OK

**NIT(이월)**: ①`③` 런타임 검증이 2개 파일 하드코딩 — 나머지 5개는 정적(①②)만이 방어선인데 R1로 뚫려 있음 ②`_SIB` 가 `javis_` 접두로 한정 — `_naver_http`·`holdout_eval` 등 비-javis 형제 import 는 게이트 밖(현재 전부 자체 가드 보유·무해) ③`assert` 는 `python -O` 에서 소거(현 레인 무영향)

## 18.4 다음 세션에서 할 일 (우선순위)

1. ~~★**R1 반영 — 회귀 테스트를 `ast` 기반으로 재작성**~~ → ✅ **완료**(§18.8)
2. ~~★**R2 반영 — Windows 잡 embeddable 실행 + 레인 등재**~~ → ✅ **완료**(§18.8)
3. **커밋 여부 오너 승인** — 승인 전 커밋 금지.
3. **설치파일 빌드 전 반영 확인** — 팩은 `build.rs`가 바이너리에 임베드하므로,
   **빌드 전에 티켓A가 들어가 있어야** 재설치·다운그레이드 시 재발하지 않는다.
4. (오너 판단) 제보자 회신 — D1 확정 사실 + `insert(0)`→`append` 변경 사유(§6.2 근거2).
5. 나머지 티켓 착수: **T-D6 → T-D3 → T-D2a → T-D5** (§6.5·§6.7 R1~R7).
   T-D2a 착수 전 **M1 규명 필요**(`\r`이 왜 취소로 처리되는지 — §3.4 미확정 항목).

## 18.5 절대 잊으면 안 되는 제약

- **보호 대상 2기능 무손상**: `src/lib.rs:103-108` `runtime_bin_dirs` / `:153-184` `runtime_prefixed_path`.
  검증 체크리스트는 §11.
- **`._pth`는 주석만. 내용(3줄·`import site`) 동결** — 건드리면 보호기능 ① 붕괴.
- **팩 97%가 System 등급** → 현장 로컬 수리는 원복된다. 반드시 저장소 소스에 반영.
- **역할 배치**: 구현은 **worker**, master는 직접 구현 금지(§10).
- **철회안 재제안 금지**: 사망감지 seat 전환 · `cfg!(unix)` 가드 제거 · 버튼 제거(D안) — 근거 §7·§6.7.

## 18.6 오너 기계(Windows) 현재 상태

- master 미기동. CSO·worker는 생존(구세대). 리뷰어 2명 소실.
- 고아 좌석 정리·`tombstone master --remove`·`cys resume` 완료.
- **즉시 우회로**(§6.6)로 master를 세울 수 있다: 새 창 → `claude-2 --dangerously-skip-permissions`
  → 신뢰창에 **사람이 직접 Enter** → `cys claim-role master` → `너는 마스터다`.
  ⚠ 이 좌석은 반쪽 좌석이다(사망 감지 없음) — 임시 방편.

## 18.7 다음 세션 첫 행동 (권장)

```
1) 이 문서 §18 을 읽는다(★**§18.9 가 최신 상태** — §18.8.3 표는 적대 검증 전 값이라 낡았다)
2) git status 로 §18.2 + §18.8.4 와 대조한다(변경이 그대로인지)
3) python3 cysjavis-pack/bin/tests/test_import_guard.py  → **70/70 PASS** 확인
   python3 cysjavis-pack/bin/tests/test_import_guard.py --selftest → **30/30 PASS**
4) 적대 검증 보고서 4종 확인: `docs/plans/review-2026-07-29-{r1-adversarial-codex,r2-ci-gemini,round2-codex,round2-gemini}.md`
```

---

## 18.8 R1·R2 반영 완료 (2026-07-29 후속 세션)

### 18.8.1 R1 — 회귀 테스트 `ast` 전환 ✅

`cysjavis-pack/bin/tests/test_import_guard.py` **전면 재작성**(정규식 → `ast.parse`).

| 이전 정규식이 뚫린 우회 | 새 판정 |
|---|---|
| (1) `_SELF_DIR=` 만 앞·`append` 는 import 200줄 뒤 | **`Call` 노드의 `lineno`** 로 본다(대입 위치 아님) |
| (2) `importlib.import_module` · `__import__` | 동적 import 별도 룰로 형제 판정 |
| (3) `import os, javis_x` (콤마) | `ast.Import.names` 전건 순회 |
| (4) `if False:` 죽은 코드 · 주석 · 문자열 · 미호출 함수 속 가드 | **모듈 로드 시 실제 실행되는 문**만 가드로 인정 |

부수 해소 — **NIT②**: 형제 판정이 `javis_` 접두 한정 → **같은 폴더에 `<이름>.py` 가 실재하는가**로 전환.
부수 해소 — **NIT①**: `③` 실런타임 재현이 2개 하드코딩 → **최상위 형제 import 를 가진 전 파일**(현재 8종) 순회.
  `run_name` 을 `__main__` 이 아닌 `__guardprobe__` 로 바꿔 `main()` 부작용 없이 모듈 레벨만 돌린다.

★**스코프 지배 규칙**(재작성 중 실측으로 도출): 가드가 함수 안에 있어도 **같은 함수 안의 import
  앞이면 유효**하다(실재 형태 — `javis_compete`·`javis_idempotency`·`javis_phoenix`·`javis_purge_verify`).
  단순히 "모듈 최상위 가드만 인정"으로 짜면 이 4파일이 **거짓 FAIL** 난다(초안이 실제로 그랬다).
  판정은 `is_protected()` — 가드가 ①더 앞줄이고 ②import 를 감싸는 스코프(모듈 또는 그 함수 자신·바깥)일 것.

**신설 `④ 셀프테스트**(`--selftest`): 우회 검체 8종 + 양성 대조 4종을 임시 폴더에 심고
  전자는 반드시 FAIL·후자는 반드시 PASS 함을 확인한다. **스캐너가 무뎌지면 여기서 먼저 터진다.**

수용 기준 대비 검증(전부 실행):
- 리뷰어 제시 4우회(+파생 4종) 심었을 때 **전건 FAIL** ✅ (셀프테스트 12/12)
- 가드 제거 뮤턴트 FAIL 유지 ✅ — 실파일 2종으로 확인:
  `javis_event` 가드블록 제거 → `naked=(28,'javis_scrub')`, `javis_compete` 함수내 가드 제거 → `naked=(402,'javis_verdict')`.
  둘 다 **문법오류가 아닌 '가드 부재' 판정 경로**로 탐지(초안은 문법오류로 우연히 탐지돼 재작성함).
- `③` 음성 시험: 가드 제거 시 Windows 실측과 동일한 `No module named 'javis_scrub'` 로 FAIL ✅
- 검사 대상 15종 유지(커버리지 회귀 0) · 전체 **52/52 PASS**

### 18.8.2 R2 — Windows 실런타임 실행 + 레인 등재 ✅

**① `windows-build.yml` 에 `T6` 스텝 신설** (`shell: pwsh` · `timeout-minutes: 5` · `PYTHONUTF8=1`):
설치된 `runtime\python\python3.exe`(동봉 embeddable)로 회귀 스위트를 **그대로 1회 실행**한다.
전제·음성 대조를 함께 건다 —
- `python312._pth` 부재 시 **throw**(embeddable 이 아니면 이 초록은 증거가 못 된다 · fail-closed)
- **음성 대조**: 임시 폴더에 `probe_sib.py`/`probe_main.py` 를 심어 이 런타임이 정말 스크립트 폴더를
  `sys.path` 에 넣지 **않는지** 먼저 증명. 여기서 exit 0 이 나오면 결함 전제가 바뀐 것이므로 **throw**.
- ⚠`$ErrorActionPreference='Stop'` + `2>&1` 변수 캡처 금지(네이티브 명령 `NativeCommandError`).
  이 프로브는 실패를 기대하므로 캡처하면 판정이 뒤집힌다 → 종료코드만 본다.

**② 레인 대조 게이트에 `windows-build` 편입** (`ci-branch.yml`).
동급 `LANES` 에 넣으면 나머지 6스위트가 전부 비대칭으로 잡혀 `ALLOWED` 가 근거 없는 목록으로 썩는다
(windows-build 는 전 스위트를 도는 레인이 아니다). 그래서 **`SUBSET_LANES`(부분 레인)** 라는
방향성 있는 약한 계약을 신설했다:
- **⊆** 여기서 도는 스위트는 완전 레인에도 있어야 한다(부분 레인 전용 고아 차단)
- **⊇** `REQUIRED`(= `test_import_guard`)는 반드시 보유(그 레인에서만 성립하는 검증의 조용한 제거 차단)

★**실측으로 잡은 함정**: T6 스텝 **제목**에 스위트 파일명을 적었더니, 실행 줄을 지워도 게이트가
  제목의 이름을 긁어 **초록을 유지**했다(게이트는 주석만 지우고 `name:` 은 긁는다). 제목에서 토큰을
  제거하고 그 이유를 워크플로 주석에 박아뒀다 — 이 레인의 보유 증거는 `$suite` 경로 **한 곳뿐**이다.

게이트 검증(로컬에서 스크립트 추출해 실행):
- 무변경 → `windows-build=1종 · 필수 보유 1/1` · exit 0 ✅
- 필수 스위트 제거 → exit 1, `필수 스위트 'test_import_guard' 이 … 사라졌다` ✅
- 부분 레인 전용 고아 주입 → exit 1, `이 부분 레인에만 있다` ✅

### 18.8.3 검증 결과 ⚠**이 표는 적대 검증 전 상태다 — 최신은 §18.9.4**

| 검증 | 결과 |
|---|---|
| `test_import_guard.py` | **52/52 PASS** (exit 0) |
| 셀프테스트 단독(`--selftest`) | 12/12 PASS |
| 레인 등재 7스위트 전건 회귀 | 전건 exit 0 |
| `py_compile` (변경 8종) | 전건 OK |
| 4개 워크플로 YAML 파싱 | 전건 OK · windows-build 스텝 13→14 |
| `._pth` 내용 3줄 | **무변경 확인**(`printf` 줄 diff 0) |
| Rust 보호 대상 ①② | `src/`·`src-tauri/src/` **변경 0건** |

### 18.8.4 §18.2 이후 추가된 변경

```
 M .github/workflows/ci-branch.yml       SUBSET_LANES(부분 레인 계약) 신설 + 한계 6 주석 갱신 + 스텝명
 M .github/workflows/windows-build.yml   T6 스텝 신설(+60/-1) · ._pth 줄 무접촉
 M cysjavis-pack/bin/tests/test_import_guard.py  전면 재작성(ast 전환 · 셀프테스트 신설) · git add 완료
```

---

## 18.9 적대 검증 1차 결과와 반영 (2026-07-29)

두 리뷰어에 **분담 위임**(producer≠evaluator — 생산 노드는 자기검증하지 않았다).
보고서 원본: `docs/plans/review-2026-07-29-r1-adversarial-codex.md` ·
`docs/plans/review-2026-07-29-r2-ci-gemini.md`

### 18.9.1 판정

| 리뷰어 | 대상 | 판정 |
|---|---|---|
| codex | R1 스캐너 | **BLOCK 3 · REVISE 3 · ACCEPT 2** |
| gemini | R2 CI 배선 + §11 감사 | **BLOCK 0 · REVISE 3**(감사는 ACCEPT — 보호 대상 무손상) |

★1차 AST판의 근본 오류: **"문법 노드 존재 + 소스 줄"을 런타임 지배로 오인**했다.

### 18.9.2 BLOCK 3건 — 전부 반영

| # | 지적(실측 재현됨) | 반영 |
|---|---|---|
| **BLOCK-1** | 동적 import **6형태**(`exec`/`eval`/변수·f-string `__import__`/변수 `import_module`/데코레이터 실행식)를 **검사 대상에서 통째로 누락** — FAIL 도 아니고 조용히 사라진다 | 정적 증명 불가 형태를 **금지(opaque)** 로 fail-closed. 데코레이터·기본값·애노테이션·클래스 base 등 **정의 시점 실행식**도 방문 |
| **BLOCK-2** | `if ():` 확정 거짓 분기를 unknown 으로 흘리고, `try/finally` 되돌리기·`sys.path.clear()` 로 **철회된 가드**를 유효로 인정 | `_static_truth` 가 리터럴 컨테이너·`not` 처리. **destroy 이벤트** 도입 — `remove/pop/clear`·`sys.path=` 재대입·`del` 이 가드↔import 사이에 있으면 그 가드 무효 |
| **BLOCK-3** | ③ 이 한 프로세스에서 순차 실행 → 앞 검체가 남긴 `sys.path`·`sys.modules` 로 뒤 검체가 **무임승차**(합성 검체로 실증) | ③ 을 **파일마다 독립 자식 프로세스**로. 중립 `cwd` + `PYTHONPATH` 제거 |

⚠BLOCK-3 은 현행 트리에서는 **잠복**이었다(실측: 8파일 전부 잔류 0). 그러나 경로를 두 번 심는
파일이 하나 생기면 즉시 살아난다 — 프로세스 격리로 그 부류를 통째로 제거했다.

### 18.9.3 REVISE 6건 — 5건 반영 · 1건 의도적 미반영

| # | 지적 | 반영 |
|---|---|---|
| codex R-1 | 정상 형태 **거짓 FAIL** 6종(`from sys import path` · `q = sys.path` · 호출되는 헬퍼 가드 · 함수 내 import 의 소스 줄 · `TYPE_CHECKING` · 데코레이터 가드) | 별칭 추적 · 헬퍼 **호출** 인정 · TYPE_CHECKING 런타임 제외 · **스코프 교차 시 줄 순서 미적용** |
| codex R-2 | `run_name='__guardprobe__'` 는 `__main__` 블록 미실행인데 분류는 최상위로 표시(현행 영향 0건이나 의미 불일치) | main 전용 import 는 ③ 대상 제외(①② 유지) |
| codex R-3 | 셀프테스트가 **이미 잡는 것만** 확인 = 자기 확인 | 검체 12 → **30종**(우회 17 + 양성 대조 13) |
| codex ACCEPT 부기 | "클래스 본문은 정의 시 실행되지 않는다"는 주석이 **거짓** | 클래스 본문을 별도 스코프가 아니라 **인라인 실행**으로 정정 |
| gemini A-major | 음성 대조가 `exit≠0` 만 봐서 **실패 이유를 검증 안 함** — 경로 오타·DLL 부재도 "정상"으로 읽힌다 | 프로브를 **전용 종료코드**로 재작성: `0`=전제 붕괴(throw) · `42`=`probe_sib` 부재(정상) · **그 외=프로브 파손(throw)** |
| gemini B-major | `if: false` 로 스텝을 죽여도 게이트는 텍스트만 읽어 **초록 유지**(실증) | `step_conditions()` 신설 — 필수 스위트 스텝에 step-level `if:` 가 있으면 실패 |
| gemini A-minor | `$env:LOCALAPPDATA` 재귀 검색 비효율 | **의도적 미반영** — T3·T4·T5 가 쓰는 **검증된 형태**와 일부러 일치시켰다. T6 만 바꾸면 설치 경로 발견 로직이 갈린다. 리뷰어도 "완료는 된다"고 판정 |

### 18.9.4 반영 후 검증 (전부 실행)

| 검증 | 결과 |
|---|---|
| `test_import_guard.py` | **70/70 PASS** (0.79s · 이전 52/52) |
| 셀프테스트 단독 | **30/30 PASS** — 우회 17종 차단 · 양성 대조 13종 오탐 0 |
| 실파일 뮤턴트 2종 | 전건 '가드 부재' 판정 경로로 탐지 |
| **BLOCK-3 격리 실증** | 앞 검체가 경로 2회 등재해도 뒤 무가드 검체는 정상 FAIL |
| 프로브 종료코드 3분기 | `0`/`42`/`43` 전부 기대대로 |
| 게이트 음성 시험 3종 | `if: false` · 필수 제거 · 고아 주입 → 전건 exit 1 |
| 레인 등재 7스위트 회귀 | 전건 exit 0 |
| 4개 워크플로 YAML | 전건 파싱 OK |
| Rust 보호 대상 · `._pth` | 변경 0건 |

---

## 18.10 적대 검증 2차 (2026-07-29)

바뀐 코드를 두 리뷰어에 **다시** 위임했다. 원칙: "고쳤다니 됐다" 판정 금지 —
**수정하면서 새로 생긴 구멍**이 이 라운드의 표적.

### 18.10.1 gemini(CI 배선) — BLOCK 1 · ACCEPT 3 → **반영 완료**

보고서: `docs/plans/review-2026-07-29-round2-gemini.md`

| 과제 | 판정 |
|---|---|
| T6 음성 대조 전용 종료코드 | **ACCEPT(공격 실패)** — 42/43/기타 분기가 `$LASTEXITCODE` 로 손실 없이 전달되고 fail-closed 동작 확인 |
| `step_conditions()` 파서 | **BLOCK** — 아래 |
| 기존 3레인 대조 회귀 | ACCEPT — `ALLOWED`·`stale`·`drift` 무손상 |
| §11 감사 재확인 | ACCEPT — 보호 대상·`._pth` 무손상 |

★**BLOCK 내용**: 내 `step_conditions()` 는 스텝 머리를 `- name:` 으로 가정했는데
  **GitHub Actions 에서 `name` 은 필수가 아니고 키 순서도 자유다**. 그래서
  ①`- shell: pwsh` 로 시작하는 스텝 ②`- if: false` 가 머리 첫 줄인 스텝 — 두 경우 모두
  파서가 스텝 경계를 잘못 잡아 `if: false` 를 **놓치고 exit 0** 을 냈다(리뷰어 실증).

★**반영**: 스텝 경계를 `steps:` **블록의 리스트 항목**으로 다시 잡는다(`_step_ranges()`).
  머리 줄의 `- if:` 와 본문의 `if:` 를 모두 보고, **스텝 경계를 못 잡으면 실패**시킨다
  (파서가 구조를 못 읽는 상태의 초록은 근거가 아니다 — fail-closed).

검증(전부 실행 · 게이트 스크립트를 워크플로에서 추출해 임시 복사본에 변형 주입):

| 음성 시험 | 결과 |
|---|---|
| 5a `- name:` 없는 스텝 + `if: false` | exit 1 탐지 |
| 5b `- if:` 가 머리 첫 줄 | exit 1 탐지 |
| 5c 본문 `if: ${{ false }}` | exit 1 탐지 |
| 5d `- uses:` 머리 + `if: false` | exit 1 탐지 |
| 필수 스위트 제거 | exit 1 탐지 |
| 부분 레인 전용 고아 | exit 1 탐지 |
| 완전 레인 드리프트(release 만 제거) | exit 1 탐지 |
| 스텝 밖 등장(경계 판별 실패) | exit 1 탐지(fail-closed) |
| **무변경 대조** | **exit 0** |

### 18.10.2 codex(스캐너) — BLOCK 4 · REVISE 1 → **4건 반영 · 1건 의도적 미반영**

보고서: `docs/plans/review-2026-07-29-round2-codex.md`

★핵심 진단(뼈아프고 정확하다): **1차 지적을 "이름 그대로" 담은 검체는 통과했지만, 그 규칙을
  동치 코드로 일반화하자 다시 뚫렸다.** 즉 1차 수정이 *원리*가 아니라 *사례*를 고친 것이었다.

| # | 지적(전부 실제 `ModuleNotFoundError` 실증) | 반영 |
|---|---|---|
| **BLOCK-1** | `sys.path[:] = []` · `__delitem__` · 2단 별칭(`r = q`) · **철회 함수 호출** — 전부 철회인데 비사건 | 파괴 판정을 열거 → **allowlist 반전**(추가·읽기전용 외 **전부** 철회) · 별칭 **고정점 전파** · 헬퍼 투영을 가드·철회 **양쪽 대칭**으로 |
| **BLOCK-2** | 같은 이름 함수 **재정의**(첫 정의만 보존 → 실제로는 빈 함수가 불리는데 PASS) · 함수 호출이 모듈 가드보다 **앞선** 경우 | 동명 재정의는 이름 기반 투영 금지(fail-closed) · 바깥 스코프 가드는 **모듈 최상위 첫 호출보다 앞**일 것을 요구 |
| **BLOCK-3** | `TYPE_CHECKING = True` 직접 바인딩 · `getattr(builtins,'ex'+'ec')` — 실행되는 import 가 분석 대상에서 **OMIT** | TYPE_CHECKING 은 `typing` 출처 + 재대입 없음일 때만 신뢰 · `builtins` 접촉은 금지(opaque) |
| **BLOCK-4** | ★③ 프로브가 형제 `ModuleNotFoundError` 만 FAIL 로 보고 **그 밖의 비정상 종료·timeout 을 통과** → 가드 평가 중 죽어 **import 에 도달조차 못 한 실행**이 "형제 import 통과"의 증거로 계상 | **rc=0 만 PASS**. 실패 사유는 구분해 보고하되 판정은 둘 다 FAIL |
| REVISE-1 | `sys.path.pop(0)` 은 우리가 끝에 붙인 항목을 안 지우므로 실제로는 정상인데 FAIL(과탐) | **의도적 미반영** — 아래 계약 참조 |

★**미반영 근거(계약으로 명문화)**: *가드를 건 뒤 형제 import 전까지 `sys.path` 를 건드리지 마라.*
  인덱스·원소 위치를 추적해 "이 조작은 무해하다"를 증명하는 대신 **조작 자체를 금지**한다.
  ①실물 팩에 해당 형태 **0건**(측정) ②틀리는 방향이 안전하다 — 과탐은 CI 에서 **시끄럽게** 드러나
  즉시 고쳐지지만 미검출은 Windows 에서 **조용히** 산다(이 게이트의 존재 이유가 후자다)
  ③"무해함을 증명하는 분석"은 경계가 없다 — 2차 검증이 그 경계 없음을 실측으로 보였다.

★**정적으로 잡을 수 없는 1건은 층을 나눠 잡는다**: 가드 식 자체가 예외를 던지는 형태
  (`sys.path.append((1/0, _S)[1])`)는 식의 평가 결과를 정적으로 아는 문제라 **결정 불가능**이다.
  BLOCK-4 를 고친 ③ 이 이걸 잡는다(rc≠0 → FAIL). ①②와 ③ 은 서로를 대체하지 않는 **다른 층**이다.

### 18.10.3 반영 후 검증 (전부 실행)

**codex 검체 10종 정적판정 ↔ 실제 실행 대조** (같은 실행에서 자식 프로세스로 실측):

| 검체 | 정적 | 실제 | ③프로브 | 판정 |
|---|---|---|---|---|
| slice_delete · dunder_delitem · alias_chain_revoke · called_revoke_helper | FAIL | ERR | False | 정합 |
| helper_redefined · outer_guard_after_call · type_checking_true · getattr_exec | FAIL | ERR | False | 정합 |
| probe_guard_crashes | PASS | ERR | **False** | ③ 이 잡음 |
| normal_pop_zero | FAIL | OK | True | **과탐(의도)** |

→ **아무것도 못 잡는 미검출 = 0건**.

| 검증 | 결과 |
|---|---|
| `test_import_guard.py` | **78/78 PASS** (70→78) |
| 셀프테스트 | **38/38 PASS** — 우회 검체 25종(2차분 8종 편입) + 양성 대조 13종 |
| 실파일 뮤턴트 2종 | 전건 '가드 부재' 경로로 탐지 |
| 레인 등재 7스위트 | 전건 exit 0 |
| `py_compile` · 4워크플로 YAML | 전건 OK |
| Rust 보호 대상 · `._pth` | 변경 0건 |

⚠**중단 1회(운영 교훈)**: codex(OpenAI) 안전 필터가 지시서의 "깨라/우회/공격" 표현을 공격형 보안
  요청으로 오분류해 응답이 차단됐다. 실제 작업은 **빌드 도구 정적 분석기의 회귀 테스트 품질 검토**다.
  맥락을 정정해 재요청하니 정상 진행됐다 — 향후 리뷰어 지시서는 이 표현을 피하라.

---

## 18.11 적대 검증 3차 — 워크플로 (2026-07-29)

18 에이전트 · 8 독립 차원 · 차원별 독립 반증. 보고서: `docs/plans/review-2026-07-29-round3-workflow.md`

### 18.11.1 판정: **BLOCK** — 확증 35(BLOCK 12 · REVISE 19 · NIT 4) · 기각 3

★**원리 판정**: 2차 수정 7건 중 **부류를 닫은 것은 0건**. 1건(TYPE_CHECKING/builtins opaque)만
  자기 축에서 부분적으로 버텼고, 나머지는 "지적된 표현 1개를 막고 동치 표현 N개를 남긴" 상태였다.
  진단: 2차 교훈("열거를 allowlist 로 뒤집어라")이 **한 층에만** 적용됐다. 파괴 판정은 '메서드 이름'
  축에서 뒤집혔지만 그 위층인 '무엇이 sys.path 인가'(객체 동일성)는 여전히 구문 열거이고 fail-open.

### 18.11.2 ★가장 무거운 것 — **거짓 초록**(내가 독립 재현 확인)

`③` 은 `rc=0` 을 "형제 import 통과"의 증거로 썼다. 그런데 **팩 관례가 형제 import 실패를 삼킨다**
(③ 대상 8종 중 **3종** — `javis_memory`(except Exception·의도된 LOUD fail-open) ·
`javis_orchestra`(3곳) · `javis_report`(except ImportError·구버전 호환)).

재현: `javis_memory` 의 가드를 **문법상 살아있되 틀린 폴더**로 바꾸자(오타 시나리오)

```
실제 실행 rc=0 | SKILLSCAN_LOADED= False      ← 형제가 실제로 안 실렸다
PASS ① javis_memory.py 경로 가드 존재
PASS ② javis_memory.py 모든 형제 import 가 유효 가드 뒤
PASS ③ javis_memory.py: 폴더 미등재 조건에서 형제 import 통과   ← **거짓 문장**
=== 78/78 PASS ===  스위트 exit = 0
```

미검출이 아니라 **적극적 거짓 보증**이다. 게이트가 침묵한 게 아니라 틀린 말을 했다.

### 18.11.3 반영한 것 (2건 — 어느 설계에서도 옳은 것만)

| 수정 | 내용 |
|---|---|
| **③ 을 프록시 → 실측으로** | 자식이 실행 후 `sys.modules` 를 보고하고, 정적으로 확정된 **모듈 레벨 형제가 실제 적재됐는지**로 판정. `rc=0` 은 더 이상 성공의 증거가 아니다. D5-1·T6-1·D6-2(가드가 엉뚱한 폴더를 넣는 경우)를 함께 닫는다 |
| **`sys.path` escape → 금지** | 값이 **모델링되지 않은 위치**(함수 인자·컨테이너 원소·튜플/for/컴프리헨션 타깃·walrus·언바운드 메서드 수신자·`operator` 인자)로 새면 `opaque`. `setattr(sys,...)`·`sys.__dict__` 도 포함. 열거를 늘린 게 아니라 **기본 처분을 뒤집었다** — 도크스트링이 선언한 원칙("증명 불가는 생략이 아니라 금지")을 destroy 축에 일관 적용. D1-F1(8/8 우회)·D1-F2 를 닫는다 |

★**비용 0 확인**: 실물 팩 76개 파일에서 `sys.path` 참조 30건 전부가 모델링된 위치, escape **0건**.

검증: 3차 우회 검체 10종 **정적↔실제 불일치 0** · 스위트 **86/86** · 셀프테스트 **46/46**(검체 8종 편입) ·
실파일 뮤턴트 **4종 전건 탐지**(가드 제거 2 + **가드 경로 오타 2 → ③ 이 잡음**) · 레인 게이트 exit 0 ·
등재 7스위트 전건 통과 · Rust·`._pth` 무변경.

### 18.11.4 ★반영하지 않은 것과 그 이유 — **오너 결정 필요**

남은 BLOCK 10건(정적층 fail-open: `first_call` 간접호출 · 헬퍼 바인딩 · 층 종속성 · 레인 배선)은
개별 표현을 또 막는 게 아니라 **값 격자 + 바인딩 환경 + 호출그래프 도달성 위의 추상해석**으로
올려야 닫힌다는 게 3차의 결론이다. 그건 분석기 재설계다.

그런데 **완결성 비평이 그 재설계의 전제를 흔들었다 — 내가 독립 실측으로 확인했다**:

> **근본 수리 층(`._pth`)이 한 번도 재개봉되지 않았다.**
> `python312._pth` 3줄은 **이 저장소가 직접 쓴다**(`windows-build.yml:62`·`release.yml:277`).
> 그 3줄에 `import site` 가 있고 get-pip 가 `Lib/site-packages` 를 만든다 →
> **`.pth` 경로가 살아 있다.** §7(610행)은 `.pth` 대안을 *"정적 경로는 한 레인에 고착 →
> 부서 격리 위반"* 으로 철회했는데, `.pth` 의 `import` 로 시작하는 줄은 **실행되는 코드**라
> 정적일 이유가 없다.

내 독립 실측(mac):
```
--- .pth 미적용(=오늘의 조건) ---   ModuleNotFoundError: No module named 'javis_scrub'
--- .pth 적용 ---                   bare import 성공 · 가드 0줄 · VALUE = 42
                                    sys.path 말미: <CYS_PACK_DIR 로 해소된 동적 경로>
```
즉 `.pth` 한 줄(`import sys, os; sys.path.append(os.environ.get("CYS_PACK_DIR", ...) + "/bin")`)이면
**레인별 동적 경로**가 되고 부서 격리도 깨지 않는다. 철회 근거의 전제가 틀렸다.

⚠이건 §18.5 "철회안 재제안 금지"에 걸리는 항목이지만, 그 금지의 취지는 *근거 없는 재제안 차단*이다.
여기서는 **철회 근거 자체가 사실과 다르다는 새 실측**이 나왔으므로 재개봉 사유가 된다 — 판단은 오너.

**오너가 정할 것 (셋 중 하나)**

| 안 | 내용 | 대가 |
|---|---|---|
| **A. 4차 재설계** | 분석기를 추상해석으로 올린다(값 격자·바인딩 환경·호출그래프 도달성, ⊤ 는 무조건 금지) | 큰 작업. 그리고 `.pth` 가 채택되면 **없어도 되는 층**에 투자한 것이 된다 |
| **B. 근본 수리 우선** | `.pth` 를 먼저 검토·채택하고, 가드는 심층방어로 **현 수준 동결**(오늘 상태로 커밋) | `.pth` 는 배송·설치 경로 변경이라 별도 검증 필요. 가드 게이트의 정적 사각은 남는다 |
| **C. 현 수준 동결만** | 오늘 반영분까지 커밋하고 4차는 하지 않는다 | 정적층 BLOCK 10건이 남는다(단, 실물 팩에 해당 형태 0건이고 ③ 실측층이 모듈 레벨은 덮는다) |

master 권고: **B** — 3라운드가 같은 방식으로 계속 뚫린 건 분석기 품질 문제가 아니라
**이 층에서 풀 문제가 아니라는 신호**로 읽는 게 맞다. `.pth` 가 성립하면 가드는 심층방어로 족하다.

### 18.11.6 ★오너 결정 = **B안** (2026-07-29) + 리스트업 과정에서 나온 정정 2건

오너 지시: *"B안으로 간다. `.pth` 검토하고 가드는 현 수준 동결."*
전 항목 리스트업: `docs/plans/2026-07-29-implementation-inventory.md` (**184건** · 미완 129 · 완료 25 ·
철회 19 · 결정필요 11 · 누락 감사 12건 포함).

#### 정정 ① — §18.11.4 의 `.pth` 서술은 **절반만 맞다** (실측으로 확인)

내가 적은 *"철회 근거의 전제가 틀렸다"* 는 **부정확**하다. 정확히는:

| §7 철회 근거 | 판정 |
|---|---|
| "**정적** 경로라 한 레인에 고착" | **틀렸다** — `.pth` 의 `import` 줄은 실행되는 코드다(실측 확인) |
| "**부서 격리 위반**" | **부분적으로 살아남는다** — 아래 |

★**env 기준 `.pth` 는 레인 교차오염을 일으킨다**(실측):
`CYS_PACK_DIR=lane_a` 인 채 `lane_b/bin/main.py` 를 스크립트 폴더 미등재 조건(=Windows
embeddable)에서 실행하면 —
```
.pth(env 기준)  → 적재된 형제 = lane_a   ← ★교차오염
가드(__file__)  → 적재된 형제 = lane_b   ← 올바름
```
즉 env 기준 설계면 §7 의 우려가 그대로 재현된다. 가드는 `__file__` 이라 구조적으로 레인-정확하다.

★**해소책 있음 — `sys.argv[0]` 기준 `.pth`**(실측 확인): `.pth` 는 `site` import 시점에 돌고,
그 시점에 **`sys.argv[0]` 은 이미 실행 스크립트 경로다**. 따라서
```python
import sys, os; _a = sys.argv[0] if sys.argv else ""; _d = os.path.dirname(os.path.abspath(_a)) if _a and os.path.isfile(_a) else ""; _d and _d not in sys.path and sys.path.append(_d)
```
이면 **실행 중인 스크립트 자기 폴더**를 넣는다 = 가드와 동일 의미 · 레인 오염 없음 · env 비의존.
이는 embeddable `._pth` 가 제거한 **CPython 표준 동작(스크립트 폴더 등재)의 복원**이다.
`-c`/`-m` 실행에서는 `argv[0]` 이 `-c` 라 아무것도 넣지 않는다(정상).
`append` 를 쓰므로 MAJ#1(stdlib shadowing 금지) 교리와도 정합.

→ **BR1·BR6 은 이 방향으로 통합해 판정할 것.** env 기준(`CYS_PACK_DIR`)은 채택하지 마라.

#### 정정 ② — §18.11.3 의 "D1-F2 를 닫았다"는 **부분적으로 거짓**이었다

`setattr(sys,...)`·`sys.__dict__` 는 닫혔으나 **AugAssign 축이 열려 있었다** —
`sys.path *= 0` 미검출 실증. 기록이 실제보다 앞서 있었다.
→ 3줄로 정정 반영(`+=` 는 순수 추가라 **가드**로, 그 외 AugAssign 은 **철회**로).
검증: 스위트 **88/88** · 셀프테스트 **48/48**(검체 2종 추가: `*= 0` 우회 · `+= [...]` 양성 대조).

⚠**이건 "동결 위반"이 아니다.** 동결은 *새 투자를 멈추는 것*이지 *내가 잘못 기록한 것을 그대로
두라는 뜻이 아니다*. 잔여 BLOCK(정적층 심화)은 예정대로 착수하지 않는다.

#### 잔여 BLOCK 숫자 정정

§18.11.4 는 "남은 BLOCK 10건"이라 적었으나, 3차 명단 12건 중 반영 확인은
**D1-F1 · D5-1 · T6-1 · (D1-F2 일부→이번에 완결)** 이므로 **잔여 8건**이다. 동결 대상.

### 18.11.5 기록해둘 사실 (다음 사람용)

- **F7-1(레인 배선)**: step-level `if:` 검사가 부분 레인 1개에만 걸려 있다. 완전 레인으로 확장하려면
  계약 설계가 필요하다 — `release.yml:167,198` 의 팩 스위트 스텝은 `matrix.target ==
  'aarch64-apple-darwin'` 조건부인데 이건 **의도된 1회 실행**이라, 일괄 금지하면 거짓 실패가 난다.
- **기각 3건**: D5-3(timeout 예산 산술은 맞으나 "죽은 코드" 결론이 실행으로 성립 안 함) ·
  D5-4(env·cwd 공유는 사실이나 격리 계약 위반은 아님) · T6-2(근거로 든 저장소 선례가 오독).
- **완결성 비평의 나머지 사각**(보고서 부록 A): ③ 이 생산 진입경로(`__main__`)를 안 돈다 ·
  ④ 셀프테스트가 검체를 실행하지 않아 오라클이 '저자의 믿음'이다 · 게이트에 억제 수단이 없어
  우회 유인이 모델링 안 됐다 · 훅이 인터프리터를 PATH 로 해소해 게이트가 검증한 인터프리터와
  생산 인터프리터가 다를 수 있다 · 게이트가 배송 산출물이 아니라 checkout 소스를 본다 ·
  **생산에 D1 탐지기가 여전히 0개**(훅 fail-open 그대로).

### 18.10.4 다음 할 일

1. **3차 재검증(권고)** — 2차 반영분(파괴 판정 allowlist 반전 · 헬퍼 대칭 투영 · 호출 시점 순서 ·
   ③ rc=0 계약 · 게이트 `_step_ranges()`)은 아직 적대 검증을 받지 않았다.
   ★2차가 남긴 교훈: **사례가 아니라 원리를 고쳤는지**를 물어야 한다.
2. **커밋 여부 오너 승인** — 승인 전 커밋 금지(§18.4-3 유지).
3. **T6 는 여전히 실제 Windows 러너에서 0회 실행** — mac 에 `pwsh`·`docker`·`brew` 부재로 구문검사 불가.
3. **T6 는 실제 Windows 러너에서 여전히 0회 실행** — 이 mac 에 `pwsh`·`docker`·`brew` 가 없어
   **구문검사 수단이 없다**. 실행 경로는 ①`feat/windows-x64-dist` push ②`workflow_dispatch` 수동 트리거.
   첫 실행 로그를 반드시 확인하라 — 그전까지 이 스텝은 **미검증**이다.
4. 나머지 티켓: **T-D6 → T-D3 → T-D2a → T-D5**(§6.5·§6.7).

---

> **전사 표기(발행 제네릭화 · 2026-08-24)**: 본문의 모든 실측 캡처에서 Windows 사용자명은
> `x` 로 치환돼 있다(`C:\Users\x` · pane 제목 꼬리 `· x`). 이 문서를 인용하는 코드가 재는 축은
> **경로의 형태**(`PS ` 접두 · 드라이브 콜론·역슬래시 · `>` 종결)이지 사용자명이 아니므로 증거가치는
> 불변이다. ★이 주석은 **문서 끝**에 둔다 — 앞에 넣으면 이 문서를 `파일:행` 으로 인용하는
> `docs/plans/2026-07-29-implementation-inventory.md`·`src/bin/cys.rs` 의 행번호가 전부 밀린다.
