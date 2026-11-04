---
name: self-service
description: 사용자의 자비스(cys)에 생긴 버그·오류·오작동·느려짐·없는 기능을 자비스가 사용자 공간에서 직접 고치고 만드는 절차(F58 셀프 서비스). 사용자가 "이거 고쳐 줘", "안 된다", "멈췄다", "에러", "이런 기능이 있으면" 이라고 하거나, 자비스가 스스로 문제를 알아챘을 때, 그리고 "제조사에 보고하라"는 글을 쓰기 전에 발동한다. 첫 대답 형식, 사례 도구 javis_selfservice.py 의 순서, 고쳐도 되는 곳과 안 되는 곳의 경계, 한 번 묻고 하는 세 경우, 제조사 보고 초안의 조건을 담는다.
---

# self-service — 자비스가 자기 문제를 스스로 고친다

> 자비스 스스로 모든 문제를 해결하고, 버그를 고치고, 새로운 기능을 만들게 해 주는 능력을 허락해 주는 기능이다.
> 고칠 수 있는 곳과 손대지 않는 곳의 지도는 `references/levers.md` 에 있다(레버 종류 T1–T7 · 경계 · 업데이트에 살아남는 곳).
> 사례 도구: `python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" <동사> …` — 사례 기록은 `~/.cys/local/self-service/<사례 id>/` 에 남고, 도구는 아무것도 보내지 않는다.

## 언제

- 사용자의 자비스나 cys 설정에 생긴 버그·오류·오작동·느려짐·없는 기능 — 사용자가 고치라고 했든, 자비스가 스스로 알아챘든.
- 사용자에게 "제조사(개발사)에 보고하라"고 말하거나 그런 글을 쓰기 **전에** — 보고는 이 절차의 마지막 단계다.

## 순서

1. **먼저 본다(읽기만).** 상태·화면·큐를 읽고(`cys status --json`, `cys read-screen --surface <n>`, `cys queue list --surface <n>`), 사례를 연다:
   `python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" open --symptom "<증상 한 줄>"` → 사례 id 를 찍는다.
   이어서 `python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" triage --case <id>` — 레버 표에서 맞는 항목을 고르고 판정과 레버 순서를 낸다.
   - 종료 8(NO_MATCH)이면 항목 목록이 찍힌다. 맞는 항목이 있으면 `triage --case <id> --entry <항목 id>` 로 다시 부른다.
   - 맞는 항목이 없으면 `references/levers.md` 의 경계 안에서 고치고, 한 일과 확인한 결과를 사용자에게 그대로 말한다(이 사례로는 제조사 보고 초안을 만들 수 없다 — 시도 기록이 없다).
2. **첫 대답.** 첫 조치(읽기가 아닌 첫 행동)보다 먼저, 판정에 맞는 한 문장으로 시작하고 같은 대답에서 할 일을 말한다:
   - 판정 `self_fixable`, `self_mitigable_product_defect`: 「우리가 직접 고칠 수 있습니다. 먼저 <할 일> 하겠습니다.」
   - 판정 `product_defect_no_lever`, `owner_needed`(사용자 공간에 고칠 길이 없을 때): 「우리가 직접 할 수 있는 것부터 하겠습니다: <할 일>.」
   - 첫 대답에는 제조사·보고·"할 수 없다"·"할 일이 없다"를 쓰지 않는다. 고치지 않은 것을 고쳤다고 하지 않는다.
3. **사용자 공간에서 고친다**, triage 가 낸 레버 순서대로. 위험 레버(ask_once)는 사용자에게 한 번 묻고, 답을 기록한 뒤에만 한다:
   `python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" consent --case <id> --lever <L> --yes` (거절이면 `--no` — 그 레버는 건너뛴다).
   동의 기록 없이 위험 레버를 기록하려 하면 도구가 종료 6(NEEDS_USER_YES)으로 거부한다.
4. **한 레버를 할 때마다 기록한다:**
   `python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" attempt --case <id> --lever <L> --action "<한 일 한 줄>" --evidence <증거 파일>`
   — 증상 probe 와 그 레버의 readback probe 는 도구가 그 자리에서 돌린다(값은 pass | fail | unknown, unknown 은 성공이 아니다).
5. **확인한다:** `python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" verify --case <id>`
   - `fixed` → 해결됐다고 말하고 더 묻지 않는다.
   - `mitigated_with_residual`(증상은 막았고 제품 결함 probe 는 통과하지 않음) → 이 한 문장으로 한 번만 묻는다: 「지금 쓰시는 데는 문제없게 막았습니다. 남는 원인은 제품 안에 있어 보입니다 — 고친 기록을 붙여 제조사 보고 초안을 만들어 둘까요?」
   - 종료 1(고쳐지지 않음) → 다음 레버로 간다. 무엇이 남았는지는 `python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" status --case <id>` 로 본다.
6. **레버가 다 끝났으면**(모든 레버를 시도했거나 거절됨 = `exhausted`): `python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" retriage --case <id>` → `product_defect` 또는 `unresolved`.
   그때 묻는 한 문장: 「이 기기에서 할 수 있는 조치로는 증상이 남아 있습니다 — 시도한 기록을 붙여 제조사 보고 초안을 만들어 둘까요? (보내지는 않습니다)」
   종료 1 과 「symptom not observed now: run verify」가 나오면 증상이 지금은 없다는 뜻이다 — 5 번 verify 로 돌아간다.
