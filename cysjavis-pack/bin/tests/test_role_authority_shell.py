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
G 미래·손상·심링크 캐시 배제, H 단조 거부 9종·rotate 면제,
I list/request-only 무조회, J 프리루드 부재 강등. 추가로 cache-none,
60초 TTL 만료, 백오프 만료, 소켓·bootepoch 키, sh/bash set -u를 핀한다.
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

    def cache(self, surface="12"):
        socket = self.env.get("CYS_SOCKET", "")
        epoch = ""
        if socket:
            epoch_file = Path(socket).parent / "boot-epoch"
            if epoch_file.exists():
                epoch = epoch_file.read_text().splitlines()[0]
        slug = lambda value: re.sub(r"[^A-Za-z0-9._-]", "_", value)
        return self.tmp / f"cys-role-authority-{surface}-{slug(socket or 'none')}-{slug(epoch)}"

    def seed(self, value="cso", age=0):
        self.cache().write_text(f"{int(time.time()) - age} {value}\n", encoding="utf-8")

    def dept(self, denied, queries, args=("down", "some-dept")):
        equal(args in (("down", "some-dept"), ("list",),
                       ("promote-if-pending", "--request-only")), True, "허용 동사")
        result = self.run(["/bin/bash", str(DEPT), *args])
        if denied:
            equal(result.returncode, 7, f"dept 거부; stderr={result.stderr!r}")
        else:
            # rc != 7만 검사하면 셸/하네스 오류도 통과하므로 정상 no-op까지 확인한다.
            equal(result.returncode, 0, f"dept 통과; stderr={result.stderr!r}")
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


@case("B CYS identity wins over JAVIS and AITERM")
def identity_priority(box):
    box.configure(JAVIS_SURFACE_ID="34", AITERM_SURFACE_ID="56")
    box.resolve("cso", "daemon")
    equal(box.calls(), ["1\tsurface-role"], "조회")
    equal(sorted(p.name for p in box.tmp.iterdir()), [box.cache().name], "선택된 신원 캐시 키")


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
    fields = marker.read_text().split()
    equal(len(fields), 2, "실패표식 형식")
    equal(fields[1], "-", "실패표식 값")
    equal(abs(int(fields[0]) - int(time.time())) < 10, True, "실패 시각")
    box.configure(STUB_RC=0)
    box.resolve("master", "env-cys-role")
    equal(box.calls(), ["1\tsurface-role"], "백오프 중 로그 줄 수")


for kind in ("future", "malformed", "symlink", "expired"):
    @case(f"G {kind} cache ignored")
    def bad_cache(box, kind=kind):
        if kind == "malformed":
            box.cache().write_text("noSpaceLine\n")
        elif kind == "symlink":
            target = box.home / "cache-target"
            content = f"{int(time.time())} worker\n"
            target.write_text(content)
            box.cache().symlink_to(target)
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
    equal(bool(re.fullmatch(r"[0-9]+ -\n", box.cache().read_text())), True, "무역할 캐시 형식")
    box.resolve("", "cache-none")
    equal(box.calls(), ["1\tsurface-role"], "무역할 캐시 무조회")


@case("30-second backoff expires")
def expired_backoff(box):
    marker = Path(str(box.cache()) + ".fail")
    marker.write_text(f"{int(time.time()) - 31} -\n")
    box.resolve("cso", "daemon")
    equal(box.calls(), ["1\tsurface-role"], "백오프 만료 조회")
    equal(marker.exists(), False, "성공 후 실패표식 제거")


@case("socket and bootepoch cache key")
def salted_cache(box):
    box.configure(CYS_SOCKET=str(box.home / "test.sock"))
    epoch = box.home / "boot-epoch"
    epoch.write_text("boot-1\n")
    box.resolve("cso", "daemon")
    equal(bool(re.fullmatch(r"[0-9]+ cso\n", box.cache().read_text())), True, "키·캐시 형식")
    epoch.write_text("boot-2\n")
    box.configure(STUB_OUT="worker")
    box.resolve("worker", "daemon")
    equal(box.calls(), ["1\tsurface-role"] * 2, "bootepoch 변경 재조회")


def dept_case(name, env, denied, queries, args=("down", "some-dept"), missing=False):
    @case(name)
    def check(box):
        box.configure(**env)
        if missing:
            empty = box.home / "empty-pack"
            empty.mkdir()
            box.configure(CYS_PACK_DIR=str(empty))
        box.dept(denied, queries, args)
        if args != ("down", "some-dept"):
            equal(box.calls(), [], "읽기 전용 전체 스텁 로그")


dept_case("H1 cso / worker denied", {"CYS_ROLE": "cso", "STUB_OUT": "worker"}, True, 1)
dept_case("H2 cso / cso allowed", {"CYS_ROLE": "cso"}, False, 1)
dept_case("H3 cso / daemon-none denied", {"CYS_ROLE": "cso", "STUB_OUT": ""}, True, 1)
dept_case("H4 no env / daemon-none allowed", {"STUB_OUT": ""}, False, 1)
dept_case("H5 cso / rc2 allowed", {"CYS_ROLE": "cso", "STUB_RC": 2}, False, 1)
dept_case("H6 master / cso still denied before query", {"CYS_ROLE": "master"}, True, 0)
dept_case("H7 rotate recursion exempts daemon clause", {"CYS_ROLE": "cso", "STUB_OUT": "worker", "CYS_DEPT_ROTATE": 1}, False, 0)
dept_case("H8 no surface / no env allowed", {"CYS_SURFACE_ID": None, "STUB_OUT": "worker"}, False, 0)
dept_case("H9 no surface / cso allowed", {"CYS_SURFACE_ID": None, "CYS_ROLE": "cso", "STUB_OUT": "worker"}, False, 0)
dept_case("I list never queries", {"CYS_ROLE": "cso", "STUB_OUT": "worker"}, False, 0, ("list",))
dept_case("I request-only never queries", {"CYS_ROLE": "master", "STUB_OUT": "worker"}, False, 0, ("promote-if-pending", "--request-only"))
dept_case("J missing prelude retains cso permission", {"CYS_ROLE": "cso", "STUB_OUT": "worker"}, False, 0, missing=True)
dept_case("J missing prelude retains master rejection", {"CYS_ROLE": "master"}, True, 0, missing=True)
dept_case("rotate exemption retains env rejection", {"CYS_ROLE": "master", "CYS_DEPT_ROTATE": 1}, True, 0)


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
