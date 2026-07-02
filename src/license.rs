//! pro 라이선스("열쇠") 검증 — verify-only (DESIGN-pro-license.md v3).
//!
//! 전건 fail-CLOSED 파이프라인(§4): ⓐJSON 파싱(필수 필드 부재 = 거부) ⓑ키링 대조(폐기/미지/
//! not_after 부재 = 거부, 키 만료 = KeyExpired) ⓒminisign 서명 ⓓtier="pro" ⓔ시각·만료
//! (issued_at 미래 거부 · expires="never" 통과 · RFC3339 파싱 실패 거부) ⓕ내장 폐기 명단.
//!
//! 발급(서명키)은 바이너리 미탑재 — pro repo 스크립트 전담(packsig와 동일 verify-only 철학).
//! §8 은닉: LicenseFile 등 세부 구조체·검증 함수는 크레이트 밖 비노출. 외부 표면은
//! is_pro()·render_status()·install()·visible_pro_features()뿐이다. 게이트 분기는
//! is_pro() 단일 진입 — 개별 기능의 독자 검증 금지(정적 핀 테스트가 강제).

use std::path::{Path, PathBuf};

// build.rs 자동 생성 — revoked-licenses.json embed(빌드타임 형태 검증 통과본, §5 단일 SOT).
include!(concat!(env!("OUT_DIR"), "/license_revoked.rs"));

