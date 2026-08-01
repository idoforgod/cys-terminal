# 위협 모델 — 임무 게이트 / 배달 원장 (SOT)

> **이 문서가 임무 게이트 보장 범위의 단일 진실원천(SOT)이다.**
> 코드 주석·디렉티브·릴리스 노트는 이 문서를 **가리키기만** 한다. 서술이 갈리면 이 문서가 이긴다.
> 서술을 늘리고 싶으면 여기에 쓰고, 다른 파일에는 포인터만 남긴다(중복 서술 금지).
>
> 제정: 2026-08-02 (라운드4) · 개정: 2026-08-02 (라운드5 — §1 #14·#15·#16 추가) ·
> 대상: `cysjavis-pack/bin/javis_mission.py` · `src/bin/cysd/delivery.rs` ·
> `src/bin/cysd/handlers.rs` · `src-tauri/src/main.rs` · `ui/src/main.ts` ·
> `cysjavis-pack/directives/MASTER_DIRECTIVE.md` §0-C

---

## 0. 이 게이트가 지키는 것

임무 게이트는 **"master가 지금 오너 지시 없이 스스로 작업을 시작해도 되는가"** 하나만 판정한다.

- 판정처는 `javis_mission.py` 하나다(exit 0=임무 있음 / 1=없음 / 2=판독 불가는 '없음'과 같게 취급).
- 소비자는 `javis_orchestra.py next-action`(exit 3) · `gate-status`(exit 4) · `hooks/role-bootstrap.sh`.
- 계약 본문은 `MASTER_DIRECTIVE.md` §0-C. **자율 착수 권한은 오너 채널에서만 나온다.**

계기가 된 사고: 2026-08-01 윈도우 실사고 — 오너가 부트스트랩 선언만 하고 **아무 임무도 주지
않았는데** 5개 노드가 무한 작업에 들어가 7일 사용량 72%를 태웠다. 원인은 자기인가
(self-authorization) 루프였다. 큐(`round/SESSION_STATE.md`)는 master 자신이 쓰는 파일인데,
그 큐가 비어 있지 않다는 사실만으로 자율 착수가 허가됐다.

---

## 1. IN SCOPE — 반드시 막는다 (하나라도 열리면 이 게이트는 실패다)

**정의**: 오너 개입이 0인 상태에서 **시스템의 평시 정상 동작**이 master에게 자율 착수 권한을
발급하는 모든 경로. 악의가 필요 없다는 것이 이 범주의 특징이며, 2026-08-01 실사고가 여기 속한다.

| # | 경로 | 왜 열렸었나 | 무엇이 닫는가 |
|---|---|---|---|
| 1 | 자기 예약 wake (`cys schedule add --text "[wakeup] 다음 액션 착수" --to master`) | 예약 문장이 stdin으로 돌아와 '오너 프롬프트'로 기록 | 배달 원장 기록 `schedule.rs::inject` + 발화 시 라벨 강제 `schedule.rs::ensure_machine_label` |
| 2 | 워커·CEO 완료 push (`cys send --to master "…"`) | 첫 위임 사이클 직후 게이트가 항구 개방 | 배달 원장 기록 `handlers.rs::surface.send_text`(Data/Inject 양 분기) |
| 3 | 큐 배달(watchdog `--queued`) | 조용해진 pane에 자동 배달된 문장이 오너 입력과 구별 불가 | `governance.rs` 큐 배달자 (Origin::Queue) |
| 4 | 승인 Feed의 CEO 자동 라우팅 주입 | 동상 | `handlers.rs` Feed 라우팅 (Origin::Feed) |
| 5 | 외부 채널(inbox) 봉투 주입 | 동상 | `channels.rs` 봉투 주입 (Origin::Channel) |
| 6 | 빈 좌석 승계 고지 주입 | 동상 | `handlers.rs` 좌석 승계 (Origin::SeatTakeover) |
| 7 | **라벨 누락·변형** (라벨 없는 push, 중첩 대괄호·라벨 내 개행·80자 초과·전각 대괄호) | 판별 근거가 문자열 라벨 정규식 **하나**뿐이었다 | 층1이 라벨과 무관하게 해시로 판정 + 층2 규칙 교체(`javis_mission.py::has_machine_label`/`_label_head`) |
| 8 | **과거 세션 임무의 잔존** | `ts`를 기록만 하고 아무도 읽지 않아 몇 달 전 임무가 오늘도 유효 | TTL(기본 12h) + 데몬 인스턴스 결박 `javis_mission.py::gate` (TTL·`daemon_epoch` 결박) |
| 9 | 남의 pane 임무 유용 | surface 미대조 | surface 결박 `javis_mission.py::gate` (surface 대조) |
| 10 | **원장 판독 실패의 조용한 통과(fail-open)** | 손상·권한·디렉터리를 '부재'와 융합하면 게이트가 영영 열린 채로 산다 | 3상 분리(`absent`/`ok`/`unreadable`) + `unreadable` 상태에서 기록된 임무는 gate가 거부 `javis_mission.py::gate` (`ledger_status==unreadable` 거부) |
| 11 | 게이트가 자기 우회법을 안내 | 차단 안내문이 `javis_mission.py set` 사용법을 친절히 알려줬다 | 안내문에서 제거 + `set`을 `cys feed push --wait` 오너 승인에 결박 `javis_mission.py::_owner_confirm`/`cmd_set` |
| 12 | 수렴 판정이 착수 권한으로 승격 | `gate-status`가 임무 미지정을 감지하고도 exit 0을 냈다 | exit 4 신설 `javis_orchestra.py GATE_EXIT_NO_MISSION` |
| 13 | **클라이언트 자기신고 `human:true`** | 원장 기록 억제 근거가 클라이언트가 붙이는 불리언이었다 — 원시 소켓 한 줄로 무기록 → 층2 폴백 → 무라벨 통과가 실측됐다(라운드3 검증자 N3) | 억제 근거를 데몬이 발급·0600 보관하는 `operator.token`으로 교체(`delivery.rs` "★R4 수리" 절 · `state.rs::write_operator_token`) |
| 14 | **창(6h) 밖 배달의 조용한 통과** | 판독자가 창 밖 레코드를 `continue`로 **버려서**, 원장에 정확히 존재하는 무라벨 기계 배달이 지연 제출되면 층1이 건너뛰고 층2도 무라벨이라 통과했다. 실측(라운드4): 5.9h 차단 / 6.1h·12h·24h **개방**, `anomalies=[]`로 흔적 0 | 창 밖도 **접는다**(sha 일치는 그 자체로 기계 증거) + `delivery_out_of_window` 이상징후 발행 `javis_mission.py::read_delivery`/`machine_origin` |
| 15 | **GUI 자동 주입이 `operator_token`으로 기록을 억제** | 토큰은 *사람이 앉은 GUI 세션*만 증명하는데 R4 배선이 **UI가 조립한 문안**(전출 지시 전문·`launchCmd`·`restartNode`·경로 삽입)에도 붙여, 무기록 → 층2 폴백 → 오너 임무로 기록됐다(실측 rc=0·흔적 0) | UI→tauri→cysd 로 `machine_origin` 표식을 흘리고 **표식이 있으면 토큰이 유효해도 기록**(원장 `origin=gui_auto`). 실키(`term.onData`) 경로는 표식 없음 = 무기록 유지 `ui/src/main.ts`·`src-tauri/src/main.rs::send_input`·`handlers.rs` |
| 16 | **두 기계 배달의 연접** | `cys send`는 텍스트만 넣고 제출(Return)은 따로라, 큐 배달 "A"가 버퍼에 남은 채 schedule 배달 "B"가 오면 TUI가 "AB"를 한 프롬프트로 제출한다. 전문 해시는 A·B 어느 쪽과도 달라 층1 통과, 무라벨이라 층2도 통과(흔적 0) | `preview` 앵커 + sha 확증으로 조각을 찾아 **전량 커버면 접고**(`delivery_concatenated`), 충분히 긴 조각을 통째로 포함해도 접는다(`delivery_substring`) `javis_mission.py::_composition` |

### 이 범주를 막는 구조 — 배달 원장(delivery ledger)

데몬(cysd)이 pane stdin에 텍스트를 밀어 넣기 **직전에** 영속 원장(JSONL)에 append하고, 훅은
프롬프트를 같은 규칙으로 정규화·해시해 대조한다. 일치하면 "기계가 방금 이 pane에 밀어 넣은
바로 그 문장"이므로 임무가 아니다.

이 구조가 사는 이유는 **라벨 문자열이 발신자가 고르는 값**인 반면 원장은 **주입한 쪽이 남긴
사실 기록**이기 때문이다. 그래서 문안 규약을 지키지 않은 push(위 #7)도 잡힌다.

불변식 3개(`delivery.rs` 모듈 주석에 박제 · 튜닝 시 보존):
1. **원장 기록이 주입보다 반드시 선행한다** — `write_tx.try_send()` 직전에 기록해 happens-before 확보.
2. **오너가 자판으로 친 실키만 기록에서 제외한다** — 오너 문장이 자기 해시와 매치돼
   기계로 접히면 온보딩이 전면 사망하기 때문이다. 제외 근거는 두 가지를 **함께** 만족해야 한다:
   ⓐ**데몬이 아는 사실**(`operator.token` — 자기신고 `human:true`를 믿던 구현이 §1 #13) **그리고**
   ⓑ**UI가 그 문안을 조립하지 않았다**(`machine_origin` 표식 없음 — 토큰만 보던 구현이 §1 #15).
   토큰은 *사람이 앉은 세션*을, 표식은 *사람이 친 문장*을 가른다. 판정 불가는 **기록**한다(fail-closed).
3. **거짓 양성(기계→오너 오인)이 치명 / 거짓 음성(오너→기계 오인)은 경미** — 애매하면 기록한다.
   이 비대칭은 판독 쪽에도 그대로 적용된다: 창(6h)은 **버리는 기준이 아니라** 이상징후 경계다(§1 #14).

---

## 2. OUT OF SCOPE — 막을 수 없고, 막는 척해서도 안 된다

**정의**: 동일 UID로 임의 셸·원시 소켓을 쓸 수 있는 구성요소의 **의도적 위조**.

이 저장소는 단일-UID 신뢰 노드 모델 위에 있다(`ARCHITECTURE-AND-PHILOSOPHY.md` §6). master는
Bash 도구를 갖고 있고, 임무 대장·배달 원장은 **서명 없는 평문 파일**이며 오너와 **같은 사용자
계정**의 권한으로 열린다. 따라서 아래는 **어떤 in-band 장치로도 원리적으로 닫히지 않는다.**

| 위조 경로 | 왜 닫히지 않는가 |
|---|---|
| 임무 대장 파일을 직접 쓴다 (`javis_mission.py path` → 그 JSON을 손으로 작성) | `write_ledger()`는 서명 없는 평문 JSON을 원자 교체로 쓸 뿐이고(`javis_mission.py::write_ledger`), `gate()`는 그 파일의 필드만 읽는다. 같은 필드를 손으로 채우면 판정은 동일하게 통과한다. |
| 배달 원장 파일을 지운다·자른다 | 원장은 평문 JSONL이고 같은 계정 권한으로 열린다. 데몬 기동 시 sentinel(`Origin::Boot`)을 넣어 "정상 원장은 0바이트가 아니다"를 성립시키므로 **빈 파일·손상은 탐지되어 fail-closed로 접히지만**, 파일 자체를 지운 상태는 "아직 기계 배달이 없다"와 구별할 수 없다. 후자를 fail-closed로 바꾸면 **첫 부팅에서 오너가 임무를 줄 수 없어 부트스트랩이 죽는다**(★부트스트랩 불가침). 그래서 이 경로는 탐지·감사로 다룬다. |
| 데몬 인스턴스 표식(`delivery-<lane>.epoch.json`)을 고쳐 쓴다 | 세션 결박이 무력화된다. 같은 이유(평문·동일 UID). |
| `CYS_MISSION`을 실은 env로 데몬을 띄운다 | 설계상 **오너 채널**이지만, 데몬을 띄울 수 있는 주체라면 누구나 쓸 수 있다. |
| 훅·팩 파일 자체를 고친다 | 게이트 코드가 곧 판정이므로 코드를 고치면 판정이 바뀐다. |

**따라서 다음 문장은 전부 거짓이며 코드·문서에 써서는 안 된다:**

- "판별 근거를 master가 **고를 수 없는** 층으로 내렸다"
- "근거가 문자열 밖에 있어 **우회면이 없다**"
- "…는 **구조적으로 불가능하다**"(임무 대장·배달 원장 조작에 대해)
- "`set`은 **자기해제 불가**"(명령 경로 한정이라는 단서 없이)

정확한 서술은 하나다:

> **평시 정상 동작 경로(§1 IN SCOPE)를 닫는다. 동일 UID의 의도적 위조는 닫지 못하며,
> 그것은 차단이 아니라 감사 흔적(§3)으로 다룬다.**

### 왜 "닫는 척"이 결함인가

과대 주장은 두 가지를 동시에 망가뜨린다. ①읽는 사람(오너·리뷰어·미래의 우리)이 실제보다 강한
보장을 전제하고 그 위에 다른 결정을 쌓는다. ②"이미 닫혔다"는 서술이 감사 층 투자를 정당화 밖으로
밀어낸다. **'완전 방어'를 주장하는 문장은 그 자체가 결함이다.**

---

## 3. 무기는 게이트가 아니라 감사 가능성 (탐지)

OUT OF SCOPE를 상대로 우리가 가진 것은 차단이 아니라 **흔적**이다. 아래는 전부 **사후 탐지**이며
**사전 차단이 아니다** — 이 구분을 흐리지 마라.

- **배달 원장** `<state>/delivery-<lane>.jsonl` — append-only. 데몬이 이 pane에 무엇을 언제
  밀어 넣었는지의 기록(`origin`·`ts_epoch`·`surface`·정규화 미리보기 64자·`chars`·sha256).
  `origin=gui_auto`는 **GUI가 조립한 자동 주입**이라 사람 키 입력과 사후에 구별된다(§1 #15).
  `preview`+`chars`는 진단용이자 **조각 대조의 앵커**다(§1 #16 — 앵커로 후보를 찾고 판정은 sha256).
- **임무 대장** `<state>/mission[-<lane>].json` — `source`(`prompt`/`declaration_residual`/
  `owner_confirm`/`anomaly_only`)·`reason`·`ts_epoch`·`boot_epoch`·`ledger_status`·`prompt_chars`·
  `anomalies`.
- **★흔적의 지속성(R5에서 실측으로 고친 것)**: 훅(`role-bootstrap.sh`)은 `javis_mission record`를
  **`>/dev/null 2>&1`** 로 부른다 — stderr는 버려진다. 그래서 프롬프트를 기계로 접은 경로(대장
  무변경)의 **프롬프트 유래** 이상(`delivery_out_of_window`·`delivery_concatenated`·
  `delivery_substring`)은 발행하자마자 사라졌다(상태 유래 이상은 매 판독마다 재관측되지만
  프롬프트 유래는 재관측되지 않는다). 이제 그 이상은 **판정 필드를 한 글자도 바꾸지 않고**
  대장 `anomalies`에 병합돼(`javis_mission.py::_persist_anomalies`) `status`/`status --json`에
  실린다. 대장이 없으면 `mission=null`·`source=anomaly_only` 최소 레코드를 만드는데, gate는 이를
  '대장 없음'과 **동일한 EXIT_NONE**으로 판정하므로 권한은 생기지 않는다(self-test로 박제).
  **발행했다는 사실과 오너가 볼 수 있다는 사실은 다르다** — 감사 층은 후자까지 확인해야 성립한다.
- **교차 대조가 위조를 드러낸다**: `source=owner_confirm`인데 대응하는 승인 Feed 항목이 없다면
  `set`을 거치지 않고 파일이 쓰였다는 뜻이다. `source=prompt`인데 세션 트랜스크립트에 대응하는
  오너 발화가 없어도 같다. **위조는 이 대응 관계를 함께 위조해야 하며, 그 범위가 넓어질수록
  흔적이 늘어난다.**
- **커밋·이벤트 로그** — 게이트 파일을 바꾸는 변경은 git 이력에 남는다(master 커밋 trailer 규약).

한계도 정직하게: 위조자가 대장과 원장을 **함께** 손보면 위 대조도 함께 무너진다. 흔적의 가치는
"불가능하게 만든다"가 아니라 "**흔적 없이 하려면 더 많은 것을 동시에 조작해야 한다**"이다.

---

## 4. 알려진 잔여 위험 (수용 또는 미해소)

1. **거짓 음성(수용)** — 오너가 기계 push와 *정규화 후 완전히 동일한* 문장을 직접 치면
   기계로 접힌다. 한 번 더 물으면 되므로 비대칭 원칙상 수용한다.
   ★2026-08-02 R5 로 이 창이 **넓어졌다**: 종전엔 "최근 6시간 내 push"였으나 이제 원장 보존
   범위(본 세대 + 회전 세대 `.1`) 전체가 대상이다(§1 #14를 닫는 대가). 넓힌 근거는 비대칭이다 —
   좁은 창이 만든 것은 거짓 양성(치명)이었고, 넓힌 창이 만드는 것은 거짓 음성(경미)이다.
   접힐 때는 `delivery_out_of_window`가 발행되므로 오너는 "왜 안 먹었는지"를 즉시 안다.
2. **층2 과확장(수용)** — 오너가 `[`로 문장을 시작하면 임무 아님으로 접힌다. 같은 이유로 수용.
2-b. **부분 포함 과확장(수용 · R5)** — 오너가 기계 배달 문안(24자 이상)을 그대로 인용해 자기
   지시를 덧붙이면 프롬프트 전체가 접힌다. 이는 이 모듈의 확정 규약("본문에 오너 문장이 섞여
   있어도 통째로 제외 · 부분 추출 금지")과 같은 방향이며, 24자 하한은 짧은 기계 문장이 아무
   프롬프트에나 우연히 포함돼 임무가 영영 안 열리는 반대편 장애를 막는다.
   ★한계: 24자 **미만** 조각이 오너 문장과 섞이면 접지 않는다. 이 경우는 정의상 오너 개입이
   있었다는 뜻이므로(IN SCOPE = 오너 개입 0) 이 게이트의 실패로 세지 않는다.
3. **원장 부재 → 층2 폴백** — §2의 두 번째 행. 부트스트랩 불가침 때문에 fail-closed로 바꿀 수
   없다. 부재와 손상을 분리해 손상 쪽만 fail-closed로 닫은 것이 현재 도달점이며, R5부터 부재
   자체를 `ledger_absent` 이상징후로 **고지**한다(차단이 아니라 가시화 — 흔적 없는 열림 금지).
4. **`CYS_MISSION` 배선 0건** — `src/`·`ui/` 어디에도 이 변수를 설정하는 코드가 없다. pane 환경은
   데몬 프로세스의 env를 상속하므로 `CYS_MISSION=… cys launch-agent …`는 전달되지 않는다.
   즉 이 신호는 "운영자가 그 env로 데몬을 띄웠다"는 뜻이고, 어떤 launcher도 자동으로 채우지 않는다.
5. **암호학적 출처증명 부재** — 대장·원장에 데몬 서명이 없다. 서명을 붙여도 시크릿이 같은 UID
   아래 있으면 위조 난도만 올라갈 뿐 §2가 IN SCOPE로 옮겨오지는 않는다. cysd attestation은
   로드맵 항목이며, 그때도 "난도 상승·탐지 강화"로만 주장해야 한다.
6. **창 밖 배달 (R5-A에서 봉합 · 잔여는 아래 ⓑ)** — 층1 대조가 **유한한 판정 창**
   (`DELIVERY_WINDOW_S` 기본 6h) 안의 레코드만 보던 시절, 창을 넘겨 제출된 기계 push는
   `read_delivery`가 `continue`로 버려 층1에 매치되지 않고 층2(라벨)로 내려갔다. 그 push가
   무라벨이면 결과는 §1 #7과 같다 — **오너 임무로 기록되고 게이트가 열렸다**. 라운드4 검증자
   실측: 5.9h=차단 / **6.1h·12h·24h=개방**, 그나마 `anomalies=[]`라 흔적조차 없었다.
   · **봉합(2026-08-02 R5-A)**: 창 밖 레코드도 `stale=True`로 **남겨** 대조에 쓴다. 일치하면
     접되 `delivery_out_of_window` 이상징후(배달시각·지연 포함)를 발행한다
     (`javis_mission.py::read_delivery` · `::machine_origin`). 창은 '버리는 기준'에서 '오래된
     일치를 표시하는 경계'로 격하됐고, **창 길이는 더 이상 판정을 바꾸지 않는다.**
   · **잔여 ⓐ(수용)**: 거짓 음성 구간이 6h → **원장 보존 범위**(본 + 회전 1세대)로 넓어졌다.
     오너가 옛 기계 문장과 정규화 후 동일한 말을 치면 접힌다 — 위 1번과 같은 성질이라 같은
     이유로 수용한다.
   · **잔여 ⓑ(미해소 · 치명 방향)**: 보존 범위 **밖**은 여전히 열려 있다. 데몬은 1세대만
     보존하므로 회전으로 소실된 구간의 기계 push는 층1로 대조할 수 없고 층2 라벨로만 걸린다.
     이건 창이 아니라 **회전**이 만드는 꼬리이며 창을 늘려서 닫히지 않는다. 현재는
     `ledger_rotated` 이상징후(판독 가능한 최고(最古) 배달 시각 포함)로 **탐지만** 한다 —
     §3의 성질 그대로 **차단이 아니다**. 따라서 "층1이 평시 경로를 **전부** 닫는다"는 서술은
     이 항목이 남아 있는 동안 여전히 과대 주장이다.

---

## 5. 이 문서를 가리키는 곳 (포인터 목록 — 중복 서술 금지)

- `src/bin/cysd/delivery.rs` 모듈 주석
- `src/bin/cysd/handlers.rs` `surface.send_text` 의 `human_verified` 주석(§1 #13·#15 분기점)
- `src-tauri/src/main.rs::send_input` · `ui/src/main.ts::sendRaw` 주석(실키 vs UI 조립 문안 경계)
- `cysjavis-pack/bin/javis_mission.py` 모듈 docstring · 층1 주석 · `set` 항목
- `cysjavis-pack/directives/MASTER_DIRECTIVE.md` §0-C
- `ARCHITECTURE-AND-PHILOSOPHY.md` §6
- `docs/RELEASE_NOTES_0.14.10.draft.md`

새로 게이트 서술을 쓰고 싶으면 **여기에 쓰고 포인터만 남긴다.**
