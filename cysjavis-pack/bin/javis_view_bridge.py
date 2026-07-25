#!/usr/bin/env python3
"""javis_view_bridge.py — 산출물 뷰어 사이드카 (DESIGN-v1.2 §2C · W-PBa)

워커·리뷰어·사람이 산출물 md/diff 를 브라우저 안에서 렌더해 읽는 계기.
cys 데몬·제품 코드 무접점. launchd·부트 훅·preflight 무접점 — lazy 기동, 상주형.

원칙(오피스 브리지 javis_hud_bridge.py 계보):
  · 파이썬 stdlib만 사용 (외부 의존 0). 뷰어 앱의 vendored OSS 는 예외 허용 대상.
  · 읽기 전용 — 쓰기 API 없음.
  · 127.0.0.1 전용 + URL 토큰 prefix(`/<token>/...`) — 토큰 없는 요청 404.
  · 포트 0-bind → state 파일에 실제 포트 기록(충돌 회피).
  · 경로 화이트리스트 + realpath 정규화 후 prefix 검사 → path traversal·심볼릭 링크 이탈 차단.
  · 유휴 자동 종료 없음(뷰어는 상주형). 죽어도 이 기능만 상실, 부트·오케스트라 무영향.

기동:  cys run --scoped -- python3 bin/javis_view_bridge.py
접속:  기동 로그 또는 ~/.cys/viewer/state.json 의 {port, token} 으로
       http://127.0.0.1:<port>/<token>/app/
"""
import json
import os
import re
import secrets
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote, urlparse, parse_qs

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
APP_DIR = os.path.join(ROOT, "web", "viewer")   # vendored 정적앱 위치
BIND = "127.0.0.1"
STATE_DIR = os.path.expanduser("~/.cys/viewer")
STATE_PATH = os.path.join(STATE_DIR, "state.json")
FILE_MAX_BYTES = 2 * 1024 * 1024        # 파일 API 상한 (토큰 폭탄 방지)
LIST_MAX_ENTRIES = 2000                 # 디렉토리 목록 상한
WATCH_POLL_SECS = 1.0                   # SSE mtime 폴링 주기
GIT_TIMEOUT = 20                        # git diff 타임아웃
# git ref 안전 문자만 허용 + 선두 '-' 거부 → git 옵션 주입(--output= 등) 차단.
REF_RE = re.compile(r"^[A-Za-z0-9_./~^@{}:\-]{1,200}$")

# 콘솔 없는 부모가 띄울 때 새 콘솔 창 억제 (타 OS 무동작 · hud_bridge 관례).
NOWIN = {"creationflags": 0x08000000} if os.name == "nt" else {}

# CSP — 악성 md 가 뷰어에서 스크립트 실행에 성공하더라도 심층방어로 피해 봉쇄:
#   · script-src 'self'  : 인라인/외부 스크립트 차단(vendored·app.js 는 동일출처 허용)
#   · connect-src 'self' : fetch/XHR/EventSource 를 loopback 사이드카로만 → 외부 exfil 봉쇄
#   · default-src 'none' : 미명시 리소스 전면 차단(화이트리스트 방식)
#   · style-src 'unsafe-inline' : hljs/mermaid 가 주입하는 인라인 <style> 허용(불가피)
#   · img-src data: : marked 가 내보내는 data: 이미지 허용
#   · base-uri/form-action 'none' : <base> 하이재킹·폼 전송 차단
#   · frame-ancestors * : cys-terminal/Tauri 웹 pane 의 iframe 임베드 허용(loopback 한정)
CSP = (
    "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; "
    "img-src 'self' data:; font-src 'self'; connect-src 'self'; "
    "base-uri 'none'; form-action 'none'; frame-ancestors *"
)

# 정적앱 확장자 → content-type (허용 목록 외 = octet-stream).
CTYPES = {
    ".html": "text/html; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".mjs": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".json": "application/json; charset=utf-8",
    ".svg": "image/svg+xml",
    ".woff2": "font/woff2",
    ".woff": "font/woff",
    ".ttf": "font/ttf",
    ".map": "application/json; charset=utf-8",
}


