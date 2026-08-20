#!/usr/bin/env python3
"""javis_detect.py — 마스터 선언 감지기 **단일 소유** (T-0147-7 W1b · 재감사 §3 CS-8 · RC8 소멸).

문제(RC8): '마스터 선언인가'라는 의도 분류기가 role-bootstrap.sh 안에서 **순서의존 셸 grep
스택**으로 인라인 구현돼 있었다(구 :46-56). 그 구조 자체가 4개 결함의 공통 원인이었다:

  · **A4(P1)** 억제(의문·인용)를 선언 감지보다 **먼저** 돌려서, 선언과 무관한 위치의 물음표
    하나가 정당한 선언을 삼켰다 — "너는 마스터다. 오늘 뭐부터 할까?"가 미발화(실측 재현).
    셸 grep 은 매치 **스팬**을 모르므로 '선언이 속한 절'이라는 의미 경계를 만들 수 없다.
  · **P3-A-NEGA(P2)** 주어 어휘에 `네가/니가/당신이`가 없어 표준 정서법 선언이 전부 미발화.
  · **G9(P3)** `grep -E '.{0,15}'` 는 로케일이 C면 **바이트**를 세어 한글 filler 창이 1/3로
    축소 — LC_ALL=C 환경에서 선언 미탐.
  · **G25(P3)** `cut -c1-200` 은 GNU 에서 **바이트** 단위라 Windows/C 로케일에서 감지창이
    한글 약 66자로 축소.

해법(CS-8): 감지+억제 전체를 **이 파일의 단일 함수**(`detect`)로 이관한다. 파이썬 `re` 는
스팬을 주므로 순서를 역전(선언 먼저 → 그 선언이 **속한 절** 내부의 억제 마커만 평가)할 수
있고, 문자열 슬라이스가 문자 단위이며 로케일에 의존하지 않는다. 셸측 슬라이스(`cut -c`·
bashism `${x:0:200}`)는 훅에서 **제거**된다 — 그게 G25 의 소멸 방식이다.

★거리 튜닝 금지: 억제의 1차 스코프는 "선언 매치와 마커 사이 N자"가 아니라 **절 경계**다.
  임의 마커에 문자거리 N 을 씌우는 처방은 재감사가 명시적으로 기각했다(A4 재검증: "문자거리
  N보다 '선언 감지 후 절 경계 스코프 억제'"). ※W-A2 의 QUOTATIVE 인접창은 이 금지의 예외가
  아니라 다른 층위다 — 임의 마커의 거리 상수가 아니라 '인용 전달 조사는 피인용문에 형태적으로
  부착된다'는 문법 사실의 형식화이며, 절 스코프를 대체하지 않고 그 **안에서** 위치·형태를
  더 좁힌다(어휘·간격 근거는 QUOTATIVE 상수 주석).

수치 스펙(§3 CS-8④ — 상수화·경계 corpus 박제 대상):
  WINDOW_CHARS=200(감지창 · **문자**) · FILLER_MAX=15(주어↔마스터 사이 filler)
  TERM_GAP_MAX=2(마스터↔종결어미 간격) · NEG_GAP_MAX=3(부정 인접 간격)

사용:
  · 라이브러리(권장 — corpus 회귀는 이 함수를 직접 때린다):
        import javis_detect; v = javis_detect.detect(prompt); v["fire"]
  · 훅 게이트(1왕복 · stdin=UserPromptSubmit hook JSON):
        printf '%s' "$INPUT" | python3 javis_detect.py hook-gate
        exit 0=FIRE / 1=judged-no(선언 없음·침묵) / 3=선언 검출 후 **억제**(stderr 1줄 로그)
        / 2=cannot-judge(입력 파싱 불가 — 호출측이 loud 분기)
    stdout 은 항상 1줄 verdict JSON 이다(훅은 `$(...)`로 회수 — 모델 컨텍스트로 새지 않는다).
  · `--self-test`: 밀폐 corpus 배터리(assert) — preflight/CI 관례.

★exit 1 과 3 의 분리가 계약의 핵심이다: '판정했고 아니다'(침묵 정당)와 '선언이었는데 억제'
  (로그 1줄 의무 — CS-8⑤ "억제 시 침묵 대신 로그 1줄")를 융합하면 왜 안 떴는지 영영 모른다.

─── W-A2 (2026-08-21) 억제 3축 정밀화 + corpus 단일 원본화 ─────────────────────────
문제(실측): 한국어 채팅체는 문장부호 생략이 기본값인데, 종전 억제는 '선언이 속한 절 안에
QUESTION 마커가 **어디든** 있으면 억제'였다. 절 경계(마침표)를 안 찍은 후속 질문의 '?'·'무엇'·
'의미'가 선언 자신의 억제 마커로 평가돼 정당한 선언이 통째로 삼켜졌다(무발화의 최빈 원인):
    "너는 마스터다 오늘 뭐부터 할까?"  → 종전 suppressed(마커 '?')  — 마침표만 찍으면 FIRE
    "너는 마스터다 무엇부터 시작할까"   → 종전 suppressed(마커 '무엇')
그렇다고 억제를 통째로 완화하면 오너의 실제 메타 요청 2형이 오발화한다(실측 — 여기서 발화하면
machine-origin 판정이 human 으로 꺾여 팀이 스폰된다 = 폭주 앵커):
    "너는 마스터다라고 말하지 마" / "너는 마스터다 처럼 들리는 문장을 만들어줘"

해법: '마커가 절 안에 있는가'(위치 무관 1축)를 **위치·형태 기반 3축**으로 쪼갠다. 억제는
아래 어느 한 축이라도 성립할 때만이다(+기존 부정 축은 그대로):
  ⓐ pre        — 절 내부에서 마커가 선언 매치보다 **앞**("무슨 뜻인지 모르겠어 너는 마스터다 …").
                 어휘는 QUESTION 그대로(mission 과 공유 — 아래 참조). 위치만 좁혔다.
  ⓑ quote      — 인용부호가 선언을 **감싼다**(왼쪽·오른쪽 모두, 절 안에서): "'너는 마스터다'가
                 무슨 뜻?" — 언급(mention)이지 선언(use)이 아니다. 한쪽만 있으면 감쌈이 아니다
                 ("너는 마스터다 '알파' 작업부터 시작해" 는 발화).
  ⓒ quotative  — 선언 종료 **직후 좁은 간격**(QUOTATIVE_GAP_MAX) 안의 인용 전달 조사
                 (라고·라는·라며·라면서·라니·이라고·…·면서·처럼)와 조사 부착 메타 의문('가 뭐/무'),
                 그리고 인접 에코 의문('너는 마스터다?'). 한국어에서 인용 전달 조사는 피인용문에
                 **형태적으로 부착**되므로(다라고·다"라고·다 처럼) 좁은 인접창이 곧 문법 경계다.
  이 분해로 '선언 뒤 멀리 있는 일반 의문어'만 정확히 풀리고(문제1), 메타 요청 2형은 ⓒ가
  계속 잡는다(문제2). 진단(exit 3 stderr)은 어느 축·어느 마커였는지를 남긴다.

★'절 경계에서 ? 취급' 재설계: '?' 는 **절 경계 역할은 유지**하되(CLAUSE_BOUNDARY 불변 —
  javis_mission.split_clauses 가 이 상수를 그대로 소비한다: 값 변경은 mission 과 같은 원자
  커밋이어야 하므로 이 파일 단독으로 바꾸지 않는다), **억제 마커 역할은 ⓒ의 인접창으로 좁혔다**.
  절 꼬리의 '?' 는 후속 질문의 것이지 선언의 것이 아니다 — 선언의 '?'(에코 의문)는 반드시
  선언 직후에 붙는다. 이렇게 하면 mission 의 절 분해·질의절 분류('?' 포함 규약)는 그대로다.

★mission 공유 어휘 계약: CLAUSE_BOUNDARY·QUESTION·DECL_KO/EN 은 javis_mission 이 속성으로
  소비한다(split_clauses·extract_mission — "어휘 단일 출처, 사본 금지"). 이 세 심볼은 이름·값
  모두 이 티켓에서 불변이고, 감지기는 QUESTION 의 **적용 위치**만 좁혔다(ⓐ pre 전용).
  ⓑⓒ 의 어휘(QUOTES·QUOTATIVE)는 감지기 전용 신설 상수라 mission 에 영향이 없다.

★corpus 단일 원본화: FIRE/SKIP corpus 는 `tests/fixtures/detect-corpus.json` 이 단일 원본이다
  (self-test 가 있으면 읽고, 없으면 이 파일의 내장 리터럴로 폴백 — 배포 팩이 tests/ 없이 깔린
  구 스큐에서도 self-test 가 죽지 않게). fixture 가 **있는데 불량**이면 폴백하지 않고 hard
  fail 한다 — 측정 실패를 통과로 접는 것이 금지이기 때문이다. fixture 는 내장 리터럴의
  상위집합이어야 한다(드리프트 검출 — 리터럴만 늘고 fixture 가 안 늘면 FAIL).
"""
import json
import os
import re
import sys