7. **제조사 보고 초안은 마지막 단계다** — 사용자가 한 번 그렇다고 답했을 때만:
   `python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" consent --case <id> --report --yes` 다음 `python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" report --case <id>`.
   도구는 `vendor-report.txt` 와 `report-evidence/` 를 쓰기만 한다 — 자비스는 보내지 않는다. 사용자가 앱의 「피드백 보내기」 창에서 붙여 보낸다.
   report 가 0 이 아닌 종료(4·5·6·7)를 내면 찍힌 다음 걸음을 따른다. 사용자가 모든 조치를 거절한 사례에는 보고 초안이 없다(고친 증거가 없다) — 사용자는 「피드백 보내기」 창으로 언제든 직접 보낼 수 있다.
8. **새 기능**은 사용자 공간에 만든다: 스킬(`cys skill new`), 로컬 훅(`~/.cys/local/hooks/<이벤트>.d/*.sh`), 예약 작업(`cys schedule add`), 작업 폴더의 도구. 제품에 걸 자리(hook point)가 없으면 가장 가까운 사용자 공간 판을 먼저 만들고, 없는 걸 자리를 사용자에게 말한다.
9. **팩 업데이트 뒤에는** 끝난 사례를 다시 잰다: `python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" recheck` — 종료 1 이면 되돌아간 사례가 있다(`triage` 부터 다시).
10. **제조사 기본값 정정.** 세션 시작 블록이 이 역할의 정정 건수를 알려 준다. 목록은
    `python3 "$CYS_PACK_DIR/bin/javis_selfservice.py" corrections --role <역할>` 로 읽는다 — 「계속: --from <id>」가 나오면 그 인자로 이어 부르고, 끝줄 「끝: n/eligible 건 …」이 나오고 n 과 eligible 이 같아야 다 읽은 것이다.
    정정은 제조사 기본값일 뿐이다 — 사용자가 고친 파일과 사용자 지침이 우선한다(도구가 그런 파일은 목록에서 뺀다).

## 주의

- 사용자에게 한 번 묻고 한다: 돈 · 사용자 자료의 되돌릴 수 없는 손실 · 공개 쓰기(git push·발송·발행·제조사 보고). 큐 항목을 버리는 것과 사람이 친 입력줄 글을 지우는 것도 되돌릴 수 없는 손실이다. 그 밖의 조치는 묻지 않고 한다.
- 제품 실행 파일(`cys`, `cysd`, 앱)과 내장 팩은 다시 빌드하거나 고치거나 바꾸지 않는다 — 업데이트가 덮어쓰고, Windows 스마트 앱 컨트롤은 서명 없는 재빌드를 막는다.
- `trusted-keys.json` 은 건드리지 않는다.
- 다른 좌석 입력줄에 Ctrl-U 를 보내지 않는다 — 필요하면 사용자에게 그 키를 눌러 달라고 한다. 떠 있는 창(모달)에 키를 넣을 때는 창을 읽고(`cys read-screen --surface <n> --composer`), 사용자에게 한 번 묻고, 읽은 output_gen 에 묶어 보낸다(`cys send-key --surface <n> --expect-output-gen <gen> <키>`) — 화면이 그 사이 바뀌었으면 데몬이 아무것도 치지 않는다.
- 사용자 소유 파일(`*_DIRECTIVE.md`, `soul.md`, `CLAUDE.md`)은 사용자가 시킬 때만 고친다 — 고치기 전에 사본을 남긴다.
- 제조사 팩 파일(`bin/`, `hooks/`, 제조사 스킬)은 지금 작동을 되살리는 응급 조치로만 고치고, 그 사실을 기록하고, `cys pack-merge --file <팩 상대 경로> --propose` 로 제안 patch 를 만든다(자동 전송 없음). 먼저 업데이트에 살아남는 곳(설정·로컬 훅·로컬 스킬)을 찾는다.
- 파일은 `encoding="utf-8"` 로 쓴다.
- 사례 기록·증거 파일·보고 초안에 환경 변수 덤프·토큰·승인 서명을 넣지 않는다.

## 확인

- "고쳤습니다"·"해결"은 `verify` 가 `fixed` 를 찍은 뒤에만 말한다. `mitigated_with_residual` 은 "막았다"이지 "고쳤다"가 아니다.
- 증상 probe 가 없는 항목(`none`)은 값이 늘 unknown 이라 도구가 `fixed` 를 기록하지 않는다 — 0.14.57 의 기본 레버 표 항목은 모두 그렇다. 그때는 한 일과 남은 확인을 말하고, 증상이 사라졌는지 사용자에게 묻는다.
- 팩 업데이트 뒤에는 `recheck` 를 돌린다 — 제조사 파일에 한 응급 조치는 병합되거나 `.user` 로 밀려났을 수 있다.
- 제조사 보고 초안은 시도 기록이 있는 사례에서만 나온다. 보내는 것은 사용자다.
- 정정 목록은 끝줄이 있고 n == eligible 일 때만 다 읽은 것이다.