def _allowed_roots():
    """허용 루트 목록(realpath 정규화). 존재하지 않아도 정규화만 하고 유지 —
    나중에 생성될 수 있고, 실검사는 요청 경로 realpath 와의 prefix 비교로 한다.
    CYS_VIEWER_ROOTS(콜론 구분)로 확장.

    ★2026-07-25 오너 승인: 코드 파일 단독 뷰어(확장자별 하이라이팅) 대상에 프로젝트
    소스(src/·ui/·cysjavis-pack/…)를 포함하려면 산출물 루트만으론 부족해 CYSjavis
    프로젝트 루트(_worktrees·cys-homepage 포함)를 허용 루트로 넓힌다. 보안 성질 불변:
    ①127.0.0.1 전용+URL 토큰 ②읽기 전용(POST/PUT/DELETE 405) ③realpath+os.sep 경계로
    traversal·심볼릭 이탈 차단 ④full-trust 역할(cys pane)만 브리지 스폰. 자격증명은
    ~/.cys/*.env(루트 밖·600)에 격리돼 이 루트로 노출되지 않는다. 더 넓히거나 좁히려면
    CYS_VIEWER_ROOTS로 조정(재빌드 불요)."""
    raw = [
        "~/Desktop/CYSjavis",          # 프로젝트 루트(소스·산출물·worktree·홈페이지 전부 하위)
        "~/.cys/_round",
        "~/.cys/browser/evidence",
    ]
    extra = os.environ.get("CYS_VIEWER_ROOTS", "")
    raw += [p for p in extra.split(os.pathsep) if p.strip()]
    roots = []
    seen = set()
    for p in raw:
        rp = os.path.realpath(os.path.expanduser(p.strip()))
        if rp and rp not in seen:
            seen.add(rp)
            roots.append(rp)
    return roots


ALLOWED_ROOTS = _allowed_roots()


def within_roots(path):
    """realpath 정규화 후 허용 루트 prefix 검사. 통과 시 정규화 경로, 아니면 None.

    realpath 가 심볼릭 링크를 먼저 해소하므로 링크로 루트 밖을 가리켜도 차단된다.
    루트 자기 자신도 허용, 그 외에는 반드시 <root>/ 하위여야 한다(형제 디렉토리 이름
    prefix 오판 방지 — os.sep 경계 강제)."""
    if not path:
        return None
    rp = os.path.realpath(os.path.expanduser(path))
    for root in ALLOWED_ROOTS:
        if rp == root or rp.startswith(root + os.sep):
            return rp
    return None


def resolve_app_asset(rel):
    """정적앱 요청 경로 → APP_DIR 내부 실제 파일. 앱 디렉토리 이탈 차단."""
    rel = rel.lstrip("/")
    if not rel or rel.endswith("/"):
        rel = (rel + "index.html") if rel else "index.html"
    cand = os.path.realpath(os.path.join(APP_DIR, rel))
    base = os.path.realpath(APP_DIR)
    if cand == base or cand.startswith(base + os.sep):
        return cand
    return None


def write_state(port, token):
    """~/.cys/viewer/state.json 원자 기록(0600). {pid, port, token}."""
    os.makedirs(STATE_DIR, exist_ok=True)
    payload = json.dumps(
        {"pid": os.getpid(), "port": port, "token": token, "ts": time.time()},
        ensure_ascii=False,
    ).encode()
    tmp = STATE_PATH + ".tmp.%d" % os.getpid()
    # O_CREAT|0o600 로 생성 순간부터 권한 제한 (게이트 자기보호).
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        os.write(fd, payload)
    finally:
        os.close(fd)
    os.replace(tmp, STATE_PATH)
    os.chmod(STATE_PATH, 0o600)