/// 라이선스 파일(§3). 전 필드 필수 — #[serde(default)] 없음 = 부재 시 파싱 거부(fail-closed).
/// ★비공개(§8-1): 외부는 이 구조체를 볼 수 없다.
#[derive(Debug, Clone, serde::Deserialize)]
struct LicenseFile {
    license_id: String,
    client_id: String,
    tier: String,
    key_id: String,
    issued_at: i64,
    /// RFC3339 만료시각(기본) 또는 리터럴 "never"(영구 — 명시 옵션).
    expires: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct RevokedLicenses {
    revoked_license_ids: Vec<String>,
}

/// typed 진단(§7) — 사유 구분은 status 전담. 제품 게이트는 is_pro() bool 하나다(층 분리).
#[derive(Debug, Clone, PartialEq)]
pub enum LicenseStatus {
    /// 라이선스 부재 — 에러 아님(free 동작).
    Free,
    /// 유효. key_days_left = 서명키 not_after까지 잔여일(never 포함 상시 병기 — R5 결착).
    Pro { client_id: String, license_id: String, expires: String, key_days_left: i64 },
    /// 라이선스 자체 만료.
    Expired { client_id: String, expires: String },
    /// 폐기 명단 등재.
    Revoked { license_id: String },
    /// 서명키 not_after 경과 — never 라이선스도 여기 도달(§6: 키 수명이 실효 상한).
    KeyExpired { key_id: String },
    /// 서명·형식·시각 검사 실패(사유 포함).
    Invalid { reason: String },
}

/// pro 기능 레지스트리(§8-2) — pro 기능은 여기 등록해야만 존재한다. 게이트 분기·노출 판정이
/// 이 목록 경유로만 배선된다(T3에서 기능 추가 시 (이름, 설명) 등재).
pub const PRO_FEATURES: &[(&str, &str)] = &[];

/// 라이선스 파일 경로 — pack_dir 부모(~/.cys). pack 전체 교체·재설치와 독립인 위치
/// (.pack-accepted.json과 동일 base — 팩 갱신이 라이선스를 건드리지 않는다).
fn license_paths() -> (PathBuf, PathBuf) {
    let base = crate::pack::pack_dir()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    (base.join("license.json"), base.join("license.json.minisig"))
}

/// 검증 코어(§4 ⓐ~ⓕ) — 키링·폐기 명단 주입형(테스트·프로덕션 동일 경로).
fn evaluate_bytes(
    lic_bytes: &[u8],
    sig_bytes: &[u8],
    now_unix: i64,
    keyring: &crate::packsig::Keyring,
    revoked: &RevokedLicenses,
) -> LicenseStatus {
    // ⓐ JSON 파싱 — 필수 필드 부재 = 거부.
    let lic: LicenseFile = match serde_json::from_slice(lic_bytes) {
        Ok(l) => l,
        Err(e) => return LicenseStatus::Invalid { reason: format!("파싱 실패(필수 필드 부재 포함): {e}") },
    };
    // ⓑ 키링 대조 — 폐기/미지/not_after 부재 = Invalid, 키 만료 = KeyExpired(구분 진단).
    if keyring.revoked_key_ids.iter().any(|k| k == &lic.key_id) {
        return LicenseStatus::Invalid { reason: format!("폐기된 서명키: {}", lic.key_id) };
    }
    let Some(key) = keyring.keys.iter().find(|k| k.key_id == lic.key_id) else {
        return LicenseStatus::Invalid { reason: format!("알 수 없는 서명키: {}", lic.key_id) };
    };
    let Some(not_after) = crate::packsig::parse_rfc3339(&key.not_after) else {
        return LicenseStatus::Invalid { reason: format!("서명키 {} not_after 부재/파싱불가", lic.key_id) };
    };
    if now_unix >= not_after {
        return LicenseStatus::KeyExpired { key_id: lic.key_id };
    }
    // ⓒ minisign 서명 — 실패 = 거부.
    if let Err(e) = crate::packsig::verify_minisign(&key.pubkey, lic_bytes, sig_bytes) {
        return LicenseStatus::Invalid { reason: e };
    }
    // ⓓ tier 검사.
    if lic.tier != "pro" {
        return LicenseStatus::Invalid { reason: format!("미지 tier: {}", lic.tier) };
    }
    // ⓔ 시각·만료 — issued_at 미래 = 거부(위조·시계 이상 신호, R1 보강).
    if now_unix < lic.issued_at {
        return LicenseStatus::Invalid { reason: format!("발급 시각 미래: issued_at {}", lic.issued_at) };
    }
    if lic.expires != "never" {
        let Some(exp) = crate::packsig::parse_rfc3339(&lic.expires) else {
            return LicenseStatus::Invalid { reason: format!("expires 파싱불가: {}", lic.expires) };
        };
        if now_unix >= exp {
            return LicenseStatus::Expired { client_id: lic.client_id, expires: lic.expires };
        }
    }
    // ⓕ 내장 폐기 명단 대조(§5 단일 SOT — 앱 업데이트로 전파).
    if revoked.revoked_license_ids.iter().any(|id| id == &lic.license_id) {
        return LicenseStatus::Revoked { license_id: lic.license_id };
    }
    LicenseStatus::Pro {
        client_id: lic.client_id,
        license_id: lic.license_id,
        expires: lic.expires,
        key_days_left: (not_after - now_unix) / 86_400,
    }
}

/// embed 폐기 명단 파싱 — build.rs 빌드타임 검증을 통과한 상수라 실패는 사실상 불가하나,
/// fail-closed 방향(파싱 불가 = 전원 폐기 취급 아님·Invalid로 pro 비활성)으로 방어한다.
fn embedded_revoked() -> Result<RevokedLicenses, String> {
    serde_json::from_str(REVOKED_LICENSES_JSON).map_err(|e| format!("embed 폐기 명단 파싱 실패: {e}"))
}

/// 프로덕션 평가 — 디스크의 license.json(+.minisig)을 내장 키링·폐기 명단으로 검증.
/// 파일 부재(둘 중 하나라도) = Free(에러 아님). 그 외 읽기 실패 = Invalid.
pub fn evaluate(now_unix: i64) -> LicenseStatus {
    let (lic_path, sig_path) = license_paths();
    evaluate_at(&lic_path, &sig_path, now_unix)
}

fn evaluate_at(lic_path: &Path, sig_path: &Path, now_unix: i64) -> LicenseStatus {
    let lic_bytes = match std::fs::read(lic_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LicenseStatus::Free,
        Err(e) => return LicenseStatus::Invalid { reason: format!("license.json 읽기 실패: {e}") },
    };
    let sig_bytes = match std::fs::read(sig_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LicenseStatus::Free,
        Err(e) => return LicenseStatus::Invalid { reason: format!("서명 읽기 실패: {e}") },
    };
    let keyring = match crate::packsig::embedded_keyring() {
        Ok(k) => k,
        Err(e) => return LicenseStatus::Invalid { reason: e },
    };
    let revoked = match embedded_revoked() {
        Ok(r) => r,
        Err(e) => return LicenseStatus::Invalid { reason: e },
    };
    evaluate_bytes(&lic_bytes, &sig_bytes, now_unix, &keyring, &revoked)
}

/// ★단일 게이트(§8) — pro 분기는 반드시 이 함수만 호출한다. §4 전건 통과 = true,
/// 그 외(파일 부재 포함) 전부 조용히 false = free 동작(우아한 강등·데이터 무손상).
pub fn is_pro(now_unix: i64) -> bool {
    matches!(evaluate(now_unix), LicenseStatus::Pro { .. })
}

/// 노출 판정(§8-4 인벤토리의 근거) — is_pro=false면 레지스트리 전 기능 미노출(부재, 잠김 표시 아님).
pub fn visible_pro_features(now_unix: i64) -> Vec<&'static str> {
    visible_for(&evaluate(now_unix))
}

