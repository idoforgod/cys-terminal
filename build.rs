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

    // ★W1 identity(3중 대조 field 1): 빌드 식별자 = git HEAD 짧은 SHA. 같은 빌드의 cys·cysd 는 동일 SHA →
    // 폴백 cys 가 데몬과 같은 빌드인지 교차대조하는 anti-shadowing 근거(embedded pack hash·protocol version 과 함께).
    // build.rs 는 이미 git 에 하드 의존(위 ls-files)하므로 추가 의존 없음. 실패 시 "unknown"(대조에서 불일치로 안전측).
    println!("cargo:rerun-if-changed=.git/HEAD");
    let build_id = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(&manifest_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=CYS_BUILD_ID={build_id}");

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

    // ★가드③(U-1 · CRLF 봉인 · 2026-08-23): 임베드 대상 텍스트에 CRLF 가 섞이면 **빌드를 죽인다**.
    //   `include_str!` 은 빌드 머신 작업 트리의 바이트를 **그대로** 팩에 넣는다. Git for Windows 의
    //   설치 기본값 `core.autocrlf=true` 아래에서 체크아웃이 CRLF 가 되면
    //     ① `cysjavis-pack/hooks/*.sh` 가 `#!/bin/bash\r` 로 출하돼 훅이 통째로 죽고,
    //     ② 같은 버전인데 Windows 임베드 팩과 ubuntu `pack.tar.gz` 의 바이트가 갈려
    //        매니페스트 해시가 흔들린다.
    //   레포 `.gitattributes`(`* text=auto eol=lf`)가 1차 방어이고, 이 가드는 그것이 없거나
    //   무시된 채로(개인 git 설정·zip 다운로드·아카이브 복원) **조용히 출하되는 것**을 막는다.
    //   검체 파리티: `cysjavis-pack/bin/tests/run_bootstrap_health.py` 의 `H-PACK-CRLF` 가 같은
    //   대상 집합에 같은 조건을 건다(빌드 게이트 ↔ 검체 게이트 2중).
    //   ★롤백 스위치는 이 env 1지점이다 — `CYS_ALLOW_CRLF_EMBED=1` 이면 중단하지 않고 경고만 낸다.
    const CRLF_ALLOW_ENV: &str = "CYS_ALLOW_CRLF_EMBED";
    println!("cargo:rerun-if-env-changed={CRLF_ALLOW_ENV}");
    println!("cargo:rerun-if-changed=.gitattributes");
    let crlf_allowed = env::var(CRLF_ALLOW_ENV).ok().as_deref() == Some("1");
    let mut crlf_hits: Vec<String> = Vec::new();
    let pack_root = Path::new(&manifest_dir).join("cysjavis-pack");
    // 팩 트리(= PACK_ALL 임베드 전량) + 팩 트리 밖에서 문자열로 임베드되는 레포 루트 파일.
    // `trusted-keys.json` 은 팩 트리 안이라 rels 에 이미 포함된다.
    let mut embed_targets: Vec<(String, std::path::PathBuf)> = rels
        .iter()
        .map(|rel| (format!("cysjavis-pack/{rel}"), pack_root.join(rel)))
        .collect();
    embed_targets.push((
        "revoked-licenses.json".to_string(),
        Path::new(&manifest_dir).join("revoked-licenses.json"),
    ));
    for (label, path) in &embed_targets {
        let Ok(bytes) = fs::read(path) else { continue };
        // NUL 을 품은 파일은 바이너리로 보고 제외한다(임베드 대상은 UTF-8 텍스트지만 방어적으로).
        if bytes.contains(&0u8) {
            continue;
        }
        if bytes.windows(2).any(|w| w == b"\r\n") {
            crlf_hits.push(label.clone());
        }
    }
    if !crlf_hits.is_empty() {
        let shown: Vec<&str> = crlf_hits.iter().take(20).map(|s| s.as_str()).collect();
        let more = crlf_hits.len().saturating_sub(shown.len());
        let msg = format!(
            "CRLF 임베드 차단(U-1) — 아래 {}개 파일이 CRLF 개행을 갖고 있다. 이대로 빌드하면 \
             `include_str!` 이 CRLF 를 그대로 팩에 넣어 훅 shebang(`#!/bin/bash\\r`)이 죽고 \
             플랫폼 간 팩 해시가 갈린다.\n  {}{}\n조치: ① 레포 루트 `.gitattributes`(`* text=auto eol=lf`)가 \
             있는지 확인 ② `git config core.autocrlf false` ③ `git rm --cached -r . && git reset --hard` \
             로 LF 재체크아웃(또는 해당 파일만 `dos2unix`). 의도적으로 우회하려면 \
             `{}=1` 로 빌드하라(경고로 강등).",
            crlf_hits.len(),
            shown.join("\n  "),
            if more > 0 {
                format!("\n  … 외 {more}개")
            } else {
                String::new()
            },
            CRLF_ALLOW_ENV
        );
        if crlf_allowed {
            println!("cargo:warning={}", msg.replace('\n', " | "));
        } else {
            panic!("{msg}");
        }
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

    // ── Windows 전용: PE 버전리소스(VERSIONINFO)·매니페스트·아이콘 임베드 ──
    // 목적: cys.exe·cysd.exe(순수 Rust CLI)에 .rsrc 섹션을 부여해 Microsoft Defender ML/SmartScreen
    // 평판을 정상화한다(무인증서·사용자 무조치). cys-app.exe(tauri-winres)는 이미 3종을 임베드해
    // 생존하나 루트 CLI 2종은 무버전·무매니페스트·무아이콘이라 평판이 낮았다.
    // ★게이트: 이 블록·winresource build-dep 모두 cfg(windows) 호스트 게이트다. build.rs는 호스트에서
    //   컴파일·실행되므로 #[cfg(target_os="windows")]는 호스트를 뜻한다 — cys의 모든 Windows 빌드 레그는
    //   windows-latest 네이티브(host==target=x86_64-pc-windows-msvc)라 host 게이트로 정확히 일치한다.
    //   macOS/Linux 호스트에선 winresource를 당기지도, 이 코드를 컴파일하지도 않아 바이트 무영향.
    #[cfg(target_os = "windows")]
    embed_windows_resources();
}

/// Windows PE 리소스(VERSIONINFO/manifest/icon)를 크레이트의 모든 bin(cys·cysd)에 임베드한다.
/// winresource::compile()은 `cargo:rustc-link-lib`/`cargo:rustc-link-search`를 방출하며, 이는 크레이트의
/// 모든 바이너리 타깃(cys·cysd)에 동일 리소스를 링크한다. winresource엔 per-bin `compile_for`가 없고
/// `write_resource_file`은 .rc 소스만 써서 링크 불가라, per-bin FileDescription 분리는 링크 위험 대비
/// 실익이 없어 미채택(공유 리소스로 두 bin 모두 버전·매니페스트·아이콘을 획득 — 목표 충족).
#[cfg(target_os = "windows")]
fn embed_windows_resources() {
    use winresource::WindowsResource;

    // 아이콘 재사용(tauri와 동일 SOT). 부재 시 무아이콘 출하(목표 미달)를 조용히 넘기지 않고 빌드 중단.
    let icon = "src-tauri/icons/icon.ico";
    assert!(
        Path::new(icon).exists(),
        "Windows 리소스 임베드용 아이콘 부재: {icon}"
    );
    println!("cargo:rerun-if-changed={icon}");
    println!("cargo:rerun-if-changed=build.rs");

    // 최소 app.manifest: requestedExecutionLevel=asInvoker(권한 상승 없음) + comctl32 v6 assemblyIdentity.
    const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="cysjavis.cys" version="0.0.0.0" processorArchitecture="*"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

    // 버전 문자열 SOT = Cargo.toml package.version. WindowsResource::new()이 CARGO_PKG_VERSION_*에서
    // FileVersion/ProductVersion을 자동 채우지만, 빌드 스크립트 런타임 env에서 명시로 다시 못박아
    // 하드코딩을 배제하고 SOT 연동을 분명히 한다.
    let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION 없음");

    let mut res = WindowsResource::new();
    res.set_icon(icon)
        .set("ProductName", "cys")
        .set("CompanyName", "cysjavis")
        .set("LegalCopyright", "Copyright (c) cysjavis — MIT License")
        .set("FileDescription", "cys command line / cys daemon (CYSJavis terminal)")
        .set("FileVersion", &version)
        .set("ProductVersion", &version)
        .set_manifest(MANIFEST);
    res.compile()
        .expect("Windows 리소스 컴파일 실패(rc.exe 부재?) — PE 메타데이터 임베드 불가");
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
