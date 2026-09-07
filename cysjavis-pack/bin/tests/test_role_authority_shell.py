#!/usr/bin/env python3
"""셸 역할 권위와 단일소유 가드의 회귀를 막는 순수 스크립트 검체.

무엇을 막는가: 낡은 env의 권위 승격, 기존 거부의 신규 허용, 무신원 조회,
캐시 오염, autostart 전파 누락과 부모 오염, rotate 재기동의 추가 거부.
밀폐: 매 케이스 새 가짜 HOME·TMPDIR·빈 depts.json을 만들고 env -i로
ambient env를 제거한다. cys-dept는 PATH 앞에 $HOME/.local/bin을 붙이고
CYS_BIN을 다시 해소하므로 가짜 HOME의 .local/bin/cys가 유일한 스텁 주입
경로다. 라이브 데몬·~/.cys·~/.local/state에는 접근하지 않는다.
핀 목록: A 해소 진리표, B 신원 우선순위·문법, C CYS_ROLE 단독 폴백,
D 자식 NO_AUTOSTART·부모 비오염, E stdout 0바이트, F 실패표식·30초 백오프,
G 미래·손상·심링크·문법위반 캐시 배제, H 단조 거부 9종·rotate 면제,
I list/request-only 무조회, J 프리루드 부재 강등. 추가로 cache-none,
60초 TTL 만료, 백오프 만료, 소켓·bootepoch 세대, sh/bash set -u를 핀한다.
R1(리뷰 반영) 추가 핀: 상속 env CYS_DEPT_ROTATE는 더 이상 게이트를 끄지 못한다(K),
rotate 면제는 argv `--rotate`이며 launch 전용·소켓 부재 조건이다(K),
캐시는 0700 전용 디렉터리 안이고 레코드는 4필드 문법이며 선두 0 타임스탬프·
공백 포함 역할·다른 소켓·다른 세대는 전부 캐시 미스다(L).
실행: CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_role_authority_shell.py
"""

from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time


PACK = Path(__file__).resolve().parents[2]
LIB = PACK / "hooks" / "_lib.sh"
DEPT = PACK / "bin" / "cys-dept"
STUB = '''#!/bin/sh
printf '%s\\t%s\\n' "${CYS_NO_AUTOSTART-unset}" "$*" >> "$STUB_LOG"
printf '%s\\n' "${STUB_OUT-}"
exit "${STUB_RC:-0}"
'''
PROBE = '''set -u
. "$1"
cys_resolve_role > "$HOME/resolver.stdout"
probe_rc=$?
printf '%s\\n%s\\n%s\\n%s\\n' "$probe_rc" "$CYS_RESOLVED_ROLE" \
    "$CYS_RESOLVED_ROLE_SOURCE" "${CYS_NO_AUTOSTART-unset}"
'''


def equal(actual, expected, label):
    if actual != expected:
        raise AssertionError(f"{label}: expected {expected!r}, got {actual!r}")