# ★로케일 비의존 I/O(G9 · 선례 javis_bootstrap.py R3/D-IMPL-3): LC_ALL=C·Windows cp949 파이프에서
#   한글 출력이 UnicodeEncodeError 로 크래시하면 '판정 불가'가 아니라 **훅 전체가 깨진다**. 입력은
#   아래 `hook-gate` 가 **바이트로 읽어 UTF-8 명시 디코드**하므로 stdin 인코딩 추론에도 의존하지 않는다.
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

# ── 수치 스펙(단일 진실원천 — 문서·주석·훅이 이 상수를 가리킨다) ──────────────
WINDOW_CHARS = 200   # 감지창: 프롬프트 앞 200 **문자**(긴 문서 본문 오발화 억제)
FILLER_MAX = 15      # 주어↔'마스터' 사이 허용 filler 문자수("너는 **지금부터** 마스터다")
TERM_GAP_MAX = 2     # '마스터'↔종결어미 사이 허용 간격("마스터**가 **되")
NEG_GAP_MAX = 3      # 부정 인접 억제의 비-한글 간격 허용치

# 절(문장) 경계 문자 — 이 문자에서 절이 끝난다(경계 문자는 **앞 절에 포함**한다).
# ★불변 계약(W-A2): 이 상수는 javis_mission.split_clauses 가 **그대로** 소비한다(사본 금지
#   규약). 값을 바꾸면 임무 본문 추출의 절 분해가 함께 바뀌므로, 변경은 mission 소비 배선과
#   같은 원자 커밋이어야 한다 — 이 파일 단독 티켓에서는 손대지 않는다.
# ★'?' 를 여기 두는 이유(W-A2 재설계 이후): 감지기에게 '?' 경계는 이제 **격리막**이다 —
#   "무슨 뜻이야? 너는 마스터다" 에서 앞 절의 의문이 선언 절로 새는 것을 막는다. 절 꼬리에
#   포함된 '?' 를 억제 마커로 쓰던 종전 역할은 폐기됐고(원거리 후속 질문의 '?' 가 선언을
#   삼키던 결함), 선언 자신의 의문('너는 마스터다?')은 QUOTATIVE 인접창이 잡는다.
#   mission 쪽에서는 앞-절-포함 규약이 여전히 실질이다("오늘 뭐부터 할까?" 절이 QUESTION 의
#   `\?` 로 질의절로 분류되려면 '?' 가 절 안에 남아 있어야 한다).
CLAUSE_BOUNDARY = ".!?;…。！？\n\r"

