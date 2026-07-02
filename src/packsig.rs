//! 무중단 팩 채널 manifest 서명·신선도·replay 검증 (DESIGN-noshutdown-pack-update §7-①/⑩).
//!
//! 전건 fail-CLOSED: 어느 한 단계라도 불충족이면 거부(Err)한다.
//! - ⓐ JSON 파싱: key_id/signed_at/expires_at 필드 부재 = 거부(serde 필수 필드).
//! - ⓑ 키링 대조: 알 수 없음/revoked/만료(now>=not_after)/not_after 부재 = 거부.
//! - ⓒ minisign 서명: manifest_bytes를 sig_bytes로 검증, 실패 = 거부.
//! - ⓓ 신선도 유효창: now ∈ [signed_at, expires_at] 밖이면 거부(Replay 만료창).
//! - ⓔ accepted-pack replay 단조: signed_at ≤ 수용본.signed_at(또는 pack_version ≤ 수용본) = 거부.
//!   파일 부재(신규 머신) = 비교 불가 → 통과(time-window-bounded 한계, §7-⑩ 필수3).
//!   파일 존재하나 손상/읽기 실패 = fail-closed 거부(부재와 구분 — 손상본 신규 머신 강등 차단).
//!
//! 시각은 Unix epoch 초(i64)로 통일한다 — manifest의 signed_at/expires_at, now_unix, 키 not_after
//! (RFC3339 → epoch 변환)가 모두 i64로 비교된다.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

// build.rs 자동 생성 키링(tauri.conf.json pubkey + cysjavis-pack/trusted-keys.json 병합).
include!(concat!(env!("OUT_DIR"), "/pack_keyring.rs"));

/// 신뢰 키링 엔트리. `not_after`(RFC3339)는 ★전 서명키 필수(만료 없는 영구키 = fail-closed 위반).
#[derive(Debug, Clone, Deserialize)]
pub struct TrustedKey {
    pub key_id: String,
    /// minisign 공개키 — tauri식(전체 .pub 파일 base64) 또는 raw 키라인 base64 둘 다 수용.
    pub pubkey: String,
    /// RFC3339(예 "2030-01-01T00:00:00Z"). 부재·파싱불가 = 해당 키 거부.
    pub not_after: String,
}

/// 신뢰 키링: 활성 키 목록 + 긴급 폐기 목록(§7-⑩ B — not_after 만료 대기 없이 즉시 거부).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Keyring {
    #[serde(default)]
    pub keys: Vec<TrustedKey>,
    #[serde(default)]
    pub revoked_key_ids: Vec<String>,
}

/// 무중단 팩 manifest(서명 대상). key_id/signed_at/expires_at는 ★#[serde(default)] 없음 =
/// 필드 부재 시 역직렬화 실패 = 검증 거부(fail-closed, §7-⑩ 필수1).
#[derive(Debug, Clone, Deserialize)]
pub struct PackManifest {
    pub pack_version: String,
    #[serde(default)]
    pub min_binary_version: String,
    pub key_id: String,
    /// 서명 발행 시각(Unix epoch 초).
    pub signed_at: i64,
    /// 서명 만료 시각(Unix epoch 초).
    pub expires_at: i64,
    /// 배포 채널(free/pro 이원 배포 v6 §3) — "free"(기본 = 구 manifest 하위호환) | "pro".
    /// 그 외 값은 검증 ⓐ-2에서 거부(fail-closed).
    #[serde(default = "default_channel")]
    pub channel: String,
    /// pro 채널 단조 증분(base semver 동일 시의 증분 배포 축). free = 0 동치.
    #[serde(default)]
    pub pro_revision: u32,
    /// rel → sha256(파일 바이트). pack.rs content_hash와 동일 산식.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

pub(crate) fn default_channel() -> String {
    "free".to_string()
}

/// 마지막으로 수용한 팩 기록(~/.cys/.pack-accepted.json) — replay 단조 게이트의 기준선.
/// channel·pro_revision은 #[serde(default)] = 구 포맷 파일이 free/0으로 판독(v4 §3 마이그레이션
/// 명세 — 미지 필드 무시·구 파일 하위호환).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AcceptedPack {
    pack_version: String,
    signed_at: i64,
    #[serde(default = "default_channel")]
    channel: String,
    #[serde(default)]
    pro_revision: u32,
}

/// embed 키링(TRUSTED_KEYS_JSON) 파싱 — 프로덕션 검증 경로(verify_manifest)와 P4 `cys pack-update`가
/// 동일 신뢰근원을 공유한다(키 SOT 단일화). build.rs 병합 실패 시 Err.
pub fn embedded_keyring() -> Result<Keyring, String> {
    serde_json::from_str(TRUSTED_KEYS_JSON).map_err(|e| format!("embed 키링 파싱 실패: {e}"))
}