class Sandbox:
    def __init__(self, root):
        self.home = root / "home"
        self.tmp = root / "tmp"
        self.log = root / "stub.log"
        fakebin = self.home / ".local" / "bin"
        for directory in (fakebin, self.tmp, self.home / ".cys"):
            directory.mkdir(parents=True, exist_ok=True)
        self.registry = self.home / ".cys" / "depts.json"
        self.registry.write_text('{"depts":{}}', encoding="utf-8")
        self.log.touch()
        stub = fakebin / "cys"
        stub.write_text(STUB, encoding="utf-8")
        stub.chmod(0o700)
        # cys-dept의 python3도 현재 검체와 동일한 인터프리터로 고정한다.
        (fakebin / "python3").symlink_to(sys.executable)
        self.env = {
            "HOME": str(self.home), "TMPDIR": str(self.tmp),
            "PATH": f"{fakebin}:/usr/bin:/bin:/usr/sbin:/sbin",
            "CYS_DEPTS_JSON": str(self.registry), "CYS_PACK_DIR": str(PACK),
            "CYS_STATE_DIR": str(self.home / ".cys" / "state"),
            "XDG_STATE_HOME": str(self.home / ".local" / "state"),
            "CYS_PY": sys.executable, "PYTHONDONTWRITEBYTECODE": "1",
            "LC_ALL": "C", "STUB_LOG": str(self.log),
            "STUB_OUT": "cso", "STUB_RC": "0", "CYS_SURFACE_ID": "12",
        }

    def configure(self, **values):
        for key, value in values.items():
            if value is None:
                self.env.pop(key, None)
            else:
                self.env[key] = str(value)

    def run(self, argv):
        return subprocess.run(
            ["/usr/bin/env", "-i", *[f"{k}={v}" for k, v in self.env.items()], *argv],
            env={}, cwd=self.home, capture_output=True, text=True, timeout=20,
        )

    def calls(self):
        return self.log.read_text(encoding="utf-8").splitlines()

    def queries(self):
        return [line for line in self.calls() if line.split("\t", 1)[-1] == "surface-role"]

    def resolve(self, role, source, shell="sh", parent="unset"):
        interpreter = shutil.which(shell, path="/usr/bin:/bin")
        if not interpreter:
            raise AssertionError(f"필수 인터프리터 부재: {shell}")
        result = self.run([interpreter, "-c", PROBE, "role-probe", str(LIB)])
        equal(result.returncode, 0, f"{shell} rc; stderr={result.stderr!r}")
        equal(result.stdout, f"0\n{role}\n{source}\n{parent}\n", "해소 결과·부모 env")
        equal((self.home / "resolver.stdout").read_bytes(), b"", "resolver stdout")

    def sock_id(self):
        """레코드에 실리는 데몬 신원 — `_lib.sh cys_role_sock_id` / `javis_role._sock_id` 와 동형."""
        socket = self.env.get("CYS_SOCKET", "")
        if socket:
            return socket[:512]
        return ("default:%s:%s" % (self.env.get("XDG_STATE_HOME", ""),
                                   self.env.get("HOME", "")))[:512]

    def epoch(self):
        socket = self.env.get("CYS_SOCKET", "")
        if socket:
            epoch_file = Path(socket).parent / "boot-epoch"
            if epoch_file.exists():
                line = epoch_file.read_text().splitlines()
                if line and re.fullmatch(r"[A-Za-z0-9._:+-]{1,64}", line[0].strip()):
                    return line[0].strip()
        return "-"

    def cache_dir(self):
        return self.tmp / "cys-role-authority.d"

    def cache(self, surface="12"):
        def slug(value):
            return re.sub(rb"[^A-Za-z0-9._-]", b"_", value.encode("utf-8")).decode()[:80]
        return self.cache_dir() / f"role-{slug(surface)}-{slug(self.sock_id())}"

    def record(self, value="cso", age=0, epoch=None, sock=None, ts=None):
        return "%s %s %s %s\n" % (
            int(time.time()) - age if ts is None else ts,
            value, self.epoch() if epoch is None else epoch,
            self.sock_id() if sock is None else sock)

    def seed(self, value="cso", age=0, **over):
        self.cache_dir().mkdir(mode=0o700, exist_ok=True)
        self.cache().write_text(self.record(value, age, **over), encoding="utf-8")

    # ★부작용 0 원칙: 아래 동사만 허용한다. `launch` 계열은 **이름 검증에서 죽는 이름**만 쓴다 —
    #   가드(exit 7)가 validate_dept_name(exit 2)보다 앞이라는 기존 계약 덕분에, 면제가 섰는지를
    #   레지스트리·데몬을 건드리지 않고 rc 로만 읽을 수 있다.
    _ALLOWED = (("down", "some-dept"), ("down", "some-dept", "--rotate"), ("list",),
                ("promote-if-pending", "--request-only"),
                ("launch", "bad name"), ("launch", "bad name", "--rotate"))

    def dept(self, denied, queries, args=("down", "some-dept"), passthrough_rc=0):
        equal(args in self._ALLOWED, True, f"허용 동사: {args}")
        result = self.run(["/bin/bash", str(DEPT), *args])
        if denied:
            equal(result.returncode, 7, f"dept 거부; stderr={result.stderr!r}")
            equal("★단일소유 강제" in result.stderr, True, "거부 사유 문면")
        else:
            # rc != 7만 검사하면 셸/하네스 오류도 통과하므로 정확한 rc까지 확인한다.
            equal(result.returncode, passthrough_rc, f"dept 통과; stderr={result.stderr!r}")
            equal("★단일소유 강제" in result.stderr, False, "게이트 무발화")
        equal(len(self.queries()), queries, "dept 역할 조회 횟수")


