#!/usr/bin/env python3
"""javis_vibecheck.py — 바이브코딩 결정론 게이트 (설계 v3.2 §4-J·§C7.4·§C10.2 포트)

세 서브커맨드가 하나의 대상 프로젝트(--project)를 검사한다. 게이트 판정은 산문이 아니라
exit code가 사실이다: 0 pass / 1 soft(경고) / 2 hard-fail(fail-closed).

지위(§C5 감사 확정): vibecheck는 **독자 done 게이트가 아니라 javis_task evidence 공급자
(advisory)**다. exit code·findings는 done 전이의 판정권을 갖지 않으며, `javis_task set-status
<id> done --evidence "javis_vibecheck.py <sub> → <verdict>(exit N)"` 형태로 증거에 인용될
뿐이다. done 판정권은 javis_task의 evidence/settle 게이트에 있다(vibecheck는 그 입력만 제공).

  docs      — 문서체인 무결성 (§C8 doc-chain·헌법 1·8조):
              Level별 필수 문서 존재 + YAML front-matter 계약 골격(sot·context·layer·inheritance)
              + CLAUDE.md 브릿지 존재. 필수 문서 결손=hard, 계약 필드/브릿지 결손=soft.
  security  — 보안 Tier 1 게이트 (§4-J·헌법 9조):
              ①secrets 스캔(작업트리 + `git log -p` 이력) ②Supabase RLS 누락(create table
              대비 enable RLS) ③관리자 경로 무가드 노출 휴리스틱 ④.env가 .gitignore에 있는지.
              secrets·RLS 누락=hard, 관리자 경로·env 휴리스틱=soft. supabase 부재=skip 명시.
  integrity — §C7.4 test-suite integrity strict flow:
              pre-run(테스트 파일 git hash + assert/skip/self-mock 센서스 기록) →
              gate(기록 대비 파일 변동·assertion 감소·skip 증가·self-mock 삽입 검출 → hard).
              순서 강제: pre-run 기록 없이 gate 호출 시 hard(2) fail-closed.

설계 근거: _research/vibecoding-mastery/PROPOSAL-jarvis-vibecoding-system-v3.md (워크스페이스 상대)
스타일: javis_task.py 등 stdlib-only 관례.
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time

EXIT_PASS, EXIT_SOFT, EXIT_HARD = 0, 1, 2

# 트리 순회 시 건너뛰는 디렉터리 — 스캔 오염·과다비용 차단.
SKIP_DIRS = {".git", "node_modules", "__pycache__", ".venv", "venv", "env",
             "dist", "build", ".next", ".vibecheck", ".mypy_cache", ".pytest_cache",
             "coverage", ".turbo", "vendor", ".idea", ".gradle"}
MAX_FILE_BYTES = 1_000_000  # 단일 텍스트 파일 스캔 상한(대형/바이너리 회피)


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


def _iter_text_files(root):
    """SKIP_DIRS를 제외한 프로젝트 하위 텍스트 파일을 (relpath, abspath, text)로 산출."""
    for dirpath, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for name in files:
            ap = os.path.join(dirpath, name)
            try:
                if os.path.getsize(ap) > MAX_FILE_BYTES:
                    continue
                with open(ap, encoding="utf-8", errors="ignore") as f:
                    text = f.read()
            except OSError:
                continue
            if "\x00" in text[:1024]:  # 바이너리 휴리스틱
                continue
            yield os.path.relpath(ap, root), ap, text


# M-5 교정: secret 스캔은 MAX_FILE_BYTES(1MB) skip을 쓰지 않는다 — 대용량 파일에 secret을 숨기면
# "조용히 통과"하던 미검출 경로였다. 대신 청크 스트리밍으로 전량 스캔(메모리 안전)하고, 극단 크기만
# SOFT 경고로 skip 사실을 보고한다(조용한 skip 금지).
SECRET_STREAM_HARD_CAP = 50 * 1024 * 1024  # 이 이상만 스캔 불가(SOFT 경고). 그 아래는 전량 스트리밍
SECRET_CHUNK = 262144   # 256KB 청크 — 전량 read 회피
SECRET_OVERLAP = 4096   # 청크 경계에 걸친 패턴 포착용 겹침(최장 패턴 + 여유)


def _iter_all_files(root):
    """SKIP_DIRS 제외 프로젝트 하위 전 파일을 (relpath, abspath)로 산출(크기 필터 없음)."""
    for dirpath, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for name in files:
            ap = os.path.join(dirpath, name)
            yield os.path.relpath(ap, root), ap


def _scan_file_secrets(path):
    """파일을 청크 스트리밍으로 스캔(전량 메모리 로드 회피). 반환 (hits, too_large).
    hits=중복 제거된 [(name, frag)]. 바이너리(첫 청크 널바이트)는 조용히 skip(secret=텍스트 전제).
    극단 크기(> SECRET_STREAM_HARD_CAP)면 (빈, True) — 호출자가 SOFT 경고."""
    try:
        if os.path.getsize(path) > SECRET_STREAM_HARD_CAP:
            return [], True
    except OSError:
        return [], False
    hits, seen = [], set()
    try:
        with open(path, encoding="utf-8", errors="ignore") as f:
            first, carry = True, ""
            while True:
                chunk = f.read(SECRET_CHUNK)
                if not chunk:
                    break
                if first and "\x00" in chunk[:1024]:  # 바이너리 휴리스틱
                    return [], False
                first = False
                for name, frag in _scan_secrets_text(carry + chunk):
                    key = (name, frag)
                    if key not in seen:  # 경계 겹침 재매칭 중복 제거
                        seen.add(key)
                        hits.append((name, frag))
                carry = (carry + chunk)[-SECRET_OVERLAP:]
    except OSError:
        return [], False
    return hits, False


def _git(project, *args, timeout=60):
    """git 서브프로세스 실행. (ok, stdout). git 부재·비저장소·실패 시 (False, "")."""
    try:
        p = subprocess.run(["git", "-C", project, *args], capture_output=True,
                           text=True, errors="ignore", timeout=timeout)
    except (OSError, subprocess.SubprocessError):
        return False, ""
    return p.returncode == 0, p.stdout


def _is_git_repo(project):
    ok, out = _git(project, "rev-parse", "--is-inside-work-tree")
    return ok and out.strip() == "true"


class Findings:
    """검사 결과 누산기 — severity(0/1/2)의 최댓값이 exit code."""

    def __init__(self):
        self.items = []

    def add(self, severity, check, message, evidence=None):
        self.items.append({"severity": severity, "check": check,
                           "message": message, "evidence": evidence})

    def note(self, check, message):
        self.add(EXIT_PASS, check, message)  # severity 0 = 정보/skip(코드에 영향 없음)

    def exit_code(self):
        return max((it["severity"] for it in self.items), default=EXIT_PASS)

    def report(self, subcmd, as_json):
        code = self.exit_code()
        verdict = {EXIT_PASS: "pass", EXIT_SOFT: "soft", EXIT_HARD: "hard-fail"}[code]
        # §C5: advisory 지위 — done 판정권 없음(javis_task evidence 공급자).
        advisory = ("advisory: javis_task 에 증거를 대는 도구(독자 done 게이트 아님) — "
                    "done 판정권은 javis_task에 있음")
        if as_json:
            print(json.dumps({"subcommand": subcmd, "verdict": verdict, "exit": code,
                             "status": "advisory", "role": "javis_task evidence supplier",
                             "findings": self.items}, ensure_ascii=False, indent=1))
            return code
        label = {EXIT_PASS: "OK", EXIT_SOFT: "SOFT", EXIT_HARD: "HARD"}
        for it in self.items:
            line = f"[{label[it['severity']]}] {it['check']}: {it['message']}"
            print(line)
            if it["evidence"]:
                print(f"        evidence: {it['evidence']}")
        print(f"VERDICT[{subcmd}]: {verdict} (exit {code})")
        print(advisory)
        return code


# ─────────────────────────────────────────────────────────────────────────────
# docs — 문서체인 무결성
# ─────────────────────────────────────────────────────────────────────────────

# 문서 종류 → 파일 stem 매칭 패턴(소문자). SOT=skills/vibecoding-docs/assets/README.md의
# "28문서→프로젝트 경로 대응표"(NLC 정본). 경로는 대부분 /docs/ 및 그 하위(design/·external/).
DOC_KINDS = {
    "requirement": ["requirement", "requirements", "srs"],       # 2-10 /docs/requirement.md (SRS)
    "spec": ["spec", "specification"],                           # 7    /docs/spec.md (Use Case Spec)
    "test": ["test", "test-plan", "tests", "tdd"],               # 9·9-1 /docs/test.md·test-plan.md
    "state": ["state-management", "state", "states", "state-machine"],  # 8 /docs/state-management.md
    "database": ["database", "schema", "db", "data-flow"],       # 6    /docs/database.md
    "boundary": ["external-integration", "boundary", "boundaries", "external", "interface"],  # 3 /docs/external/*.md
    "design": ["ui", "ux", "design", "architecture", "adr", "fds"],  # 4-2·5-1 /docs/design/ui.md·ux.md
    "userflow": ["userflow", "user-flow", "userflows"],          # 5    /docs/userflow.md
    "visual": ["visual", "style-guide", "design-principles"],    # 8-2  /docs/design/visual.md
    "prd": ["prd"],                                              # 4    /docs/prd.md
    "plan": ["plan", "roadmap", "implementation-plan"],          # 10-1 /docs/plan.md
}

# 디렉토리 경로 규칙(별칭 stem 매칭 불가한 kind). boundary=/docs/external/<서비스명>.md 형태라
# stem이 서비스명이어서 별칭으로 못 잡는다(대응표 3단계) → 디렉토리에 .md ≥1 이면 충족(별칭 병행).
DIR_RULES = {"boundary": os.path.join("docs", "external")}

# Level별 필수 문서 종류. SOT=README "Level별 필수 템플릿". 격상 허용·격하 금지(헌법 7조).
#   L5 canon="풀 세트(28문서 전체)"이나, 헌법 계층(AGENTS/rules/ruler)·env 템플릿은 doc-chain
#   per-feature 게이트 대상이 아니고 rules 계층·V3 Specs Gate가 별도 검증한다. 따라서 L5는
#   대응표에 실존하는 load-bearing 문서 집합(기획 prd·userflow + 설계 design·visual + 실행 plan)을
#   L4 위에 얹어 강제한다(정본 정렬·판단 위임 근거: team-lead §C5 감사). "security" kind는 28문서에
#   해당 문서가 없어 제거(보안은 rules 계층 + security 서브커맨드 담당).
LEVEL_DOCS = {
    "L1": [],  # L1~L2 스크립트·데모: 필수 문서 없음
    "L3": ["requirement", "spec", "test"],  # 기능 단위: 무엇→어떻게 작동→어떻게 검증
    "L4": ["requirement", "spec", "test", "state", "database", "boundary"],  # +상태·데이터·경계
    "L5": ["requirement", "spec", "test", "state", "database", "boundary",
           "prd", "userflow", "design", "visual", "plan"],  # 풀스택(정본 load-bearing 집합)
}

# M-2 정합(협조·viberoute 어휘): 설계·viberoute는 경량 티어를 "L1-2"(L2 포함)로 표기하나 doc-chain
# 상 L1과 L2는 동일(필수 문서 0)이라 별도 행이 없다. viberoute 출력을 --level로 그대로 흘려도
# usage 에러가 나지 않도록 L1-2·L2를 L1으로 매핑한다(어휘 정합만·의미 무변경·과잉 변경 없음).
LEVEL_ALIASES = {"L1-2": "L1", "L2": "L1"}


def _normalize_level(level):
    return LEVEL_ALIASES.get(level, level)


# 계약 골격 필수 필드(SOT: README "계약 골격 불변" + templates/*.md front-matter 실측).
# context(SCDP 상속 목록)는 계약 골격의 핵심 필드 — 누락 시 상속 체인이 끊긴다. 대소문자는
# 아래 검사에서 lower() 정규화하므로 소문자로 통일(sot·context·layer·inheritance 실존 확인).
FRONTMATTER_FIELDS = ["sot", "context", "layer", "inheritance"]


def _find_doc(project, kind):
    """kind에 해당하는 .md 문서 경로 반환. 없으면 None.
    ① 디렉토리 경로 규칙(DIR_RULES) 우선 — boundary=/docs/external/*.md 처럼 stem이 별칭과
       무관한 경우. ② 별칭 stem 매칭(루트 + docs/ 재귀) 병행."""
    drule = DIR_RULES.get(kind)
    if drule:
        d = os.path.join(project, drule)
        if os.path.isdir(d):
            for name in sorted(os.listdir(d)):
                if name.lower().endswith(".md"):
                    return os.path.join(d, name)
    patterns = DOC_KINDS[kind]
    roots = [project, os.path.join(project, "docs")]
    for base in roots:
        if not os.path.isdir(base):
            continue
        for dirpath, dirs, files in os.walk(base):
            dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
            for name in files:
                if not name.lower().endswith(".md"):
                    continue
                stem = name[:-3].lower()
                if stem in patterns:
                    return os.path.join(dirpath, name)
            if base == project:
                break  # 루트는 재귀하지 않음(docs/만 재귀) — 오검출 억제
    return None


def _read_frontmatter(path):
    """상단 --- ... --- YAML front-matter의 키 집합 반환. 없으면 None. (stdlib 파서)."""
    try:
        with open(path, encoding="utf-8", errors="ignore") as f:
            text = f.read()
    except OSError:
        return None
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return None
    keys = set()
    for line in lines[1:]:
        if line.strip() == "---":
            return keys
        m = re.match(r"\s*([A-Za-z0-9_-]+)\s*:", line)
        if m:
            keys.add(m.group(1))
    return None  # 닫는 --- 부재 = 유효 front-matter 아님


def cmd_docs(a):
    project = os.path.abspath(a.project)
    f = Findings()
    if not os.path.isdir(project):
        f.add(EXIT_HARD, "project", f"프로젝트 디렉터리 없음: {project}")
        return f.report("docs", a.json)

    level = _normalize_level(a.level)  # M-2: viberoute "L1-2"/"L2" → L1 어휘 정합
    required = LEVEL_DOCS[level]
    if a.level != level:
        f.note("level", f"level 별칭 정규화: {a.level} → {level}")
    if not required:
        f.note("level", f"{level}: 필수 문서 없음(0~2 재량) — doc-chain 하드 요구 생략")

    for kind in required:
        path = _find_doc(project, kind)
        if path is None:
            f.add(EXIT_HARD, "doc-chain",
                  f"필수 문서 결손: '{kind}' ({level} 요구) — {DOC_KINDS[kind][0]}.md 등",
                  evidence=f"patterns={DOC_KINDS[kind]}")
            continue
        rel = os.path.relpath(path, project)
        keys = _read_frontmatter(path)
        if keys is None:
            f.add(EXIT_SOFT, "frontmatter",
                  f"'{kind}' 문서에 YAML front-matter 없음 — 계약 골격 미충족", evidence=rel)
            continue
        keys_lower = {k.lower() for k in keys}
        missing = [x for x in FRONTMATTER_FIELDS if x.lower() not in keys_lower]
        if missing:
            f.add(EXIT_SOFT, "frontmatter",
                  f"'{kind}' front-matter 계약 필드 결손: {missing}", evidence=rel)
        else:
            f.note("doc-chain", f"'{kind}' OK — {rel} (front-matter 계약 충족)")

    bridge = os.path.join(project, "CLAUDE.md")
    if os.path.isfile(bridge):
        f.note("bridge", "CLAUDE.md 브릿지 존재")
    else:
        f.add(EXIT_SOFT, "bridge", "CLAUDE.md 브릿지 부재 — 헌법·파이프라인 연결 누락")
    return f.report("docs", a.json)


# ─────────────────────────────────────────────────────────────────────────────
# security — Tier 1
# ─────────────────────────────────────────────────────────────────────────────

# (name, 정규식, 확정신호?) — 확정신호=True면 placeholder 필터 없이 항상 보고.
SECRET_PATTERNS = [
    ("aws_access_key", re.compile(r"\bAKIA[0-9A-Z]{16}\b"), True),
    ("private_key", re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH |DSA |PGP )?PRIVATE KEY-----"), True),
    ("slack_token", re.compile(r"\bxox[baprs]-[0-9A-Za-z-]{10,}\b"), True),
    ("generic_api_key", re.compile(
        r"(?i)(?:api[_-]?key|secret[_-]?key|access[_-]?token|auth[_-]?token)"
        r"\s*[:=]\s*['\"][A-Za-z0-9_\-\.]{16,}['\"]"), False),
    ("password_literal", re.compile(
        r"(?i)password\s*[:=]\s*['\"][^'\"]{6,}['\"]"), False),
]
_PLACEHOLDER = re.compile(
    r"(?i)example|placeholder|your[_-]?|xxxx|changeme|dummy|redacted|sample|"
    r"process\.env|os\.environ|getenv|<[^>]+>|\$\{")


def _scan_secrets_text(text):
    """텍스트에서 (name, snippet) 히트 목록. placeholder는 비확정 패턴에 한해 필터."""
    hits = []
    for name, rx, definite in SECRET_PATTERNS:
        for m in rx.finditer(text):
            frag = m.group(0)
            if not definite and _PLACEHOLDER.search(frag):
                continue
            hits.append((name, frag[:60]))
    return hits


def _check_secrets(project, f, scan_history):
    tree_hits = 0
    # M-5: 크기 무관 전 파일을 스트리밍 스캔 — 대용량 파일 은닉 secret의 조용한 통과 제거.
    for rel, ap in _iter_all_files(project):
        hits, too_large = _scan_file_secrets(ap)
        if too_large:
            f.add(EXIT_SOFT, "secrets",
                  f"파일이 너무 커서(> {SECRET_STREAM_HARD_CAP // 1024 // 1024}MB) secret 스캔 "
                  "불가 — 수동 확인 필요(조용한 skip 아님)", evidence=rel)
            continue
        for name, frag in hits:
            tree_hits += 1
            f.add(EXIT_HARD, "secrets", f"작업트리 secret 의심({name})", evidence=f"{rel}: {frag}")
    if scan_history and _is_git_repo(project):
        ok, out = _git(project, "log", "-p", "--all", "--no-color", timeout=90)
        if ok:
            hist = 0
            for line in out.splitlines():
                if not line.startswith("+") or line.startswith("+++"):
                    continue  # 추가된 라인만 — 도입 시점 포착
                for name, frag in _scan_secrets_text(line[1:]):
                    hist += 1
                    f.add(EXIT_HARD, "secrets", f"git 이력 secret 의심({name})", evidence=frag)
                    if hist >= 50:
                        break
                if hist >= 50:
                    break
    elif scan_history:
        f.note("secrets", "git 이력 스캔 skip — git 저장소 아님")
    if tree_hits == 0:
        f.note("secrets", "작업트리 secret 미검출")


def _normalize_table(raw):
    """스키마·따옴표·괄호 제거 후 소문자 테이블명."""
    raw = raw.strip().strip('"').strip("`")
    if "." in raw:
        raw = raw.split(".")[-1].strip('"').strip("`")
    return raw.lower()


_CREATE_TABLE_RE = re.compile(
    r"create\s+table\s+(?:if\s+not\s+exists\s+)?([A-Za-z0-9_.\"`]+)", re.IGNORECASE)
_ENABLE_RLS_RE = re.compile(
    r"alter\s+table\s+(?:only\s+)?([A-Za-z0-9_.\"`]+)\s+enable\s+row\s+level\s+security",
    re.IGNORECASE)


def _check_rls(project, f):
    mig_dir = os.path.join(project, "supabase", "migrations")
    if not os.path.isdir(mig_dir):
        f.note("rls", "Supabase 부재 — RLS 검사 skip (supabase/migrations 없음)")
        return
    created, enabled = {}, set()
    for name in sorted(os.listdir(mig_dir)):
        if not name.endswith(".sql"):
            continue
        try:
            with open(os.path.join(mig_dir, name), encoding="utf-8", errors="ignore") as fh:
                sql = fh.read()
        except OSError:
            continue
        for m in _CREATE_TABLE_RE.finditer(sql):
            t = _normalize_table(m.group(1))
            created.setdefault(t, name)
        for m in _ENABLE_RLS_RE.finditer(sql):
            enabled.add(_normalize_table(m.group(1)))
    if not created:
        f.note("rls", "Supabase migrations에 create table 없음 — RLS 대상 없음")
        return
    missing = sorted(t for t in created if t not in enabled)
    for t in missing:
        f.add(EXIT_HARD, "rls",
              f"테이블 '{t}' RLS 미활성 — enable row level security 누락",
              evidence=f"created in {created[t]}")
    if not missing:
        f.note("rls", f"전 테이블 RLS 활성 확인({len(created)}개)")


# 관리자 라우트 신호 / 인증 가드 신호 휴리스틱.
_ADMIN_ROUTE_RE = re.compile(
    r"""(?ix)
    (?:\.(?:get|post|put|patch|delete|use|all|route)\s*\(\s*['"][^'"]*/admin) |
    (?:@app\.route\s*\(\s*['"][^'"]*/admin) |
    (?:path\s*[:=]\s*['"][^'"]*/admin) |
    (?:router\.[a-z]+\s*\(\s*['"][^'"]*/admin)
    """)
_AUTH_GUARD_RE = re.compile(
    r"(?i)require[_-]?auth|isadmin|is_admin|ensure[_-]?admin|require[_-]?role|"
    r"authenticate|authorize|getserversession|middleware|@login_required|"
    r"admin[_-]?required|protect|verifytoken|verify_token|guard")


def _check_admin_exposure(project, f):
    flagged = 0
    for rel, _ap, text in _iter_text_files(project):
        if not _ADMIN_ROUTE_RE.search(text):
            continue
        if not _AUTH_GUARD_RE.search(text):
            flagged += 1
            f.add(EXIT_SOFT, "admin-exposure",
                  "관리자 경로가 인증 가드 없이 노출된 것으로 의심(휴리스틱)", evidence=rel)
    # 파일시스템 라우트(Next.js 등) app/admin·pages/admin 디렉터리
    for d in ("app/admin", "pages/admin", "src/app/admin", "src/pages/admin"):
        if os.path.isdir(os.path.join(project, d)):
            f.note("admin-exposure",
                   f"파일시스템 관리자 라우트 존재({d}) — 미들웨어 인증 가드 수동 확인 권장")
    if flagged == 0:
        f.note("admin-exposure", "무가드 관리자 경로 미검출")


def _check_env_gitignore(project, f):
    gi = os.path.join(project, ".gitignore")
    ignored = False
    if os.path.isfile(gi):
        try:
            with open(gi, encoding="utf-8", errors="ignore") as fh:
                for line in fh:
                    s = line.strip()
                    if s in (".env", "*.env", ".env*", ".env.*") or s.endswith("/.env"):
                        ignored = True
                        break
        except OSError:
            pass
    env_exists = os.path.isfile(os.path.join(project, ".env"))
    if env_exists and _is_git_repo(project):
        ok, _ = _git(project, "ls-files", "--error-unmatch", ".env")
        if ok:
            f.add(EXIT_HARD, "env-gitignore", ".env가 git에 커밋됨 — 즉시 제거·회전 필요")
            return
    if ignored:
        f.note("env-gitignore", ".env가 .gitignore에 포함됨")
    else:
        f.add(EXIT_SOFT, "env-gitignore",
              ".env가 .gitignore에 없음 — 비밀 유출 위험(패턴 추가 권장)")


def cmd_security(a):
    project = os.path.abspath(a.project)
    f = Findings()
    if not os.path.isdir(project):
        f.add(EXIT_HARD, "project", f"프로젝트 디렉터리 없음: {project}")
        return f.report("security", a.json)
    _check_secrets(project, f, scan_history=not a.no_history)
    _check_rls(project, f)
    _check_admin_exposure(project, f)
    _check_env_gitignore(project, f)
    return f.report("security", a.json)


# ─────────────────────────────────────────────────────────────────────────────
# integrity — §C7.4 strict flow
# ─────────────────────────────────────────────────────────────────────────────

TEST_FILE_RES = [
    re.compile(r"(^|/)test_[^/]*\.py$"),
    re.compile(r"[^/]*_test\.py$"),
    re.compile(r"[^/]*\.test\.(js|jsx|ts|tsx)$"),
    re.compile(r"[^/]*\.spec\.(js|jsx|ts|tsx)$"),
]
ASSERT_RE = re.compile(r"\bassert\b|\bself\.assert[A-Za-z]+|\bexpect\s*\(|\bassertThat\b")
SKIP_RE = re.compile(
    r"@unittest\.skip|@pytest\.mark\.skip|pytest\.mark\.skip|\.skipTest\s*\(|"
    r"unittest\.SkipTest|@skip\b|\bxit\s*\(|\bxdescribe\s*\(|\.skip\s*\(")
MOCK_RE = re.compile(
    r"unittest\.mock|from\s+unittest\s+import\s+mock|MagicMock|@patch\b|mock\.patch|"
    r"jest\.mock|sinon\.|createMock|@mock\.patch")


def _is_test_file(rel):
    rel = rel.replace(os.sep, "/")
    return any(rx.search(rel) for rx in TEST_FILE_RES)


def _census(project):
    """테스트 파일별 {sha256, asserts, skips, mocks} 센서스."""
    files = {}
    for rel, _ap, text in _iter_text_files(project):
        if not _is_test_file(rel):
            continue
        files[rel.replace(os.sep, "/")] = {
            "sha256": hashlib.sha256(text.encode("utf-8", "ignore")).hexdigest(),
            "asserts": len(ASSERT_RE.findall(text)),
            "skips": len(SKIP_RE.findall(text)),
            "mocks": len(MOCK_RE.findall(text)),
        }
    return files


def _record_path(project, override):
    if override:
        return os.path.abspath(override)
    return os.path.join(project, ".vibecheck", "integrity_prerun.json")


def cmd_integrity(a):
    project = os.path.abspath(a.project)
    f = Findings()
    if not os.path.isdir(project):
        f.add(EXIT_HARD, "project", f"프로젝트 디렉터리 없음: {project}")
        return f.report("integrity", a.json)
    rec_path = _record_path(project, a.record)

    if a.phase == "pre-run":
        files = _census(project)
        record = {"created_at": _now(), "project": project, "files": files,
                  "totals": {
                      "files": len(files),
                      "asserts": sum(v["asserts"] for v in files.values()),
                      "skips": sum(v["skips"] for v in files.values()),
                      "mocks": sum(v["mocks"] for v in files.values())}}
        os.makedirs(os.path.dirname(rec_path), exist_ok=True)
        with open(rec_path, "w", encoding="utf-8") as fh:
            json.dump(record, fh, ensure_ascii=False, indent=1)
        f.note("pre-run", f"센서스 기록: {record['totals']['files']}개 테스트 파일, "
               f"assert {record['totals']['asserts']}, skip {record['totals']['skips']}, "
               f"mock {record['totals']['mocks']} → {rec_path}")
        return f.report("integrity", a.json)

    # phase == gate — 순서 강제: pre-run 기록 없으면 fail-closed(hard).
    if not os.path.isfile(rec_path):
        f.add(EXIT_HARD, "order",
              "pre-run 기록 부재 — integrity gate는 pre-run 선행 필수(strict flow 위반)",
              evidence=rec_path)
        return f.report("integrity", a.json)
    try:
        with open(rec_path, encoding="utf-8") as fh:
            record = json.load(fh)
    except (OSError, json.JSONDecodeError) as e:
        f.add(EXIT_HARD, "order", f"pre-run 기록 손상 — 재기록 필요: {e}", evidence=rec_path)
        return f.report("integrity", a.json)

    before = record.get("files", {})
    after = _census(project)
    clean = True
    for rel, prev in before.items():
        cur = after.get(rel)
        if cur is None:
            f.add(EXIT_HARD, "integrity", f"테스트 파일 삭제됨: {rel}")
            clean = False
            continue
        if cur["sha256"] != prev["sha256"]:
            f.add(EXIT_HARD, "integrity", f"테스트 파일 변동(인가되지 않은 수정): {rel}",
                  evidence=f"{prev['sha256'][:12]} → {cur['sha256'][:12]}")
            clean = False
        if cur["asserts"] < prev["asserts"]:
            f.add(EXIT_HARD, "integrity",
                  f"assertion 감소: {rel} ({prev['asserts']} → {cur['asserts']})")
            clean = False
        if cur["skips"] > prev["skips"]:
            f.add(EXIT_HARD, "integrity",
                  f"skip 마커 증가: {rel} ({prev['skips']} → {cur['skips']})")
            clean = False
        if cur["mocks"] > prev["mocks"]:
            f.add(EXIT_HARD, "integrity",
                  f"self-mock 삽입: {rel} ({prev['mocks']} → {cur['mocks']})")
            clean = False
    for rel in after:
        if rel not in before:
            f.note("integrity", f"신규 테스트 파일(기록에 없음): {rel}")
    if clean:
        f.note("integrity", f"integrity 통과 — {len(before)}개 테스트 파일 무변동")
    return f.report("integrity", a.json)


def main(argv=None):
    p = argparse.ArgumentParser(description="바이브코딩 결정론 게이트 (docs·security·integrity)")
    sub = p.add_subparsers(dest="cmd", required=True)

    d = sub.add_parser("docs", help="문서체인 무결성")
    d.add_argument("--project", default=".", help="대상 프로젝트 디렉터리")
    d.add_argument("--level", default="L3",
                   choices=list(LEVEL_DOCS.keys()) + list(LEVEL_ALIASES.keys()),
                   help="복잡도 Level. viberoute 어휘 L1-2·L2는 L1으로 정규화")
    d.add_argument("--json", action="store_true")
    d.set_defaults(fn=cmd_docs)

    s = sub.add_parser("security", help="보안 Tier 1")
    s.add_argument("--project", default=".")
    s.add_argument("--no-history", action="store_true", help="git 이력 secret 스캔 생략")
    s.add_argument("--json", action="store_true")
    s.set_defaults(fn=cmd_security)

    i = sub.add_parser("integrity", help="test-suite integrity gate (§C7.4)")
    i.add_argument("phase", choices=["pre-run", "gate"])
    i.add_argument("--project", default=".")
    i.add_argument("--record", default=None, help="센서스 기록 경로(기본 <project>/.vibecheck/)")
    i.add_argument("--json", action="store_true")
    i.set_defaults(fn=cmd_integrity)

    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
