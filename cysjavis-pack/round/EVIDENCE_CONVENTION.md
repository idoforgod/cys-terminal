# 매체별 증거 규약 (EVIDENCE_CONVENTION) — 레버② SOT

- 출처: `_round/QUALITY_LEVERS_DESIGN.md` §2 (레버②) — 이 문서가 §2 표를 완결 규약으로 확정한다.
- 상태: 규약(convention) — **코드 변경 0**. `~/.cys/pack/bin/javis_verdict.py` 등 어떤 코드도 수정하지 않는다.
- 적용 대상: 검증자(agy·codex·sub-agent·검증 페인·video-verify 관문)가 결함을 지적하는 **모든 verdict**.

---

## 0. 왜 규약인가 (코드 무수정 원리)

`javis_verdict.py`의 verdict 스키마는 이미 `evidence[].ref`(비어있으면 거부)를 강제한다.
그러나 ref 문자열의 **형식은 자유**다 — 즉 "무엇을 어떻게 담는가"는 스키마가 강제하지 못한다.
그 빈틈을 **사람·에이전트가 지키는 규약**으로 메운다. 코드는 손대지 않는다.

**불가침 스키마 사실 (실측 — javis_verdict.py):**
- `EVIDENCE_KEYS = ["claim", "ref", "verified"]` **고정**. 이 3개 외 새 키를 evidence 항목에 추가하면
  스키마 위반(exit 1). → **매체별 증거는 반드시 `claim`·`ref` 문자열 안에 담는다. 새 키 금지.**
- `evidence[].ref`는 공백만 있으면 거부. 비어서는 안 된다(근거 없는 주장 차단).
- 점수류 키(`score`/`grade`/`rating`)는 어느 깊이든 금지 — 증거에 점수를 넣지 않는다.
- verdict enum = `ACCEPT | REVISE | BLOCK | ESCALATE`(검증기 강등 타깃 `INVESTIGATE`는 emit 전용).
- **CHAI R2**: `REVISE`·`BLOCK` verdict는 `issues[]`에 실행가능한 `fix`(비어있지 않은 문자열)가
  하나 이상 있어야 한다. 없으면 `INVESTIGATE`로 자동 강등되어 lint 발생 → validate exit 1.
  → **결함을 지적하는 verdict(REVISE/BLOCK)는 반드시 교정안(fix)을 동반한다.**

---

## 1. 매체별 필수 증거 3종 + ref 기입 규약 (§2 표 확정)

| 매체 | 필수 증거 3종 | 위치 ref 기입 규약 |
|---|---|---|
| **웹/앱 UI** | ① 스크린샷 경로 · ② DOM ref · ③ 코드 추정 위치(**미상이면 '미상' 명시**) | `경로.png#ref_N` |
| **영상/음성** | ① 타임코드 구간 · ② 프레임 캡처 경로 · ③ 결함 서술 | `경로.mp4#t=MM:SS-MM:SS` |
| **텍스트**(설교·대본·연구) | ① 파일#문단(또는 행) · ② 결함 문장 직접 인용 · ③ 위반 기준 항목 | `파일.md#p12` |

- ref 기입 규약의 `#` 뒤 조각(`#ref_N` / `#t=MM:SS-MM:SS` / `#p12`)은 **결함 위치를 정확히 가리키는 앵커**다.
  파일 전체를 가리키는 ref(앵커 없음)는 "근거 없는 주장"에 준한다 — 앵커를 반드시 붙인다.

---

## 2. 3종을 evidence[] 배열에 담는 매핑 규칙

evidence 항목은 `{claim, ref, verified}` 3키뿐이다. 매체별 3종을 아래처럼 나눠 담는다.
**한 결함당 최소 evidence 항목 수**는 규약이 정한 매핑을 따른다(3종 전부가 배열 안에 실재해야 한다).

### 웹/앱 UI (한 결함 → evidence 최소 2항목)
- **항목 A** — 스크린샷 경로 + DOM ref (ref 규약이 둘을 한 문자열로 융합):
  `ref = "경로.png#ref_N"` (경로=스크린샷, `#ref_N`=해당 요소의 DOM ref 앵커) · `claim`=결함 시각 서술
- **항목 B** — 코드 추정 위치: `ref = "src/파일:행"` · `claim`=추정 근거.
  **위치 미상이면 `ref = "미상"`으로 명시**(공백 금지·침묵 금지). `verified`는 추정이면 `false`.

### 영상/음성 (한 결함 → evidence 최소 2항목)
- **항목 A** — 타임코드 구간: `ref = "경로.mp4#t=MM:SS-MM:SS"` · `claim`=결함 서술(③).
- **항목 B** — 프레임 캡처 경로: `ref = "frames/f_MMSS.png"` · `claim`=이 프레임이 보이는 결함.