class Handler(BaseHTTPRequestHandler):
    token = None

    def log_message(self, *a):   # 조용히
        pass

    # ---------------------------------------------------------------- 응답
    def _send(self, code, ctype, body, extra=None):
        if isinstance(body, str):
            body = body.encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        # 읽기 전용 loopback 사이드카 — 교차출처 임베드/스크립트 표면 최소화.
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Content-Security-Policy", CSP)
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError, OSError):
            pass

    def _json(self, code, obj):
        self._send(code, "application/json; charset=utf-8",
                   json.dumps(obj, ensure_ascii=False))

    # ---------------------------------------------------------------- 라우팅
    def do_GET(self):
        parsed = urlparse(self.path)
        parts = parsed.path.split("/", 2)   # ["", token, rest]
        # 토큰 prefix 검증 — 부재·불일치 = 404(존재 확인 회피).
        if len(parts) < 3 or not self.token or \
                not secrets.compare_digest(parts[1], str(self.token)):
            return self._send(404, "text/plain; charset=utf-8", "not found")
        rest = "/" + parts[2]
        qs = parse_qs(parsed.query)

        if rest.startswith("/app"):
            return self._serve_app(rest[len("/app"):])
        if rest == "/api/file":
            return self._api_file(qs)
        if rest == "/api/list":
            return self._api_list(qs)
        if rest == "/api/watch":
            return self._api_watch(qs)
        if rest == "/api/diff":
            return self._api_diff(qs)
        return self._send(404, "text/plain; charset=utf-8", "not found")

    # 쓰기 메서드 전면 거부 (읽기 전용).
    def do_POST(self):
        self._send(405, "text/plain; charset=utf-8", "read-only")
    do_PUT = do_DELETE = do_PATCH = do_POST

    def do_OPTIONS(self):   # preflight deny → 교차출처 프리플라이트 차단
        self._send(403, "text/plain; charset=utf-8", "denied")

    # ---------------------------------------------------------------- 정적앱
    def _serve_app(self, rel):
        fp = resolve_app_asset(rel)
        if not fp:
            return self._send(404, "text/plain; charset=utf-8", "not found")
        try:
            with open(fp, "rb") as f:
                body = f.read()
        except OSError:
            return self._send(404, "text/plain; charset=utf-8", "missing asset")
        ext = os.path.splitext(fp)[1].lower()
        ctype = CTYPES.get(ext, "application/octet-stream")
        self._send(200, ctype, body)

    # ---------------------------------------------------------------- file API
    def _api_file(self, qs):
        raw = (qs.get("path") or [""])[0]
        rp = within_roots(unquote(raw))
        if not rp:
            return self._json(403, {"ok": False, "error": "path_denied"})
        if not os.path.isfile(rp):
            return self._json(404, {"ok": False, "error": "not_file"})
        try:
            size = os.path.getsize(rp)
            with open(rp, "rb") as f:
                data = f.read(FILE_MAX_BYTES + 1)
        except OSError as e:
            return self._json(500, {"ok": False, "error": "read_error: %s" % e})
        truncated = len(data) > FILE_MAX_BYTES
        if truncated:
            data = data[:FILE_MAX_BYTES]
        text = data.decode("utf-8", errors="replace")
        return self._json(200, {
            "ok": True, "path": rp, "size": size,
            "truncated": truncated, "content": text,
        })

    # ---------------------------------------------------------------- list API
    def _api_list(self, qs):
        raw = (qs.get("path") or [""])[0]
        rp = within_roots(unquote(raw))
        if not rp:
            return self._json(403, {"ok": False, "error": "path_denied"})
        if not os.path.isdir(rp):
            return self._json(404, {"ok": False, "error": "not_dir"})
        entries = []
        try:
            with os.scandir(rp) as it:
                for de in it:
                    if len(entries) >= LIST_MAX_ENTRIES:
                        break
                    try:
                        st = de.stat()
                        entries.append({
                            "name": de.name,
                            "type": "dir" if de.is_dir() else "file",
                            "size": st.st_size,
                            "mtime": st.st_mtime,
                        })
                    except OSError:
                        continue
        except OSError as e:
            return self._json(500, {"ok": False, "error": "list_error: %s" % e})
        entries.sort(key=lambda e: (e["type"] != "dir", e["name"].lower()))
        return self._json(200, {"ok": True, "path": rp, "entries": entries})

    # ---------------------------------------------------------------- diff API
    def _api_diff(self, qs):
        repo_raw = (qs.get("repo") or [""])[0]
        base = (qs.get("base") or [""])[0]
        repo = within_roots(unquote(repo_raw))
        if not repo:
            return self._json(403, {"ok": False, "error": "repo_denied"})
        base = unquote(base).strip()
        # git 옵션 주입 차단: 선두 '-' 거부 + ref 문자 화이트리스트.
        if base.startswith("-") or not REF_RE.match(base):
            return self._json(400, {"ok": False, "error": "bad_ref"})
        # 고정 argv — 임의 인자 주입 표면 없음(shell 미경유).
        argv = ["git", "-C", repo, "diff", base]
        try:
            r = subprocess.run(argv, capture_output=True, text=True,
                               timeout=GIT_TIMEOUT, **NOWIN)
        except (OSError, subprocess.TimeoutExpired) as e:
            return self._json(500, {"ok": False, "error": "git_error: %s" % e})
        if r.returncode != 0:
            return self._json(200, {
                "ok": False, "error": "git_exit_%d" % r.returncode,
                "stderr": (r.stderr or "")[:2000],
            })
        return self._json(200, {"ok": True, "repo": repo, "base": base,
                                "diff": r.stdout})

    # ---------------------------------------------------------------- watch SSE
    def _api_watch(self, qs):
        raw = (qs.get("path") or [""])[0]
        rp = within_roots(unquote(raw))
        if not rp:
            return self._json(403, {"ok": False, "error": "path_denied"})
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Content-Security-Policy", CSP)
        self.end_headers()

        def emit(obj):
            self.wfile.write(("data: %s\n\n" % json.dumps(obj)).encode())
            self.wfile.flush()

        def mtime_of():
            try:
                return os.stat(rp).st_mtime
            except OSError:
                return None

        try:
            last = mtime_of()
            emit({"type": "init", "mtime": last})
            keep = 0.0
            while True:
                time.sleep(WATCH_POLL_SECS)
                cur = mtime_of()
                if cur != last:
                    last = cur
                    emit({"type": "change", "mtime": cur,
                          "exists": cur is not None})
                    keep = 0.0
                else:
                    keep += WATCH_POLL_SECS
                    if keep >= 15.0:   # keepalive 주석 (프록시·유휴 끊김 방지)
                        self.wfile.write(b": keepalive\n\n")
                        self.wfile.flush()
                        keep = 0.0
        except (BrokenPipeError, ConnectionResetError, OSError):
            pass


def main():
    if not os.path.isdir(APP_DIR):
        sys.stderr.write("[view-bridge] WARN: 정적앱 디렉토리 부재: %s\n" % APP_DIR)
    token = secrets.token_urlsafe(24)     # URL prefix 토큰 — 기동마다 회전
    Handler.token = token
    srv = ThreadingHTTPServer((BIND, 0), Handler)   # 0-bind → 커널 할당 포트
    port = srv.server_address[1]
    write_state(port, token)
    url = "http://%s:%d/%s/app/" % (BIND, port, token)
    print("[view-bridge] %s  (읽기 전용 · 127.0.0.1 한정)" % url, flush=True)
    print("[view-bridge] state: %s" % STATE_PATH, flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        # state 정리 — 스테일 파일이 다음 소비자를 오도하지 않게.
        try:
            cur = json.load(open(STATE_PATH))
            if cur.get("pid") == os.getpid():
                os.remove(STATE_PATH)
        except (OSError, ValueError):
            pass


if __name__ == "__main__":
    main()