# ── 어휘 ─────────────────────────────────────────────────────────────────────
# 주어(P3-A-NEGA): 표준 정서법 `네가`·구어 `니가`·`당신이`를 추가한다.
# ★과확장 금지: `네`·`당신` **단독**은 넣지 않는다("네 마스터 브랜치"·"당신 마스터키" 오발화).
#   교착사(는/가/이)까지 포함한 형태만 주어로 인정한다. 단독 `너`는 구 계약 보존(맨 뒤 —
#   정규식 대안은 앞에서부터 시도되므로 긴 형태가 먼저 매치된다).
SUBJECT = r"(?:너는|넌|너가|네가|니가|당신은|당신이|너)"
MASTER = r"(?:마스터|master)"
# 종결: 서술격·명령형·'로 각성/승격'·'가 되/돼/된'
TERM = r"(?:다|야|이다|입니다|임|이야|여|로 *각성|로 *승격|가 *되|가 *돼|가 *된)"

DECL_KO = re.compile(SUBJECT + r".{0,%d}" % FILLER_MAX + MASTER
                     + r".{0,%d}" % TERM_GAP_MAX + TERM, re.IGNORECASE)
DECL_EN = re.compile(r"you\s+are\s+(?:the\s+|our\s+|now\s+)*master", re.IGNORECASE)

# 부정 인접 억제(adv#7): 선언 자리 자체가 부정인 경우 — "너는 마스터가 아니다/말고".
NEG = re.compile(MASTER + r"[^가-힣A-Za-z]{0,%d}(?:가|는|를)?[^가-힣A-Za-z]{0,%d}"
                 r"(?:아니|아냐|말고)" % (NEG_GAP_MAX, NEG_GAP_MAX), re.IGNORECASE)

# 의문·인용 어휘(adv#8 유래 · 구 셸 grep 과 동일 — 어휘 확장은 별 결함).
# ★공유 계약(W-A2): javis_mission.extract_mission 이 이 컴파일 정규식을 속성으로 소비해
#   질의·인용 **절**을 임무 본문에서 제외한다("오늘 뭐부터 할까?" 는 보고 요구지 임무가 아니다).
#   그래서 이름·어휘 모두 불변이다. 감지기 자신은 이제 이것을 **선언보다 앞**(축ⓐ pre)에만
#   적용한다 — 선언 뒤 원거리 일반 의문어('무엇'·'의미'·절 꼬리 '?')로 선언을 삼키던 결함의
#   소멸 방식이 '어휘 수정'이 아니라 '적용 위치 축소'인 이유가 이 공유 계약이다.
#   (pre 위치에서 `\?` 는 구조적으로 죽은 어휘다 — '?' 는 절 경계라 선언 앞 같은 절에 있을 수
#   없다. mission 쪽 절 분류에서는 살아 있으므로 제거하지 않는다.)
QUESTION = re.compile(r"(?:무슨|무엇|뜻|의미|가 뭐|가 무|\?|라고 (?:말|하지|입력)|처럼|예시|예를)")