/// embed 키링(TRUSTED_KEYS_JSON)으로 manifest를 검증한다(프로덕션 진입점).
pub fn verify_manifest(
    manifest_bytes: &[u8],
    sig_bytes: &[u8],
    now_unix: i64,
    accepted_path: &Path,
) -> Result<PackManifest, String> {
    let keyring = embedded_keyring()?;
    verify_with_keyring(manifest_bytes, sig_bytes, now_unix, accepted_path, &keyring)
}

/// 검증 코어(전건 fail-CLOSED). 키링 주입형 — 테스트·키 회전 오버라이드·P4 오프라인 통합테스트가
/// 임의 키링을 넘긴다.
pub fn verify_with_keyring(
    manifest_bytes: &[u8],
    sig_bytes: &[u8],
    now_unix: i64,
    accepted_path: &Path,
    keyring: &Keyring,
) -> Result<PackManifest, String> {
    // ⓐ JSON 파싱 — 필수 필드(key_id/signed_at/expires_at) 부재 = 거부.
    let m: PackManifest = serde_json::from_slice(manifest_bytes)
        .map_err(|e| format!("manifest 파싱 실패(필수 필드 부재 포함): {e}"))?;

    // ⓐ-2 채널 검증(v6 §3): channel은 free|pro 단 둘 — 미지 값 거부(fail-closed).
    //     channel=pro는 min_binary_version **필수·파싱 가능**(version_gates 이전 거부 —
    //     구 바이너리 × 신 pro 팩 호환 파손 차단, R1 누락 보강 + R3 codex minor).
    if m.channel != "free" && m.channel != "pro" {
        return Err(format!("미지 channel: {} (free|pro만 유효)", m.channel));
    }
    if m.channel == "pro" && crate::pack::parse_semver(&m.min_binary_version).is_none() {
        return Err(format!(
            "channel=pro manifest는 min_binary_version 필수(비어있음/파싱불가: {:?})",
            m.min_binary_version
        ));
    }

    // ⓑ 키링 대조 — 폐기/미지/만료/not_after 부재 = 거부.
    if keyring.revoked_key_ids.iter().any(|k| k == &m.key_id) {
        return Err(format!("폐기된 key_id: {}", m.key_id));
    }
    let key = keyring
        .keys
        .iter()
        .find(|k| k.key_id == m.key_id)
        .ok_or_else(|| format!("알 수 없는 key_id: {}", m.key_id))?;
    let not_after = parse_rfc3339(&key.not_after)
        .ok_or_else(|| format!("key_id {} not_after 부재/파싱불가 — 키 거부", m.key_id))?;
    if now_unix >= not_after {
        return Err(format!(
            "만료된 키 {}: now {now_unix} >= not_after {not_after}",
            m.key_id
        ));
    }

    // ⓒ minisign 서명 검증 — 실패 = 거부.
    verify_minisign(&key.pubkey, manifest_bytes, sig_bytes)?;

    // ⓓ 신선도 유효창 — now ∈ [signed_at, expires_at] 밖이면 거부(Replay 만료창).
    if m.signed_at > m.expires_at {
        return Err(format!(
            "불가능 유효창: signed_at {} > expires_at {}",
            m.signed_at, m.expires_at
        ));
    }
    if now_unix < m.signed_at {
        return Err(format!(
            "서명 시각 미래: now {now_unix} < signed_at {}",
            m.signed_at
        ));
    }
    if now_unix > m.expires_at {
        return Err(format!(
            "서명 만료: now {now_unix} > expires_at {}",
            m.expires_at
        ));
    }

    // ⓔ accepted-pack replay 단조 게이트(★조건부 아님·항상). 파일 부재 = 통과(신규 머신 한계).
    //    존재하나 손상/읽기 실패 = fail-closed 거부(부재와 구분 — 손상본의 신규 머신 강등 차단).
    if let Some(acc) = read_accepted(accepted_path)? {
        if m.signed_at <= acc.signed_at {
            return Err(format!(
                "replay 거부: signed_at {} <= 수용본 signed_at {}",
                m.signed_at, acc.signed_at
            ));
        }
        // 보조 단조(v6 §3 튜플 확장): (base semver, pro_revision) 튜플이 수용본보다 **strictly
        // 낮으면** replay 거부. ★동일 튜플은 통과시킨다 — 1차 게이트가 signed_at > 수용본을 이미
        // 보장하므로, 동일 튜플 + 더 새 서명 = self-heal 후보(파일 반영은 version_gates가
        // UpToDate로 차단하고, accepted 갱신은 디스크 트리 해시 4조건 검사를 통과해야만 한다).
        // free 경로는 pro_revision=0 동치라 기존 `<=` 거부와 실효 동일(무회귀).
        if let (Some(new_v), Some(acc_v)) = (
            crate::pack::parse_semver(&m.pack_version),
            crate::pack::parse_semver(&acc.pack_version),
        ) {
            if (new_v, m.pro_revision) < (acc_v, acc.pro_revision) {
                return Err(format!(
                    "replay 거부: 튜플 {}(pro.{}) < 수용본 {}(pro.{})",
                    m.pack_version, m.pro_revision, acc.pack_version, acc.pro_revision
                ));
            }
        }
    }

    Ok(m)
}

