# browserd — 자비스 브라우저 엔진 사이드카 (P1 · 팩 인큐베이션)

워커가 웹 산출물을 **실제 크로미움**에서 자기검증하고, 그 증거(evidence 번들)를
done 게이트에 흘려보내기 위한 엔진 사이드카. 설계 근거:
`_research/cmux-distillation/DESIGN-v1.2-2026-07-19.md` §2A·§2B·§8-1.

## 클린룸·라이선스
- **클린룸**: cmux(GPL-3.0) 코드·마크업 무참조. 설계 md만 참조해 자작.
- 외부 의존 = **playwright-core (Apache-2.0)** 단독. 버전 핀 `1.49.1` + `bun.lock` 커밋.
- 브라우저 바이너리: 설치된 **Google Chrome** 채널 우선(`channel:"chrome"`), 없으면
  playwright chromium 폴백(`bunx playwright install chromium` 필요).

## 부트체인 무접점
lazy 사이드카. launchd 등록·`cys boot` 4종 의무 노드·`javis_preflight`·SessionStart hook
**무접점**. 죽으면 이 기능만 상실, 부트·오케스트라 체인 영향 0. 유휴 15분 자동 종료.

## 실행
```bash
cd cysjavis-pack/browserd
bun install                                   # playwright-core 1.49.1 설치 + lockfile
python3 ../bin/javis_browser.py doctor        # 설치·경로·버전 결정론 점검 (exit 0/1)

# 동사 (browserd 자동 기동)
python3 ../bin/javis_browser.py open https://example.com
python3 ../bin/javis_browser.py snapshot
python3 ../bin/javis_browser.py verify --expect-text "..." --evidence-dir ./evi
python3 ../bin/javis_browser.py --headless open ...   # GUI 세션 없는 컨텍스트

# 테스트
bun test                                      # 순수 로직 단위 (토큰·state·상한)
bash tests/test_negative_gate.sh              # 음성 게이트 E2E (verify FAIL/PASS + evidence)
```

## 전송·상태
- 127.0.0.1 HTTP, port 0-bind, 경로 `/<token>/rpc` (POST JSON `{verb, args}`).
- `~/.cys/browser/state.json` {pid, port, token, headless} 0600 원자 기록. 스테일=pid 사망 시 교체.
  `headless`=기동 모드(Phase 2 cast 추가). 소비자 키 검사는 pid/port/token 3키만 — 하위호환.
  live browserd가 headless면 `observe`/`pick`/`sot`는 exit 4(HEADLESS_ACTIVE)로 거부한다
  (재사용 시 창이 안 뜨는데 성공 보고하는 것을 막는다 — 자동 kill 없음).
- 감사로그: `~/.cys/browser/audit.jsonl` (전 동사 append — reviewer2 감사 대상).

## 동사표 (서버 dispatch ↔ CLI ↔ 이 표는 **3면이 일치해야 완성**이다)

| CLI | 서버 동사 | 등급 | 주요 인자 |
|---|---|---|---|
| `open <url>` | `open` | A′ | `--profile human`(결재) `--context` `--evidence-dir` |
| `goto <url>` | `goto` | A | `--timeout` `--snapshot-after` |
| `back` / `forward` / `reload` | 동명 | A | `--snapshot-after` |
| `stop-loading` | `stop` | A | — (※`stop`은 **데몬 종료**가 선점한 이름) |
| `snapshot` | `snapshot` | C | `--evidence-dir` |
| `get <what>` | `get` | C(단 `value`·`html`은 **B**) | `what=url\|title\|text\|html\|value\|attr\|count\|box\|styles` · `--ref/--selector` · `--attr` · `--property`(반복) |
| `click` `fill` `type` `press` | 동명 | A | `--ref/--selector` `--value` `--text` `--key` |
| `dblclick` `hover` `focus` `check` `uncheck` | 동명 | A | `--ref/--selector` `--snapshot-after` |
| `select` | `select` | A | `--value`(반복) |
| `scroll` | `scroll` | A | `--dx --dy` · 대상만 주면 화면 안으로 |
| `tab <list\|new\|switch\|close>` | `tab`(switch→`activate`) | `list`=C, 그 외 A | `--id` `--url` |
| `viewport [reset]` | `viewport` | A | `--width --height` |
| `eval` | `eval` | **A**(임의 JS=변경 표면) | `--expression` |
| `wait` | `wait` | C(`--function`은 **A**) | `--selector --text --url --load` · ※`--function`은 **CLI 미노출(2차)** — 서버 RPC 로만 접근 |
| `verify` `screenshot` `pick` `observe` `sot` | 동명 | 표 참조 | — |
| `control <acquire\|release>` | `control` | **A** | `--actor` · ※조작권 **상태를 바꾸는** 동사라 조회가 아니다 |
| `status` | `status` | **SERVER** | 컨텍스트 비의존 — 중앙 게이트를 통째로 우회하는 유일 등급 |