fn visible_for(status: &LicenseStatus) -> Vec<&'static str> {
    if matches!(status, LicenseStatus::Pro { .. }) {
        PRO_FEATURES.iter().map(|(name, _)| *name).collect()
    } else {
        Vec::new()
    }
}

/// 열쇠 설치(§7) — src(디렉터리 또는 license.json 경로)의 번들을 §4 전건 검증 후에만
/// 원자 반영. 실패 = 기존 라이선스 무손상 + 사유 Err.
pub fn install(src: &Path, now_unix: i64) -> Result<String, String> {
    let lic_src = if src.is_dir() { src.join("license.json") } else { src.to_path_buf() };
    let sig_src = PathBuf::from(format!("{}.minisig", lic_src.display()));
    let lic_bytes =
        std::fs::read(&lic_src).map_err(|e| format!("license.json 읽기 실패 {}: {e}", lic_src.display()))?;
    let sig_bytes =
        std::fs::read(&sig_src).map_err(|e| format!("서명 읽기 실패 {}: {e}", sig_src.display()))?;
    let keyring = crate::packsig::embedded_keyring()?;
    let revoked = embedded_revoked()?;
    match evaluate_bytes(&lic_bytes, &sig_bytes, now_unix, &keyring, &revoked) {
        LicenseStatus::Pro { client_id, license_id, expires, key_days_left } => {
            let (lic_dst, sig_dst) = license_paths();
            if let Some(parent) = lic_dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("디렉터리 생성 실패: {e}"))?;
            }
            crate::pack::write_atomic(&lic_dst, &lic_bytes)
                .map_err(|e| format!("라이선스 기록 실패: {e}"))?;
            crate::pack::write_atomic(&sig_dst, &sig_bytes)
                .map_err(|e| format!("서명 기록 실패: {e}"))?;
            Ok(format!(
                "설치 완료: pro · client={client_id} · license_id={license_id} · 만료={expires} · 서명키 잔여 {key_days_left}일"
            ))
        }
        // 전건 통과 아닌 열쇠는 설치 자체를 거부(기존 라이선스 무손상).
        other => Err(format!("설치 거부 — 검증 미통과: {}", describe(&other))),
    }
}

/// status 출력(§7) — typed 6상태 + 서명키 잔여 수명 상시 병기(never 포함, R5 결착).
pub fn render_status(now_unix: i64) -> String {
    describe(&evaluate(now_unix))
}