# ── W-A2 신설: 감지기 전용 억제 축 어휘(ⓑ·ⓒ — mission 은 소비하지 않는다) ──────────
# 축ⓒ 인접창: 선언 종료점과 인용 전달 조사 사이에 허용되는 비문자(따옴표·공백·문장부호) 수.
# ★축ⓐ의 '절 스코프'와 별개로 ⓒ에만 좁은 간격을 쓰는 근거: 한국어 인용 전달 조사는 피인용문에
#   **형태적으로 부착**된다(다라고 · 다"라고 · 다 처럼) — 간격이 크면 그것은 조사가 아니라 다음
#   문장의 어휘다("너는 마스터다 오늘 라디오 켜줘" 의 '라'는 조사가 아니다). 재감사가 기각한
#   '거리 튜닝'은 임의 마커에 문자거리 N 을 씌우는 일반 처방이었고(절 스코프의 대체물), 이것은
#   조사 부착이라는 문법 사실의 형식화다(절 스코프를 대체하지 않고 보완한다 — W-A2 티켓 승인).
QUOTATIVE_GAP_MAX = 2   # 닫는 따옴표+공백("다' 라고")까지 흡수 — 3+ 는 부착으로 보지 않는다
# 간격에 허용되는 문자 = 한글 음절·영숫자를 **제외한** 전부(따옴표·공백·문장부호). 한글이 끼면
# 그건 새 어절이지 조사 부착이 아니다 — "마스터다 오라고 했잖아" 의 '라고'(간격 ' 오')를 조사로
# 오인해 정당 선언을 삼키는 것을 이 배제가 막는다.
# 어휘 선정(실측 → 채택/기각 사유 명기):
#   · 이?라고(?= {0,2}[가-힣]): 인용 조사 본대("다라고 말하지 마"·"임이라고 적지 마").
#     ★뒤에 한글 어절이 와야만 억제다 — "너는 마스터다라고!"(문미·문장부호)는 좌절한 오너의
#     **강조 재선언**이지 인용이 아니고, 구 코드도 발화했다(구 어휘는 '라고 (말|하지|입력)'
#     복합형이라 문미 '라고'를 안 잡았다). 동사 열거 대신 '한글 후속' 조건으로 일반화한
#     이유: 인용 동사는 열거 불가능하게 많고(적다·쓰다·보내다·치다…), 재선언과의 구분선은
#     동사 종류가 아니라 '뒤에 전달 구문이 이어지는가'다.
#   · 이?라(면서|는|며): 언급·전문(傳聞) 인용("다라는 문장"·"다라며 웃었다"·"다라면서 왜") —
#     재선언 독법이 없는 형태라 무조건 억제. 구 코드는 이 형태들을 못 잡아 오발화했다(개선).
#   · 이?라니(?= {1,2}[가-힣]): 반문 인용("다라니 무슨 소리야" — 구 코드도 '무슨'으로 억제).
#     ★공백 **필수**: 부착형 '라니까'("다라니까 믿어봐")는 강조 재선언이라 발화가 정답이고,
#     "라니냐(La Niña) 보고서" 류 선두 어절 충돌도 공백+한글 조건이 함께 막는다.
#   · 면서: 전문 반문("마스터다면서 왜 직접 안 해") — '면서' 두 음절 요구라 '며칠' 무해.
#   · 처럼: 비교 인용("다 처럼 들리는") — '처럼'은 비교 외 용법이 없다.
#   · 가 ?뭐|가 ?무: 조사 부착 메타 의문("마스터다가 무슨 뜻") — 인용부호 없는 언급의 최빈형.
#   · [?？]: 인접 에코 의문("너는 마스터다?") — 종전 억제 동작 보존(원거리 '?' 만 풀었다).
#   · 기각 '같은': "같은 팀/방식/시간"의 '동일' 용법이 흔해 정당 선언("너는 마스터다 같은 팀
#     워커를 지휘해")을 삼킨다. 종전 코드도 '같은'을 억제하지 않았다 — 무변경이 보수다.
#   · 기각 '며'·'란' 단독: 한 음절이라 '며칠'·'란색' 류 선두 어절과 충돌(간격이 비문자만
#     허용해도 "다 며칠 안에"가 걸린다).
QUOTATIVE = re.compile(
    r"[^0-9A-Za-z가-힣]{0,%d}"
    r"(이?라고(?= {0,2}[가-힣])|이?라(?:면서|는|며)|이?라니(?= {1,2}[가-힣])"
    r"|면서|처럼|가 ?뭐|가 ?무|[?？])" % QUOTATIVE_GAP_MAX)
# 축ⓑ 인용부호 집합 — 절 안에서 선언의 왼쪽·오른쪽 **양쪽**에 하나씩 있어야 '감쌈'이다.
# 쌍 검증(여는/닫는 짝 맞춤)은 하지 않는다: 채팅 입력은 짝이 어긋나기 일쑤고, '양쪽 존재'만으로
# 언급(mention) 판정에 충분하다(한쪽만 있는 케이스는 과제명 인용 등 정상 선언 — corpus 박제).
QUOTES = "'\"`‘’“”「」『』"   # 직선·둥근 따옴표 + 백틱(개발 채팅) + 낫표. 《〈 류 서명 괄호는
                              # 문장 인용 관례가 아니라서 제외(감쌈 오탐 표면 최소화).


def _flatten(text):
    """개행·탭 → 공백 **1:1 치환**(길이·오프셋 보존).

    구 훅은 `tr '\\n' ' '` 로 개행을 없앤 뒤 grep 했다. 여기서도 매칭은 평탄화 텍스트에서
    하되(선언이 줄바꿈을 걸쳐도 감지 — 구 동작 보존), **절 경계 계산은 원문**에서 한다
    (개행은 절 경계다). 두 문자열의 인덱스가 1:1로 정렬돼야 하므로 길이를 바꾸지 않는다.
    """
    return text.replace("\r", " ").replace("\n", " ").replace("\t", " ")


def _clause_bounds(raw, start, end):
    """`raw` 에서 [start,end) 매치를 **완전히 포함하는 절**의 (lo, hi) 반환.

    lo = 매치 시작 왼쪽의 가장 가까운 경계 문자 다음 / hi = 매치 끝 오른쪽의 가장 가까운
    경계 문자 **포함**(없으면 문자열 끝). 매치 전체를 감싸는 이유: filler 안에 마침표가
    들어간 병리 입력("너는. 마스터다'가 무슨 뜻?")에서 절이 매치를 갈라 억제 마커를
    놓치는 것을 막는다(fail-closed 방향).
    """
    lo = 0
    for i in range(start - 1, -1, -1):
        if raw[i] in CLAUSE_BOUNDARY:
            lo = i + 1
            break
    hi = len(raw)
    for i in range(end, len(raw)):
        if raw[i] in CLAUSE_BOUNDARY:
            hi = i + 1
            break
    return lo, hi


def _matches(flat):
    """선언 후보 전량(한국어·영문)을 시작 오프셋 순으로 반환."""
    found = [(m.start(), m.end(), m.group(0), "ko") for m in DECL_KO.finditer(flat)]
    found += [(m.start(), m.end(), m.group(0), "en") for m in DECL_EN.finditer(flat)]
    found.sort(key=lambda t: (t[0], t[1]))
    return found


