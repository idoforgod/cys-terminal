# THIRD_PARTY — 외부 유래 스킬 출처 고지

아래 14종 스킬은 [NomaDamas/k-skill](https://github.com/NomaDamas/k-skill) (MIT License)에서
vendoring했다. 업스트림 커밋 핀: `66f12cb43d833e4b9aa4593d430bd5524fff9d58` (2026-06-12).
채택 기준·감사 기록은 cysjavis 선별 감사(2026-06-12, 96종 중 14종 채택 — 실작동·보편성·
오류 절감력 기준) 참조. 갱신 시 업스트림 diff 후 동일 기준으로 재감사한다.

korean-humanizer · korean-spell-check · korean-character-count · naver-blog-research ·
kosis-stats · hwp · rhwp-edit · joseon-sillok-search · geeknews-search · k-dart ·
korean-patent-search · korean-stock-search · daishin-report-search · library-book-search

주의: kosis-stats·korean-stock-search·library-book-search 기본 경로는 운영자 프록시
(`k-skill-proxy.nomadamas.org`) 경유다 — `KSKILL_PROXY_BASE_URL`로 자가 호스팅 전환 가능,
kosis는 BYOK 직접 경로 지원. (korean-law-search·naver-news-search는 미채택 —
법령은 전용 MCP로 대체 예정.)

## MIT License (NomaDamas/k-skill)

MIT License

Copyright (c) 2026

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## obra/superpowers (MIT License, Copyright (c) 2025 Jesse Vincent)

아래 9종은 [obra/superpowers](https://github.com/obra/superpowers)에서 vendoring했다.
업스트림 커밋 핀: `6fd4507659784c351abbd2bc264c7162cfd386dc` (2026-05-29).
채택 기준: 2026-06-12 감사 — A급 5(pack 공백: systematic-debugging·test-driven-development·
subagent-driven-development·dispatching-parallel-agents·verification-before-completion) +
B급 4(보완: brainstorming·receiving-code-review·writing-plans·using-git-worktrees).
패치: `superpowers:` 플러그인 네임스페이스 접두 제거. 저작 부산물(CREATION-LOG·
test-pressure-*·test-academic*)은 제외. hook·부트스트랩 메타 스킬(using-superpowers)은
거버넌스 점유형이라 불채택 — 우리 디렉티브 주입 체계가 대체.

## mattpocock/skills (MIT License, Copyright (c) 2026 Matt Pocock)

아래 9종은 [mattpocock/skills](https://github.com/mattpocock/skills)에서 vendoring했다.
업스트림 커밋 핀: `694fa30311e02c2639942308513555e61ee84a6f` (2026-06-10).
채택 기준: 2026-06-12 감사 — A급 1(git-guardrails-claude-code: denylist의 PreToolUse hook
기계화) + B급 5(grill-with-docs·prototype·improve-codebase-architecture·zoom-out·handoff) +
집필 3부작(writing-fragments·writing-beats·writing-shape — 업스트림 in-progress 단계,
채택·자체 보강 예정). grill-me는 본 pack의 상위판(work 앵커 제작)이 이미
존재해 업스트림판을 덮지 않는다. tdd·diagnose는 superpowers 채택분과 중복이라 불채택.

## 설계 차용 (코드 미벤더링) — Voicebox (MIT License, Copyright (c) 2026 Voicebox Contributors)

`voice-local` 스킬은 cysjavis **원작**이되, 로컬 음성 스택의 설계 DNA와 기법 근거를
[jamiepine/voicebox](https://github.com/jamiepine/voicebox) (MIT License)에서 차용했다.
전수조사: 보고서 `_research/Voicebox_박사급_연구보고서.md`, 메모리 `voicebox-upgrade-research`.

차용한 기법·근거(설계 참조이며 현재 **코드 파일은 벤더링하지 않음**):
엔진→언어/능력 매핑(`backend/backends/__init__.py`·`qwen_custom_voice_backend.py`),
무제한 길이 문장경계 청킹+크로스페이드(`backend/utils/chunked_tts.py`),
TTS 아티팩트 trim·결정론 정제(`backend/utils/audio.py`·`refinement.py`).

★주의: 위 Voicebox 소스 파일을 **직접 이식(코드 복사)**하는 시점에는 그 파일 헤더에 본 MIT
저작권 고지를 동봉해야 한다(MIT 4항). 현재 voice-local은 playbook(설계 차용)이라 벤더링
대상이 아니다 — 코드 이식이 발생하면 이 절을 '벤더링'으로 격상하고 핀 커밋을 기재한다.

## insane-search (MIT License, Copyright (c) 2026 fivetaku)

`insane-search` 스킬은 [fivetaku/insane-search](https://github.com/fivetaku/insane-search)
(gptaku-plugins 일부, MIT License)에서 **디렉터리째 vendoring**했다. 업스트림 커밋 핀:
`49306346b59aa89b5e96d98e1104da0890deed72` (2026, "chore: revert to MIT"). 차단내성 공개페이지
리더 — curl_cffi TLS 임퍼소네이션·yt-dlp(1,858 미디어)·Phase 0 무인증 공개 API·WAF-프로파일
fetch chain·Playwright fallback. API 키 0(HC1 정합)·공개 콘텐츠 한정(인증 우회 아님, DISCLAIMER 승계).
전수조사: 보고서 `_research/InsaneSearch_박사급_연구보고서.md`, 구현설계서
`_research/InsaneSearch기반_cys_업그레이드_구현설계서.md`, 메모리 `insane-search-upgrade-research`.

채택 시 cysjavis 패치(업스트림과의 차이 — 무인 세션·외부발행·프라이버시 부작용 제거):
업스트림 `setup/`(GitHub star write `gh api -X PUT user/starred`·`~/.claude/settings.json` SessionStart
hook 직접 기입·대화기록 언어감지)는 **벤더링하지 않음**(skills/ 디렉터리 외부). SKILL.md Step 0(setup.sh
ask → AskUserQuestion star)도 **제거**. 의존성 무인 자동설치(pip -U / npm i -g)는 기본 OFF·graceful
degrade·CSO 승인 게이트 경유. 갱신 시 업스트림 diff 후 동일 4부작용 제거 기준으로 재감사한다.

DISCLAIMER 승계: 업스트림 `DISCLAIMER.md`(공개 콘텐츠 리더·인증 우회 도구 아님·자격증명 미저장)는
본 고지로 포인터 승계한다. MIT 의무를 넘는 DISCLAIMER 전문 강제임베드 여부는 오너 결정(ESCALATE).

### MIT License (fivetaku/insane-search)

MIT License

Copyright (c) 2026 fivetaku

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## google-research/timesfm (Apache License 2.0, Copyright 2025 Google LLC)

`timesfm-forecasting` 스킬은 cysjavis **원작 래퍼**이되, Google TimesFM 2.5(시계열 기반모델)를
호출하며 그 SKILL.md 패키징·preflight 게이트 설계 DNA를 [google-research/timesfm](https://github.com/google-research/timesfm)
(Apache License 2.0)에서 차용했다. 전수조사: 보고서 `_research/TimesFM_박사급_연구보고서.md`,
구현설계서 `_research/TimesFM기반_cys_업그레이드_구현설계서.md`, 메모리 `timesfm-upgrade-research`.

벤더링 범위: SKILL.md + scripts(timesfm_forecast.py·check_timesfm.py·holdout_eval.py·test_eval_gate.py)는
cysjavis 규약으로 **재작성한 원작**이며 TimesFM 소스 코드를 직접 복사하지 않았다. TimesFM **코드 패키지**는
런타임 의존(`pip install timesfm[torch]`)으로 사용자 환경에 설치되고, **모델 가중치(~800MB)는 PACK에
임베드하지 않고**(include_str! 텍스트 전용) 첫 사용 시 HuggingFace에서 `~/.cache/huggingface`로 1회
다운로드한다. Apache-2.0은 상업·임베드·재배포 친화이며 copyleft가 없어 본 PACK 코드사이닝과 마찰이 없다.

★주의: HuggingFace 모델카드는 *"This open version is not an officially supported Google product"*로
면책하며, TimesFM 사용·정확도 책임은 운영자에게 있다. TimesFM 소스 파일을 **직접 이식**하는 시점에는
그 파일에 Apache-2.0 라이선스·NOTICE 고지를 동봉해야 한다(Apache-2.0 §4). 현재는 런타임 의존이라
벤더링 대상이 아니다.

### Apache License 2.0 (google-research/timesfm)

Copyright 2025 Google LLC

Licensed under the Apache License, Version 2.0 (the "License"); you may not use
this file except in compliance with the License. You may obtain a copy of the
License at http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software distributed
under the License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR
CONDITIONS OF ANY KIND, either express or implied. See the License for the
specific language governing permissions and limitations under the License.