CASES = []


def case(name):
    def register(function):
        CASES.append((name, function))
        return function
    return register


def helper_case(name, env, role, source, calls=1, shell="sh"):
    @case(name)
    def check(box):
        box.configure(**env)
        box.resolve(role, source, shell=shell)
        equal(box.calls(), ["1\tsurface-role"] * calls, "스텁 호출·자식 env")


helper_case("A env master / daemon cso", {"CYS_ROLE": "master"}, "cso", "daemon")
helper_case("A env master / rc2", {"CYS_ROLE": "master", "STUB_RC": 2}, "master", "env-cys-role")
helper_case("A no env / rc2 (bash set -u)", {"STUB_RC": 2}, "", "none", shell="bash")
helper_case("A env cso / daemon-none", {"CYS_ROLE": "cso", "STUB_OUT": ""}, "", "daemon-none")
helper_case("A no surface / env cso", {"CYS_SURFACE_ID": None, "CYS_ROLE": "cso", "STUB_OUT": ""}, "cso", "env-cys-role", 0)


@case("A fresh cache hit")
def fresh_cache(box):
    box.seed()
    box.resolve("cso", "cache")
    equal(box.calls(), [], "캐시 무조회")


for key in ("JAVIS_SURFACE_ID", "AITERM_SURFACE_ID"):
    helper_case(f"B {key} only", {"CYS_SURFACE_ID": None, key: "12"}, "cso", "daemon")
helper_case("B surface:12 accepted", {"CYS_SURFACE_ID": "surface:12"}, "cso", "daemon")
for value in ("12345678901234567890", "abc", ""):
    helper_case(f"B invalid identity {value!r}", {"CYS_SURFACE_ID": value, "CYS_ROLE": "cso"}, "cso", "env-cys-role", 0)


# ★R1(reviewer-codex): Rust 는 **선택된 값 전체**를 파싱한다 — 첫 줄만 떼어 통과시키면
#   여기선 유효 신원인데 CLI 는 파싱 실패로 rc0+빈 줄(=권위 무역할)을 내서, 해소기가
#   **유효 surface 아래에 '권위 무역할'을 캐시**해 이후 정상 CSO 호출을 거짓 거부하게 된다.
helper_case("B multiline identity is rejected (Rust rejects it too)",
            {"CYS_SURFACE_ID": "12\njunk", "CYS_ROLE": "cso"}, "cso", "env-cys-role", 0)
# Rust `env_compat` 은 **빈 문자열만** 건너뛴다 — 공백만 있는 값은 다음 키로 넘어가지 않는다.
helper_case("B whitespace-only primary does not fall through to JAVIS",
            {"CYS_SURFACE_ID": " ", "JAVIS_SURFACE_ID": "12", "CYS_ROLE": "cso"},
            "cso", "env-cys-role", 0)
# Rust 는 `+12` 를 받지만 우리는 **일부러 더 엄격**하다 — 거절의 귀결은 조회 없음 → env 폴백
# (= 이 WP 이전 동작)이라 새 허용이 없다.
helper_case("B leading-plus identity rejected (stricter than Rust, no new allow)",
            {"CYS_SURFACE_ID": "+12", "CYS_ROLE": "cso"}, "cso", "env-cys-role", 0)


