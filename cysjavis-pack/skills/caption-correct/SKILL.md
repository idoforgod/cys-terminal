---
name: caption-correct
description: ASR 전사 텍스트를 검증된 원본 대본(ground-truth)에 맞춰 정정하는 대표 스킬 — 타임스탬프는 절대 불변, 오인식 단어만 대본 철자로 교정한다. provider는 하드코딩하지 않고 javis_select로 결정론 선택(무키 로컬 시퀀스 정렬 바닥부터, deny-by-default). transcription이 들은 대로 받아적었다면 이 스킬은 대본대로 고쳐쓴다 — caption-align(타이밍 부여) *이전* 단계. "자막 교정 / SRT 정정 / 전사 오인식 수정 / 대본 대조 / ASR correction / transcript correction / 동음이의 교정" 트리거, 또는 검증된 대본이 있는 아키타입(talking-head·screen-demo 등 대본→나레이션→전사 체인)의 transcription과 caption-align 사이로 발동.
cys:
  capability: caption-correct
  stability: beta
  cost_class: light
  best_for: ASR 전사 오인식을 원본 대본 ground-truth로 정정(타임스탬프 불변·텍스트만 교정) — 자막 정확도의 빠진 게이트
---

# caption-correct

`transcription`이 산출한 `transcript.json`은 **들리는 대로** 받아적은 것이라, 동음이의·고유명사·
전문용어에서 오인식이 난다(예: "김민준"→"김민중"). 그런데 `script-writer`가 만든 **검증된 대본이
ground-truth로 이미 존재**하는데 활용되지 않는다 — 자막 정확도 파이프라인의 **빠진 게이트**다.
이 스킬은 ASR 텍스트를 대본에 맞춰 **단어 철자만 정정**하고, **타임스탬프는 절대 건드리지 않는다**.

> ★Jay Choi "AI 영상편집" 영상의 SRT corrector 워크플로우(2026-06)의 cys 이식. 코드를 복사하지
> 않고 '대본 ground-truth로 ASR 정정·타이밍 불변' 원리만 차용했다(클린룸). 실제 정정은 아래에서
> 결정론 선택한 로컬 도구(시퀀스 정렬)가 수행한다.

> ★위치(불가침): `transcription → caption-correct → caption-align`. caption-align은 정정된
> `transcript_corrected.json`을 소비한다. 정정은 **align 이전** — 이 시점엔 아직 `cues`가 없고
> 보호 대상은 `transcript.json`의 단어 타임스탬프다.

## provider 선택 (하드코딩 금지 — javis_select 위임)

```bash
javis_select rank --catalog "${CYS_PACK_DIR:-$HOME/.cys/pack}/round/video_provider_catalog.json" \
  --capability caption-correct --intent "<용도: 대본 대조 자막 정정>" --free-first
```

- **무료·로컬 바닥**: `correct_align_local`(difflib류 시퀀스 정렬·무키·로컬·LLM 호출 0). 대본 토큰과
  ASR 토큰을 정렬해 대본 철자를 ASR 타이밍 위로 전사(轉寫)한다. 키 없이 항상 가용. **토큰 소비 0
  — 구독 한도를 쓰지 않는다**(자원 거버넌스·토큰 경제).
- 결정론 정렬로 애매한 동음이의만 남으면 상위 티어(LLM 보조)를 `--prefer`로 선택 가능(선택적).
- `--free-first`로 무료 바닥 우선.

## 입력·산출물 계약

- **입력 (둘 다 필수)**:
  - `transcript.json`의 `segments[].text`·`segments[].words[]`(start/end/w) — `transcription` 산출물.
  - **검증된 대본**(ground-truth): `script-writer` 산출 대본(`script.md`/나레이션 텍스트) 또는
    사용자가 명시 제공한 대본. **대본이 없으면 이 스킬을 쓰지 마라**(아래 환각0).
- **산출**: `transcript_corrected.json`(작업 폴더) — 입력과 **동일 구조**, `text`/`words[].w`만 정정:

```json
{
  "source": "<원본 미디어 경로>",
  "provider": "<javis_select가 고른 id>",
  "ground_truth": "<대조한 대본 경로>",
  "segments": [
    {"start": 0.0, "end": 3.2, "text": "...(대본 철자로 정정)", "words": [{"w": "...", "start": 0.0, "end": 0.4}]}
  ]
}
```

- **타임스탬프 불변이 핵심** — 모든 `segments[].start`/`end`·`words[].start`/`end`는 입력
  `transcript.json`과 **byte-동일**해야 한다. 정정은 `text`/`words[].w`(철자)만 바꾼다.
- 다운스트림 `caption-align`이 `transcript_corrected.json`을 소비해 큐 타이밍을 생성한다
  (D4 매니페스트 `field_present: segments`로 강제 가능).

## 환각0·producer≠evaluator (불가침)

- **타이밍 절대 불변**: `start`/`end` 한 글자도 바꾸지 마라 — 정정 전후 타임스탬프 결정론 diff가
  0이어야 한다. 벗어나면 산출 무효(다듬지 말고 재실행 — garbage-in 차단).
- **대본 밖은 지어내지 마라**: 대본에 매칭되지 않는 구간(애드립·즉흥 발화·대본 이탈)은 **ASR 원문을
  그대로 유지**하고 정정하지 마라. 대본을 억지로 욱여넣으면 **오답 자막**이 된다(영상 근거: 즉흥
  발화 시 대본 강제는 틀린 자막). 애매하면 `grill-me`로 사람에게 물어 합의 후 정정.
- **추측 교정 금지**: 동음이의가 대본에 없으면 ASR 원문 유지. `hallucination-guard`로 정정 근거를
  대본 위치로 추적 가능하게 둔다.
- 검증은 산출 노드가 자가채점하지 않는다 — `video-verify`(별 노드)가 (a)타임스탬프 불변 (b)정정이
  대본 ground-truth에 근거하는지를 독립 확인한다(D1 verdict 계약·점수금지).

## 거버넌스 배선

- 위임 티켓 전제: `--requires-skills transcription,caption-correct`(D6 자동주입 — 전사·대본 선행).
  무접촉 경계로 `--dont "transcript.json의 start/end 타임스탬프(절대 불변)"`를 함께 주면 외과적이다.
- **발동 조건(배선 게이트)**: 검증된 대본이 존재하는 체인에서만 transcription과 caption-align
  사이에 삽입한다(talking-head·screen-demo처럼 대본→나레이션→전사). **즉흥 발화로 대본이 없는
  아키타입(podcast-repurpose 등)에는 배선하지 마라** — ground-truth 부재 시 정정 불가.
- 아키타입 매니페스트(D4) 정정 단계 success_criteria 예(닫힌 check enum만 사용):
  `{"kind":"json_valid"}` + `{"kind":"field_present","value":"segments"}` +
  `{"kind":"min_items","value":{"field":"segments","min":1}}`. 타임스탬프 불변은 video-verify가 확인.
- cost_class=light: 로컬 결정론 정렬·LLM 무호출이 기본 — 토큰 한도 보존(기본 제약 "구독제만·
  종량제 금지" 정합).