def _suppression(flat, lo, hi, start, end):
    """한 선언 후보의 억제 판정(W-A2 3축 + 부정). 반환 (axis, marker, reason) 또는 None.

    축 평가 순서 = neg → ⓐpre → ⓑquote → ⓒquotative. 순서는 진단 품질용이다(가장 특정적인
    사유를 먼저) — 어느 축이든 하나면 억제이므로 순서가 판정 결과를 바꾸지는 않는다.
    좌표계: flat 과 raw 는 _flatten 이 1:1 길이 보존이라 인덱스가 정렬돼 있다. 절 슬라이스는
    전부 flat 기준([lo:start]=선언 앞, [end:hi]=선언 뒤 절 꼬리).
    """
    clause = flat[lo:hi]
    # 부정 축(adv#7 · 종전 그대로 절 전체 스코프): "마스터 아니다/말고"는 위치 불문 선언 부정이다.
    neg = NEG.search(clause)
    if neg:
        return ("neg", neg.group(0),
                "선언 인접 부정 — 절 내 부정 표현 %r" % neg.group(0))
    pre = flat[lo:start]
    # ⓐ pre — 선언보다 **앞**의 의문·인용 마커만 억제한다. 어휘는 QUESTION 그대로(mission 공유).
    #   종전엔 절 전체를 뒤져 선언 **뒤** 후속 질문의 어휘('무엇'·'의미'·절 꼬리 '?')까지 억제
    #   사유가 됐다 — 무구두점 채팅체 무발화의 최빈 원인(W-A2 문제1). 뒤쪽은 ⓑⓒ가 형태 근거
    #   있는 것만 잡는다.
    q = QUESTION.search(pre)
    if q:
        return ("pre", q.group(0),
                "억제(축ⓐ 선행 마커): 절 안, 선언 앞의 의문·인용 마커 %r — 선언이 언급 대상"
                % q.group(0))
    # ⓑ quote — 인용부호가 절 안에서 선언 양쪽을 감싼다(mention). 한쪽만은 감쌈이 아니다.
    post_clause = flat[end:hi]
    lq = next((c for c in pre if c in QUOTES), None)
    rq = next((c for c in post_clause if c in QUOTES), None)
    if lq and rq:
        marker = "%s…%s" % (lq, rq)
        return ("quote", marker,
                "억제(축ⓑ 인용 감쌈): 선언이 인용부호 %s 안에 있음 — 언급이지 선언 아님" % marker)
    # ⓒ quotative — 선언 종료 직후 인접창의 인용 전달 조사·조사 부착 메타 의문·에코 '?'.
    #   ★탐색은 flat[end:] 기준이라 절 경계를 **의도적으로 넘는다**: 피인용문이 자기 문장부호를
    #   갖는 형태("너는 마스터다."라고 말해줘)에서 조사는 경계 문자 너머에 붙는다 — 인접창이
    #   2자라 다음 절의 본문 어휘를 집을 수는 없다(경계+공백까지가 상한).
    m = QUOTATIVE.match(flat, end)
    if m:
        return ("quotative", m.group(1),
                "억제(축ⓒ 인용 전달 조사): 선언 종료 직후(간격≤%d) %r — 선언의 인용·전달(메타 요청)"
                % (QUOTATIVE_GAP_MAX, m.group(1)))
    return None


def detect(prompt, window_chars=WINDOW_CHARS):
    """마스터 선언 판정(순수 함수 · 로케일 비의존 · 부작용 0).

    반환 dict:
      fire(bool) · reason(str) · verdict('fire'|'no_declaration'|'suppressed')
      matched(선언 매치 문자열 또는 None) · marker(억제 마커 또는 None)
      axis(억제 축 'neg'|'pre'|'quote'|'quotative' 또는 None — W-A2 진단 계약)
      clause(억제 판정에 쓰인 절) · span([start,end] 또는 None)
      window_chars · filler_max · term_gap_max · quotative_gap_max · truncated(창 절단 여부)

    ★후보 여러 개면 **하나라도 억제되지 않은 선언**이 있으면 FIRE 한다(단조성):
      "'너는 마스터다'가 무슨 뜻? 아무튼 너는 마스터다." 는 발화가 정답이다.
    """
    text = prompt if isinstance(prompt, str) else ""
    raw = text[:window_chars]               # ★문자 단위 슬라이스(G25) — 바이트 아님
    flat = _flatten(raw)
    spec = {"window_chars": window_chars, "filler_max": FILLER_MAX,
            "term_gap_max": TERM_GAP_MAX, "quotative_gap_max": QUOTATIVE_GAP_MAX,
            "truncated": len(text) > len(raw)}
    cands = _matches(flat)
    if not cands:
        return dict(spec, fire=False, verdict="no_declaration",
                    reason="감지창(%d자) 내 마스터 선언 없음" % window_chars,
                    matched=None, marker=None, axis=None, clause=None, span=None)
    last = None
    for start, end, matched, lang in cands:
        lo, hi = _clause_bounds(raw, start, end)
        supp = _suppression(flat, lo, hi, start, end)
        if supp:
            axis, marker, reason = supp
            last = dict(spec, fire=False, verdict="suppressed", reason=reason,
                        matched=matched, marker=marker, axis=axis,
                        clause=flat[lo:hi], span=[start, end])
            continue
        return dict(spec, fire=True, verdict="fire",
                    reason="마스터 선언 확정(%s · 억제 3축(pre/quote/quotative)·부정 모두 비성립)"
                           % lang,
                    matched=matched, marker=None, axis=None, clause=flat[lo:hi],
                    span=[start, end])
    return last