/// 수용 시 {pack_version, signed_at, channel, pro_revision}을 원자 기록(pack::write_atomic).
/// 다음 검증의 replay 기준선 + repair 권위 증거(accepted.channel — v4 §5).
pub fn record_accepted(accepted_path: &Path, m: &PackManifest) -> Result<(), String> {
    let acc = AcceptedPack {
        pack_version: m.pack_version.clone(),
        signed_at: m.signed_at,
        channel: m.channel.clone(),
        pro_revision: m.pro_revision,
    };
    let json = serde_json::to_vec_pretty(&acc).map_err(|e| format!("accepted 직렬화 실패: {e}"))?;
    crate::pack::write_atomic(accepted_path, &json)
        .map_err(|e| format!("accepted 기록 실패 {}: {e}", accepted_path.display()))
}

/// repair 권위·자가치유 증거 판독(free/pro v4 §5) — accepted 기록의 (channel, pro_revision,
/// pack_version). 부재 = Ok(None)(순수 free 설치 — pack-update 이력 전무 = pro 증거 없음).
/// 손상 = Err(fail-closed — 증거 판독 불가는 보존 방향으로 처리하라는 신호).
pub fn read_accepted_evidence(path: &Path) -> Result<Option<(String, u32, String)>, String> {
    Ok(read_accepted(path)?.map(|a| (a.channel, a.pro_revision, a.pack_version)))
}

/// 파일별 sha256 == manifest.files 대조(§7-⑤ post-verify, staging 트리용·P4 호출).
/// content_hash는 pack.rs와 동일 산식: sha256(파일 바이트). 하나라도 불일치/누락 = 거부.
pub fn verify_files(manifest: &PackManifest, root: &Path) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    for (rel, want) in &manifest.files {
        let path = root.join(rel);
        let bytes =
            std::fs::read(&path).map_err(|e| format!("파일 읽기 실패 {}: {e}", path.display()))?;
        let got = format!("{:x}", Sha256::digest(&bytes));
        if &got != want {
            return Err(format!("sha256 불일치 {rel}: 기대 {want} 실제 {got}"));
        }
    }
    Ok(())
}

/// 역방향 커버리지(§7-①, §2-①): staging 트리의 ★모든★ 파일이 서명된 manifest.files에 등재돼야
/// 한다. tarball(pack.tar.gz) 자체는 미서명이고 manifest만 서명되므로, verify_files(전방: manifest
/// → staging)만으로는 manifest에 없는 파일이 staging에 추가된 변조(전송 중 변조·악성 --from 디렉터리)를
/// 탐지하지 못한다 — 예: 신규 bin/*.py가 install 시 exec-bit를 부여받아 서명·해시 0검증으로 설치될 수
/// 있다. verify_files(전방)와 합쳐 manifest ⇔ staging 집합 동치를 강제한다. 미등재 파일 = fail-closed 거부.
pub fn verify_no_extra_files(manifest: &PackManifest, root: &Path) -> Result<(), String> {
    fn walk(base: &Path, dir: &Path, manifest: &PackManifest) -> Result<(), String> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("read_dir 실패 {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("dir entry 실패: {e}"))?;
            let path = entry.path();
            let ft = entry.file_type().map_err(|e| format!("file_type 실패: {e}"))?;
            if ft.is_dir() {
                walk(base, &path, manifest)?;
            } else if ft.is_file() {
                let rel = path
                    .strip_prefix(base)
                    .map_err(|e| format!("rel 경로 실패: {e}"))?
                    .to_string_lossy()
                    .replace('\\', "/");
                if !manifest.files.contains_key(&rel) {
                    return Err(format!("미등재 파일(서명 manifest에 없음): {rel}"));
                }
            }
        }
        Ok(())
    }
    walk(root, root, manifest)
}

/// accepted 기록 읽기. 파일 부재 = Ok(None)(신규 머신, §7-⑩ 필수3) ↔ 존재하나 읽기/파싱
/// 실패 = Err(손상 = fail-closed 거부). '부재'와 '손상'을 구분해 손상본이 신규 머신으로 강등돼
/// replay 단조 게이트를 우회하는 것을 차단한다.
fn read_accepted(path: &Path) -> Result<Option<AcceptedPack>, String> {
    let s = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("accepted 기록 읽기 실패 {}: {e}", path.display())),
    };
    serde_json::from_str(&s)
        .map(Some)
        .map_err(|e| format!("accepted 기록 파싱 실패(손상) {}: {e}", path.display()))
}

/// RFC3339(예 "2030-01-01T00:00:00Z") → Unix epoch 초. 빈 문자열·파싱불가 = None.
/// pub(crate): license.rs가 동일 시각 규칙을 공유한다(이중 구현 드리프트 차단).
pub(crate) fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