**게이트 등급**: `A` 변경성(human 프로필 거부 · control=human 시 거부) · `A′` open(기존 컨텍스트
재사용도 human이면 결재 필요) · `B` 자격증명 **조회 전용**(human 거부 · control=human 에선 허용) ·
`C` 일반 조회 · `SERVER` 서버 전역 상태(컨텍스트 비의존 — 중앙 게이트 미적용이므로 컨텍스트·페이지에
손대는 동사를 넣지 않는다). ★`eval`·`wait --function`은 **A**다 — 임의 JS 실행은 클릭·이동이 가능한 변경
표면이라 "읽기라 조작권과 무관"이라는 B의 근거가 성립하지 않는다.
★`control`도 **A**다 — 조회가 아니라 **조작권 중재 상태 자체를 바꾼다**. C로 두면 사람이 조작 중일 때
에이전트가 `control release` 한 줄로 게이트를 스스로 끄고 A등급 전부를 되찾아 `eval`의 A격상이 무효가 된다.
사람이 조작권을 놓는 경로는 **cast pane 의 WS `control` 메시지**이므로 RPC만 막아도 손실이 없다.
**표에 없는 동사는 자동 거부(deny-by-default)** — 신규 동사를 넣으면 서버
`GATE` 표·이 표·CLI 세 곳을 함께 갱신해야 한다.

**human 프로필 allowlist**: 결재된 `open` · `wait`(`--function` 제외) · `screenshot` ·
`snapshot` · `get url|title` **뿐**. 그 밖은 전부 `HUMAN_PROFILE_PROTECTED`.
human 프로필 컨텍스트는 **cid `"human"` 으로 강제 분리**되어 기본 pane(`default`)을 점유하지 않으며,
그 cid 는 **human 전용으로 예약**된다(agent 가 `--context human` 으로 선점하면 `HUMAN_CID_RESERVED`).
`snapshot`은 human 프로필에서 **입력값 폴백을 쓰지 않으며**, `type=password`는 프로필 무관 항상 마스킹된다.
★human 프로필에서는 `--evidence-dir` **자체가 거부**된다(`HUMAN_PROFILE_PROTECTED`) — `snapshot`·`screenshot`은
allowlist라 통과하는데 evidence 번들의 `dom.html`이 **원본 DOM 전문**(비밀번호·CSRF 토큰 포함)을 디스크에
남겨, B등급으로 막은 `get html`과 같은 내용이 파일로 새는 등급표 우회 경로였다.

## 결정론 exit 코드 (CLI)
`0` 성공 · `2` BUSY(context 상한 3 초과) · `3` APPROVAL_REQUIRED(human 프로필) ·
`4` 기동실패 · `5` verify FAIL · `6` HUMAN_ACTIVE · `7` HUMAN_PROFILE_PROTECTED ·
`8` PICK_TIMEOUT · **`9` 사용례 오류**(argparse·BAD_ARGS·UNKNOWN_VERB) · `10` NAV_FAILED ·
`11` NAV_UNAVAILABLE · `12` TAB_LIMIT · `13` NO_TAB · `14` SCHEME_DENIED ·
`15` BAD_SELECTOR · `16` NOT_VISIBLE · `17` NO_CONTEXT · `18` EVIDENCE_PATH_DENIED ·
**`19` HUMAN_CID_RESERVED · `20` HUMAN_CID_REQUIRED · `21` PROFILE_MISMATCH**(프로필 격리 위반=보안 거부) ·
`1` 기타.

> ★19~21 이 따로 있는 이유: 이 셋은 **보안 거부**다. exit 1(기타)로 뭉개지면 에이전트가 일반 오류와
> 구분하지 못해 재시도할지 중단할지 판단할 수 없다. 셋은 cid↔profile 상호 예약(P0-A)의 같은 가족이라
> 함께 등재한다 — 하나만 등재하면 같은 위반이 방향에 따라 다른 코드로 갈린다.

