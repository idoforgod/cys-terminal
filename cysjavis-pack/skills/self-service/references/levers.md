# self-service 레버 지도 — 고쳐도 되는 곳, 손대지 않는 곳

레버 종류 T1–T7 은 레버 표(`<팩>/round/self_service_levers.json` 의 `classes`)와 같은 이름이다. 사례 도구 triage 가
내는 레버마다 이 중 하나가 붙는다. 위험 레버(ask_once)는 사용자에게 한 번 묻고
`python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" consent --case <id> --lever <L> --yes` 로 답을 남긴 뒤에 한다.

## 업데이트에 살아남는 곳 (먼저 여기를 쓴다)

- 사용자 소유(user class) 파일: `*_DIRECTIVE.md`, `soul.md`, `CLAUDE.md`, `agents.json`, `acl.json`, `schedule.json` — 업데이트가 덮지 않는다. 제조사 새 판은 옆에 `<파일>.new` 로 온다.
- `~/.cys/local/`: `skills/<이름>/`(`cys skill new` 의 기본 위치 · 같은 이름의 제조사 스킬을 가린다), `hooks/<이벤트>.d/*.sh`(cys-hook.sh 가 돌린다), `notes/`, `directives/<역할>_DIRECTIVE.local.md`(그 역할 지침 뒤에 붙는다). 업데이트는 이 폴더를 읽지도 쓰지도 않는다.
  - 로컬 지침 덧붙임의 읽는 쪽은 일부 낱말이 든 줄을 조용히 버리고 길이를 자른다 — 덧붙인 줄이 실제로 세션에 들어왔는지 확인한다.
- 사용자의 작업 폴더.

## 손대지 않는 곳

- 제품 실행 파일(`cys`, `cysd`, 앱)과 내장 팩 — 다시 빌드·패치·교체하지 않는다(업데이트가 덮어쓰고, Windows 스마트 앱 컨트롤은 서명 없는 재빌드를 막는다).
- `trusted-keys.json` — 업데이트가 늘 제조사 판으로 되돌린다.

## 한 번 묻고 하는 세 경우

(A) 돈(유료 요금제·유료 API·예산 상한 올리기) (B) 사용자 자료의 되돌릴 수 없는 손실(사용자 파일·휴지통 지우기, 큐 항목 버리기, 사람이 친 입력줄 글 지우기, 공장 초기화) (C) 공개 쓰기(git push, 메일·메시지 보내기, 발행, 제조사 보고 보내기). 그 밖에는 묻지 않고 한다.

## T1 설정 (setting)

- 환경 변수 노브, `agents.json`(프롬프트 표식·주입 지연 등), `acl.json`, 예약(`cys schedule add` / `cys schedule remove`), 역할 성격(`cys persona set`).
- 사용자 소유라 제자리에서 고친다. 단 `acl.json` 을 더 넓은 발신자에게 여는 것은 사용자의 보안 결정이라 한 번 묻는다.

## T2 팩 파일 상태 (pack file state)

- 진단·자동 수리: `javis_preflight.py --fix`, `cys doctor --fix`.
- 팩 동기화: `cys pack-plan`, `cys pack-merge`, `cys pack-adopt`, `cys pack-heal`, `cys pack-rollback` — 각자 확인 프롬프트를 띄운다.
- 제조사(system) 파일을 제자리에서 고치는 것은 응급 조치뿐이다. 다음 제조사 판이 병합하거나 `.user` 로 밀어낸다 — 기록하고, `cys pack-merge --file <팩 상대 경로> --propose` 로 제안 patch 를 만든다(자동 전송 없음).

## T3 좌석·큐 상태 (seat or queue state)

- 읽기: `cys status --json`, `cys read-screen --surface <n>`, `cys queue list --surface <n>`.
- 기계만의 상태는 묻지 않고 고친다: 컨텍스트 순환(`cys cycle-agent`), 세션 고정(`cys usage-register`), 원치 않는 복원 좌석(`cys tombstone`), 지침을 잃은 좌석(`cys reinject`, `cys node-recover`), 큐 되살리기(`cys queue revive` — master 또는 그 항목을 보낸 좌석 본인만).
- 큐 강제 배달(`cys queue deliver`)은 데몬이 허락하는 막힘에서만 듣는다 — 데몬이 거부하면 그 거부 문면의 다음 걸음을 따른다.
- 한 번 묻는다: 큐 항목 버리기, 사람이 친 입력줄 글 지우기. 다른 좌석 입력줄에 Ctrl-U 를 보내지 않는다 — 사용자에게 그 키를 부탁한다.
- 떠 있는 창(모달)의 키: 창을 읽고(`cys read-screen --surface <n> --composer`), 한 번 묻고, 읽은 output_gen 에 묶어 보낸다(`cys send-key --surface <n> --expect-output-gen <gen> <키>`).

## T4 사용자 스크립트·작업·훅 (user's own script, job or hook)

- `~/.cys/local/hooks/<이벤트>.d/*.sh`, `cys schedule add`, `~/.cys/local/` 이나 작업 폴더의 스크립트 — 사용자 것이므로 고치고, 고친 뒤 그 일을 한 번 돌려 확인한다.

## T5 없는 기능 → 사용자 공간에 만든다 (missing feature)

- 스킬(`cys skill new` → `~/.cys/local/skills/<이름>/`), 로컬 훅, 예약 작업, 작업 폴더 도구.
- 제품에 걸 자리가 없으면 가장 가까운 사용자 공간 판을 먼저 만들고, 없는 자리를 사용자에게 말한다.

## T6 제품 실행 파일의 결함 → 먼저 막는다 (defect in a product binary)

- 실행 파일은 고치지 않는다. 사용자 공간에서 증상을 막는다(설정·로컬 훅·우회 절차).
- 막고 나서 verify 가 `mitigated_with_residual` 이면 남은 원인은 제품 안에 있다 — 그때만 제조사 보고 초안을 묻는다(SKILL.md 순서 5·7).

## T7 서명·설치·OS 정책 → 작동하는 판을 지킨다 (signing, installer or OS policy)

- 서명·설치 프로그램·OS 정책(예: 스마트 앱 컨트롤)이 막으면 지금 작동하는 판을 지킨다. 서명 없는 재빌드로 우회하지 않는다.
- 무엇을 설치할지는 사용자가 정한다.