@case("B leading zeros normalize to one cache key")
def leading_zero_identity(box):
    box.configure(CYS_SURFACE_ID="0012")
    box.resolve("cso", "daemon")
    equal(sorted(p.name for p in box.cache_dir().iterdir()), [box.cache().name],
          "선두 0 은 캐시 키에서 정규화된다")


@case("B CYS identity wins over JAVIS and AITERM")
def identity_priority(box):
    box.configure(JAVIS_SURFACE_ID="34", AITERM_SURFACE_ID="56")
    box.resolve("cso", "daemon")
    equal(box.calls(), ["1\tsurface-role"], "조회")
    equal(sorted(p.name for p in box.cache_dir().iterdir()), [box.cache().name],
          "선택된 신원 캐시 키")


helper_case("C CYS_SURFACE_ROLE cannot override CYS_ROLE", {"CYS_SURFACE_ROLE": "cso", "CYS_ROLE": "worker", "STUB_RC": 2}, "worker", "env-cys-role")
helper_case("D child export / parent remains unset (bash)", {}, "cso", "daemon", shell="bash")


@case("D existing parent export preserved")
def parent_preserved(box):
    box.configure(CYS_NO_AUTOSTART="parent-value")
    box.resolve("cso", "daemon", parent="parent-value")
    equal(box.calls(), ["1\tsurface-role"], "자식 export")


helper_case("E resolver stdout is zero bytes", {"STUB_OUT": "cso\nignored"}, "cso", "daemon")


@case("F failure marker / second call suppressed")
def backoff(box):
    box.configure(STUB_RC=2, CYS_ROLE="master")
    box.resolve("master", "env-cys-role")
    marker = Path(str(box.cache()) + ".fail")
    line = marker.read_text().rstrip("\n")
    fields = line.split(" ", 3)
    equal(len(fields), 4, f"실패표식 4필드 문법: {line!r}")
    equal(fields[1], "-", "실패표식 값")
    equal(fields[2], box.epoch(), "실패표식 세대")
    equal(fields[3], box.sock_id(), "실패표식 데몬 신원")
    equal(abs(int(fields[0]) - int(time.time())) < 10, True, "실패 시각")
    box.configure(STUB_RC=0)
    box.resolve("master", "env-cys-role")
    equal(box.calls(), ["1\tsurface-role"], "백오프 중 로그 줄 수")


for kind in ("future", "malformed", "symlink", "expired",
             "leading-zero-ts", "spaced-role", "other-socket", "other-epoch",
             "three-fields", "fifo"):
    @case(f"G {kind} cache ignored")
    def bad_cache(box, kind=kind):
        box.cache_dir().mkdir(mode=0o700, exist_ok=True)
        target = content = None
        if kind == "malformed":
            box.cache().write_text("noSpaceLine\n")
        elif kind == "symlink":
            target = box.home / "cache-target"
            content = box.record("worker")
            target.write_text(content)
            box.cache().symlink_to(target)
        elif kind == "leading-zero-ts":
            # ★선두 0 타임스탬프는 bash 산술에서 8진수로 읽혀 "value too great for base"로
            #   죽었다(reviewer-codex). 문법이 거절하므로 산술에 닿지 않는다.
            box.cache().write_text(box.record("worker", ts="01780000000"))
        elif kind == "spaced-role":
            # ★공백을 남기던 종전 문법에서는 셸이 "cso "를, 파이썬이 "cso"를 읽어 갈렸다.
            box.cache().write_text(box.record("cso "))
        elif kind == "other-socket":
            box.cache().write_text(box.record("worker", sock="/tmp/other-daemon.sock"))
        elif kind == "other-epoch":
            box.cache().write_text(box.record("worker", epoch="boot-other"))
        elif kind == "three-fields":
            box.cache().write_text("%d worker -\n" % int(time.time()))
        elif kind == "fifo":
            # ★FIFO 는 판독을 영원히 붙잡을 수 있었다 — `-f` 가 거절하고, 파이썬 짝은
            #   O_NONBLOCK + fstat 로 거절한다. 어느 쪽도 매달리지 않는다(20s 타임아웃이 증인).
            import os as _os
            _os.mkfifo(str(box.cache()), 0o600)
        else:
            box.seed("worker", age=-9999 if kind == "future" else 61)
        box.resolve("cso", "daemon")
        equal(box.calls(), ["1\tsurface-role"], "무효 캐시 조회")
        if kind == "symlink":
            equal(target.read_text(), content, "심링크 대상 불변")