# ══════════════════════════════════════════════════════════════════════════════
# CLI — 훅 게이트(1왕복)
# ══════════════════════════════════════════════════════════════════════════════
EXIT_FIRE = 0
EXIT_NO_DECL = 1        # judged-no — 침묵이 정당(로그 없음)
EXIT_CANNOT_JUDGE = 2   # 입력 파싱 불가 — 호출측 loud 분기(CS-2⑨)
EXIT_SUPPRESSED = 3     # 선언 검출 후 억제 — **stderr 1줄 로그 의무**(CS-8⑤)


def cmd_hook_gate(argv):
    """stdin(UserPromptSubmit hook JSON) → verdict. 프롬프트 추출도 여기서 한다(왕복 1회)."""
    try:
        # ★바이트로 읽고 UTF-8 명시 디코드: LC_ALL=C 환경에서 sys.stdin 이 ASCII 로 추론돼
        #   한글 프롬프트가 UnicodeDecodeError 로 죽는 것을 원천 차단한다(G9 로케일 비의존).
        raw = sys.stdin.buffer.read() if hasattr(sys.stdin, "buffer") else sys.stdin.read()
        payload = raw.decode("utf-8", "replace") if isinstance(raw, bytes) else raw
    except Exception as e:                      # stdin 소실 — 판정 불가
        sys.stderr.write("[detect] cannot-judge: stdin 읽기 실패(%s)\n" % e)
        print(json.dumps({"fire": False, "verdict": "cannot_judge",
                          "reason": "stdin 읽기 실패"}, ensure_ascii=False))
        return EXIT_CANNOT_JUDGE
    if not (payload or "").strip():
        print(json.dumps({"fire": False, "verdict": "no_declaration",
                          "reason": "빈 stdin"}, ensure_ascii=False))
        return EXIT_NO_DECL
    try:
        obj = json.loads(payload)
        prompt = obj.get("prompt", "") if isinstance(obj, dict) else ""
    except Exception as e:
        sys.stderr.write("[detect] cannot-judge: hook JSON 파싱 실패(%s) — 선언 감지 불가\n" % e)
        print(json.dumps({"fire": False, "verdict": "cannot_judge",
                          "reason": "hook JSON 파싱 실패: %s" % e}, ensure_ascii=False))
        return EXIT_CANNOT_JUDGE
    if not isinstance(prompt, str) or not prompt:
        print(json.dumps({"fire": False, "verdict": "no_declaration",
                          "reason": "prompt 필드 부재/빈값"}, ensure_ascii=False))
        return EXIT_NO_DECL
    v = detect(prompt)
    print(json.dumps(v, ensure_ascii=False))
    if v["fire"]:
        return EXIT_FIRE
    if v["verdict"] == "suppressed":
        # ★CS-8⑤: 억제는 침묵하지 않는다 — 왜 안 떴는지 1줄로 남긴다.
        #   W-A2 강화: 어느 **축**(neg/pre/quote/quotative)·어느 마커였는지까지 특정한다 —
        #   억제 오판을 신고받았을 때 축이 없으면 4개 규칙 중 무엇을 고칠지 역추적이 안 된다.
        sys.stderr.write("[detect] SKIP(억제): %s | 선언=%r 축=%s 마커=%r\n"
                         % (v["reason"], v["matched"], v.get("axis"), v["marker"]))
        return EXIT_SUPPRESSED
    return EXIT_NO_DECL


# ── corpus 배터리(밀폐 self-test) ────────────────────────────────────────────
# ★producer≠evaluator 경계: 이 배터리는 **정규식 작성자가 역산한 케이스가 아니라** 재감사가
#   실측으로 확정한 결함 재현 케이스(A4·P3-A-NEGA·P3-A-FILLER·G9·G25)와 기존 회귀 corpus
#   (test_role_bootstrap_hook.py 원본 15건), 그리고 W-A2 가 실측으로 확정한 무구두점 채팅체
#   무발화 케이스를 옮긴 것이다. 확장 corpus 는 오너 실발화 채집으로 늘린다(§3 CS-8⑤).
#
# ★단일 원본(W-A2): corpus 의 정본은 `tests/fixtures/detect-corpus.json` 이다. 아래 리터럴은
#   fixture 가 **없는** 배포 팩(tests/ 미동봉 구 스큐)에서 self-test 가 죽지 않기 위한 폴백
#   최소셋이며, fixture 는 항상 이 리터럴의 상위집합이어야 한다(_load_corpus 가 검사 — 리터럴에
#   케이스를 추가하면 fixture 에도 넣어야 self-test 가 통과한다). 새 케이스는 fixture 에 먼저
#   넣고, '엔진 결함 재현급'만 여기 승격한다(리터럴은 엔진과 같은 파일이라 스큐가 불가능하다).
FIRE_CORPUS = [
    # ── 기존 corpus(구 훅 시대부터 통과해야 했던 것 — 전량 보존) ──
    "너는 마스터다", "너는 이제 마스터다", "너는 지금부터 마스터다", "너가 마스터야",
    "당신은 우리의 마스터입니다", "너는 마스터로 각성하라", "you are the master",
    "지금부터 너는 마스터가 된다",
    # ── A4: 혼합 의도(선언 + 후속 질문) — 절 경계 스코프의 핵심 케이스 ──
    "너는 마스터다. 오늘 뭐부터 할까?",
    "너는 이제 마스터다! 무슨 일부터 시작할까?",
    "너는 마스터다.\n오늘 작업 목록을 뭐로 잡을까?",
    # ── P3-A-NEGA: 표준 정서법·구어 주어 ──
    "네가 마스터다", "니가 마스터다", "당신이 마스터다", "네가 이제 마스터야",
    # ── 억제 마커가 **다른 절**에 있으면 발화 ──
    "'설계 문서'를 예시로 보여줘. 그리고 너는 마스터다.",
    # ── W-A2 문제1: 무구두점 채팅체 — 선언 뒤 후속 질문·계획이 절 경계 없이 붙어도 발화.
    #    (종전 실측 suppressed: '?'·'무엇'·'의미'가 절 꼬리에서 억제 마커로 오평가됐다) ──
    "너는 마스터다 오늘 뭐부터 할까?", "너는 마스터다 준비됐지?",
    "너는 마스터다 무엇부터 시작할까", "너는 마스터다 의미있게 시작하자",
]
SKIP_CORPUS = [
    # ── 기존 corpus ──
    "너는 마스터가 아니다", "'너는 마스터다'가 무슨 뜻이야?", "너는 마스터다라고 말하지 마",
    "오늘 작업 지시해줘", "너는 워커다", "마스터 브랜치를 확인해줘", "너는 오늘 마스터 브랜치 봐",
    # ── A4 반대편: 선언을 감싼 인용·선언에 부착된 인용 전달 조사는 여전히 억제 ──
    "'네가 마스터다'가 무슨 뜻?", "\"당신이 마스터다\"라고 입력하면 어떻게 되나요?",
    "너는 마스터다 처럼 들리는 문장을 만들어줘",
    # ── P3-A-NEGA 과확장 금지: 주어 아닌 `네`·`당신` 단독 ──
    "네 마스터 브랜치를 봐줘", "당신 마스터키 어디 뒀어",
    # ── W-A2 축ⓒ 추가 핀: 인용 전달 조사 변형 + 인접 에코 의문(원거리 '?' 만 풀렸음을 증명) ──
    "너는 마스터다라는 문장을 분석해줘", "너는 마스터다?",
]