### 텍스트 (한 결함 → evidence 최소 1항목, 3종을 claim+ref로 결합)
- `ref = "파일.md#p12"` (파일#문단 또는 `#L88` 행 앵커) ·
  `claim = "위반 기준: <기준 항목> — 직접 인용: \"<결함 문장 원문>\""` (위반 기준 항목 + 결함 문장 직접 인용).

`issues[].where` 필드에도 동일 ref 규약을 적어 결함 위치를 이중으로 남긴다(교정 정밀도↑).

---

## 3. 침묵 금지 규칙 (미상 명시)

**결함 위치를 특정하지 못했다면 침묵하지 말고 '미상'을 명시한다.**
- 웹/앱 코드 추정 위치를 못 찾으면 `ref = "미상"` + `claim`에 "코드 위치 특정 불가 — 화면 증거만 확보".
- ref를 아예 비우는 것은 스키마 위반(거부)이자 규약 위반이다. 모른다는 사실도 근거로 기록한다.
- 이는 "찾지 못함"을 "결함 없음"으로 오독하는 것을 막는 안전장치다(garbage-in 차단).

---

## 4. 매체별 올바른 verdict 예시 (validate exit 0 확인됨)

> 모두 `python3 ~/.cys/pack/bin/javis_verdict.py validate <FILE>` exit 0. 결함 지적이므로
> `REVISE`/`BLOCK`이며 CHAI R2 준수를 위해 `issues[]`에 실행가능한 `fix`를 동반한다.

### 웹/앱 UI — `webui_verdict.json`
```json
{
  "verdict": "REVISE",
  "justification": "체크아웃 화면의 결제 버튼이 세이프 영역을 벗어나 잘림 — 스크린샷·DOM ref로 확인, 코드 위치는 추정.",
  "evidence": [
    {"claim": "결제 버튼이 우측 경계 밖으로 잘려 클릭 불가(시각 확인)", "ref": "artifacts/screenshots/checkout.png#ref_12", "verified": true},
    {"claim": "추정 원인: 컨테이너 고정폭 오버플로 — 코드 위치는 추정(미검증)", "ref": "src/components/Checkout.tsx:88", "verified": false}
  ],
  "issues": [
    {"severity": "major", "where": "artifacts/screenshots/checkout.png#ref_12", "what": "결제 버튼이 세이프 영역을 벗어나 잘림(DOM ref_12)", "fix": "Checkout.tsx:88 컨테이너를 max-width + flex-wrap으로 바꿔 버튼을 세이프 영역 안으로 되돌린다"}
  ]
}
```

### 영상/음성 — `video_verdict.json`
```json
{
  "verdict": "BLOCK",
  "justification": "01:23~01:27 구간에서 아바타 립싱크가 나레이션보다 밀림 — 타임코드 구간·프레임 캡처로 확인.",
  "evidence": [
    {"claim": "01:23-01:27 구간 아바타 입 움직임이 발화보다 약 0.4초 지연(결함 서술)", "ref": "final/video.mp4#t=01:23-01:27", "verified": true},
    {"claim": "지연이 보이는 대표 프레임 캡처", "ref": "final/frames/f_0125.png", "verified": true}
  ],
  "issues": [
    {"severity": "blocking", "where": "final/video.mp4#t=01:23-01:27", "what": "아바타 립싱크가 나레이션보다 약 0.4초 지연", "fix": "video-stitch 단계로 회송해 해당 클립의 오디오 오프셋을 -0.4초 보정 후 재합성·재검증"}
  ]
}
```

### 텍스트(설교·대본·연구) — `text_verdict.json`
```json
{
  "verdict": "REVISE",
  "justification": "대본 12문단이 출처 없는 통계를 단정 — 파일#문단·직접 인용·위반 기준으로 확인.",
  "evidence": [
    {"claim": "위반 기준: 환각0(무근거 단정 금지) — 직접 인용: \"전 세계 인구의 73%가 이미 이 기술을 쓴다\"(출처 없음)", "ref": "draft/script.md#p12", "verified": true}
  ],
  "issues": [
    {"severity": "major", "where": "draft/script.md#p12", "what": "출처 없는 73% 통계 단정 — 사실대장(facts.md) 미등재", "fix": "해당 문장을 검증된 출처 수치로 교체하거나 삭제 후 script-writer-factcheck 재실행"}
  ]
}
```

---

## 5. 스킬 배선 (규약 참조처)

아래 스킬의 산출물 포맷 섹션이 이 규약을 참조·의무화한다(각 매체별):
- **웹/앱 UI**: `[[appbuild-orchestrate-verify]]`(findings 리포트), `[[appbuild-orchestrate-route]]`(라우팅 근거)
- **영상/음성**: `[[video-verify]]`(verify-report.md), `[[video-verify-visual]]`(관문 issue 증거 포맷 — audio-sync 등 `{timecode,type,evidence}` 공유 관문 동일 적용)
- **텍스트**: 리뷰 라운드 verdict(agy·codex·sub-agent) — 도메인 스킬(factcheck·코칭·페르소나)이 지적하는 결함도 이 규약을 따른다.

## 6. 검증 방법

```bash
python3 ~/.cys/pack/bin/javis_verdict.py validate <verdict.json>   # exit 0=규약·스키마 준수
```
샘플 3종: `_round/compete_evidence_samples/{webui,video,text}_verdict.json` — 3건 모두 exit 0.