@case("cache-none round trip / one-line cache format")
def cache_none(box):
    box.configure(STUB_OUT="", CYS_ROLE="cso")
    box.resolve("", "daemon-none")
    equal(bool(re.fullmatch(r"[1-9][0-9]{0,11} - %s %s\n" % (re.escape(box.epoch()),
                                                             re.escape(box.sock_id())),
                            box.cache().read_text())), True,
          f"무역할 캐시 형식: {box.cache().read_text()!r}")
    box.resolve("", "cache-none")
    equal(box.calls(), ["1\tsurface-role"], "무역할 캐시 무조회")


@case("30-second backoff expires")
def expired_backoff(box):
    box.cache_dir().mkdir(mode=0o700, exist_ok=True)
    marker = Path(str(box.cache()) + ".fail")
    marker.write_text(box.record("-", age=31))
    box.resolve("cso", "daemon")
    equal(box.calls(), ["1\tsurface-role"], "백오프 만료 조회")
    equal(marker.exists(), False, "성공 후 실패표식 제거")


@case("socket and bootepoch cache key")
def salted_cache(box):
    box.configure(CYS_SOCKET=str(box.home / "test.sock"))
    epoch = box.home / "boot-epoch"
    epoch.write_text("boot-1\n")
    box.resolve("cso", "daemon")
    equal(bool(re.fullmatch(r"[1-9][0-9]{0,11} cso boot-1 %s\n" % re.escape(box.sock_id()),
                            box.cache().read_text())), True,
          f"키·캐시 형식: {box.cache().read_text()!r}")
    epoch.write_text("boot-2\n")
    box.configure(STUB_OUT="worker")
    box.resolve("worker", "daemon")
    equal(box.calls(), ["1\tsurface-role"] * 2, "bootepoch 변경 재조회")
    equal(sorted(p.name for p in box.cache_dir().iterdir()), [box.cache().name],
          "★세대는 파일명이 아니라 레코드에 있다 — 재기동 고아 0")


def dept_case(name, env, denied, queries, args=("down", "some-dept"), missing=False,
              rc=0, prepare=None):
    @case(name)
    def check(box):
        box.configure(**env)
        if missing:
            empty = box.home / "empty-pack"
            empty.mkdir()
            box.configure(CYS_PACK_DIR=str(empty))
        if prepare:
            prepare(box)
        box.dept(denied, queries, args, passthrough_rc=rc)
        if args in (("list",), ("promote-if-pending", "--request-only")):
            equal(box.calls(), [], "읽기 전용 전체 스텁 로그")


dept_case("H1 cso / worker denied", {"CYS_ROLE": "cso", "STUB_OUT": "worker"}, True, 1)
dept_case("H2 cso / cso allowed", {"CYS_ROLE": "cso"}, False, 1)
dept_case("H3 cso / daemon-none denied", {"CYS_ROLE": "cso", "STUB_OUT": ""}, True, 1)
dept_case("H4 no env / daemon-none allowed", {"STUB_OUT": ""}, False, 1)
dept_case("H5 cso / rc2 allowed", {"CYS_ROLE": "cso", "STUB_RC": 2}, False, 1)
dept_case("H6 master / cso still denied before query", {"CYS_ROLE": "master"}, True, 0)
# ★K: rotate 면제는 **상속되지 않는 argv** 다(0.14.31 P6 R1 · 두 리뷰어 blocking).
# 2026-09-08 라이브 실측: dept-2 cysd(pid 2634)와 그 좌석 3기(4147/5087/7981)가 전부
# `CYS_DEPT_ROTATE=1` 을 물고 있었다 — 종전 판에서는 그 부서의 모든 pane 에서 이 게이트가
# **영구 no-op** 이었다(정본 §3-4 "게이트를 끄는 노브 없음" 위반). 아래 K1 이 그 재발을 막는다.
dept_case("K1 ★inherited CYS_DEPT_ROTATE no longer disables the gate",
          {"CYS_ROLE": "cso", "STUB_OUT": "worker", "CYS_DEPT_ROTATE": 1}, True, 1)