# 축 진단 파리티 핀(W-A2 요구5): 억제 사유가 '어느 축·어느 마커'로 특정되는지 자체를 계약으로
# 박제한다 — 이 대응이 무너지면 exit 3 stderr 진단이 거짓말을 한다(억제 오판 신고를 역추적 불능).
AXIS_PINS = (
    ("너는 마스터다라고 말하지 마",            "quotative", "라고"),
    ("너는 마스터다 처럼 들리는 문장을 만들어줘", "quotative", "처럼"),
    ("너는 마스터다?",                        "quotative", "?"),
    ("'너는 마스터다'가 무슨 뜻이야?",          "quote",     "'"),
    ("무슨 뜻인지 모르겠어 너는 마스터다 이 문장", "pre",       "무슨"),
)


def _corpus_fixture_path():
    """corpus 단일 원본의 위치 — 팩 상대 고정 경로(레인 이동에도 함께 움직인다)."""
    return os.path.join(os.path.dirname(os.path.abspath(__file__)),
                        "tests", "fixtures", "detect-corpus.json")


def _load_corpus():
    """corpus 적재. 반환 (source_label, fire_items, skip_items, err).

    스키마: {"fire":[{"text","why"}], "skip":[{"text","why","marker"}]} — marker 는 억제를
    일으켜야 하는 마커 부분문자열(무선언 케이스는 null). why 는 사람용 근거(판정엔 불참).

    ★분기 계약(W-A2):
      · fixture 부재 → 내장 리터럴 폴백(구 팩 스큐 안전 — tests/ 없이 깔린 팩에서도 self-test
        는 의미 있게 돈다. 라벨로 폴백 사실을 드러낸다).
      · fixture 존재 + 불량(판독 불가·스키마 위반·내장 리터럴 미포함) → **hard fail**(err 반환).
        조용한 폴백은 corpus 부식을 통과로 접는다 — '측정 불능은 통과가 아니다' 원칙.
    """
    path = _corpus_fixture_path()
    if not os.path.isfile(path):
        fire = [{"text": t, "why": "내장 폴백", "marker": None} for t in FIRE_CORPUS]
        skip = [{"text": t, "why": "내장 폴백", "marker": None} for t in SKIP_CORPUS]
        return ("내장 리터럴(fixture 부재 폴백)", fire, skip, None)
    try:
        with open(path, "rb") as f:
            data = json.loads(f.read().decode("utf-8"))
    except Exception as e:
        return (None, None, None, "fixture 판독 불가 %s: %s" % (path, e))
    if (not isinstance(data, dict) or not isinstance(data.get("fire"), list)
            or not isinstance(data.get("skip"), list)):
        return (None, None, None,
                "fixture 스키마 위반 %s: 최상위가 {\"fire\":[…],\"skip\":[…]} 이 아니다" % path)
    fire, skip = [], []
    for kind, out in (("fire", fire), ("skip", skip)):
        for i, it in enumerate(data[kind]):
            if not isinstance(it, dict) or not isinstance(it.get("text"), str) or not it["text"]:
                return (None, None, None,
                        "fixture 스키마 위반 %s: %s[%d] 는 비어있지 않은 text 를 가진 객체여야 한다"
                        % (path, kind, i))
            marker = it.get("marker")
            if marker is not None and not isinstance(marker, str):
                return (None, None, None,
                        "fixture 스키마 위반 %s: %s[%d].marker 는 문자열 또는 null" % (path, kind, i))
            out.append({"text": it["text"], "why": it.get("why") or "", "marker": marker})
    # 단일 원본 불변식: fixture ⊇ 내장 리터럴. 리터럴에만 추가하고 fixture 를 안 늘리는
    # 드리프트(사본 분화의 재발 경로)를 여기서 잡는다.
    for name, lits, items in (("fire", FIRE_CORPUS, fire), ("skip", SKIP_CORPUS, skip)):
        have = set(x["text"] for x in items)
        missing = [t for t in lits if t not in have]
        if missing:
            return (None, None, None,
                    "fixture 가 내장 %s corpus 를 포함하지 않는다(단일 원본 위반): %r"
                    % (name, missing))
    return ("fixture %s" % path, fire, skip, None)


