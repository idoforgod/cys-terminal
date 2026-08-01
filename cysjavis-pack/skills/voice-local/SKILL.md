---
name: voice-local
description: 텍스트를 100% 온디바이스 로컬 TTS로 합성하는 대표 스킬 — 종량제 API 0(기본 제약 "구독제만·종량제 금지" 부합). 엔진을 필요(한국어·클로닝·CPU·표현)별로 선택, 무제한 길이 문장경계 청킹+크로스페이드, TTS 아티팩트 trim, 경계 음질 QC, 후처리는 audio-post로 위임. voice-clone-elevenlabs(종량제 API)의 로컬 대체. "로컬 음성 합성 / 온디바이스 tts / 종량제 없이 나레이션 / 한국어 음성 / 로컬 보이스 클론 / voice-local" 트리거. ★기본 음성 라우팅 절체는 오너 결정(ESCALATE).
---

# voice-local

텍스트를 **클라우드 없이, 사용량 과금 없이** 음성으로 합성한다. 기본 제약(구독제만·종량제
API 금지)을 음성 영역에서 충족하는 유일한 길 — 모델·음성·텍스트가 기기를 떠나지 않는다. 설계 DNA는
Voicebox(MIT, github.com/jamiepine/voicebox)의 검증된 로컬 음성 스택에서 채택했다(전수조사
[[voicebox-upgrade-research]], 보고서 `_research/Voicebox_박사급_연구보고서.md`).

> ★근거(실측): 아래 엔진→언어/능력 매핑은 Voicebox 실코드에서 직접 확인했다
> (`backend/backends/__init__.py`, `backend/backends/qwen_custom_voice_backend.py`). 단,
> 한국어 품질의 *정량 측정*(오너 음성으로의 클론 품질)은 아직 미수행 — 'supported(코드상 지원)'
> 이지 'verified(품질 검증)'이 아니다. 측정 전 "검증됨"으로 보고 금지.

## 키·도구

- **API 키 없음 — 이게 핵심이다.** ElevenLabs/HeyGen 같은 `*_API_KEY`·토큰 과금이 **0**.
  그래서 `[[suite-runtime-keys]]` deny-게이트 대상이 아니다(로컬 = 키 불필요 = 제약 자동 충족).
- 엔진은 로컬 가중치로 구동: Apple Silicon=MLX/Metal, NVIDIA=CUDA, 그 외=PyTorch CPU(디바이스 폭포).
  모델은 HuggingFace에서 **1회 다운로드 후 오프라인**(이후 egress 0).
- 청킹·크로스페이드·trim 로직은 Voicebox MIT 코드를 이식/재구현한다
  (`backend/utils/chunked_tts.py`, `backend/utils/audio.py` — MIT라 출처표기 시 채택 합법).

## 엔진 선택 (필요 → 엔진, 실측 매핑)

| 필요 | 엔진 | 한국어 | 근거(Voicebox) |
|---|---|---|---|
| **한국어 + 프리셋(레퍼런스 불필요)** | **Qwen CustomVoice** ('Sohee' 한국어 여성 프리셋) | ✅ | `qwen_custom_voice_backend.py:46`,`:14` |
| **한국어 + 고품질 클로닝/딜리버리 지시** | **Qwen3-TTS** (0.6B/1.7B) | ✅ | `__init__.py:244,254` (langs에 ko) |
| **한국어 + 23개 다국어 + zero-shot 클론** | **Chatterbox Multilingual** | ✅ | `__init__.py:306-310` (ko 포함) |
| 영어 전용·초경량·CPU 150x실시간 | LuxTTS (~1GB) | ❌ | `__init__.py:297` (en only) |
| 영어 표현형(`[laugh]`·`[sigh]` 태그) | Chatterbox Turbo | ❌ | `__init__.py:348` (en) |
| 영어 50+ 프리셋·82M 초소형 | Kokoro | ❌ | `__init__.py:365` (ko 없음) |