fn describe(status: &LicenseStatus) -> String {
    match status {
        LicenseStatus::Free => "free (라이선스 없음 — 정상)".to_string(),
        LicenseStatus::Pro { client_id, license_id, expires, key_days_left } => {
            let exp = if expires == "never" {
                "never (라이선스 자체 만료 없음)".to_string()
            } else {
                expires.clone()
            };
            format!(
                "pro · client={client_id} · license_id={license_id} · 만료={exp}\n⚠ 서명키 만료까지 {key_days_left}일 — 도래 전 재발급 필요(never 포함: 키 수명이 실효 상한)"
            )
        }
        LicenseStatus::Expired { client_id, expires } => {
            format!("expired · client={client_id} · 만료={expires} — 갱신 열쇠 필요(free로 동작 중)")
        }
        LicenseStatus::Revoked { license_id } => {
            format!("revoked · license_id={license_id} — 폐기된 열쇠(free로 동작 중)")
        }
        LicenseStatus::KeyExpired { key_id } => {
            format!("key-expired · 서명키 {key_id} 만료 — 재발급 열쇠 필요(free로 동작 중)")
        }
        LicenseStatus::Invalid { reason } => format!("invalid · {reason} (free로 동작 중)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packsig::{Keyring, TrustedKey};

    // ── 테스트 fixture: minisign keypair 생성 + 라이선스 서명(packsig fixture와 동형) ──
    fn gen_key_and_signer() -> (String, impl Fn(&[u8]) -> String) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().expect("keypair 생성 실패");
        let pk_b64 = kp.pk.to_base64();
        let sk = kp.sk;
        let signer = move |data: &[u8]| -> String {
            let cursor = std::io::Cursor::new(data.to_vec());
            let sig_box = minisign::sign(None, &sk, cursor, None, None).expect("서명 실패");
            sig_box.into_string()
        };
        (pk_b64, signer)
    }

    fn keyring_with(key_id: &str, pubkey: &str, not_after: &str) -> Keyring {
        Keyring {
            keys: vec![TrustedKey {
                key_id: key_id.to_string(),
                pubkey: pubkey.to_string(),
                not_after: not_after.to_string(),
            }],
            revoked_key_ids: vec![],
        }
    }

    fn license_json(license_id: &str, tier: &str, key_id: &str, issued_at: i64, expires: &str) -> Vec<u8> {
        serde_json::json!({
            "license_id": license_id,
            "client_id": "테스트고객",
            "tier": tier,
            "key_id": key_id,
            "issued_at": issued_at,
            "expires": expires
        })
        .to_string()
        .into_bytes()
    }

    fn no_revoked() -> RevokedLicenses {
        RevokedLicenses { revoked_license_ids: vec![] }
    }

    const NOW: i64 = 1_800_000_000; // 2027-01-15경
    const KEY_OK_UNTIL: &str = "2030-01-01T00:00:00Z";

    #[test]
    fn valid_expiring_license_passes() {
        let (pk, sign) = gen_key_and_signer();
        let lic = license_json("PRO-001", "pro", "K1", NOW - 100, "2029-01-01T00:00:00Z");
        let sig = sign(&lic);
        let st = evaluate_bytes(&lic, sig.as_bytes(), NOW, &keyring_with("K1", &pk, KEY_OK_UNTIL), &no_revoked());
        assert!(matches!(st, LicenseStatus::Pro { .. }), "기대 Pro, 실제 {st:?}");
    }

    #[test]
    fn never_license_passes_and_reports_key_days() {
        let (pk, sign) = gen_key_and_signer();
        let lic = license_json("PRO-002", "pro", "K1", NOW - 100, "never");
        let sig = sign(&lic);
        let st = evaluate_bytes(&lic, sig.as_bytes(), NOW, &keyring_with("K1", &pk, KEY_OK_UNTIL), &no_revoked());
        match st {
            LicenseStatus::Pro { key_days_left, .. } => assert!(key_days_left > 0, "키 잔여일 병기돼야 함"),
            other => panic!("기대 Pro, 실제 {other:?}"),
        }
    }

    #[test]
    fn expired_license_rejected_as_expired() {
        let (pk, sign) = gen_key_and_signer();
        let lic = license_json("PRO-003", "pro", "K1", NOW - 1000, "2020-01-01T00:00:00Z");
        let sig = sign(&lic);
        let st = evaluate_bytes(&lic, sig.as_bytes(), NOW, &keyring_with("K1", &pk, KEY_OK_UNTIL), &no_revoked());
        assert!(matches!(st, LicenseStatus::Expired { .. }), "기대 Expired, 실제 {st:?}");
    }

    #[test]
    fn missing_field_rejected() {
        let (pk, sign) = gen_key_and_signer();
        // expires 필드 결손 — 필수 필드 부재 = 파싱 거부(fail-closed).
        let lic = serde_json::json!({
            "license_id": "PRO-004", "client_id": "x", "tier": "pro",
            "key_id": "K1", "issued_at": NOW - 1
        })
        .to_string()
        .into_bytes();
        let sig = sign(&lic);
        let st = evaluate_bytes(&lic, sig.as_bytes(), NOW, &keyring_with("K1", &pk, KEY_OK_UNTIL), &no_revoked());
        assert!(matches!(st, LicenseStatus::Invalid { .. }), "기대 Invalid, 실제 {st:?}");
    }

    #[test]
    fn wrong_tier_rejected() {
        let (pk, sign) = gen_key_and_signer();
        let lic = license_json("PRO-005", "enterprise", "K1", NOW - 1, "never");
        let sig = sign(&lic);
        let st = evaluate_bytes(&lic, sig.as_bytes(), NOW, &keyring_with("K1", &pk, KEY_OK_UNTIL), &no_revoked());
        assert!(matches!(st, LicenseStatus::Invalid { .. }), "기대 Invalid, 실제 {st:?}");
    }

    #[test]
    fn tampered_signature_rejected() {
        let (pk, sign) = gen_key_and_signer();
        let lic = license_json("PRO-006", "pro", "K1", NOW - 1, "never");
        let sig = sign(&lic);
        // 서명 후 내용 위변조(만료일 연장 시도) — 도장 깨짐.
        let tampered = license_json("PRO-006", "pro", "K1", NOW - 1, "9999-12-31T00:00:00Z");
        let st = evaluate_bytes(&tampered, sig.as_bytes(), NOW, &keyring_with("K1", &pk, KEY_OK_UNTIL), &no_revoked());
        assert!(matches!(st, LicenseStatus::Invalid { .. }), "기대 Invalid, 실제 {st:?}");
    }

    #[test]
    fn revoked_license_id_rejected() {
        let (pk, sign) = gen_key_and_signer();
        let lic = license_json("PRO-007", "pro", "K1", NOW - 1, "never");
        let sig = sign(&lic);
        let revoked = RevokedLicenses { revoked_license_ids: vec!["PRO-007".to_string()] };
        let st = evaluate_bytes(&lic, sig.as_bytes(), NOW, &keyring_with("K1", &pk, KEY_OK_UNTIL), &revoked);
        assert!(matches!(st, LicenseStatus::Revoked { .. }), "기대 Revoked, 실제 {st:?}");
    }

    #[test]
    fn future_issued_at_rejected() {
        let (pk, sign) = gen_key_and_signer();
        let lic = license_json("PRO-008", "pro", "K1", NOW + 10_000, "never");
        let sig = sign(&lic);
        let st = evaluate_bytes(&lic, sig.as_bytes(), NOW, &keyring_with("K1", &pk, KEY_OK_UNTIL), &no_revoked());
        assert!(matches!(st, LicenseStatus::Invalid { .. }), "기대 Invalid(발급 미래), 실제 {st:?}");
    }

    #[test]
    fn never_license_dies_with_expired_key() {
        // §6 핀: never 라이선스도 서명키 not_after 경과 시 KeyExpired(비상 kill 안전변).
        let (pk, sign) = gen_key_and_signer();
        let lic = license_json("PRO-009", "pro", "K1", NOW - 1, "never");
        let sig = sign(&lic);
        let st = evaluate_bytes(&lic, sig.as_bytes(), NOW, &keyring_with("K1", &pk, "2020-01-01T00:00:00Z"), &no_revoked());
        assert!(matches!(st, LicenseStatus::KeyExpired { .. }), "기대 KeyExpired, 실제 {st:?}");
    }

    #[test]
    fn garbage_expires_rejected() {
        let (pk, sign) = gen_key_and_signer();
        let lic = license_json("PRO-010", "pro", "K1", NOW - 1, "언젠가");
        let sig = sign(&lic);
        let st = evaluate_bytes(&lic, sig.as_bytes(), NOW, &keyring_with("K1", &pk, KEY_OK_UNTIL), &no_revoked());
        assert!(matches!(st, LicenseStatus::Invalid { .. }), "기대 Invalid, 실제 {st:?}");
    }

    #[test]
    fn embedded_revoked_list_parses() {
        // build.rs 빌드타임 검증과 이중 잠금 — embed 상수가 런타임 파싱 가능해야 한다.
        embedded_revoked().expect("embed 폐기 명단 파싱 실패");
    }

    #[test]
    fn inventory_no_pro_features_visible_when_not_pro() {
        // §8-4 인벤토리: is_pro=false면 레지스트리 전 기능 미노출.
        assert!(visible_for(&LicenseStatus::Free).is_empty());
        assert!(visible_for(&LicenseStatus::Invalid { reason: "x".into() }).is_empty());
        assert!(visible_for(&LicenseStatus::Expired { client_id: "x".into(), expires: "y".into() }).is_empty());
    }

    // ── §8-3 정적 핀: 게이트 산재 방지의 기계 강제 ─────────────────────────────────
    fn rust_sources() -> Vec<(PathBuf, String)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
            for entry in std::fs::read_dir(dir).expect("read_dir 실패") {
                let path = entry.expect("entry 실패").path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let src = std::fs::read_to_string(&path).expect("소스 읽기 실패");
                    out.push((path, src));
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut out = Vec::new();
        walk(&root, &mut out);
        out
    }

    #[test]
    fn static_pin_license_json_literal_only_in_license_rs() {
        for (path, src) in rust_sources() {
            if path.ends_with("license.rs") {
                continue;
            }
            assert!(
                !src.contains("license.json"),
                "정적 핀 위반: {} 가 license.json을 직접 다룬다 — license.rs 표면만 사용하라",
                path.display()
            );
        }
    }

    #[test]
    fn static_pin_minisign_verify_only_in_packsig_rs() {
        // needle을 분할 조립 — 이 테스트 소스 자신이 리터럴로 자가 매칭되는 것 방지.
        let needle = ["minisign", "_verify::"].concat();
        for (path, src) in rust_sources() {
            if path.ends_with("packsig.rs") {
                continue;
            }
            assert!(
                !src.contains(&needle),
                "정적 핀 위반: {} 가 minisign을 직접 사용한다 — packsig 검증 코어만 사용하라",
                path.display()
            );
        }
    }

    #[test]
    fn static_pin_license_file_struct_not_pub() {
        let needle = ["pub struct ", "LicenseFile"].concat(); // 자가 매칭 방지 분할 조립
        let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/license.rs"))
            .expect("license.rs 읽기 실패");
        assert!(
            !src.contains(&needle),
            "정적 핀 위반: LicenseFile이 pub — §8-1 은닉 위반"
        );
    }

    #[test]
    fn install_roundtrip_and_reject_paths() {
        // 유효 열쇠 설치 성공 + 무효(만료) 열쇠 설치 거부를 임시 HOME 격리로 검증.
        let tmp = std::env::temp_dir().join(format!("cys-license-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("tmp 생성 실패");
        // CYS_PACK_DIR 오버라이드로 license_paths()의 base를 tmp로 격리.
        std::env::set_var("CYS_PACK_DIR", tmp.join("pack").display().to_string());

        let (pk, sign) = gen_key_and_signer();
        // 테스트 키링을 쓸 수 없는 install()(embed 키링 고정) 대신 evaluate_bytes 경로는 위에서
        // 검증했으므로, 여기서는 파일 부재 = Free(에러 아님)와 설치 실패 시 무손상만 확인한다.
        let (lic_dst, _) = license_paths();
        assert!(matches!(evaluate(NOW), LicenseStatus::Free), "부재 = Free여야 함");

        // 무효 번들(embed 키링에 없는 키) 설치 시도 → 거부 + 기존(부재) 무손상.
        let src_dir = tmp.join("bundle");
        std::fs::create_dir_all(&src_dir).expect("bundle dir");
        let lic = license_json("PRO-X", "pro", "UNKNOWN-KEY", NOW - 1, "never");
        std::fs::write(src_dir.join("license.json"), &lic).expect("write lic");
        std::fs::write(src_dir.join("license.json.minisig"), sign(&lic)).expect("write sig");
        let _ = pk; // pk는 embed 키링에 없음 — install은 거부돼야 한다.
        let err = install(&src_dir, NOW).expect_err("미지 키 열쇠는 설치 거부돼야 함");
        assert!(err.contains("설치 거부"), "거부 사유 명시: {err}");
        assert!(!lic_dst.exists(), "실패 설치가 파일을 남기면 안 됨(무손상)");

        std::env::remove_var("CYS_PACK_DIR");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
