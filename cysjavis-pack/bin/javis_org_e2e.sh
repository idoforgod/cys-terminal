#!/usr/bin/env bash
# 격리 HOME E2E: compile더미→validate→(apply/status/destroy는 dry 검증). 라이브 데몬 무접촉.
set -euo pipefail
T=$(mktemp -d)
export CYS_DEPT_CATALOG="$T/catalog.json"
export CYS_DEPTS_JSON="$T/depts.json"
export CYS_DEPT_MISSIONS="$T/missions"
BIN="$(cd "$(dirname "$0")" && pwd)/javis_org.py"

# 더미 합의 md + 매니페스트
DOC="$T/org-design.md"
cat > "$DOC" <<'MD'
# 조직 설계
미래연구부는 모든 통찰의 원천 엔진으로 가동한다 충분히 길게.
첫 작업: 미래연구부는 종교·교회의 미래 환경 스캐닝을 수행한다 충분히 길게.
MD
SHA=$(python3 -c "import hashlib;print(hashlib.sha256(open('$DOC',encoding='utf-8').read().encode()).hexdigest())")
# catalog에 future-research 등록(good=기존key) — exploit의 shadow-ops는 미등록(fabricated)
echo '{"version":1,"accounts":{"acct-a":"x","owner":"y"},"departments":{"future-research":{"display":"미래연구부","account":"acct-a"}}}' > "$CYS_DEPT_CATALOG"
cat > "$T/m.json" <<JSON
{"manifest_version":1,"kind":"org-manifest","reconcile_mode":"additive",
 "source":{"design_doc":"$DOC","design_doc_sha256":"$SHA"},
 "departments":[{"key":"future-research","display":"미래연구부","account":"acct-a",
   "cwd":"$T/work/미래연구부","mission_md":"# 미션","source_quote":"미래연구부는 모든 통찰의 원천 엔진으로 가동한다 충분히 길게."}],
 "tasks":[{"dept":"future-research","to":"worker","task":"환경스캐닝","scope":"_round/",
   "source_quote":"첫 작업: 미래연구부는 종교·교회의 미래 환경 스캐닝을 수행한다 충분히 길게."}]}
JSON

echo "== self-test =="; python3 "$BIN" --self-test
echo "== validate (PASS 기대) =="; python3 "$BIN" validate "$T/m.json"
echo "== validate 풀익스플로잇 오귀속 (FAIL 기대) =="
# fabricated key(shadow-ops·미등록) + 실재 quote(미래연구부) + display 위장 + tasks 일관(v_refs 선점 제거)
# → F1 결속(역인덱스)·신규key 승인플래그가 실제 실행되는 경로로 차단되는지 독립 검증
python3 -c "import json;m=json.load(open('$T/m.json'));m['departments'][0]['key']='shadow-ops';m['tasks'][0]['dept']='shadow-ops';json.dump(m,open('$T/bad.json','w'))"
if python3 "$BIN" validate "$T/bad.json"; then echo "E2E FAIL: 풀익스플로잇 오귀속이 통과됨"; exit 1; else echo "  → 기대대로 FAIL"; fi
echo "== apply CSO 게이트 (exit3 기대·CYS_ROLE 없음) =="
if CYS_ROLE= python3 "$BIN" apply "$T/m.json"; then echo "E2E FAIL: 비-CSO apply 통과"; exit 1; else echo "  → 기대대로 차단"; fi

echo "== destroy workdir 부재 (rc0·skip 기대·R1 REVISE-2) =="
# ★cys-dept 스텁: 실제 down은 격리 reg_count==0 시 라이브 ceo_demote(MASTER_DIRECTIVE.md.pre-ceo)를
#   건드림 → 무접촉 보장 위해 스텁으로 대체(라이브 cys-dept 무호출).
mkdir -p "$T/bin"; printf '#!/bin/bash\necho "[stub cys-dept] $*"\nexit 0\n' > "$T/bin/cys-dept"; chmod +x "$T/bin/cys-dept"
echo '{"depts":{"e2e-dept":{"cwd":"'"$T"'/work/없는부서","socket":"'"$T"'/fake.sock","mission_key":"e2e-dept"}}}' > "$CYS_DEPTS_JSON"
OUT=$(PATH="$T/bin:$PATH" CYS_ROLE=cso python3 "$BIN" destroy --dept e2e-dept --purge-workdir); RC=$?
echo "$OUT"
if [ "$RC" != "0" ]; then echo "E2E FAIL: workdir 부재 destroy rc=$RC (영구 락인)"; exit 1; fi
echo "$OUT" | grep -q workdir_absent_skip || { echo "E2E FAIL: workdir_absent_skip 미포함"; exit 1; }
echo "  → 기대대로 rc0·skip"

echo "== mutation 본체 직접호출 CSO 게이트 (REVISE-1·exit3 기대) =="
# 게이트를 명령 래퍼가 아니라 '효과를 일으키는 함수'에 둠 — import 직접호출 우회 차단(defense-in-depth)
BINDIR="$(dirname "$BIN")"
if PATH="$T/bin:$PATH" CYS_ROLE=worker python3 -c "
import sys; sys.path.insert(0,'$BINDIR'); import javis_org
ok=True
for fn,args in [('apply_manifest',({},)),('destroy_dept',('x','x')),('create_dept',('x',))]:
    try: getattr(javis_org,fn)(*args); print('FAIL',fn,'no exit'); ok=False
    except SystemExit as e:
        if e.code!=3: print('FAIL',fn,'code',e.code); ok=False
    except Exception as ex: print('FAIL',fn,'raised',type(ex).__name__); ok=False
sys.exit(0 if ok else 1)
"; then echo "  → 3개 본체(apply_manifest·destroy_dept·create_dept) 모두 exit3 차단"; else echo "E2E FAIL: mutation 본체 무가드"; exit 1; fi

echo "ALL E2E PASS (라이브 무접촉)"; rm -rf "$T"