★오너 1차 언어=한국어 → 기본 후보는 **Qwen CustomVoice(Sohee)** 또는 **Chatterbox Multilingual**.
**Kokoro·LuxTTS·Chatterbox Turbo는 한국어 미지원이니 한국어 작업엔 선택 금지**(엔진→언어 매핑 오류는
연구에서 적발된 실제 함정).

## 절차

1. **엔진·음성 결정** → 검증: 위 표로 필요(언어·클론여부·자원)→엔진 확정. 한국어면 Qwen
   CustomVoice/Qwen3-TTS/Chatterbox 중에서만. 클로닝이면 ~수초 레퍼런스 샘플(zero-shot), 프리셋이면
   레퍼런스 불필요(Sohee 등).
2. **로컬 런타임 확보** → 검증: 엔진 모델을 HF에서 1회 다운로드(이후 `HF_HUB_OFFLINE`). 디바이스
   폭포(MLX→CUDA→CPU) 자동. **모델 캐시 존재를 합성 전에 확인**(캐시 없이 합성 시작 금지 — 무한 대기
   방지, Voicebox `/capture/readiness` 패턴).
3. **무제한 길이 청킹** → 검증: 텍스트를 문장 경계로 분할(기본 ≤800자/청크, `chunked_tts.py:22`),
   약어·CJK 문장부호·`[tags]` 존중. ≤800자면 단일 청크 zero-overhead fast path(`chunked_tts.py:8`).
4. **합성 + 이어붙이기** → 검증: 청크별 로컬 합성 후 **크로스페이드(기본 50ms,
   `chunked_tts.py:175-192`)로 클릭음 없이 연결**. 동일 엔진·시드·설정 잠금으로 청크 간 음색 일관 유지.
5. **trim + 정제** → 검증: Chatterbox 계열은 `trim_tts_output`로 앞뒤 묵음/아티팩트 제거
   (`audio.py:113`, `engine_needs_trim` 게이트 `generation.py:74`). 입력이 STT 경유면
   `collapse_repetitive_artifacts`로 루프 환각 제거(`refinement.py`). ★한국어 정상 반복(설교 litany
   등)은 결정론 임계값6에 오삭제될 수 있으니 한국어는 비파괴(로그+플래그)로.
6. **경계 음질 QC** → 검증: 청크 경계 불연속·클릭·음색 점프를 청취/측정. 실패 청크만 시드 변경
   재생성. producer≠evaluator — 생성 노드가 아닌 별도 점검.
7. **후처리(선택)** → 검증: 피치·리버브·컴프·EQ 등은 [[audio-post]]로 위임(중복 구현 금지).

## 출력 계약

- `audio/voice-local-NN.wav` — 청크/세그먼트 오디오(동일 엔진·설정·시드)
- `audio/voice-local.wav` — 크로스페이드 병합 최종본
- `audio/synth-manifest.json` — `[{id, engine, model_size, language, seed, device}]`(재현·추적)
- 경계 QC 결과. 상위 [[youtube-video-pipeline]]·[[audio-post]]·콘텐츠 파이프라인으로 반환.

## 경계 (ESCALATE — 자율 실행 금지)

- **기본 음성 라우팅 절체**(오너 상시 콘텐츠의 기본 음성을 ElevenLabs→voice-local로 전환)는
  **오너 결정**이다(사유: 승인 패턴이 **구독제(정액) 전용·종량제 과금 금지**라, 종량 과금되는
  외부 음성 API를 기본 경로로 두는가가 정책 사안이다 — 상태 `pending owner decision`).
  이 스킬은 *역량*만 제공한다 — 기본 전환은 오너 승인 후.
- **한국어 품질 정량 측정**(오너 실제 음성으로 Sohee/Chatterbox 클론 품질)은 후속 PoC(연구보고서
  이니셔티브1). 측정 전 '검증됨' 보고 금지.
- voice-clone-elevenlabs는 폐기하지 않고 **공존**한다(부활조건=로컬 품질 미달 시 폴백, retention gate).
