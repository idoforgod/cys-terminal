//! cysjavis-pack의 git-추적 전체 트리를 컴파일 타임에 자동 임베드하는 매니페스트 생성기.
//!
//! 파일을 pack.rs 목록에 손으로 추가하는 방식은 임베드 드리프트(소스 수정 후 목록 누락 →
//! 신규 머신에 구버전/누락 배포)의 원천이라, `git ls-files cysjavis-pack`(추적전용) 소싱으로
//! 결정론 환원한다. cysjavis-pack/ 아래에 파일을 두고 git add 하면 빌드가 자동 임베드한다.
//! 추적 집합을 SOT로 삼아 gitignore(개인정보) 경계를 구조적으로 강제하고, untracked 개인파일은
//! 임베드하지 않는다.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    // cysjavis-pack 전체를 임베드한다. 소스 = git 인덱스(`git ls-files`) — 디렉터리 워크가
    // 아니라 추적 집합을 SOT로 삼아 gitignore 경계를 그대로 따른다. 어떤 파일이든 변경 시 재빌드.
    println!("cargo:rerun-if-changed=cysjavis-pack");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR 없음");
    let output = Command::new("git")
        .args(["ls-files", "cysjavis-pack"])
        .current_dir(&manifest_dir)
        .output();
    let stdout = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => String::new(),
    };
    // ★가드①: git 명령 실패/빈 출력 → 빈 pack 출하(비-hermetic) 차단. loud fail.
    if stdout.trim().is_empty() {
        panic!("pack 소스 비었음 — git 인덱스 부재? 빌드 중단");
    }

    // 추적 파일 → cysjavis-pack/ 접두 제거한 rel. 제외규칙(기존 walk와 동형): 경로 컴포넌트가
    // '.'로 시작(.gitignore 등 dotfile/dotdir)·tests·__pycache__ 이면 배포 대상이 아니다.
    let mut rels: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        let Some(rel) = line.strip_prefix("cysjavis-pack/") else {
            continue;
        };
        if rel
            .split('/')
            .any(|c| c.starts_with('.') || c == "tests" || c == "__pycache__")
        {
            continue;
        }
        rels.push(rel.to_string());
    }
    rels.sort();
    rels.dedup();

    // ★가드②: 임베드 엔트리 < 250 → 비정상 빈-pack(빌드 이상)으로 보고 차단.
    if rels.len() < 250 {
        panic!(
            "pack 임베드 엔트리 {}개 < 250 — 비정상(빌드 이상?). 빌드 중단",
            rels.len()
        );
    }

    let mut code = String::from(
        "/// build.rs 자동 생성 — cysjavis-pack git-추적 전체 트리 임베드 (수동 목록 드리프트 차단).\n\
         pub const PACK_ALL: &[(&str, &str)] = &[\n",
    );
    for rel in &rels {
        code.push_str(&format!(
            "    (\"{rel}\", include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/cysjavis-pack/{rel}\"))),\n"
        ));
    }
    code.push_str("];\n");

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR 없음");
    fs::write(Path::new(&out_dir).join("pack_all.rs"), code).expect("pack_all.rs 생성 실패");

    // T1-2: 단일진실 enum → OUT_DIR/cys_kinds.json (스키마·검증기 파리티의 기준).
    // 기존 디렉터리스캔 코드젠 철학과 동형(손목록 드리프트 차단). enum 정의는 src/edit_kinds.rs가
    // 진실이나 build.rs는 컴파일 전이라 그 타입을 못 본다 → 리터럴 목록을 여기 둔다(serde_json
    // build-dep 불요 — 평문 JSON 문자열). edit_kinds.rs enum과 어긋나면 tests/round-trip이 fail
    // (이중 잠금: 한쪽만 고치면 빨개짐). 추가 인프라 0 — std fs::write만.
    println!("cargo:rerun-if-changed=src/edit_kinds.rs");
    let kinds_json = "{\n  \"edit_kind\": [\"avatar\", \"broll\", \"graphic\", \"caption\", \"audio\", \"music\"],\n  \"mode\": [\"fullscreen\", \"left-card\", \"rounded-crop-pip\"],\n  \"transition\": [\"cut\", \"dissolve\", \"slide\"]\n}\n";
    fs::write(Path::new(&out_dir).join("cys_kinds.json"), kinds_json).expect("cys_kinds.json 생성 실패");

    // §7-①/⑩: minisign 신뢰 키링 embed. 공개키 단일 SOT = src-tauri/tauri.conf.json(updater.pubkey).
    // build.rs가 그 pubkey를 회전용 키링(cysjavis-pack/trusted-keys.json)의 부트스트랩 엔트리
    // (pubkey "")에 주입해 병합 → OUT_DIR 상수로 방출(skills walk와 동형 코드젠·손목록 드리프트 0).
    // 키를 두 곳에 두지 않으므로 양쪽 동일 보장. 기존 skills/kinds 코드젠은 불변(추가만).
    println!("cargo:rerun-if-changed=src-tauri/tauri.conf.json");
    println!("cargo:rerun-if-changed=cysjavis-pack/trusted-keys.json");
    let tauri_conf =
        fs::read_to_string("src-tauri/tauri.conf.json").expect("tauri.conf.json 읽기 실패");
    let pubkey = extract_json_string(&tauri_conf, "pubkey")
        .expect("tauri.conf.json updater.pubkey 부재 — 키링 embed 불가");
    let keyring_src =
        fs::read_to_string("cysjavis-pack/trusted-keys.json").expect("trusted-keys.json 읽기 실패");
    // 부트스트랩 엔트리의 빈 pubkey("")에 tauri pubkey 주입(단일 SOT 유지).
    let keyring = keyring_src.replace("\"pubkey\": \"\"", &format!("\"pubkey\": \"{pubkey}\""));
    let keyring_code = format!(
        "/// build.rs 자동 생성 — minisign 신뢰 키링(tauri.conf.json pubkey + trusted-keys.json 병합).\npub const TRUSTED_KEYS_JSON: &str = r####\"{keyring}\"####;\n"
    );
    fs::write(Path::new(&out_dir).join("pack_keyring.rs"), keyring_code)
        .expect("pack_keyring.rs 생성 실패");

    // DESIGN-pro-license §5: pro 라이선스 폐기 명단 embed + ★빌드타임 형태 검증.
    // 손상·형태 불일치 폐기 명단은 빌드 실패로 출하 자체를 차단한다(런타임 도달 0).
    // 소스는 repo 루트(팩 트리 밖 — pro 팩에 사본을 두지 않는 단일 SOT).
    println!("cargo:rerun-if-changed=revoked-licenses.json");
    let revoked_src =
        fs::read_to_string("revoked-licenses.json").expect("revoked-licenses.json 읽기 실패");
    let parsed: serde_json::Value = serde_json::from_str(&revoked_src)
        .expect("revoked-licenses.json 파싱 실패 — 손상 폐기 명단 출하 금지(빌드 중단)");
    let ids = parsed
        .get("revoked_license_ids")
        .and_then(|v| v.as_array())
        .expect("revoked-licenses.json에 revoked_license_ids 배열 부재 — 빌드 중단");
    for id in ids {
        if !id.is_string() {
            panic!("revoked_license_ids에 문자열 아닌 항목: {id} — 빌드 중단");
        }
    }
    let revoked_code = format!(
        "/// build.rs 자동 생성 — revoked-licenses.json embed(빌드타임 형태 검증 통과본).\npub const REVOKED_LICENSES_JSON: &str = r####\"{revoked_src}\"####;\n"
    );
    fs::write(Path::new(&out_dir).join("license_revoked.rs"), revoked_code)
        .expect("license_revoked.rs 생성 실패");
}

/// tauri.conf.json 등에서 `"key": "value"` 첫 매치의 value를 추출(JSON 파서 build-dep 없이).
/// minisign base64 pubkey엔 `"`가 없어 안전. updater.pubkey가 파일 내 유일한 "pubkey"다.
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = json.find(&needle)? + needle.len();
    let after_colon = &json[start..][json[start..].find(':')? + 1..];
    let q1 = after_colon.find('"')? + 1;
    let q2 = after_colon[q1..].find('"')? + q1;
    Some(after_colon[q1..q2].to_string())
}