dept_case("H8 no surface / no env allowed", {"CYS_SURFACE_ID": None, "STUB_OUT": "worker"}, False, 0)
dept_case("H9 no surface / cso allowed", {"CYS_SURFACE_ID": None, "CYS_ROLE": "cso", "STUB_OUT": "worker"}, False, 0)
dept_case("I list never queries", {"CYS_ROLE": "cso", "STUB_OUT": "worker"}, False, 0, ("list",))
dept_case("I request-only never queries", {"CYS_ROLE": "master", "STUB_OUT": "worker"}, False, 0, ("promote-if-pending", "--request-only"))
dept_case("J missing prelude retains cso permission", {"CYS_ROLE": "cso", "STUB_OUT": "worker"}, False, 0, missing=True)
dept_case("J missing prelude retains master rejection", {"CYS_ROLE": "master"}, True, 0, missing=True)
dept_case("K2 inherited CYS_DEPT_ROTATE keeps the env clause too",
          {"CYS_ROLE": "master", "CYS_DEPT_ROTATE": 1}, True, 0)
dept_case("K3 --rotate is launch-only (down still denied)",
          {"CYS_ROLE": "cso", "STUB_OUT": "worker"}, True, 1,
          args=("down", "some-dept", "--rotate"))
# launch 계열은 **이름 검증에서 죽는 이름**만 쓴다(부작용 0). 가드(7)가 이름 검증(2)보다 앞이라
# rc 하나로 면제 여부가 읽힌다.
dept_case("K4 launch denied at the gate before name validation",
          {"CYS_ROLE": "cso", "STUB_OUT": "worker"}, True, 1, args=("launch", "bad name"))
dept_case("K5 ★argv --rotate exempts launch (falls through to name validation rc=2)",
          {"CYS_ROLE": "cso", "STUB_OUT": "worker"}, False, 0,
          args=("launch", "bad name", "--rotate"), rc=2)


def _plant_socket(box):
    sock = box.home / ".local" / "state" / "cys-dept-bad name" / "cys.sock"
    sock.parent.mkdir(parents=True, exist_ok=True)
    sock.write_text("")


dept_case("K6 ★--rotate is not honored while the dept socket still exists",
          {"CYS_ROLE": "cso", "STUB_OUT": "worker"}, True, 1,
          args=("launch", "bad name", "--rotate"), prepare=_plant_socket)


def main():
    failures = 0
    for name, check in CASES:
        try:
            # 케이스마다 새 루트 — 케이스 내부 연속 호출만 캐시를 공유한다. 샌드박스가 자기
            # `TMPDIR`(root/tmp)을 따로 세우므로 ambient 캐시는 어차피 닿지 않는다.
            # (`dir="/tmp"` 하드코딩은 걷어냈다 — 그 경로가 없거나 못 쓰는 러너에서 전멸한다.)
            with tempfile.TemporaryDirectory(prefix="role-authority-shell-") as root:
                check(Sandbox(Path(root)))
        except Exception as error:
            failures += 1
            detail = str(error).replace("\n", "\\n")
            print(f"FAIL {name}: {type(error).__name__}: {detail}", flush=True)
        else:
            print(f"PASS {name}", flush=True)
    if failures:
        return 1
    print("ROLE-AUTHORITY-SHELL-OK", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