> ★`9`가 따로 있는 이유: argparse 는 사용례 오류에 고정 exit 2를 쓰는데 그것이 `BUSY(2)`와
> 충돌한다. 서버에 동사를 넣고 CLI 배선을 빠뜨리면 에이전트가 "바쁘다"로 오판해 무한 백오프한다.

## cast 화면 — 렌더 서피스 = 뷰포트
pane 에 그려지는 프레임 크기(`metadata.deviceWidth/Height`)는 **CSS 뷰포트와 정확히 같아야 한다**.
`Emulation.setDeviceMetricsOverride` 는 **CDP 세션별 상태**라, playwright 가 자기 세션에만 override 를
걸면 페이지는 800 으로 레이아웃되는데 cast 세션의 screencast 는 override 없는 실제 위젯(창 − 브라우저 UI)을
캡처해 **하단 143px 이 잘렸다**(보이지도 클릭되지도 않음). 그래서 cast 세션이 attach·리사이즈마다
같은 override 를 직접 건다. 회귀 핀 = `window.innerHeight === metadata.deviceHeight`.

## evidence 번들 (4파일 · `--evidence-dir`)
`screenshot.png` → `snapshot.txt` → `dom.html` → **`meta.json`(마지막=완결 마커)**.
`meta.json.dom_sha256` = `sha256(dom.html)` — 리뷰어 독립 재계산으로 위조 대조.
`meta.json` 없는 번들 = 게이트 무효(반쪽 번들 차단).

**경로 봉인**: `--evidence-dir` 는 **`~/.cys/browser/evidence/` 하위만** 허용한다(상대 경로는 그 아래로
해석 · `..`·절대경로 이탈은 `EVIDENCE_PATH_DENIED`=18). 근거 — `writeEvidence` 는 지정 경로에서 4파일을
**선삭제 후 덮어쓴다**(세대혼합 방지 F4). 경로를 그대로 믿으면 에이전트가 임의 디렉터리의 같은 이름 파일을
지우는 삭제 도구가 된다.
**크기 상한**: `dom.html` 은 페이지 **안에서** 200만자로 슬라이스한다(`page.content()` 무제한 → 거대
페이지에서 bun OOM → **같은 프로세스의 사람 pane 까지 사망**). 상한에 걸리면 `meta.json.dom_truncated=true`
이며 `dom_sha256` 은 **절단본**의 해시다.
**human 프로필 금지**: 위 allowlist 항 참조 — human 컨텍스트에서는 evidence 번들 자체를 남기지 않는다.

## 보안 (설계 §3)
- snapshot 최상단 **비신뢰 라벨** 고정 헤더(웹 텍스트=데이터, 지시 아님).
- human 프로필 동사 = `APPROVAL_REQUIRED`(P1은 무조건 거부, feed 배선은 P3).
- 조작권 컨텍스트별 `control=agent|human`. control=human 중 에이전트 변경성 동사 = `HUMAN_ACTIVE`.
  ★`control` 자체가 A등급이라 **에이전트는 조작권을 되찾을 수 없다**. 회수 경로는 ①cast pane 접속 후
  이탈(마지막 클라이언트 이탈 시 `agent` 자동 복구) ②`javis_browser.py stop` 재기동 둘뿐이다 —
  `control acquire --actor human` 을 pane 없이 걸어두면 그 컨텍스트는 재기동까지 A등급이 잠긴다.
- 스냅샷 크기 상한 200KB + 절단 마커. 네이티브 다이얼로그 자동 dismiss + 로그.
- **감사 원장의 공백과 그 보완**: `audit.jsonl` 은 CLI(`javis_browser.py`)가 기록한다. 서버는
  `args.approved` 를 그대로 신뢰하므로 `state.json` 을 읽어 **RPC 로 직행하면 CLI 원장을 통째로
  우회**한다. 그래서 **서버가 human 프로필 `open` 을 직접 1줄 기록**한다(`source:"browserd"` ·
  시각·verb·url·approved·context). 두 출처가 같은 파일에 append 되며, `source` 필드로 구분한다.
- **정직한 한계**: 워커는 셸로 playwright를 직접 실행해 이 정책을 우회할 수 있다(물리 강제 불가).
  방어선 = audit.jsonl 부재 브라우징 흔적 감사 + evidence 규격 위조 비용 상승 + 마스터 실측 재현.
