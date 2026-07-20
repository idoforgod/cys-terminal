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

## 결정론 exit 코드 (CLI)
`0` 성공 · `2` BUSY(context 상한 2 초과) · `3` APPROVAL_REQUIRED(human 프로필) ·
`4` 기동실패 · `5` verify FAIL · `6` HUMAN_ACTIVE · `1` 기타.

## evidence 번들 (4파일 · `--evidence-dir`)
`screenshot.png` → `snapshot.txt` → `dom.html` → **`meta.json`(마지막=완결 마커)**.
`meta.json.dom_sha256` = `sha256(dom.html)` — 리뷰어 독립 재계산으로 위조 대조.
`meta.json` 없는 번들 = 게이트 무효(반쪽 번들 차단).

## 보안 (설계 §3)
- snapshot 최상단 **비신뢰 라벨** 고정 헤더(웹 텍스트=데이터, 지시 아님).
- human 프로필 동사 = `APPROVAL_REQUIRED`(P1은 무조건 거부, feed 배선은 P3).
- 조작권 컨텍스트별 `control=agent|human`. control=human 중 에이전트 변경성 동사 = `HUMAN_ACTIVE`.
- 스냅샷 크기 상한 200KB + 절단 마커. 네이티브 다이얼로그 자동 dismiss + 로그.
- **정직한 한계**: 워커는 셸로 playwright를 직접 실행해 이 정책을 우회할 수 있다(물리 강제 불가).
  방어선 = audit.jsonl 부재 브라우징 흔적 감사 + evidence 규격 위조 비용 상승 + 마스터 실측 재현.