def cmd_self_test():
    source, fire_items, skip_items, err = _load_corpus()
    if err:
        # fixture 가 존재하는데 불량 — 폴백하지 않는다(측정 실패는 hard fail).
        print("javis_detect self-test FAIL: corpus %s" % err, file=sys.stderr)
        return 1
    fails = []
    for it in fire_items:
        v = detect(it["text"])
        if not v["fire"]:
            fails.append("FALSE-NEGATIVE %r → %s%s"
                         % (it["text"], v["reason"],
                            (" [why: %s]" % it["why"]) if it["why"] else ""))
    for it in skip_items:
        v = detect(it["text"])
        if v["fire"]:
            fails.append("FALSE-POSITIVE %r → %s%s"
                         % (it["text"], v["reason"],
                            (" [why: %s]" % it["why"]) if it["why"] else ""))
            continue
        # marker 가 명시된 skip 은 '억제(선언 검출됨)'여야 하고 마커도 일치해야 한다 —
        # 무선언으로 접혀도 fire=False 라 겉으론 통과처럼 보이는 유형 융합을 여기서 가른다.
        if it["marker"]:
            if v["verdict"] != "suppressed":
                fails.append("SKIP 유형 융합 %r: marker %r 명시인데 verdict=%s(억제 아님)"
                             % (it["text"], it["marker"], v["verdict"]))
            elif it["marker"] not in (v["marker"] or ""):
                fails.append("억제 마커 불일치 %r: 기대 %r ⊄ 실측 %r(축=%s)"
                             % (it["text"], it["marker"], v["marker"], v.get("axis")))
    # 축 진단 파리티(요구5) — reason 문자열이 아니라 기계 필드(axis·marker)로 검증한다.
    for text, axis, marker in AXIS_PINS:
        v = detect(text)
        if v["verdict"] != "suppressed" or v.get("axis") != axis \
                or marker not in (v["marker"] or ""):
            fails.append("축 진단 이탈 %r: 기대 axis=%s marker⊇%r / 실측 axis=%s marker=%r(%s)"
                         % (text, axis, marker, v.get("axis"), v["marker"], v["verdict"]))
    # 경계 스펙 — filler 15=발화 / 16=미발화(P3-A-FILLER: 주석 12 ≠ 코드 15 불일치 해소본)
    if not detect("너는" + "가" * FILLER_MAX + "마스터다")["fire"]:
        fails.append("filler %d자 경계에서 미발화" % FILLER_MAX)
    if detect("너는" + "가" * (FILLER_MAX + 1) + "마스터다")["fire"]:
        fails.append("filler %d자에서 발화(창 초과 오발화)" % (FILLER_MAX + 1))
    # 감지창 — 한글 **문자** 200자 기준(G25: 바이트면 약 66자에서 잘린다)
    pad = "가" * (WINDOW_CHARS - len("너는 마스터다"))
    if not detect(pad + "너는 마스터다")["fire"]:
        fails.append("감지창 끝(문자 %d) 선언 미발화 — 바이트 슬라이스 회귀 의심" % WINDOW_CHARS)
    if detect("가" * WINDOW_CHARS + "너는 마스터다")["fire"]:
        fails.append("감지창 밖 선언이 발화(창 미적용)")
    # 억제/미검출 분리(exit 1 vs 3)
    if detect("'너는 마스터다'가 무슨 뜻?")["verdict"] != "suppressed":
        fails.append("인용·의문 케이스가 suppressed 로 분류되지 않음")
    if detect("오늘 작업 지시해줘")["verdict"] != "no_declaration":
        fails.append("무선언 케이스가 no_declaration 으로 분류되지 않음")
    if fails:
        print("javis_detect self-test FAIL (%d):" % len(fails), file=sys.stderr)
        for f in fails:
            print("  -", f, file=sys.stderr)
        return 1
    print("javis_detect self-test OK (corpus=%s · FIRE %d · SKIP %d · 축 진단 핀 %d · "
          "filler 15/16 경계 · 감지창 %d자 경계 · 억제/미검출 분리)"
          % (source, len(fire_items), len(skip_items), len(AXIS_PINS), WINDOW_CHARS))
    return 0


def main(argv):
    if "--self-test" in argv:
        return cmd_self_test()
    cmd = argv[1] if len(argv) > 1 else "hook-gate"
    if cmd == "hook-gate":
        return cmd_hook_gate(argv[2:])
    if cmd == "text":                     # 진단용: 인자 텍스트 1건 판정(개발자 편의)
        v = detect(" ".join(argv[2:]))
        print(json.dumps(v, ensure_ascii=False, indent=1))
        return EXIT_FIRE if v["fire"] else EXIT_NO_DECL
    sys.stderr.write("usage: javis_detect.py [hook-gate|text <문장>|--self-test]\n"
                     "  hook-gate: stdin=UserPromptSubmit hook JSON → "
                     "exit 0=FIRE 1=선언없음 2=판정불가 3=억제\n")
    return 64  # EX_USAGE — 미지 서브커맨드는 거부(fail-closed · A14 동형)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