/// minisign 서명 검증. pubkey는 tauri식(전체 .pub 파일 base64) 또는 raw 키라인 base64 모두 수용.
/// allow_legacy=true: prehashed(tauri 기본)·legacy ed25519 서명 모두 수용(둘 다 암호학적으로 안전).
/// pub(crate): license.rs가 동일 검증 코어를 재사용한다(신뢰근원·검증 규칙 단일화).
pub(crate) fn verify_minisign(pubkey: &str, data: &[u8], sig_bytes: &[u8]) -> Result<(), String> {
    use minisign_verify::Signature;
    let pk = load_public_key(pubkey)?;
    let sig_str = std::str::from_utf8(sig_bytes).map_err(|e| format!("서명 UTF-8 아님: {e}"))?;
    let signature = Signature::decode(sig_str).map_err(|e| format!("서명 디코드 실패: {e}"))?;
    pk.verify(data, &signature, true)
        .map_err(|e| format!("서명 검증 실패: {e}"))
}

/// tauri식(전체 .pub 파일 base64) 또는 raw 키라인 base64에서 minisign PublicKey 로드.
fn load_public_key(pubkey: &str) -> Result<minisign_verify::PublicKey, String> {
    use minisign_verify::PublicKey;
    let pubkey = pubkey.trim();
    // 1) raw 키라인 base64(.pub 둘째 줄) 직접 시도.
    if let Ok(pk) = PublicKey::from_base64(pubkey) {
        return Ok(pk);
    }
    // 2) tauri식: 전체 .pub 파일을 base64 인코딩한 형태 → 디코드 후 키라인(주석 다음) 추출.
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(pubkey)
        .map_err(|e| format!("pubkey base64 디코드 실패: {e}"))?;
    let text = String::from_utf8(decoded).map_err(|e| format!("pubkey 텍스트 아님: {e}"))?;
    let key_line = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
        .next_back()
        .ok_or_else(|| "pubkey 키라인 부재".to_string())?;
    PublicKey::from_base64(key_line).map_err(|e| format!("pubkey 파싱 실패: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// embed 키링이 파싱되고 부트스트랩 키(build.rs가 tauri pubkey 주입)가 비어있지 않다 —
    /// build.rs 병합·OUT_DIR 방출이 깨지면 여기서 잡힌다(결정론 게이트).
    #[test]
    fn embedded_keyring_parses_with_bootstrap_pubkey() {
        let kr: Keyring = serde_json::from_str(TRUSTED_KEYS_JSON).expect("embed 키링 파싱 실패");
        assert!(!kr.keys.is_empty(), "키링에 키가 없음");
        let boot = kr
            .keys
            .iter()
            .find(|k| k.key_id == "39E60A702949D6C3")
            .expect("부트스트랩 key_id 부재");
        assert!(!boot.pubkey.is_empty(), "build.rs가 tauri pubkey를 주입하지 않음");
        assert!(!boot.not_after.is_empty(), "not_after 부재(fail-closed 위반)");
        // 부트스트랩 pubkey는 실제 minisign 공개키로 로드 가능해야 한다(형식 검증).
        load_public_key(&boot.pubkey).expect("부트스트랩 pubkey 로드 실패");
    }

    // ── 테스트 fixture: minisign keypair 생성 + manifest 서명 ──────────────────────
    /// (pubkey_base64_rawline, sign_fn) 반환. sign_fn(bytes) → .minisig 문자열.
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

    fn keyring_with(key_id: &str, pubkey: &str, not_after: &str, revoked: &[&str]) -> Keyring {
        Keyring {
            keys: vec![TrustedKey {
                key_id: key_id.to_string(),
                pubkey: pubkey.to_string(),
                not_after: not_after.to_string(),
            }],
            revoked_key_ids: revoked.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn manifest_json(key_id: &str, pack_version: &str, signed_at: i64, expires_at: i64) -> Vec<u8> {
        serde_json::json!({
            "pack_version": pack_version,
            "min_binary_version": "0.4.1",
            "key_id": key_id,
            "signed_at": signed_at,
            "expires_at": expires_at,
            "files": {}
        })
        .to_string()
        .into_bytes()
    }

    fn tmp_accepted(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "cys-packsig-{tag}-{}-accepted.json",
            std::process::id()
        ))
    }

    // (a) 필수 필드 부재 = 거부 ──────────────────────────────────────────────
    #[test]
    fn manifest_missing_required_fields_rejected() {
        let kr = keyring_with("K", "x", "2030-01-01T00:00:00Z", &[]);
        let acc = tmp_accepted("missing");
        let _ = std::fs::remove_file(&acc);
        // key_id 누락
        let no_key_id = serde_json::json!({
            "pack_version":"1.0.0","signed_at":100,"expires_at":200,"files":{}
        })
        .to_string();
        assert!(verify_with_keyring(no_key_id.as_bytes(), b"sig", 150, &acc, &kr).is_err());
        // signed_at 누락
        let no_signed = serde_json::json!({
            "pack_version":"1.0.0","key_id":"K","expires_at":200,"files":{}
        })
        .to_string();
        assert!(verify_with_keyring(no_signed.as_bytes(), b"sig", 150, &acc, &kr).is_err());
        // expires_at 누락
        let no_exp = serde_json::json!({
            "pack_version":"1.0.0","key_id":"K","signed_at":100,"files":{}
        })
        .to_string();
        assert!(verify_with_keyring(no_exp.as_bytes(), b"sig", 150, &acc, &kr).is_err());
    }

    // (b) 만료창 밖 거부·안 통과 + (e) replay 단조 + (d) 서명 roundtrip ────────────
    #[test]
    fn freshness_window_and_replay_and_signature_roundtrip() {
        let (pk_b64, sign) = gen_key_and_signer();
        let kr = keyring_with("KID1", &pk_b64, "2030-01-01T00:00:00Z", &[]);

        // 유효창 [1000, 2000], 신규 머신(기록 부재) → 통과
        let acc = tmp_accepted("fresh");
        let _ = std::fs::remove_file(&acc);
        let mbytes = manifest_json("KID1", "1.0.0", 1000, 2000);
        let sig = sign(&mbytes);
        // 만료창 안(now=1500) + 정상 서명 → 통과
        let m = verify_with_keyring(&mbytes, sig.as_bytes(), 1500, &acc, &kr)
            .expect("유효창 안 정상 서명인데 거부됨");
        assert_eq!(m.pack_version, "1.0.0");

        // 만료창 밖(now=2500 > expires 2000) → 거부
        assert!(
            verify_with_keyring(&mbytes, sig.as_bytes(), 2500, &acc, &kr).is_err(),
            "만료창 밖인데 통과"
        );
        // 서명 시각 이전(now=500 < signed 1000) → 거부
        assert!(
            verify_with_keyring(&mbytes, sig.as_bytes(), 500, &acc, &kr).is_err(),
            "signed_at 이전인데 통과"
        );

        // (d) 변조: 문자열 값 내부 1바이트 변경(JSON 유효 유지) → ⓒ 서명 검증 실패.
        //     ★유효 JSON을 유지해 ⓐ serde 파싱이 아니라 ⓒ 서명 단계에서 거부됨을 보장한다.
        //     (min_binary_version 값 "0.4.1"의 '0'→'9' : 검증 경로 이전 필드 아님 + 유효 JSON 문자열.)
        let mut tampered = mbytes.clone();
        let needle = b"\"0.4.1\"";
        let pos = mbytes
            .windows(needle.len())
            .position(|w| w == needle)
            .expect("min_binary_version 값 위치");
        tampered[pos + 1] = b'9'; // "0.4.1" → "9.4.1"
        // 변조본은 반드시 유효 JSON이어야 한다(ⓐ 파싱 통과) — 그래야 ⓒ가 실제로 행사됨.
        assert!(
            serde_json::from_slice::<PackManifest>(&tampered).is_ok(),
            "변조본이 JSON 파싱에서 먼저 깨짐 — 서명 경로(ⓒ) 미도달"
        );
        assert!(
            verify_with_keyring(&tampered, sig.as_bytes(), 1500, &acc, &kr).is_err(),
            "변조 manifest가 통과(서명 무력)"
        );

        // (e) replay 단조: 수용 기록 후 같은/옛 signed_at 거부, 새 signed_at 통과
        record_accepted(&acc, &m).expect("accepted 기록 실패");
        // 같은 팩 재전송(signed_at 1000 <= 수용 1000) → 거부
        assert!(
            verify_with_keyring(&mbytes, sig.as_bytes(), 1500, &acc, &kr).is_err(),
            "replay(같은 signed_at)인데 통과"
        );
        // 더 새 팩(signed_at 1500, pack_version 1.0.1, 유효창 [1500,2600]) → 통과
        let newer = manifest_json("KID1", "1.0.1", 1500, 2600);
        let newer_sig = sign(&newer);
        let nm = verify_with_keyring(&newer, newer_sig.as_bytes(), 1600, &acc, &kr)
            .expect("새 팩(단조 증가)인데 거부됨");
        assert_eq!(nm.pack_version, "1.0.1");

        let _ = std::fs::remove_file(&acc);
    }

    // (c-iso) ⓒ 단독 격리: 유효 JSON·올바른 키링·유효창 모두 충족하되 '다른 키로 서명' →
    //         서명 검증(ⓒ)만 단독으로 거부됨을 증명. verify_minisign을 no-op(항상 Ok)로 회귀시키면
    //         이 테스트가 빨개진다 — §7-① 서명 보증의 적대 커버리지.
    #[test]
    fn valid_json_wrong_key_signature_rejected() {
        let (pk_b64, _) = gen_key_and_signer();
        let (_pk_other, sign_other) = gen_key_and_signer();
        let kr = keyring_with("KID3", &pk_b64, "2030-01-01T00:00:00Z", &[]);
        let acc = tmp_accepted("wrongsig");
        let _ = std::fs::remove_file(&acc);

        let mbytes = manifest_json("KID3", "1.0.0", 1000, 2000);
        // manifest 자체는 유효 JSON·키링 key_id/유효창 모두 충족 — 오직 서명만 '다른 키'.
        assert!(
            serde_json::from_slice::<PackManifest>(&mbytes).is_ok(),
            "manifest가 유효 JSON 아님 — ⓒ 격리 실패"
        );
        let wrong_sig = sign_other(&mbytes);
        assert!(
            verify_with_keyring(&mbytes, wrong_sig.as_bytes(), 1500, &acc, &kr).is_err(),
            "다른 키 서명(ⓒ)인데 통과 — no-op 회귀 미검출"
        );

        // 대조: 같은 manifest를 '올바른 키'로 서명하면 통과(ⓒ 외 단계는 모두 충족 확인).
        let (pk_ok, sign_ok) = gen_key_and_signer();
        let kr_ok = keyring_with("KID3", &pk_ok, "2030-01-01T00:00:00Z", &[]);
        let good_sig = sign_ok(&mbytes);
        assert!(
            verify_with_keyring(&mbytes, good_sig.as_bytes(), 1500, &acc, &kr_ok).is_ok(),
            "올바른 키 서명인데 거부 — ⓒ 대조 실패"
        );
        let _ = std::fs::remove_file(&acc);
    }

    // (e-corrupt) 손상된 accepted 기록 = fail-closed 거부(부재와 구분) ──────────────
    #[test]
    fn corrupt_accepted_record_rejected() {
        let (pk_b64, sign) = gen_key_and_signer();
        let kr = keyring_with("KID4", &pk_b64, "2030-01-01T00:00:00Z", &[]);
        let acc = tmp_accepted("corrupt");
        // 존재하나 파싱 불가(손상) → 신규 머신 강등 금지·하드 거부.
        std::fs::write(&acc, b"{not json").unwrap();
        let mbytes = manifest_json("KID4", "1.0.0", 1000, 2000);
        let sig = sign(&mbytes);
        assert!(
            verify_with_keyring(&mbytes, sig.as_bytes(), 1500, &acc, &kr).is_err(),
            "손상 accepted 기록인데 신규 머신으로 강등(통과)"
        );
        let _ = std::fs::remove_file(&acc);
    }

    // (e) 신규 머신(기록 부재) 통과 vs 옛 signed_at 거부를 독립 검증 ────────────────
    #[test]
    fn replay_fresh_machine_passes_old_rejected() {
        let (pk_b64, sign) = gen_key_and_signer();
        let kr = keyring_with("KID2", &pk_b64, "2030-01-01T00:00:00Z", &[]);
        let acc = tmp_accepted("replay2");
        let _ = std::fs::remove_file(&acc);

        // 신규 머신: 기록 없음 → signed_at 비교 불가 → 통과
        let m2 = manifest_json("KID2", "2.0.0", 5000, 9000);
        let s2 = sign(&m2);
        let parsed = verify_with_keyring(&m2, s2.as_bytes(), 6000, &acc, &kr)
            .expect("신규 머신(기록 부재)인데 거부됨");
        record_accepted(&acc, &parsed).unwrap();

        // 옛 팩(signed_at 4000 < 수용 5000) → 거부(유효창 안이어도)
        let old = manifest_json("KID2", "1.9.0", 4000, 9000);
        let so = sign(&old);
        assert!(
            verify_with_keyring(&old, so.as_bytes(), 6000, &acc, &kr).is_err(),
            "옛 signed_at replay인데 통과"
        );
        let _ = std::fs::remove_file(&acc);
    }

    // (c) 키링: 매칭 통과·revoked 거부·만료키 거부·미지 key_id 거부 ──────────────
    #[test]
    fn keyring_match_revoke_expire_unknown() {
        let (pk_b64, sign) = gen_key_and_signer();
        let acc = tmp_accepted("keyring");
        let _ = std::fs::remove_file(&acc);
        let mbytes = manifest_json("KIDX", "1.0.0", 1000, 2000);
        let sig = sign(&mbytes);

        // 매칭(미만료·미폐기) → 통과
        let kr_ok = keyring_with("KIDX", &pk_b64, "2030-01-01T00:00:00Z", &[]);
        assert!(verify_with_keyring(&mbytes, sig.as_bytes(), 1500, &acc, &kr_ok).is_ok());

        // revoked → 거부
        let kr_rev = keyring_with("KIDX", &pk_b64, "2030-01-01T00:00:00Z", &["KIDX"]);
        assert!(
            verify_with_keyring(&mbytes, sig.as_bytes(), 1500, &acc, &kr_rev).is_err(),
            "폐기 키인데 통과"
        );

        // 만료 키(now=1500 > not_after 1970-01-01T00:00:01Z=1초) → 거부
        let kr_exp = keyring_with("KIDX", &pk_b64, "1970-01-01T00:00:01Z", &[]);
        assert!(
            verify_with_keyring(&mbytes, sig.as_bytes(), 1500, &acc, &kr_exp).is_err(),
            "만료 키인데 통과"
        );

        // not_after 부재(빈 문자열) → 거부
        let kr_noexp = keyring_with("KIDX", &pk_b64, "", &[]);
        assert!(
            verify_with_keyring(&mbytes, sig.as_bytes(), 1500, &acc, &kr_noexp).is_err(),
            "not_after 부재인데 통과"
        );

        // 미지 key_id(manifest=KIDX, 키링=OTHER) → 거부
        let kr_unknown = keyring_with("OTHER", &pk_b64, "2030-01-01T00:00:00Z", &[]);
        assert!(
            verify_with_keyring(&mbytes, sig.as_bytes(), 1500, &acc, &kr_unknown).is_err(),
            "미지 key_id인데 통과"
        );
        let _ = std::fs::remove_file(&acc);
    }

    // verify_files: 일치 통과·불일치/누락 거부 ──────────────────────────────────
    #[test]
    fn verify_files_match_and_mismatch() {
        use sha2::{Digest, Sha256};
        let root = std::env::temp_dir().join(format!("cys-packsig-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/a.txt"), b"hello").unwrap();
        let good = format!("{:x}", Sha256::digest(b"hello"));

        let mut files = BTreeMap::new();
        files.insert("sub/a.txt".to_string(), good.clone());
        let m = PackManifest {
            pack_version: "1.0.0".into(),
            min_binary_version: String::new(),
            key_id: "K".into(),
            signed_at: 0,
            expires_at: 0,
            channel: default_channel(),
            pro_revision: 0,
            files,
        };
        assert!(verify_files(&m, &root).is_ok(), "일치인데 거부");

        // 불일치
        let mut bad_files = BTreeMap::new();
        bad_files.insert("sub/a.txt".to_string(), "deadbeef".to_string());
        let mbad = PackManifest { files: bad_files, ..m.clone() };
        assert!(verify_files(&mbad, &root).is_err(), "해시 불일치인데 통과");

        // 누락 파일
        let mut miss = BTreeMap::new();
        miss.insert("nope.txt".to_string(), good);
        let mmiss = PackManifest { files: miss, ..m.clone() };
        assert!(verify_files(&mmiss, &root).is_err(), "누락 파일인데 통과");

        let _ = std::fs::remove_dir_all(&root);
    }

    // verify_no_extra_files: 동치 집합 통과·미등재 파일 거부(역방향 커버리지) ──────────
    #[test]
    fn verify_no_extra_files_rejects_unlisted() {
        let root =
            std::env::temp_dir().join(format!("cys-packsig-extra-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("soul.md"), b"S").unwrap();

        // manifest가 soul.md만 등재 → staging도 soul.md만 = 동치 통과.
        let mut files = BTreeMap::new();
        files.insert("soul.md".to_string(), "irrelevant".to_string());
        let m = PackManifest {
            pack_version: "1.0.0".into(),
            min_binary_version: String::new(),
            key_id: "K".into(),
            signed_at: 0,
            expires_at: 0,
            channel: default_channel(),
            pro_revision: 0,
            files,
        };
        assert!(verify_no_extra_files(&m, &root).is_ok(), "동치 집합인데 거부");

        // staging에 미등재 파일(bin/evil.py) 추가 → 거부.
        std::fs::write(root.join("bin/evil.py"), b"#!/x\n").unwrap();
        assert!(
            verify_no_extra_files(&m, &root).is_err(),
            "미등재 파일(bin/evil.py)인데 통과 — 서명 우회"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // ── free/pro 채널·튜플 replay(v6 §3) ────────────────────────────────────────

    fn manifest_json_pro(
        key_id: &str,
        pack_version: &str,
        pro_revision: u32,
        signed_at: i64,
        expires_at: i64,
    ) -> Vec<u8> {
        serde_json::json!({
            "pack_version": pack_version,
            "min_binary_version": "0.4.1",
            "key_id": key_id,
            "signed_at": signed_at,
            "expires_at": expires_at,
            "channel": "pro",
            "pro_revision": pro_revision,
            "files": {}
        })
        .to_string()
        .into_bytes()
    }

    /// R1 실증 결함의 교정 핀: 동일 base의 pro 증분(pro.1→pro.2)이 replay 게이트를 통과하고,
    /// pro 역행은 거부되며, ★동일 튜플 + 더 새 서명은 통과한다(self-heal 후보 — v4 §3).
    #[test]
    fn replay_tuple_pro_increment_passes_regression_rejected() {
        let (pk, sign) = gen_key_and_signer();
        let kr = keyring_with("K1", &pk, "2099-01-01T00:00:00Z", &[]);
        let acc = tmp_accepted("protuple");
        let _ = std::fs::remove_file(&acc);
        // 수용 기준선: 0.8.0 pro.1 (signed 1000).
        std::fs::write(
            &acc,
            br#"{"pack_version":"0.8.0","signed_at":1000,"channel":"pro","pro_revision":1}"#,
        )
        .unwrap();

        // pro.2 (signed 2000) → 통과 (구현 전: parse_semver 접미 절단으로 replay 거부되던 경로).
        let m2 = manifest_json_pro("K1", "0.8.0", 2, 2000, 9_000_000_000);
        let s2 = sign(&m2);
        verify_with_keyring(&m2, s2.as_bytes(), 5000, &acc, &kr).expect("pro.2 증분이 거부됨");

        // pro 역행(pro.0, signed 3000) → 튜플 strictly-less = 거부.
        let m0 = manifest_json_pro("K1", "0.8.0", 0, 3000, 9_000_000_000);
        let s0 = sign(&m0);
        assert!(
            verify_with_keyring(&m0, s0.as_bytes(), 5000, &acc, &kr).is_err(),
            "pro 역행인데 통과"
        );

        // 동일 튜플(pro.1) + 더 새 서명(signed 4000) → 통과(self-heal 후보 핀).
        let m1 = manifest_json_pro("K1", "0.8.0", 1, 4000, 9_000_000_000);
        let s1 = sign(&m1);
        verify_with_keyring(&m1, s1.as_bytes(), 5000, &acc, &kr)
            .expect("동일 튜플·신서명(self-heal 후보)이 거부됨");

        // 동일 튜플 + 같은/낡은 서명(signed 1000) → 1차 게이트가 replay 거부.
        let mr = manifest_json_pro("K1", "0.8.0", 1, 1000, 9_000_000_000);
        let sr = sign(&mr);
        assert!(
            verify_with_keyring(&mr, sr.as_bytes(), 5000, &acc, &kr).is_err(),
            "낡은 서명 replay인데 통과"
        );

        let _ = std::fs::remove_file(&acc);
    }

    /// v4 §3 마이그레이션 명세 핀: 구 포맷 accepted({pack_version, signed_at}만)는
    /// channel=free·pro_revision=0으로 판독되고(read_accepted_evidence 포함) 검증이 정상 동작.
    #[test]
    fn accepted_old_format_reads_as_free_zero() {
        let (pk, sign) = gen_key_and_signer();
        let kr = keyring_with("K1", &pk, "2099-01-01T00:00:00Z", &[]);
        let acc = tmp_accepted("oldfmt");
        std::fs::write(&acc, br#"{"pack_version":"1.0.0","signed_at":1000}"#).unwrap();

        let ev = read_accepted_evidence(&acc).expect("구 포맷 판독 실패");
        assert_eq!(ev, Some(("free".to_string(), 0, "1.0.0".to_string())));

        // 신버전 free 번들(1.1.0, rev 0, signed 2000) 검증 정상 통과(무회귀).
        let m = manifest_json("K1", "1.1.0", 2000, 9_000_000_000);
        let s = sign(&m);
        verify_with_keyring(&m, s.as_bytes(), 5000, &acc, &kr).expect("구 포맷 기준선 위 신버전 거부됨");

        let _ = std::fs::remove_file(&acc);
    }

    /// v6 ⓐ-2 핀: channel=pro는 min_binary_version 필수(비어있음 = 거부), 미지 channel = 거부.
    #[test]
    fn pro_manifest_requires_min_binary_and_known_channel() {
        let (pk, sign) = gen_key_and_signer();
        let kr = keyring_with("K1", &pk, "2099-01-01T00:00:00Z", &[]);
        let acc = tmp_accepted("chanvalid");
        let _ = std::fs::remove_file(&acc);

        // channel=pro + min_binary 빈 값 → 거부.
        let m = serde_json::json!({
            "pack_version": "0.8.0", "min_binary_version": "", "key_id": "K1",
            "signed_at": 1000, "expires_at": 9_000_000_000i64,
            "channel": "pro", "pro_revision": 1, "files": {}
        })
        .to_string()
        .into_bytes();
        let s = sign(&m);
        assert!(
            verify_with_keyring(&m, s.as_bytes(), 5000, &acc, &kr).is_err(),
            "pro + 빈 min_binary인데 통과"
        );

        // 미지 channel → 거부.
        let m2 = serde_json::json!({
            "pack_version": "0.8.0", "min_binary_version": "0.4.1", "key_id": "K1",
            "signed_at": 1000, "expires_at": 9_000_000_000i64,
            "channel": "enterprise", "pro_revision": 0, "files": {}
        })
        .to_string()
        .into_bytes();
        let s2 = sign(&m2);
        assert!(
            verify_with_keyring(&m2, s2.as_bytes(), 5000, &acc, &kr).is_err(),
            "미지 channel인데 통과"
        );

        // 구 포맷(channel 필드 자체 부재) = free 기본 → 통과(하위호환 무회귀).
        let m3 = manifest_json("K1", "1.0.0", 1000, 9_000_000_000);
        let s3 = sign(&m3);
        verify_with_keyring(&m3, s3.as_bytes(), 5000, &acc, &kr).expect("구 포맷 manifest 거부됨");

        let _ = std::fs::remove_file(&acc);
    }
}
