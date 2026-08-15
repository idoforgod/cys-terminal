# Windows 실검증 보고 (WINDOWS_VALIDATION)

> ★역사 기록(v0.14.1 검증 시점) — 본 문서는 당시 스냅샷이며 현행 릴리스의 Windows 검증 절차가 아니다(현행은 `docs/plans/v4-runbook.md`·`docs/RELEASE.md`의 win T3 게이트).

> 근거: `scripts/win/win-test*-result.txt` 결과 로그 4종 + `scripts/win/run-test*.bat` 5종 +
> `scripts/win-test.ps1` + git log(커밋 `6ae8d66`). 이 문서의 모든 서술은 위 파일들에서 직접
> 추적 가능하며, 파일에 없는 내용은 "기록 없음"으로 명시한다.

## 1. 검증 환경

| 항목 | 값 | 출처 |
|---|---|---|
| 게스트 OS | Microsoft Windows [Version 10.0.26200.8457] | `win-test-result.txt` 3행 |
| 가상화 | Parallels Windows 11 ARM64 | 커밋 `6ae8d66` 메시지 |
| 소스/바이너리 전달 | `\\Mac\Home` 공유 폴더 (호스트 macOS ↔ 게스트) | `scripts/win-test.ps1` 주석 |
| 데몬 소켓 | named pipe `\\.\pipe\aiterm` | `win-test-result.txt` identify 출력 |
| 데몬 버전 | 0.1.0 | `win-test-result.txt` identify 출력 |
| 검증 일시 | 2026-06-11 09:12 ~ 10:16 (게스트 로그 타임스탬프) · 커밋 10:20 KST | 각 result 로그 1행, git log |
| 실행 방식 | `run-test*.bat`이 `aitermd.exe`/`aiterm.exe`를 `%TEMP%\aiterm-win-test`로 복사 후 실행 | 각 bat 파일 |

## 2. 검증 항목 표

최종 상태(round4 시점 + 커밋 `6ae8d66` 선언) 기준. "초기 FAIL" 항목은 §3의 DSR 버그로
인한 것으로, 수정 후 정상 동작이 확인되었다.

| # | 항목 | 결과 | 증거 |
|---|---|---|---|
| 1 | named pipe RPC: `ping` → `pong` | PASS | `win-test-result.txt` |
| 2 | `identify` → JSON 응답(daemon_pid·socket_path·version) | PASS | `win-test-result.txt` |
| 3 | `new-surface` → ConPTY surface 생성(`surface:1`) | PASS | `win-test-result.txt` 외 전 라운드 |
| 4 | `send` / `send-key Return` → RPC OK 응답 | PASS | round1~5 전부 `OK`/`OK` |
| 5 | send→**셸 stdin 주입 실제 실행** (파일 쓰기 증명) | PASS | `win-test4-result.txt`: `PROOF FILE EXISTS:` / `PROOF_VIA_STDIN` |
| 6 | `read-screen` (vt100 화면 재구성) | 초기 FAIL → **PASS** | round1~3 빈 출력 → `win-test4-result.txt`에 프롬프트·실행 명령 전문 표시 |
| 7 | `feed push` / `feed list` | PASS | `win-test-result.txt`: `req-4796-…` 발급, `[pending] permission` 항목 조회 |
| 8 | `close-surface` (자식 트리 종료) | PASS | round1~3: `closed surface:1 (descendants killed)` |
| 9 | `AITERM_SHELL=cmd.exe` 셸 교체 | PASS | `win-test3-result.txt` surface list: `cmd-shell` / `pid=14196` / `exited=false` |
| 10 | 셸 자식 프로세스 생존 확인 | PASS | `win-test4-result.txt` tasklist: `cmd.exe`·`aitermd.exe` 목록 |
| 11 | `aiterm list` (surface 목록) | PASS | `win-test3-result.txt`: `surface:1 pid=14196 exited=false cmd-shell C:\Users\cys` |

추가로 커밋 `6ae8d66`는 같은 변경에서 **Feed 영속화**(`feed.jsonl` append + 기동 시 복원,
Windows는 `%LOCALAPPDATA%\aiterm` 하위 — `state.rs::state_dir`)와 `event.seq` 단조 유지를
포함한다.

## 3. ConPTY DSR 핸드셰이크 버그 — 발견·진단·수정

### 3-1. 발견 (round1, 09:12 — `win-test-result.txt`)

RPC(ping·identify)·surface 생성·send/send-key·feed·close까지 전부 정상인데,
**`read-screen`만 빈 출력**이었다. 즉 데몬↔CLI 통신과 surface 수명주기는 살아 있는데
PTY 출력이 화면 파서에 도달하지 않는 증상.

### 3-2. 가설 배제 (round2·round3, 09:14 / 10:13)

- **round2** (`win-test2-result.txt`): PowerShell cold start를 의심해 15초 대기 후
  read-screen, echo 후 재확인, `--lines 30` scrollback까지 조회 — **전부 빈 출력**.
  → 타이밍 문제 아님.
- **round3** (`win-test3-result.txt`): `AITERM_SHELL=cmd.exe`로 셸을 교체 — 역시 빈 화면.
  단, `aiterm list`는 `pid=14196 exited=false`를 보여 **셸 프로세스는 살아 있음**을 확인.
  → 셸 종류 문제 아님, 프로세스 사망 아님. 출력 경로만 막힘.

### 3-3. 진단 (round4, 10:16 — `win-test4-result.txt`)

`run-test4.bat`은 `AITERM_DEBUG=1` + 데몬을 보이는 콘솔로 띄워(reader 스레드의 read
바이트 수·DSR 응답 디버그 라인 관찰용) 입력·출력 경로를 분리 검증했다:

- **입력 경로 정상**: `send`로 `echo PROOF_VIA_STDIN > …` 주입 → 게스트 파일시스템에
  `aiterm-proof.txt` 실제 생성 (`PROOF FILE EXISTS`). stdin 주입은 처음부터 동작했다.
- **출력 경로 회복**: 같은 라운드의 `read-screen`이 처음으로 화면 전문(프롬프트·입력한
  명령 echo)을 재구성해 출력했다. round4가 사용한 데몬 바이너리는 `AITERM_DEBUG` 코드를
  포함하므로(해당 코드는 커밋 `6ae8d66` diff에서 처음 추가) 아래 3-4의 수정이 들어간
  빌드다.

### 3-4. 원인·수정 (커밋 `6ae8d66`, `src/bin/aitermd/state.rs`)

커밋 기록: **ConPTY(INHERIT_CURSOR)는 기동 시 DSR(`\x1b[6n`, 커서 위치 질의)을 보내고
응답이 올 때까지 입출력 펌프를 정지한다.** 실제 터미널 에뮬레이터는 이 질의에 응답하지만
aiterm 데몬의 reader는 출력을 파서에 넣기만 했으므로, ConPTY가 영원히 대기 → 화면 출력이
한 바이트도 나오지 않는 증상(§3-1~3-2)이 됐다.

수정(diff 원문 기준): reader 스레드가 PTY 출력 청크에서 `\x1b[6n`을 감지하면 vt100
파서의 현재 커서 위치를 읽어 `\x1b[{row};{col}R`을 PTY writer로 즉시 회신한다.

```rust
// DSR cursor-position query: a real terminal must answer, or
// ConPTY(Windows)가 응답을 기다리며 입출력 펌프를 멈춘다.
if chunk.windows(4).any(|w| w == b"\x1b[6n") {
    let (row, col) = { /* vt100 파서의 cursor_position() + 1 */ };
    let resp = format!("\x1b[{row};{col}R");
    /* PTY writer에 write + flush */
}
```

### 3-5. 재검증

- `run-test5.bat`("round5: DSR fix")가 수정 검증 전용으로 작성됨: 6초 후 프롬프트 표시
  기대, echo + 파일 쓰기 증명, close까지.
- **증거 공백**: `win-test5-result.txt`는 작업트리·git 히스토리 모두에 없다(로그 미보존).
  수정 후 정상 동작의 1차 증거는 round4 로그(§3-3)와 커밋 `6ae8d66`의 "전부 PASS" 선언이다.

## 4. 크로스빌드 레시피 (macOS 호스트 → Windows ARM64)

배경: 호스트의 homebrew rust에는 Windows target std가 없다(README "알려진 한계" 기록).
커밋 `6ae8d66` 기록 — "크로스 빌드: zigbuild aarch64-pc-windows-gnullvm (rustup 격리
`~/.aiterm-*`)". 호스트 머신에서 실증 확인한 구성:

1. **격리 toolchain**: `RUSTUP_HOME=~/.aiterm-rustup`, `CARGO_HOME=~/.aiterm-cargo`로
   rustup을 시스템 rust(homebrew)와 격리 설치. 호스트에 두 디렉터리 존재 확인,
   toolchain에 `aarch64-pc-windows-gnullvm` rust-std 설치 확인.
2. **링커**: `zig` + `cargo-zigbuild` (둘 다 `/opt/homebrew/bin`에 존재 확인) —
   zig가 MinGW/LLVM 링킹을 대신해 macOS에서 windows-gnullvm 바이너리를 만든다.
3. **빌드**: `cargo zigbuild --target aarch64-pc-windows-gnullvm` 형태.
   (정확한 플래그·프로필은 repo에 기록 없음 — 산출물 `aiterm.exe`·`aitermd.exe`만
   `scripts/win/`에 존재하며 `.gitignore`에 `scripts/win/*.exe`로 제외됨.)
4. **게스트 전달·실행**: `\\Mac\Home` 공유 폴더(= `scripts/win/`)에서 각 `run-test*.bat`이
   exe를 `%TEMP%\aiterm-win-test`로 복사해 실행. 정리는 `cleanup.bat`
   (aitermd 종료·테스트 디렉터리·증명 파일 삭제).

참고: `scripts/win-test.ps1`은 게스트 내 **네이티브 빌드** 경로(robocopy 소스 동기화 →
`cargo build` → E2E 6단계)로 작성된 별도 스크립트다.

## 5. 증거 파일 목록·한계

| 파일 | 내용 |
|---|---|
| `scripts/win/win-test-result.txt` | round1: RPC·surface·feed·close PASS, read-screen 빈 출력(버그 발견) |
| `scripts/win/win-test2-result.txt` | round2: cold start 15s 대기에도 빈 화면(타이밍 배제) |
| `scripts/win/win-test3-result.txt` | round3: cmd.exe 셸에서도 빈 화면, 프로세스 생존 확인(셸 배제) |
| `scripts/win/win-test4-result.txt` | round4: stdin 주입 파일 증명 + read-screen 화면 재구성 성공 |
| `scripts/win/run-test.bat`~`run-test5.bat` | 각 라운드 실행 스크립트(게스트 E2E 배치 5종) |
| `scripts/win/cleanup.bat` | 게스트 잔여물 정리 |
| `scripts/win-test.ps1` | 게스트 네이티브 빌드 E2E(별도 경로) |

한계(기록 없는 것):
- `win-test5-result.txt` 부재 — round5(DSR fix 전용 검증) 결과 로그 미보존(§3-5).
- 크로스빌드의 정확한 명령 플래그·프로필 미기록(§4-3).
- UI(Tauri)·MSI 패키징은 이번 검증 범위 밖(결과 로그에 해당 항목 없음).
