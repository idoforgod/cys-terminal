# REVIEWER_VERDICT_CONTRACT — 리뷰어 verdict 타입 계약(생성물 · 손편집 금지)

생성 명령: `python3 javis_verdict.py contract`

## §1 점수 금지

어느 깊이든 점수류 키는 금지한다. SCORE_KEY_RE 패턴: `score|grade|rating` (대소문자 무시).
score(0-100) 금지 · 0-1 점수도 금지한다.

## §2 스키마

| 최상위 키 | 필수 여부 | 설명 |
| --- | --- | --- |
| `verdict` | 필수 |  |
| `justification` | 필수 |  |
| `evidence` | 필수 |  |
| `issues` | 필수 |  |
| `missing` | 선택 |  |
| `revision` | 선택 | 대상 커밋 해시(비-git 대상이면 파일 해시·타임스탬프) · REVIEWER_DIRECTIVE §3 리비전 바인딩 |

리뷰어 verdict enum: `ACCEPT | REVISE | BLOCK | ESCALATE`

INVESTIGATE 는 검증기(CHAI R2) 전용이다. 실행가능 fix 없는 BLOCK/REVISE의 강등값이다.

severity enum: `blocking | major | minor`

evidence[] 키: `{claim,ref,verified}` · ref는 file:line 근거를 기록한다.

issues[] 계약 형태: `{severity,where,what,fix}`

드리프트 형태 `{severity,ref,issue}`는 `--lenient-issues`에서만 수용한다. fix가 없으면 CHAI R2가 발화한다.

## §3 빈 JSON 골격

값 힌트를 실제 판정으로 채운다. 선택 키는 사용하지 않으면 생략한다.

```json
{
  "verdict": "ACCEPT|REVISE|BLOCK|ESCALATE",
  "justification": "",
  "evidence": [
    {
      "claim": "",
      "ref": "file:line",
      "verified": false
    }
  ],
  "issues": [
    {
      "severity": "blocking|major|minor",
      "where": "file:line",
      "what": "",
      "fix": ""
    }
  ],
  "missing": [],
  "revision": ""
}
```

## §4 검증

`javis_verdict.py validate <파일>`
