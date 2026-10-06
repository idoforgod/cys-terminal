//! HMAC signed-prefix 승인 primitive (규약→기술 강제 ①).
//!
//! 자율주행 denylist 위험명령의 "1회 승인"을 `command_prefix + cwd + environment`에 대한
//! HMAC-SHA256 서명 레코드로 영속하고, 이후 동일 prefix 명령은 서명 검증으로만 자동 통과시킨다.
//! 시크릿 없이는 레코드를 위조할 수 없으므로(서명 불일치 hard-reject) 승인은 암호학적으로
//! 위조 불가능하다. base64·HMAC-SHA256은 외부 crate 0(sha2 0.10만)으로 수동 구현한다 —
//! recall.rs:hash_step의 Sha256 패턴을 ipad/opad로 확장. RFC 4231 KAT로 정확성을 박제한다.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

// ── 자료구조 ────────────────────────────────────────────────────────────────

/// 서명된 prefix 승인 레코드. cmux SurfaceResumeApprovalRecord의 cys 단순화(단일머신).
/// environment는 정렬된 Vec<(String,String)>로 — serde_json 맵 순서 비결정성을 피하고
/// 서명 직렬화와 일치시킨다(결정론 서명의 핵심).
#[derive(Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub version: u32,
    pub id: String,
    pub command_prefix: Vec<String>, // 빈 벡터 금지(폴백 차단)
    pub cwd: Option<String>,         // normalized
    pub environment: Vec<(String, String)>, // 정렬·민감키 drop 후
    pub created_at: f64,
    pub updated_at: f64,
    /// ★(0.14.31 · CONTRACTS B-3) TTL 만료 시각(epoch초). `None` = 무기한(구 레코드 포함).
    ///
    /// **서명 페이로드에 들어간다** — 그러나 `Some` 일 때만 줄이 덧붙는다(아래 `signing_payload`).
    /// 그래서 TTL 없는 구 레코드의 페이로드는 **바이트 동일**이고 기존 서명이 그대로 유효하다
    /// (승인 전수 무효화 = 자율주행 정지 사고를 만들지 않는다). 반대로 만료 시각을 떼거나
    /// 늘리려는 편집은 페이로드를 바꾸므로 서명 불일치로 hard-reject 된다.
    /// `skip_serializing_if` 로 None 은 키 자체를 쓰지 않는다 — 구 데몬이 읽어도 형상 무변화.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<f64>,
    pub signature: String, // base64(HMAC-SHA256(payload))
}

// ── 수동 base64 (표준 알파벳, 의존 0) ─────────────────────────────────────────

const B64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 표준 base64 인코딩(패딩 `=` 포함). 서명 직렬화·HMAC 출력 인코딩 전용.
pub fn b64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_ALPHABET[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// 표준 base64 디코딩(패딩 허용·내부 공백 무시). 잘못된 문자가 있으면 None.
pub fn b64_decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut symbols: Vec<u32> = Vec::with_capacity(input.len());
    let mut pad = 0usize;
    let mut seen_pad = false;
    for &c in input.as_bytes() {
        match c {
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            b'=' => {
                pad += 1;
                seen_pad = true;
            }
            _ => {
                if seen_pad {
                    return None; // 패딩 뒤 데이터 = 손상
                }
                symbols.push(val(c)?);
            }
        }
    }
    if (symbols.len() + pad) % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(symbols.len() / 4 * 3);
    for chunk in symbols.chunks(4) {
        let n = (chunk[0] << 18)
            | (chunk.get(1).copied().unwrap_or(0) << 12)
            | (chunk.get(2).copied().unwrap_or(0) << 6)
            | chunk.get(3).copied().unwrap_or(0);
        out.push(((n >> 16) & 0xff) as u8);
        if chunk.len() > 2 {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if chunk.len() > 3 {
            out.push((n & 0xff) as u8);
        }
    }
    Some(out)
}

// ── 수동 HMAC-SHA256 (recall.rs:hash_step 확장 — ipad/opad, 의존 0) ────────────

/// HMAC-SHA256(RFC 2104). 키>64B면 sha256(key)로 축약, 키<64B면 0패딩.
/// RFC 4231 KAT(hmac_kat 테스트)가 정확성을 증명한다 — KAT 실패=구현 버그.
pub fn hmac_sha256(secret: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut key = [0u8; BLOCK];
    if secret.len() > BLOCK {
        let h: [u8; 32] = {
            let mut s = Sha256::new();
            s.update(secret);
            s.finalize().into()
        };
        key[..32].copy_from_slice(&h);
    } else {
        key[..secret.len()].copy_from_slice(secret);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= key[i];
        opad[i] ^= key[i];
    }
    let inner: [u8; 32] = {
        let mut s = Sha256::new();
        s.update(ipad);
        s.update(msg);
        s.finalize().into()
    };
    let outer: [u8; 32] = {
        let mut s = Sha256::new();
        s.update(opad);
        s.update(inner);
        s.finalize().into()
    };
    outer
}

/// 상수시간 바이트 비교 — 서명 검증의 조기반환 타이밍 사이드채널 차단.
/// 길이 다르면 즉시 false(길이는 비밀이 아님), 같으면 전 바이트 XOR 누적 후 0 판정.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

// ── 결정론 직렬화 + 서명/검증 ─────────────────────────────────────────────────

impl ApprovalRecord {
    /// cmux signingPayloadData 1:1 흡수 — 각 필드 base64 후 newline-join, environment는
    /// 키 정렬(이미 정렬 전제) 후 `b64key=b64val,…`. base64+구분자가 충돌(따옴표/등호/콤마/
    /// 줄바꿈을 값에 넣어 필드 경계를 위조)을 차단한다.
    pub fn signing_payload(&self) -> Vec<u8> {
        let prefix = self
            .command_prefix
            .iter()
            .map(|t| b64_encode(t.as_bytes()))
            .collect::<Vec<_>>()
            .join(",");
        let env = self
            .environment
            .iter() // 이미 정렬됨(sort_norm_env가 보장)
            .map(|(k, v)| format!("{}={}", b64_encode(k.as_bytes()), b64_encode(v.as_bytes())))
            .collect::<Vec<_>>()
            .join(",");
        let mut fields = vec![
            format!("version={}", self.version),
            format!("id={}", self.id),
            format!("commandPrefix={prefix}"),
            format!(
                "cwd={}",
                self.cwd
                    .as_deref()
                    .map(|c| b64_encode(c.as_bytes()))
                    .unwrap_or_default()
            ),
            format!("environment={env}"),
            format!("createdAt={}", self.created_at),
            format!("updatedAt={}", self.updated_at),
        ];
        // ★(0.14.31 · B-3) TTL 은 **조건부 말미 추가**다 — `None` 이면 한 글자도 붙지 않는다.
        //   이 비대칭이 계약의 전부다: 구 레코드(무TTL)의 페이로드가 종전과 바이트 동일해야
        //   기존 서명이 살아남고(무효화 0), TTL 이 있는 레코드는 그 값까지 서명에 묶여
        //   만료 연장·제거가 위조로 판정된다. 순서를 바꾸거나 무조건 추가로 바꾸지 마라 —
        //   그 순간 설치된 모든 승인이 한 번에 무효가 된다(자율주행 전면 정지).
        if let Some(exp) = self.expires_at {
            fields.push(format!("expiresAt={exp}"));
        }
        fields.join("\n").into_bytes()
    }

    pub fn sign(&mut self, secret: &[u8]) {
        self.signature = b64_encode(&hmac_sha256(secret, &self.signing_payload()));
    }

    /// 상수시간 비교로 서명 검증 — 재서명 후 동치 비교(타이밍릭 차단).
    pub fn has_valid_signature(&self, secret: &[u8]) -> bool {
        let expect = b64_encode(&hmac_sha256(secret, &self.signing_payload()));
        constant_time_eq(self.signature.as_bytes(), expect.as_bytes())
    }

    /// ★(0.14.31 · B-3) 이 레코드가 `now` 기준 만료됐는가. TTL 없는 레코드는 만료되지 않는다
    /// (무기한 — 종전 계약 보존). 경계는 `expires_at <= now` = 만료(만료 시각 그 순간은 이미 죽었다).
    pub fn is_expired(&self, now: f64) -> bool {
        self.expires_at.is_some_and(|e| e <= now)
    }

    /// 명령이 이 레코드 prefix에 매칭하는가: prefix가 명령 토큰의 정확한 접두 + cwd 완전일치
    /// + environment 부분집합(레코드 env가 호출 env에 모두 포함). 빈 prefix·미닫힌 따옴표 거부.
    #[cfg_attr(not(test), allow(dead_code))] // 운영 경로는 `matches_ctx` 를 부른다 — 이 이름은 종전 호출처·시험용으로 남긴다
    pub fn matches(&self, command: &str, cwd: Option<&str>, env: &[(String, String)]) -> bool {
        self.matches_ctx(command, cwd, env, None)
    }

    /// [`matches`] + ★(0.14.44 · A3) 데몬 묶음 문맥. `ctx` 가 `None` 이면 0.14.43 의 규칙 그대로다(스케줄 게이트가 이 길을 쓴다).
    /// `ctx` 가 있을 때만: ① 예약 값(`CYS_APPROVAL_LANE`)이 든 레코드는 명령에 대상 데몬을 바꾸는 옵션이 있으면 맞지 않는다
    /// ② 폴더가 다른데 [`neutral_skip`] 의 다섯 조건이 모두 참이면 폴더 비교만 건너뛴다.
    /// (예약 값 일치 자체는 기존 규칙 "레코드 환경 ⊆ 확인 환경"이 맡는다 — 데몬이 확인용 환경에 자기 값을 넣는다.)
    pub fn matches_ctx(
        &self,
        command: &str,
        cwd: Option<&str>,
        env: &[(String, String)],
        ctx: Option<&MatchCtx>,
    ) -> bool {
        if self.command_prefix.is_empty() {
            return false; // 폴백 차단
        }
        let Some(toks) = tokenize(command) else {
            return false; // 미닫힌 따옴표 = prefix injection 차단
        };
        if toks.len() < self.command_prefix.len() {
            return false;
        }
        if toks[..self.command_prefix.len()] != self.command_prefix[..] {
            return false;
        }
        if ctx.is_some() && self.lane_value().is_some() && has_socket_option(&toks) {
            return false; // 이 데몬에서 확인받고 다른 데몬에 집행하는 길을 막는다(A3 · 재확인 R6)
        }
        if let Some(rc) = &self.cwd {
            if normalize_cwd(cwd).as_deref() != Some(rc.as_str()) {
                let skip = ctx.is_some_and(|c| neutral_skip(self, command, c));
                if !skip {
                    return false;
                }
            }
        }
        // environment 부분집합: 레코드의 (민감키 drop·정렬된) env 항목이 모두 호출 env에 존재.
        // 호출 측이 추가 env를 더 가져도 매칭(미세 변동 내성) — 단 레코드가 요구한 키-값은 강제.
        let call_env = sort_norm_env(env);
        self.environment
            .iter()
            .all(|kv| call_env.binary_search(kv).is_ok())
    }
}

/// 서명 유효 + 매칭 레코드 중 최장 prefix(동률은 updated_at 최신) 선택.
///
/// ★(0.14.31 · B-3) 시각·TTL 축은 [`best_match_at`] 이 소유한다. 이 얇은 래퍼는 "지금"과
/// `require_ttl=false`(TTL 요구 없음)로 위임할 뿐이다 — **만료 레코드는 여기서도 매칭되지
/// 않는다**(만료는 요구 여부와 무관한 사실이다).
pub fn best_match<'a>(
    records: &'a [ApprovalRecord],
    secret: &[u8],
    command: &str,
    cwd: Option<&str>,
    env: &[(String, String)],
) -> Option<&'a ApprovalRecord> {
    best_match_at(records, secret, command, cwd, env, crate::state::now_epoch(), false)
}

/// [`best_match`] + 시각 주입(순수 테스트용) + `require_ttl`.
///
/// · `require_ttl=false`: 무기한 레코드도 통과(종전 계약) · 만료된 TTL 레코드는 **거부**.
/// · `require_ttl=true` : `expires_at` 이 **있고** 아직 만료되지 않은 레코드만 통과.
///   TTL 없는 구 레코드는 거부된다(계약 B-3 — '무기한 승인으로 TTL 게이트를 통과' 차단).
pub fn best_match_at<'a>(
    records: &'a [ApprovalRecord],
    secret: &[u8],
    command: &str,
    cwd: Option<&str>,
    env: &[(String, String)],
    now: f64,
    require_ttl: bool,
) -> Option<&'a ApprovalRecord> {
    best_match_index_at(records, secret, command, cwd, env, now, require_ttl).map(|i| &records[i])
}

/// [`best_match_at`] 과 **같은 선택**의 인덱스 판(호출부가 그 레코드를 갱신해야 할 때).
///
/// ★왜 id 가 아니라 인덱스인가(codex 적대검증 major): `approval.check` 는 매칭 레코드의
/// `updated_at` 을 갱신하고 **재서명**한다. 그 대상을 `id` 로 다시 찾으면, 승인 파일에 **같은 id
/// 를 가진 위조 레코드**(서명 무효)를 앞에 끼워 넣은 공격자가 검증받은 적 없는 그 레코드를
/// 데몬의 손으로 정당 서명시킬 수 있다(서명 세탁 — 다음 check 부터 공격자가 정한 범위·만료가
/// 유효해진다). 검증한 **그 자리**를 갱신하면 그 경로가 원리상 닫힌다.
pub fn best_match_index_at(
    records: &[ApprovalRecord],
    secret: &[u8],
    command: &str,
    cwd: Option<&str>,
    env: &[(String, String)],
    now: f64,
    require_ttl: bool,
) -> Option<usize> {
    best_match_index_ctx(records, secret, command, cwd, env, now, require_ttl, None)
}

/// [`best_match_index_at`] + ★(0.14.44 · A3) 데몬 묶음 문맥(`ctx = None` 이면 0.14.43 과 같은 선택).
#[allow(clippy::too_many_arguments)]
pub fn best_match_index_ctx(
    records: &[ApprovalRecord],
    secret: &[u8],
    command: &str,
    cwd: Option<&str>,
    env: &[(String, String)],
    now: f64,
    require_ttl: bool,
    ctx: Option<&MatchCtx>,
) -> Option<usize> {
    records
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.has_valid_signature(secret)
                && !r.is_expired(now)
                && (!require_ttl || r.expires_at.is_some())
                && r.matches_ctx(command, cwd, env, ctx)
        })
        .max_by(|(_, a), (_, b)| {
            a.command_prefix
                .len()
                .cmp(&b.command_prefix.len())
                .then(
                    a.updated_at
                        .partial_cmp(&b.updated_at)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        })
        .map(|(i, _)| i)
}

// ── 토큰화 / 정규화 / 민감 env ─────────────────────────────────────────────────

/// 셸 토크나이저 — ★(0.14.44 · A3) 본문은 공용 라이브러리 `cys::approval_tokenize`(src/lib.rs)로 **한 글자도 바꾸지 않고** 옮겼다(CLI 의 `approval check` 숫자 토큰 떼기가
/// 데몬과 같은 코드를 쓰게 하려는 것). 이 이름은 그대로 쓰는 호출처·시험을 위한 재공개다.
pub use cys::approval_tokenize as tokenize;

/// cwd 정규화 — tilde 확장 + 후행 슬래시 제거. 단일머신 전제(symlink 정규화는 비용·미사용).
pub fn normalize_cwd(cwd: Option<&str>) -> Option<String> {
    let raw = cwd?;
    let expanded = if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(rest).to_string_lossy().to_string()
        } else {
            raw.to_string()
        }
    } else if raw == "~" {
        dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|| raw.to_string())
    } else {
        raw.to_string()
    };
    let trimmed = expanded.trim_end_matches('/');
    Some(if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    })
}

/// 민감키 drop(서명 페이로드에 시크릿값 미포함) + 키 정렬(결정론·binary_search 전제).
/// cmux isSensitiveEnvironmentKey 흡수 — 키 대문자화 후 부분일치 drop.
pub fn sort_norm_env(env: &[(String, String)]) -> Vec<(String, String)> {
    const SENSITIVE: &[&str] = &[
        "API_KEY",
        "ACCESS_KEY",
        "AUTH_TOKEN",
        "BEARER_TOKEN",
        "PRIVATE_KEY",
        "PASSWORD",
        "PASSWD",
        "SECRET",
        "TOKEN",
        "CREDENTIAL",
        "COOKIE",
    ];
    let mut out: Vec<(String, String)> = env
        .iter()
        .filter(|(k, _)| {
            let up = k.to_uppercase();
            !SENSITIVE.iter().any(|s| up.contains(s))
        })
        .cloned()
        .collect();
    out.sort();
    out
}

// ── 시크릿 저장 (cmux fileBackedSecret 흡수 — keyring crate 부재 대안) ──────────

const ENV_SECRET_B64: &str = "CYS_APPROVAL_SECRET_B64";

/// 시크릿 파일 경로: ~/.cys/.approval-secret — pack(~/.cys/pack) 밖, ~/.cys/ 직하.
/// pack은 배포·git 추적 대상일 수 있으므로 시크릿이 새지 않게 분리한다.
fn secret_path() -> PathBuf {
    store_root().join(".cys").join(".approval-secret")
}

/// 승인 저장소·시크릿의 **루트** — 릴리스에서는 언제나 `dirs::home_dir()` 하나다.
///
/// ★(0.14.31 수렴 R2 · codex 재검증 major) **검체 이음매**(`cfg(test)` 한정): Windows 의
/// `dirs::home_dir()` 은 `SHGetKnownFolderPath(FOLDERID_Profile)` 이라 `HOME` 을 바꿔도
/// 격리되지 않는다. `HOME` 만 임시 디렉터리로 돌린 승인 검체는 그 플랫폼에서 **실제 사용자
/// 프로필의 승인 저장소**를 읽고 쓰고(테스트 승인이 프로필에 남는다), 만들지도 않은 임시
/// 디렉터리의 파일을 찾다 실패한다. 그래서 검체는 루트를 이 이음매로 **직접 주입**한다.
///
/// **env 로는 열지 않는다.** 호출자가 정할 수 있는 값 하나로 승인 저장소를 옮길 수 있으면
/// 그것이 곧 게이트 우회다(빈 저장소를 가리키게 하는 것은 거부 방향이지만, 공격자가 자기
/// 서명 저장소를 가리키게 하는 것은 통과 방향이다). 릴리스 빌드에는 이 분기가 아예 없다.
fn store_root() -> PathBuf {
    #[cfg(test)]
    if let Some(p) = tests::store_root_override() {
        return p;
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// 승인 레코드 영속 경로: ~/.cys/approvals.json (0600).
fn records_path() -> PathBuf {
    store_root().join(".cys").join("approvals.json")
}

/// 우선순위: ① env override(B64, 로깅 금지) → ② 0600 파일 → ③ 생성·0600 저장.
/// 파일 쓰기 실패해도 in-memory secret 반환(세션 한정 — 재시작 시 재생성).
pub fn signing_secret() -> Option<Vec<u8>> {
    // ① env override(B64) — 로깅·이벤트 payload에 절대 미포함.
    if let Ok(b64) = std::env::var(ENV_SECRET_B64) {
        if let Some(d) = b64_decode(&b64) {
            if !d.is_empty() {
                return Some(d);
            }
        }
    }
    // ② 0600 파일(pack 밖 ~/.cys/ 하위) — Keychain crate 부재라 1차 경로.
    let path = secret_path();
    match std::fs::read(&path) {
        Ok(d) if !d.is_empty() => return Some(d),
        // 0바이트 = 미완성 쓰기의 잔재. 잃을 것이 없으므로 아래에서 새로 만든다.
        Ok(_) => {}
        // 부재 = 아직 만들지 않았다(정상 초기 상태) → 아래에서 만든다.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // ★(독립 재유도 · codex 설계비평 #15) **읽지 못한 시크릿은 덮어쓰지 않는다.**
        //   종전엔 권한 오류·IO 오류도 '없음'으로 접고 새 시크릿을 그 자리에 썼다 — 성공하면
        //   기존 승인 **전부**가 서명 불일치로 영구 무효가 된다(승인 저장소 되쓰기와 같은
        //   클래스의 파괴). 모르면 서명 능력을 포기한다: 호출부는 미서명 취급(fail-closed)이라
        //   방향은 '거부'이고, 사람이 권한을 고치면 그대로 돌아온다.
        Err(e) => {
            eprintln!(
                "cysd: 승인 시크릿을 읽지 못했다({}: {e}) — 새로 만들지 않고 미서명으로 접는다\
                 (기존 승인을 무효화하지 않기 위함 · 권한을 고치면 복구된다)",
                path.display()
            );
            return None;
        }
    }
    // ③ 생성 + 0600 저장.
    let secret = random_32()?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&path, &secret).is_ok() {
        set_owner_only(&path);
        return Some(secret);
    }
    Some(secret) // 파일 실패해도 세션 한정 secret 반환.
}

/// 0600 권한 부여(Unix). Windows는 ACL 미설정(단일 사용자 데스크톱 전제).
fn set_owner_only(path: &PathBuf) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path; // no-op
    }
}

/// 32바이트 난수. Unix=/dev/urandom 직접 read(getrandom/OsRng crate 부재).
/// 비-Unix는 시간 기반 PRNG 폴백(단일 데스크톱 전제 — 위협모델상 허용, 명문화).
fn random_32() -> Option<Vec<u8>> {
    #[cfg(unix)]
    {
        // ★/dev/urandom은 무한 스트림 — std::fs::read(전체 읽기)는 EOF가 없어 영영 반환하지
        //   않는다(hang+무한메모리). 반드시 read_exact로 32바이트만 채운다.
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let mut buf = [0u8; 32];
            if f.read_exact(&mut buf).is_ok() {
                return Some(buf.to_vec());
            }
        }
        // /dev/urandom 읽기 실패(컨테이너 등) — 아래 폴백으로.
    }
    // 폴백: 시간+pid 기반 SHA256(약한 엔트로피 — Unix urandom 실패 시 한정).
    let seed = format!(
        "{}-{}-{:?}",
        std::process::id(),
        crate::state::now_epoch(),
        std::time::SystemTime::now()
    );
    let mut s = Sha256::new();
    s.update(seed.as_bytes());
    let h: [u8; 32] = s.finalize().into();
    Some(h.to_vec())
}

// ── 레코드 영속 (JSON 0600, atomic tmp+rename) ────────────────────────────────

/// ★(0.14.31 · WP-4 R2 · codex major) **TTL 레코드 전용 저장소** — `~/.cys/approvals-ttl.json`.
///
/// 【무엇을 고치는가】 승인 저장소는 `$HOME` 아래 **공유 파일**이고, 기준 커밋(0.14.30)의
/// `ApprovalRecord` 에는 `expires_at` 이 없다. 구 데몬은 파일 전체를 typed 레코드로 읽고 다시
/// 직렬화하므로(모르는 필드는 조용히 버린다), 같은 HOME 에서 신·구 데몬이 함께 돌 때 구 데몬이
/// **다른** 승인 하나를 정상 사용하기만 해도 TTL 레코드의 `expires_at` 이 사라진다. 그 값은
/// 서명 페이로드에 들어가 있으므로 신 데몬은 그 뒤로 그 레코드를 **서명 불일치로 영구 거부**한다
/// — 만료 전 승인이 되살릴 수 없이 망가진다(codex 적대검증 R2 major).
///
/// 【어떻게 고치는가】 TTL 이 있는 레코드는 **구 데몬이 존재조차 모르는 파일**에만 쓴다.
///   · 파괴 불가 — 구 writer 는 이 파일을 열지 않는다.
///   · 그리고 **구 데몬이 TTL 승인을 통과시키지도 못한다**(그 데몬은 `expires_at` 을 무시하므로
///     종전 배치에서는 만료된 승인을 무기한으로 오독했다). 못 보는 것이 오독보다 안전하다.
/// 병합은 읽을 때 한 번(`load_records`), 분리는 쓸 때 한 번(`save_records`) 일어나므로 호출부는
/// 종전 그대로다.
fn ttl_records_path() -> PathBuf {
    store_root().join(".cys").join("approvals-ttl.json")
}

/// 한 파일에서 레코드 목록 디코드: `{"records":[...]}` 또는 bare 배열 둘 다(cmux 하위호환).
///
/// ★(0.14.31 · 독립 재유도 · codex blocking #4) **부재와 실패를 가른다.** 종전 판은 열기 실패도
/// 파싱 실패도 `Vec::new()` 로 접었고, 호출부(`mutate_records`)는 그 빈 목록을 **그대로 저장**
/// 했다 — 권한이 막힌 파일·레코드 하나가 깨진 파일이 아무 `approval.check` 한 번에
/// `{"records":[]}` 로 갈아끼워졌다(사람이 서명한 승인의 영구 파괴 · tmp→rename 은 상위
/// 디렉터리 권한만 있으면 성공한다). 그래서:
///   · 파일 **부재** → `Ok(vec![])` (승인이 아직 없다 — 정상 상태다)
///   · 그 밖의 IO 오류·JSON 파싱 실패 → `Err` → 호출부는 **쓰기 없이 트랜잭션을 중단**한다.
/// 판정 방향은 그대로 fail-closed 다: 읽지 못한 저장소는 "승인 없음"으로 답한다(거부).
fn load_records_from(path: &PathBuf) -> Result<Vec<ApprovalRecord>, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: 읽기 실패: {e}", path.display())),
    };
    // 빈 파일은 '아직 아무 것도 안 썼다'로 읽는다(0바이트는 JSON 이 아니다 — 여기서 Err 로
    // 접으면 첫 서명 전 상태의 데몬이 승인을 만들 수 없다).
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    // ① {"records":[...]} 형태
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(arr) = v.get("records") {
            return serde_json::from_value::<Vec<ApprovalRecord>>(arr.clone())
                .map_err(|e| format!("{}: records 디코드 실패: {e}", path.display()));
        }
    }
    // ② bare 배열
    serde_json::from_str::<Vec<ApprovalRecord>>(&content)
        .map_err(|e| format!("{}: JSON 디코드 실패: {e}", path.display()))
}

/// 저장 포맷: `{"records":[...]}` 또는 bare 배열 둘 다 디코드(cmux 하위호환).
///
/// ★(R2) 두 저장소(공용 `approvals.json` + TTL 전용 `approvals-ttl.json`)를 **병합**해 돌려준다.
/// 같은 `id` 가 양쪽에 있으면 **TTL 쪽이 이긴다**: 그 상태는 분리 저장 중 중단(TTL 먼저 쓰고
/// 공용을 쓰기 전에 죽음)에서만 생기고, 그때 살아 있는 사실은 만료를 포함한 TTL 판이다.
pub fn load_records() -> Vec<ApprovalRecord> {
    // 읽기·파싱 실패는 **승인 없음**으로 답한다(fail-closed). 저장은 하지 않는다 —
    // 그 결정은 `try_load_records` 를 직접 쓰는 `mutate_records` 가 한다.
    try_load_records().unwrap_or_default()
}

/// [`load_records`] 의 **실패를 숨기지 않는** 판 — 두 저장소 중 하나라도 읽거나 파싱하지
/// 못하면 `Err`. 되쓰기 여부를 결정하는 트랜잭션(`mutate_records`)만 이 판을 쓴다.
pub fn try_load_records() -> Result<Vec<ApprovalRecord>, String> {
    let ttl = load_records_from(&ttl_records_path())?;
    let main = load_records_from(&records_path())?;
    let ttl_ids: std::collections::HashSet<&str> = ttl.iter().map(|r| r.id.as_str()).collect();
    let mut out: Vec<ApprovalRecord> = main
        .into_iter()
        .filter(|r| !ttl_ids.contains(r.id.as_str()))
        .collect();
    out.extend(ttl);
    Ok(out)
}

/// atomic write: tmp 작성·0600 부여 후 rename. 디렉토리 자동 생성.
fn save_records_to(path: &PathBuf, records: &[&ApprovalRecord]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(&serde_json::json!({"records": records}))
        .map_err(|e| e.to_string())?;
    // ★(독립 재유도 · codex major #5 ②) tmp 이름에 **pid** 를 넣어 writer 를 분리한다.
    //   같은 `$HOME` 에 부서 데몬이 여럿 뜨는 배치에서 공용 `<name>.json.tmp` 하나를 두면,
    //   A 가 쓰는 중인 tmp 를 B 가 덮어쓰고 A 가 그 **B 의 내용**을 rename 하는 창이 있다.
    //   rename 은 같은 디렉터리 안이므로 원자성은 그대로다.
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, body.as_bytes()).map_err(|e| e.to_string())?;
    set_owner_only(&tmp);
    // ★(수렴 R2 · claude minor) rename 이 실패하면 **그 자리에서 tmp 를 지운다.** pid 가 이름에
    //   들어간 뒤로는 다음 실행이 같은 이름을 재사용하지 않으므로, 지우지 않으면 승인 전체를
    //   담은 0600 사본이 pid 하나당 하나씩 `~/.cys` 에 쌓인다(디스크 가득참·AV 잠금으로 rename
    //   이 반복 실패하는 기계). 삭제 실패는 무시한다 — 보고할 사실은 rename 실패 쪽이다.
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    set_owner_only(path);
    Ok(())
}

/// atomic write: tmp 작성·0600 부여 후 rename. 디렉토리 자동 생성.
///
/// ★(R2) `expires_at` 유무로 **두 파일에 나눠** 쓴다(§ttl_records_path). 순서는 **TTL 먼저**다:
/// 중간에 죽으면 TTL 판이 두 파일에 겹쳐 남는데, 그 상태를 `load_records` 가 TTL 우선으로 접어
/// 손실 없이 복구한다(반대 순서면 만료 필드가 없는 판이 살아남아 서명이 깨진다).
/// 공용 파일에 `expires_at` 을 가진 레코드가 있으면 이 쓰기가 **자동 이주**시킨다.
pub fn save_records(records: &[ApprovalRecord]) -> Result<(), String> {
    let (ttl, plain): (Vec<&ApprovalRecord>, Vec<&ApprovalRecord>) =
        records.iter().partition(|r| r.expires_at.is_some());
    save_records_to(&ttl_records_path(), &ttl)?;
    save_records_to(&records_path(), &plain)
}

/// ★(R2 · codex major) 승인 저장소의 **읽기-변경-쓰기 트랜잭션**.
///
/// 【무엇을 고치는가】 호출부는 종전에 `load_records()` 로 스냅샷을 뜨고, 한참 뒤 그 스냅샷
/// 전체를 `save_records()` 로 되썼다. 그 사이 다른 요청이 승인을 하나 추가하면 **그 승인이
/// 사라진다**(갱신 손실). 종전엔 파일이 하나라 같은 클래스의 사고였지만, 저장소가 둘이 되면
/// 일반 승인 검사 하나가 TTL 파일까지 덮으므로 폭발 반경이 커진다.
///
/// 【계약】 이 함수 **안에서** 다시 읽고, 변경하고, 쓴다. 프로세스 안의 모든 변경은 이 뮤텍스로
/// 직렬화된다. 프로세스 **밖**(같은 HOME 의 다른 데몬)은 여전히 경쟁할 수 있다 — 그것은 이
/// 저장소가 처음부터 안고 있던 성질이고(파일 락 없음), 이 커밋의 범위 밖이다.
///
/// ★(0.14.31 · 독립 재유도) 이 트랜잭션이 파일을 **쓰는 조건**은 둘 다 참일 때뿐이다:
///   ① 두 저장소를 **읽어냈다**(`try_load_records` 가 `Ok`) — 읽지 못한 파일을 빈 목록으로
///      되쓰면 사람이 서명한 승인이 영구 소멸한다(codex blocking #4).
///   ② 클로저가 목록을 **실제로 바꿨다** — 무매칭 `approval.check` 하나까지 두 파일을
///      tmp→rename 으로 갈아끼우면, 그 사이 다른 부서 데몬이 서명한 승인이 이 스냅샷에
///      덮여 사라진다(프로세스 뮤텍스 밖 갱신 손실 · claude major / codex major #5).
/// 변경 판정은 **직렬화 동등성**이다: 필드 하나가 늘어도 자동으로 따라오고, `updated_at`
/// 재서명처럼 진짜 변경은 반드시 다르게 나온다. 승인 목록은 수십 건 규모라 비용이 무시된다.
/// 읽기에 실패하면 **클로저를 돌리지 않는다**(아래 `Err` 팔). 그 자리에서 `(None, Err(..))`
/// 로 나가므로 변형도 부수효과도 없고, 호출부는 `None` 을 "판정하지 못했다"로 읽어 거부로
/// 접는다(fail-closed). 실패 사유는 `Err` 가 싣는다.
pub fn mutate_records<R>(
    f: impl FnOnce(&mut Vec<ApprovalRecord>) -> R,
) -> (Option<R>, Result<(), String>) {
    static STORE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // poison 되어도 승인 저장은 계속돼야 한다(잠금 목적은 순서 직렬화뿐 — 불변식 보호가 아니다).
    let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut records = match try_load_records() {
        Ok(r) => r,
        Err(e) => {
            // ★읽지 못한 저장소는 **건드리지 않고, 클로저도 돌리지 않는다**(codex 설계비평 #9):
            //   빈 목록 위에서 돌리면 "실패 상태에서 변형·부수효과를 실행한다"는 계약이 되고,
            //   호출부가 그 결과를 성공과 구별할 수 없다. `None` 이 곧 "판정하지 못했다"이고,
            //   호출부는 그것을 **거부**로 접는다(fail-closed).
            return (None, Err(format!("승인 저장소를 읽지 못해 트랜잭션을 중단했다 — {e}")));
        }
    };
    let before = fingerprint(&records);
    let out = f(&mut records);
    if before.is_some() && fingerprint(&records) == before {
        return (Some(out), Ok(())); // 변경 없음 = 쓸 이유 없음(경합 반경 0).
    }
    let saved = save_records(&records);
    (Some(out), saved)
}

/// 변경 판정용 지문 — 직렬화 실패(사실상 불가)는 `None` 으로 두고 "변경됐다"로 접는다
/// (모르면 쓰는 쪽이 데이터 보존 방향이다).
fn fingerprint(records: &[ApprovalRecord]) -> Option<Vec<u8>> {
    serde_json::to_vec(records).ok()
}

/// ★(R2F-DM · 성찰 2회차) 승인 레코드가 서명하는 시각의 **해상도 = 마이크로초**.
///
/// 서명 재료(`signing_payload`)의 `createdAt`·`updatedAt`·`expiresAt` 는 `f64` 의 `Display` 문자열이고 레코드는 `serde_json` 으로
/// 저장·재독한다. 윈도우 `SystemTime` 은 100ns(리눅스는 1ns) 해상도라 시각이 유효숫자 17자리가 되는데, `serde_json` 의 기본 파서는
/// `significand as f64 / 10^k` 로 두 번 반올림해 그런 값의 약 10 % 를 같은 값으로 돌려주지 못한다 → 저장→읽기 뒤 서명 재료가 달라져
/// 서명이 어긋나고 승인이 거부됐다(윈도우 `cys approval check` · guard 훅의 LOOSE 우회가 먹지 않는다). 시각을 마이크로초로 맞추면
/// 값이 `N / 10^6`(N < 2^53 · 서기 2255 년까지)에 가장 가까운 `f64` 가 되어 유효숫자가 16자리 이하이고, 기본 파서의 빠른 길(정수 유효수 ÷ 10^k 한 번 =
/// 정확한 반올림)이 같은 값을 돌려준다 — 맥의 시계(1µs)가 이미 그렇게 동작하던 것과 같다(맥에서는 사실상 값이 바뀌지 않는다).
///
/// ★`serde_json` 의 `float_roundtrip` 기능으로 막지 **않는다**: 0.14.43 릴리스 노트가 그 설계안을 기각했다 — 켜면 데몬이 부동소수 프레임에도 길이 표지(`_flen`)를
/// 붙이게 되어, 읽기가 best-effort 인 구 클라이언트(0.14.42 이하 `cys`)가 값에 따라 `abi: LenMismatch` 로 간헐 실패한다(`abi_frame_tests`·`wire::tests::k1_*` 가 그 전제를 지킨다).
/// 서명·검증·매칭 함수(`signing_payload`·`has_valid_signature`·`matches`)는 건드리지 않았다 — 이 함수는 그 함수들에 들어가는 **값**만 정한다.
/// 종전에 저장된(17자리) 승인은 되살아나지 않는다 — 읽는 쪽 파서가 그대로라서다(다시 서명해야 한다 · 거부 방향 불변).
pub fn quantize_epoch_us(t: f64) -> f64 {
    let us = (t * 1e6).round();
    if us.is_finite() {
        us / 1e6
    } else {
        t
    }
}

/// `approval.sign`·`approval.check`(재서명) 가 레코드에 박는 "지금" — 마이크로초로 양자화한 벽시계([`quantize_epoch_us`]).
/// 승인 레코드의 시각은 **반드시 이 함수로** 만든다(원문 핀 `r2f_dm_record_producers_use_the_quantized_clock` 이 지킨다).
pub fn record_now() -> f64 {
    quantize_epoch_us(raw_epoch_now())
}

/// 양자화 전 벽시계 — 릴리스에서는 언제나 `state::now_epoch()` 하나다. `cfg(test)` 한정 이음매(`tests::with_raw_clock_script`)로 윈도우 100ns·리눅스 1ns 시계를
/// 맥에서 재현한다(맥의 실제 시계는 1µs 라 양자화가 없어도 어긋나지 않는다). 스레드 지역이라 병렬 검체끼리 간섭하지 않는다.
fn raw_epoch_now() -> f64 {
    #[cfg(test)]
    if let Some(t) = tests::raw_clock_next() {
        return t;
    }
    crate::state::now_epoch()
}

/// 신규 레코드 id 생성: epoch초 + 프로세스 카운터(동일 초 충돌 차단).
pub fn new_record_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "ap-{}-{}-{}",
        std::process::id(),
        crate::state::now_epoch() as u64,
        COUNTER.fetch_add(1, Ordering::Relaxed),
    )
}

/// JSON 객체 {"K":"V",...}를 정렬·민감키 drop된 Vec<(String,String)>로 — RPC env 파라미터 정규화.
pub fn env_from_json(v: &serde_json::Value) -> Vec<(String, String)> {
    let raw: Vec<(String, String)> = v
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    sort_norm_env(&raw)
}

// ── ★(0.14.44 · A3) 대상을 직접 지정하는 cys 명령의 승인은 "폴더" 대신 "서명한 데몬"에 묶는다 ─────────

/// 레코드 `environment` 에 데몬이 넣는 예약 이름. **민감 키 거르기**(`sort_norm_env` — 이름에 TOKEN·SECRET·PASSWORD 같은 글자가 들어 있으면 값을 지운다)에
/// 걸리지 않는 글자로 정했고, 걸리지 않음을 단위 시험이 고정한다(걸리면 묶음이 조용히 사라진다). 요청이 같은 이름을 실어 오면 모든 갈래에서 버린다.
pub const LANE_ENV_KEY: &str = "CYS_APPROVAL_LANE";

/// 폴더 비교를 건너뛸 수 있는 대상 동사 7종 — 게이트 훅이 시간 한정 승인을 요구하는 대상 전부(`role-capability-gate.sh` 의 `CSO_CYS_TTL_VERBS` 6종 + `CSO_CYS_OPT_TTL` 열쇠 `cycle-agent`).
/// 훅의 두 집합과 같은지 대조하는 시험이 있다(`approval_a_tests`).
pub const CWD_NEUTRAL_VERBS: [&str; 7] = cys::APPROVAL_TARGET_VERBS;

/// 매칭 문맥 — 이 데몬의 묶음 값과 "폴더 건너뛰기" 손잡이. 잠금 **밖에서** 만들어 값으로 넘긴다(잠금 안에서는 파일 읽기·기다림이 없다).
#[derive(Clone, Debug)]
pub struct MatchCtx {
    pub lane: String,
    pub neutral: bool,
}

impl ApprovalRecord {
    /// 레코드의 예약 값(데몬 묶음). 없으면 `None` — 무기한 승인과 대상 동사가 아닌 명령의 승인이 그렇다.
    pub fn lane_value(&self) -> Option<&str> {
        self.environment
            .iter()
            .find(|(k, _)| k == LANE_ENV_KEY)
            .map(|(_, v)| v.as_str())
    }
}

/// 요청이 실어 온 환경에서 예약 이름을 버린다 — 데몬만 넣는다(서명·확인의 모든 갈래).
pub fn strip_reserved_env(env: Vec<(String, String)>) -> Vec<(String, String)> {
    env.into_iter().filter(|(k, _)| k != LANE_ENV_KEY).collect()
}

/// 환경에 데몬의 묶음 값을 더해 다시 정렬한다(서명과 매칭이 "정렬돼 있음"을 전제한다).
pub fn with_lane_env(env: Vec<(String, String)>, lane: &str) -> Vec<(String, String)> {
    let mut e = strip_reserved_env(env);
    e.push((LANE_ENV_KEY.to_string(), lane.to_string()));
    sort_norm_env(&e)
}

/// 접두가 `cys`(또는 `cys.exe`) + 대상 동사 7종인가 — 그 동사(소문자 그대로)를 돌려준다.
fn target_verb(prefix: &[String]) -> Option<&'static str> {
    let bin = prefix.first()?;
    if bin != "cys" && bin != "cys.exe" {
        return None;
    }
    let verb = prefix.get(1)?;
    CWD_NEUTRAL_VERBS.iter().copied().find(|v| *v == verb.as_str())
}

/// 이 접두의 서명에 묶음 값을 넣는가(㉡ 의 앞 반 — 접두가 cys + 대상 동사이고 대상 데몬을 바꾸는 옵션이 없다). 시간 한정·손잡이는 호출부가 본다.
pub fn lane_eligible_prefix(prefix: &[String]) -> bool {
    target_verb(prefix).is_some() && !has_socket_option(prefix)
}

/// 대상 데몬을 바꾸는 옵션(`--socket` · `--socket=…` · `-S`)이 **따옴표를 푼 토큰** 가운데 있는가.
pub fn has_socket_option(tokens: &[String]) -> bool {
    tokens
        .iter()
        .any(|t| t == "--socket" || t.starts_with("--socket=") || t == "-S")
}

/// `launch-agent` 의 `--cwd` 값이 **글자 그대로의 절대 경로**인가(마지막 확인 D1) — 데몬이 쪼갠 토큰 기준:
/// `--cwd` 가 정확히 하나, 값이 비어 있지 않고, `/` 로 시작하거나 `<드라이브 문자>:` 뒤에 `/` 또는 `\` 가 오며,
/// 셸이나 CLI 가 실행 좌석의 폴더로 풀어 버릴 글자(`$` · 백틱 · `~` · `*` · `?` · `[` · `{`)가 없다.
pub fn launch_cwd_literal_absolute(tokens: &[String]) -> bool {
    let mut value: Option<&str> = None;
    let mut count = 0usize;
    let mut iter = tokens.iter().peekable();
    while let Some(t) = iter.next() {
        if t == "--cwd" {
            count += 1;
            match iter.peek() {
                Some(v) => value = Some(v.as_str()),
                None => return false, // 값 없이 끝남
            }
        } else if let Some(v) = t.strip_prefix("--cwd=") {
            count += 1;
            value = Some(v);
        }
    }
    if count != 1 {
        return false;
    }
    let Some(v) = value else { return false };
    if v.is_empty() || v.chars().any(|c| matches!(c, '$' | '`' | '~' | '*' | '?' | '[' | '{')) {
        return false;
    }
    if v.starts_with('/') {
        return true;
    }
    let mut cs = v.chars();
    match (cs.next(), cs.next(), cs.next()) {
        (Some(d), Some(':'), Some(sep)) => d.is_ascii_alphabetic() && (sep == '/' || sep == '\\'),
        _ => false,
    }
}

/// 원문 스캔의 한 낱말 — 따옴표 · 이스케이프를 따라가며 모은 **원문 그대로의 글자**와, 따옴표 밖에 연결 기호가 있었는지.
struct ScanWord {
    raw: String,
    meta: bool,
}

/// 토큰화 **전의 원문**을 인용 상태를 따라가며 훑는다(독립 검증 X21 — 토크나이저는 따옴표를 벗겨 `"$(x)"` 와 `'$(x)'` 의 토큰이 같아진다).
/// 복합 명령의 표지(따옴표 밖의 줄바꿈 · 작은따옴표 밖의 백틱과 `$(` · 닫히지 않은 따옴표)가 있으면 `None`.
/// 따옴표 밖의 `;` `&` `|` `<` `>` 는 낱말의 `meta` 로 표시만 한다(출력을 버리는 꼬리는 호출부가 가려낸다).
#[deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
fn scan_words(raw: &str) -> Option<Vec<ScanWord>> {
    let mut words: Vec<ScanWord> = Vec::new();
    let mut cur = String::new();
    let mut meta = false;
    let mut has_word = false;
    let mut quote: Option<char> = None;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some('\'') => {
                cur.push(c);
                if c == '\'' {
                    quote = None;
                }
            }
            Some(_) => {
                // 큰따옴표 안: 백슬래시는 `" \ $ \`` 를 이스케이프한다. 백틱과 `$(` 는 명령 치환(셸이 실행한다)이라 복합으로 본다.
                match c {
                    '"' => {
                        cur.push(c);
                        quote = None;
                    }
                    '\\' => {
                        cur.push(c);
                        if let Some(&n) = chars.peek() {
                            if matches!(n, '"' | '\\' | '$' | '`') {
                                cur.push(n);
                                chars.next();
                            }
                        }
                    }
                    '`' => return None,
                    '$' => {
                        if chars.peek() == Some(&'(') {
                            return None;
                        }
                        cur.push(c);
                    }
                    _ => cur.push(c),
                }
            }
            None => match c {
                '\\' => {
                    cur.push(c);
                    has_word = true;
                    match chars.next() {
                        Some('\n') => return None, // 줄 이음 — 보수적으로 복합
                        Some(n) => cur.push(n),
                        None => {}
                    }
                }
                '\'' | '"' => {
                    quote = Some(c);
                    cur.push(c);
                    has_word = true;
                }
                '\n' | '`' => return None,
                '$' => {
                    if chars.peek() == Some(&'(') {
                        return None;
                    }
                    cur.push(c);
                    has_word = true;
                }
                ';' | '&' | '|' | '<' | '>' => {
                    meta = true;
                    cur.push(c);
                    has_word = true;
                }
                ' ' | '\t' | '\r' => {
                    if has_word {
                        words.push(ScanWord { raw: std::mem::take(&mut cur), meta });
                        meta = false;
                        has_word = false;
                    }
                }
                _ => {
                    cur.push(c);
                    has_word = true;
                }
            },
        }
    }
    if quote.is_some() {
        return None;
    }
    if has_word {
        words.push(ScanWord { raw: cur, meta });
    }
    Some(words)
}

/// 출력을 버리는 꼬리 — 토큰이 **정확히** 이 글자일 때만(띄어 쓴 꼴은 이어진 두 토큰).
fn is_tail_single(t: &str) -> bool {
    matches!(t, "2>&1" | ">/dev/null" | "2>/dev/null" | "&>/dev/null")
}
fn is_tail_pair(a: &str, b: &str) -> bool {
    b == "/dev/null" && matches!(a, ">" | "2>" | "&>")
}

/// ㉢ **단순 명령**인가 — 따옴표 밖에 `;` `&` `|` `<` `>` · 줄바꿈이 없고 작은따옴표 밖에 백틱 · `$(` 가 없다(출력을 버리는 꼬리는 연결 기호로 치지 않는다).
/// 단순 명령이면 **꼬리를 뗀 토큰**을 돌려준다(㉣ 정확 명령 비교용). 토큰화 결과로 판정하지 않는다 — 원문 스캐너 + 토큰화의 낱말 수 일치.
#[deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
pub fn simple_command_core(raw: &str) -> Option<Vec<String>> {
    let words = scan_words(raw)?;
    let toks = tokenize(raw)?;
    if words.len() != toks.len() {
        return None; // 두 눈이 어긋나면 복합으로 본다(보수)
    }
    let mut n = toks.len();
    loop {
        let raw_at = |i: usize| words.get(i).map(|w| w.raw.as_str());
        let tok_at = |i: usize| toks.get(i).map(|t| t.as_str());
        if n >= 1 {
            let i = n - 1;
            if let (Some(t), Some(r)) = (tok_at(i), raw_at(i)) {
                if is_tail_single(t) && t == r {
                    n -= 1;
                    continue;
                }
            }
        }
        if n >= 2 {
            let (i, j) = (n - 2, n - 1);
            if let (Some(ta), Some(tb), Some(ra), Some(rb)) = (tok_at(i), tok_at(j), raw_at(i), raw_at(j)) {
                if is_tail_pair(ta, tb) && ta == ra && tb == rb {
                    n -= 2;
                    continue;
                }
            }
        }
        break;
    }
    if words.iter().take(n).any(|w| w.meta) {
        return None;
    }
    Some(toks.into_iter().take(n).collect())
}

/// **폴더 비교를 건너뛰는가**(A3 매칭의 5번 조건) — 아래가 전부 참일 때뿐이다.
/// ㉠ 레코드에 이 데몬의 예약 값이 있다 · ㉡ 접두가 `cys`(`cys.exe`) + 대상 동사 7종(`launch-agent` 는 낳을 폴더가 글자 그대로의 절대 경로로 적힘)
/// · ㉢ 확인하는 명령이 단순 명령 · ㉣ 정확 명령(꼬리를 뗀 토큰 전체 = 레코드 토큰) · ㉤ 만료 전의 시간 한정 승인(호출부의 만료 필터 + 여기서 `expires_at` 유무).
/// 손잡이(`ctx.neutral`)가 꺼져 있으면 항상 거짓이다.
pub fn neutral_skip(rec: &ApprovalRecord, raw_command: &str, ctx: &MatchCtx) -> bool {
    neutral_skip_verdict(rec, raw_command, ctx) == SkipVerdict::Applies
}

#[derive(Debug, PartialEq, Eq)]
pub enum SkipVerdict {
    Applies,
    /// 건너뛰지 않는다 — 이유 표시(A2 의 보조 표시)
    No { not_exact: bool, needs_launch_cwd: bool, neutral_off: bool },
}

#[deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
pub fn neutral_skip_verdict(rec: &ApprovalRecord, raw_command: &str, ctx: &MatchCtx) -> SkipVerdict {
    let none = SkipVerdict::No { not_exact: false, needs_launch_cwd: false, neutral_off: false };
    // ㉡ + ㉤(접두가 cys + 대상 동사이고 만료 시각이 있는 시간 한정 레코드만)
    let Some(verb) = target_verb(&rec.command_prefix) else { return none };
    if rec.expires_at.is_none() {
        return none;
    }
    // 손잡이가 꺼져 있으면 건너뛰지 않는다 — 꺼진 상태가 조용히 숨지 않게 `neutral_off` 를 말한다(재확인 R7 · 예약 값 유무와 무관).
    if !ctx.neutral {
        return SkipVerdict::No { not_exact: false, needs_launch_cwd: false, neutral_off: true };
    }
    // ㉠
    if rec.lane_value() != Some(ctx.lane.as_str()) {
        return none;
    }
    if verb == "launch-agent" && !launch_cwd_literal_absolute(&rec.command_prefix) {
        return SkipVerdict::No { not_exact: false, needs_launch_cwd: true, neutral_off: false };
    }
    // ㉢ + ㉣
    match simple_command_core(raw_command) {
        Some(core) if core == rec.command_prefix => SkipVerdict::Applies,
        _ => SkipVerdict::No { not_exact: true, needs_launch_cwd: false, neutral_off: false },
    }
}

// ── 묶음 식별자(상태 폴더의 파일) · 정책 손잡이 ─────────────────────────────────────

/// 묶음 식별자 파일 이름 — 데몬의 상태 폴더 안. 데몬마다 다르고(부서 격리), 부서를 지우면 상태 폴더가 보관함으로 옮겨져 다시 만든 부서는 새 값을 받는다.
pub const LANE_FILE: &str = "approval-lane";

fn lane_cache() -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, String>> {
    static C: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<PathBuf, String>>> =
        std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 이 데몬의 예약 값(`소켓 경로|묶음 식별자`). **프로세스가 사는 동안 바뀌지 않는다**(소켓 경로별 캐시 — 같은 값만 돌려준다).
/// 식별자는 `<상태 폴더>/approval-lane` 의 무작위 값이다. 없으면 만든다. 읽지도 만들지도 못하면(권한 · 디스크) 메모리에만 두는 값으로 떨어지고 로그 한 줄을 남긴다.
/// 잠금은 이 함수의 캐시 하나뿐이고 그 안에서 파일 입출력을 하지 않는다.
#[deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
pub fn lane_value(socket_path: &std::path::Path, state_dir: &std::path::Path) -> String {
    {
        let g = lane_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = g.get(socket_path) {
            return v.clone();
        }
    }
    let id = load_or_create_lane_id(state_dir);
    let value = format!("{}|{}", socket_path.display(), id);
    let mut g = lane_cache().lock().unwrap_or_else(|e| e.into_inner());
    g.entry(socket_path.to_path_buf()).or_insert(value).clone()
}

/// 시험 이음매 — 캐시를 비워 "데몬을 다시 띄움"을 흉내 낸다(릴리스 경로에는 호출처가 없다).
#[cfg(test)]
pub(crate) fn forget_lane_cache() {
    lane_cache().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
fn load_or_create_lane_id(state_dir: &std::path::Path) -> String {
    let path = state_dir.join(LANE_FILE);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let t = text.trim();
        if (16..=128).contains(&t.len()) && t.chars().all(|c| c.is_ascii_alphanumeric()) {
            return t.to_string();
        }
    }
    let id = match random_32() {
        Some(b) => b.iter().take(16).map(|x| format!("{x:02x}")).collect::<String>(),
        None => format!("t{:x}{:x}", std::process::id(), crate::state::now_epoch() as u64),
    };
    let written = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(state_dir)?;
        std::fs::write(&path, id.as_bytes())?;
        set_owner_only(&path);
        Ok(())
    })();
    if let Err(e) = written {
        eprintln!(
            "[approval] 묶음 식별자 파일을 만들지 못해 메모리 값으로 둔다({}: {e}) — 데몬을 다시 띄우면 이 데몬의 시간 한정 승인은 다시 서명해야 한다",
            path.display()
        );
    }
    id
}

static NEUTRAL_LOGGED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn policy_path() -> PathBuf {
    store_root().join(".cys").join("policy.json")
}

/// 손잡이 `approval_cwd_neutral` — `~/.cys/policy.json`. 파일이나 키가 없으면 켬(기본). **파일이 있는데 읽지 못하거나 깨졌거나 값이 불리언이 아니면 끈 것으로 본다**
/// (설정을 확실히 읽었을 때만 새 규칙을 쓴다 · 쓰는 도중의 파일을 위해 한 번은 잠깐 뒤에 다시 읽는다). 이 파일은 좌석도 쓸 수 있으므로,
/// 이 키로 갈 수 있는 가장 느슨한 상태가 0.14.43 의 규칙이다. **저장소 잠금 밖에서** 부른다. 켬/끔이 바뀔 때마다 데몬 로그에 한 줄.
#[deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
pub fn cwd_neutral_enabled() -> bool {
    let path = policy_path();
    let read_once = || -> Option<bool> {
        match std::fs::read_to_string(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(true),
            Err(_) => None,
            Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                Err(_) => None,
                Ok(v) => match v.get("approval_cwd_neutral") {
                    None => Some(true),
                    Some(x) => x.as_bool(),
                },
            },
        }
    };
    let on = match read_once() {
        Some(b) => b,
        None => {
            std::thread::sleep(std::time::Duration::from_millis(40));
            read_once().unwrap_or(false)
        }
    };
    let now = if on { 1u8 } else { 2u8 };
    let prev = NEUTRAL_LOGGED.swap(now, std::sync::atomic::Ordering::Relaxed);
    if prev != now {
        eprintln!(
            "[approval] approval_cwd_neutral {} (정책 파일 {})",
            if on { "켬 — 대상 동사 승인은 서명한 데몬에 묶고 폴더를 건너뜀" } else { "끔 — 0.14.43 의 규칙(폴더 비교)으로 돌아감" },
            path.display()
        );
    }
    on
}

// ── ★(0.14.44 · A2) 미승인 사유 설명 — 읽기 전용 순수 함수 ──────────────────────────────

/// 미승인 응답의 `detail`(객체)을 만든다. **저장소를 쓰지 않고 판정을 바꾸지도 않는다** — 이미 "맞는 레코드 없음"으로 끝난 확인에 사유 한 줄을 붙일 뿐이다.
///
/// 저장소 잠금(`mutate_records`) **안에서** 불린다(레코드는 그 트랜잭션 안에서만 보인다) — 그래서 패닉이 없어야 한다: 새 코드는 `unwrap`·`expect`·색인 접근을 린트로 거부한다.
/// 서명값·비밀키는 싣지 않는다.
///
/// 코드 여섯: `bad_quote` · `cwd_mismatch` · `ttl_required` · `expired` · `lane_mismatch` · `no_record`.
/// 고르는 순서: `bad_quote` 는 레코드를 보기 전에. 그다음은 이 데몬의 레코드(예약 값이 없거나 이 데몬 것)를 먼저 설명한다
/// (`cwd_mismatch` → `ttl_required` → `expired`). 그런 레코드가 없을 때만 `lane_mismatch`, 아무것도 없으면 `no_record`.
#[deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
#[allow(clippy::too_many_arguments)]
pub fn explain_no_match(
    records: &[ApprovalRecord],
    secret: &[u8],
    command: &str,
    cwd: Option<&str>,
    env: &[(String, String)],
    now: f64,
    require_ttl: bool,
    ctx: Option<&MatchCtx>,
) -> serde_json::Value {
    let requested_cwd = normalize_cwd(cwd);
    let Some(toks) = tokenize(command) else {
        return serde_json::json!({"code": "bad_quote", "requested_cwd": requested_cwd});
    };
    let socket_opt = has_socket_option(&toks);
    let call_env = sort_norm_env(env);
    // 후보 = 서명이 유효하고 접두가 맞는 레코드. 예약 값이 없거나 이 데몬 것이면 "이 데몬의 레코드"(폴더 · 만료 · TTL 요구를 따로 본다),
    // 다른 데몬 것이면 `lane_mismatch` 후보다(예약 값이 아예 없는 레코드는 이 데몬의 레코드와 같이 설명한다 — 마지막 확인 D9).
    let mut cwd_bad: Vec<&ApprovalRecord> = Vec::new();
    let mut ttl_missing: Option<&ApprovalRecord> = None;
    let mut expired: Option<&ApprovalRecord> = None;
    let mut target_opt: Option<&ApprovalRecord> = None;
    let mut other_lane: Option<&ApprovalRecord> = None;
    for r in records {
        if r.command_prefix.is_empty() || toks.len() < r.command_prefix.len() {
            continue;
        }
        if toks.get(..r.command_prefix.len()) != Some(r.command_prefix.as_slice()) {
            continue;
        }
        if !r.has_valid_signature(secret) {
            continue;
        }
        let ours = match (r.lane_value(), ctx) {
            (None, _) => false,
            (Some(l), Some(c)) if l == c.lane => true,
            (Some(_), _) => {
                other_lane = newer(other_lane, r);
                continue;
            }
        };
        // 환경은 예약 값을 뺀 나머지로 본다(예약 값은 위에서 갈랐다).
        if !r
            .environment
            .iter()
            .filter(|(k, _)| k != LANE_ENV_KEY)
            .all(|kv| call_env.binary_search(kv).is_ok())
        {
            continue;
        }
        if ours && socket_opt {
            target_opt = newer(target_opt, r);
            continue;
        }
        if r.is_expired(now) {
            expired = newer(expired, r);
            continue;
        }
        if require_ttl && r.expires_at.is_none() {
            ttl_missing = newer(ttl_missing, r);
            continue;
        }
        let cwd_ok = match &r.cwd {
            Some(rc) => requested_cwd.as_deref() == Some(rc.as_str()),
            None => true,
        };
        if cwd_ok {
            continue;
        }
        // 폴더가 다르다 — 건너뛰기가 적용되는 레코드면 폴더 때문에 거부될 일이 아니다(다른 이유로 안 맞았다).
        let skipped = ctx.is_some_and(|c| neutral_skip(r, command, c));
        if !skipped {
            cwd_bad.push(r);
        }
    }
    if let Some(first) = cwd_bad.iter().copied().reduce(|a, b| newer(Some(a), b).unwrap_or(a)) {
        let mut signed: Vec<String> = Vec::new();
        for r in &cwd_bad {
            if let Some(c) = &r.cwd {
                if !signed.contains(c) && signed.len() < 3 {
                    signed.push(c.clone());
                }
            }
        }
        let mut d = serde_json::json!({
            "code": "cwd_mismatch", "record_id": first.id,
            "signed_cwd": signed, "requested_cwd": requested_cwd,
        });
        if let (Some(c), Some(o)) = (ctx, d.as_object_mut()) {
            if let SkipVerdict::No { not_exact, needs_launch_cwd, neutral_off } = neutral_skip_verdict(first, command, c) {
                for (k, on) in [("not_exact", not_exact), ("needs_launch_cwd", needs_launch_cwd), ("neutral_off", neutral_off)] {
                    if on {
                        o.insert(k.into(), serde_json::Value::Bool(true));
                    }
                }
            }
        }
        return d;
    }
    if let Some(r) = ttl_missing {
        return serde_json::json!({
            "code": "ttl_required", "record_id": r.id, "requested_cwd": requested_cwd,
        });
    }
    if let Some(r) = expired {
        return serde_json::json!({
            "code": "expired", "record_id": r.id, "expired_at": r.expires_at,
            "requested_cwd": requested_cwd,
        });
    }
    if let Some(r) = target_opt {
        return serde_json::json!({
            "code": "lane_mismatch", "target_option": true, "record_id": r.id,
            "requested_cwd": requested_cwd,
        });
    }
    if let Some(r) = other_lane {
        return serde_json::json!({
            "code": "lane_mismatch", "record_id": r.id, "requested_cwd": requested_cwd,
        });
    }
    serde_json::json!({"code": "no_record", "requested_cwd": requested_cwd})
}

/// 둘 중 더 최근에 갱신된 레코드(같으면 새로 들어온 쪽이 아니라 기존 쪽을 유지).
fn newer<'a>(cur: Option<&'a ApprovalRecord>, cand: &'a ApprovalRecord) -> Option<&'a ApprovalRecord> {
    match cur {
        Some(c) if c.updated_at >= cand.updated_at => Some(c),
        _ => Some(cand),
    }
}

// ── 테스트 (E-n: 10종, hmac_kat = RFC 4231) ──────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-approval-secret-32-bytes!!!";

    // ── 승인 저장소 루트 이음매(§`store_root`) ────────────────────────────────
    /// 검체가 주입한 루트. `None` 이면 릴리스 경로(`dirs::home_dir()`) 그대로다.
    /// 프로세스 **전역** 값이므로 승인 저장소를 만지는 검체들은 서로 직렬화돼야 한다
    /// (handlers 쪽 `ACL_ENV_LOCK` 이 그 역할을 한다 — `HOME` 교체도 같은 이유로 그 락 안이다).
    static STORE_ROOT: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

    pub(crate) fn store_root_override() -> Option<PathBuf> {
        STORE_ROOT.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 되돌리기를 잊을 수 없게 **가드**로 준다 — 살아 있는 동안만 루트가 바뀐다.
    /// 이것이 필요한 이유는 `store_root` 의 doc 참조(Windows 의 `home_dir()` 은 `HOME` 을 보지
    /// 않는다 — `HOME` 만 바꾼 검체는 실제 사용자 프로필의 승인 저장소를 만진다).
    #[must_use]
    pub(crate) fn with_store_root(root: &std::path::Path) -> StoreRootGuard {
        // 이음매 자체를 직렬화한다 — 루트는 프로세스 전역이라, 병렬 검체 둘이 동시에 세우면
        // 한쪽이 다른 쪽의 임시 저장소를 읽는다(그 실패는 산발적이라 진단이 가장 비싸다).
        let held = SEAM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *STORE_ROOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(root.to_path_buf());
        StoreRootGuard(held)
    }

    static SEAM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub(crate) struct StoreRootGuard(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

    impl Drop for StoreRootGuard {
        fn drop(&mut self) {
            // 필드(`SEAM_LOCK` 가드)는 이 본문 **뒤에** 풀린다 — 루트를 되돌린 뒤에 다음
            // 검체가 들어온다.
            *STORE_ROOT.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    /// 이음매가 실제로 경로를 옮기는가 — 이것이 거짓이면 위 검체들이 **사용자 프로필**을 만진다.
    #[test]
    fn store_root_seam_moves_every_approval_path() {
        let _g = with_store_root(std::path::Path::new("/tmp/cys-approval-seam-probe"));
        for p in [records_path(), ttl_records_path(), secret_path()] {
            assert!(
                p.starts_with("/tmp/cys-approval-seam-probe"),
                "이음매가 적용되지 않은 경로가 있다: {}",
                p.display()
            );
        }
        drop(_g);
        // 가드가 풀리면 릴리스 경로로 돌아온다(검체가 서로의 저장소를 물려받지 않는다).
        assert!(store_root_override().is_none());
    }

    fn rec(prefix: &[&str], cwd: Option<&str>, env: &[(&str, &str)]) -> ApprovalRecord {
        ApprovalRecord {
            version: 1,
            id: "ap-test-1".to_string(),
            command_prefix: prefix.iter().map(|s| s.to_string()).collect(),
            cwd: cwd.map(|c| c.to_string()),
            environment: sort_norm_env(
                &env.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect::<Vec<_>>(),
            ),
            created_at: 1000.0,
            updated_at: 1000.0,
            expires_at: None,
            signature: String::new(),
        }
    }

    /// TTL 있는 레코드(만료 시각 지정) — B-3 검체 전용.
    fn rec_ttl(prefix: &[&str], cwd: Option<&str>, expires_at: f64) -> ApprovalRecord {
        let mut r = rec(prefix, cwd, &[]);
        r.expires_at = Some(expires_at);
        r
    }

    #[test]
    fn sign_verify_roundtrip() {
        let mut r = rec(&["git", "push"], None, &[]);
        r.sign(SECRET);
        assert!(!r.signature.is_empty());
        assert!(r.has_valid_signature(SECRET));
    }

    #[test]
    fn tampered_rejected() {
        // 변종 1: command_prefix 변조
        let mut r = rec(&["git", "push"], None, &[]);
        r.sign(SECRET);
        let mut t1 = r.clone();
        t1.command_prefix = vec!["git".into(), "pull".into()];
        assert!(!t1.has_valid_signature(SECRET), "command_prefix 변조 통과");

        // 변종 2: cwd 변조
        let mut r2 = rec(&["git", "push"], Some("/a"), &[]);
        r2.sign(SECRET);
        let mut t2 = r2.clone();
        t2.cwd = Some("/b".into());
        assert!(!t2.has_valid_signature(SECRET), "cwd 변조 통과");

        // 변종 3: env 변조
        let mut r3 = rec(&["git", "push"], None, &[("CI", "1")]);
        r3.sign(SECRET);
        let mut t3 = r3.clone();
        t3.environment = vec![("CI".into(), "2".into())];
        assert!(!t3.has_valid_signature(SECRET), "env 변조 통과");

        // 변종 4: signature 한 글자 변조
        let mut t4 = r.clone();
        let mut sig: Vec<char> = t4.signature.chars().collect();
        // 첫 글자를 다른 base64 문자로 치환
        sig[0] = if sig[0] == 'A' { 'B' } else { 'A' };
        t4.signature = sig.into_iter().collect();
        assert!(!t4.has_valid_signature(SECRET), "signature 변조 통과");
    }

    #[test]
    fn wrong_secret_rejected() {
        let mut r = rec(&["git", "push"], None, &[]);
        r.sign(SECRET);
        assert!(!r.has_valid_signature(b"a-completely-different-secret-key"));
    }

    #[test]
    fn prefix_match() {
        let r = rec(&["git", "push"], None, &[]);
        assert!(r.matches("git push origin main", None, &[]));
        assert!(!r.matches("git status", None, &[]), "git status 오매칭");
        assert!(!r.matches("git", None, &[]), "토큰 부족인데 매칭");
    }

    #[test]
    fn empty_prefix_no_fallback() {
        let r = rec(&[], None, &[]);
        assert!(!r.matches("anything goes here", None, &[]));
    }

    #[test]
    fn unclosed_quote_rejected() {
        assert!(tokenize("git push 'x").is_none(), "미닫힌 따옴표 토큰화 통과");
        let r = rec(&["git", "push"], None, &[]);
        assert!(
            !r.matches("git push 'unterminated", None, &[]),
            "미닫힌 따옴표 명령 매칭 통과(prefix injection)"
        );
    }

    #[test]
    fn longest_prefix_wins() {
        let mut short = rec(&["git"], None, &[]);
        short.id = "ap-short".into();
        short.sign(SECRET);
        let mut long = rec(&["git", "push"], None, &[]);
        long.id = "ap-long".into();
        long.sign(SECRET);
        let recs = vec![short, long];
        let best = best_match(&recs, SECRET, "git push origin main", None, &[]).unwrap();
        assert_eq!(best.id, "ap-long", "최장 prefix 미선택");
    }

    #[test]
    fn sensitive_env_dropped() {
        let r = rec(&["deploy"], None, &[("API_KEY", "leak"), ("CI", "1")]);
        // 레코드 environment에서 API_KEY가 제거되고 CI만 남는다.
        assert!(
            r.environment.iter().all(|(k, _)| k != "API_KEY"),
            "API_KEY가 레코드에 잔존"
        );
        assert!(r.environment.iter().any(|(k, _)| k == "CI"));
        // 서명 페이로드에도 시크릿값이 없어야 한다.
        let payload = String::from_utf8(r.signing_payload()).unwrap();
        let leaked = b64_encode(b"leak");
        assert!(!payload.contains(&leaked), "시크릿값이 서명 페이로드에 유출");
    }

    #[test]
    fn determinism() {
        let mut r1 = rec(&["git", "push"], Some("/x"), &[("CI", "1"), ("AAA", "2")]);
        let mut r2 = rec(&["git", "push"], Some("/x"), &[("AAA", "2"), ("CI", "1")]);
        r1.sign(SECRET);
        r2.sign(SECRET);
        assert_eq!(r1.signature, r2.signature, "동일 입력 2회 서명 불일치(비결정)");
    }

    /// RFC 4231 Test Case 2 — 수동 HMAC-SHA256 정확성 박제.
    /// Key = "Jefe", Data = "what do ya want for nothing?"
    /// HMAC-SHA256 = 5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843
    #[test]
    fn hmac_kat() {
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let expected: [u8; 32] = [
            0x5b, 0xdc, 0xc1, 0x46, 0xbf, 0x60, 0x75, 0x4e, 0x6a, 0x04, 0x24, 0x26, 0x08, 0x95,
            0x75, 0xc7, 0x5a, 0x00, 0x3f, 0x08, 0x9d, 0x27, 0x39, 0x83, 0x9d, 0xec, 0x58, 0xb9,
            0x64, 0xec, 0x38, 0x43,
        ];
        assert_eq!(hmac_sha256(key, data), expected, "RFC 4231 TC2 KAT 실패");

        // RFC 4231 Test Case 1 — Key = 0x0b*20, Data = "Hi There"
        // HMAC-SHA256 = b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7
        let key1 = [0x0bu8; 20];
        let data1 = b"Hi There";
        let expected1: [u8; 32] = [
            0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
            0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
            0x2e, 0x32, 0xcf, 0xf7,
        ];
        assert_eq!(hmac_sha256(&key1, data1), expected1, "RFC 4231 TC1 KAT 실패");

        // RFC 4231 Test Case 3 — long key path (key>64B 축약 검증):
        // Key = 0xaa*131, Data = "Test Using Larger Than Block-Size Key - Hash Key First"
        // HMAC-SHA256 = 60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54
        let key3 = [0xaau8; 131];
        let data3 = b"Test Using Larger Than Block-Size Key - Hash Key First";
        let expected3: [u8; 32] = [
            0x60, 0xe4, 0x31, 0x59, 0x1e, 0xe0, 0xb6, 0x7f, 0x0d, 0x8a, 0x26, 0xaa, 0xcb, 0xf5,
            0xb7, 0x7f, 0x8e, 0x0b, 0xc6, 0x21, 0x37, 0x28, 0xc5, 0x14, 0x05, 0x46, 0x04, 0x0f,
            0x0e, 0xe3, 0x7f, 0x54,
        ];
        assert_eq!(
            hmac_sha256(&key3, data3),
            expected3,
            "RFC 4231 TC3 KAT 실패(키 축약)"
        );
    }

    // ── ★(0.14.31 · CONTRACTS B-3) 승인 TTL ────────────────────────────────────
    //
    // 계약: `sign --ttl <secs>` 가 `expires_at`(epoch)을 **서명 페이로드에 포함**하고,
    // `check --require-ttl` 은 만료되지 않은 TTL 레코드가 있을 때만 통과한다. TTL 없는
    // 구 레코드는 `--require-ttl` 에서 실패한다. 실패 방향은 전부 **거부(deny)** 다.

    /// ★무TTL 레코드의 서명 페이로드는 종전과 **바이트 동일**이어야 한다 — 아니면 이 릴리스가
    /// 설치된 모든 승인 레코드를 한 번에 무효화한다(자율주행 전면 정지 · 가용성 사고).
    ///
    /// ★R1 에서 고친 것(codex 적대검증 major): 종전 검체는 "마지막 줄이 `updatedAt=` 이고
    /// `expiresAt` 이 없다"는 **모양**만 봤다 — `createdAt` 의 포맷을 바꾸거나 `commandPrefix`
    /// 의 구분자를 바꿔도 초록이었다(그러면 설치된 서명이 전부 죽는데도). 그래서 여기서는
    /// **0.14.30 이 실제로 서명하던 그 바이트열과 그 서명**을 냉동 검체로 박는다. 이 두 상수는
    /// 코드에서 파생되지 않는다 — 밖에서 계산한 값이므로 페이로드 조립이 한 글자라도 달라지면
    /// 즉시 빨강이다.
    const FROZEN_LEGACY_PAYLOAD: &str = "version=1\nid=ap-test-1\ncommandPrefix=Z2l0,cHVzaA==\ncwd=L3g=\nenvironment=Q0k==MQ==\ncreatedAt=1000\nupdatedAt=1000";
    const FROZEN_LEGACY_SIG: &str = "2i7Ob8Rdqy1HDkUYBMGWbk9tiZ45iMSmWptf6GwXdVA=";

    #[test]
    fn ttl_absent_payload_is_byte_identical_to_legacy() {
        let r = rec(&["git", "push"], Some("/x"), &[("CI", "1")]);
        let payload = String::from_utf8(r.signing_payload()).unwrap();
        assert_eq!(
            payload, FROZEN_LEGACY_PAYLOAD,
            "무TTL 페이로드가 0.14.30 의 바이트열과 다르다 — 설치된 모든 승인 서명이 죽는다"
        );
        // 그 시절 서명이 **지금 코드에서도** 유효해야 한다(레코드 무효화 0의 진짜 정의).
        let mut legacy = r.clone();
        legacy.signature = FROZEN_LEGACY_SIG.to_string();
        assert!(
            legacy.has_valid_signature(SECRET),
            "구 릴리스가 만든 서명이 이 릴리스에서 거부됐다(승인 전멸 = 자율주행 정지)"
        );
        assert!(!payload.contains("expiresAt"), "무TTL 레코드에 expiresAt 줄이 붙었다");
        // TTL 이 붙으면 줄이 정확히 하나 늘고 그 줄이 말미다 — 그리고 구 서명은 그 순간 무효다.
        let mut t = legacy.clone();
        t.expires_at = Some(4242.0);
        let p2 = String::from_utf8(t.signing_payload()).unwrap();
        assert_eq!(p2, format!("{FROZEN_LEGACY_PAYLOAD}\nexpiresAt=4242"));
        assert!(!t.has_valid_signature(SECRET), "만료 주입이 구 서명을 통과했다");
    }

    /// TTL 은 서명에 묶인다 — 만료 연장(값 변경)·제거·주입 전부 서명 불일치로 거부.
    #[test]
    fn ttl_tamper_rejected() {
        let mut r = rec_ttl(&["git", "push"], Some("/x"), 2000.0);
        r.sign(SECRET);
        assert!(r.has_valid_signature(SECRET));

        let mut extend = r.clone();
        extend.expires_at = Some(9_999_999.0);
        assert!(!extend.has_valid_signature(SECRET), "만료 연장이 서명을 통과했다");

        let mut strip = r.clone();
        strip.expires_at = None;
        assert!(!strip.has_valid_signature(SECRET), "만료 제거(무기한 승격)가 서명을 통과했다");

        // 반대 방향: 무기한 레코드에 만료를 주입해도(=축소) 서명 불일치.
        let mut base = rec(&["git", "push"], Some("/x"), &[]);
        base.sign(SECRET);
        let mut inject = base.clone();
        inject.expires_at = Some(1.0);
        assert!(!inject.has_valid_signature(SECRET), "만료 주입이 서명을 통과했다");
    }

    /// 신선한 TTL 레코드는 `--require-ttl` 유무와 무관하게 매칭된다.
    #[test]
    fn ttl_fresh_matches_both_modes() {
        let mut r = rec_ttl(&["git", "push"], Some("/x"), 2000.0);
        r.sign(SECRET);
        let recs = vec![r];
        let now = 1500.0;
        assert!(
            best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], now, false)
                .is_some(),
            "신선한 TTL 레코드가 일반 check 에서 탈락"
        );
        assert!(
            best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], now, true)
                .is_some(),
            "신선한 TTL 레코드가 --require-ttl 에서 탈락"
        );
    }

    /// ★만료 레코드는 **요구 여부와 무관하게** 매칭되지 않는다(만료는 사실이지 옵션이 아니다).
    /// 경계 포함: `now == expires_at` 도 만료다.
    #[test]
    fn ttl_expired_never_matches() {
        let mut r = rec_ttl(&["git", "push"], Some("/x"), 2000.0);
        r.sign(SECRET);
        let recs = vec![r];
        for (now, label) in [(2000.0_f64, "경계(now==expires_at)"), (2000.1, "만료 후")] {
            assert!(
                best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], now, false)
                    .is_none(),
                "{label}: 만료 레코드가 일반 check 를 통과했다"
            );
            assert!(
                best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], now, true)
                    .is_none(),
                "{label}: 만료 레코드가 --require-ttl 을 통과했다"
            );
        }
    }

    /// ★음성 대조(결측은 값이 아니다): TTL **없는** 레코드는 `--require-ttl` 에서 실패하고,
    /// 그 실패가 '일반 check 도 못 쓴다'로 번지지 않는다(무기한 승인의 종전 계약 보존).
    #[test]
    fn ttl_missing_record_fails_require_ttl_but_keeps_legacy_check() {
        let mut r = rec(&["git", "push"], Some("/x"), &[]);
        r.sign(SECRET);
        let recs = vec![r];
        let now = 5000.0;
        assert!(
            best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], now, true)
                .is_none(),
            "TTL 없는 레코드가 --require-ttl 을 통과했다(계약 B-3 위반)"
        );
        assert!(
            best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], now, false)
                .is_some(),
            "TTL 없는 레코드의 종전 check 통과가 깨졌다(구 승인 전멸)"
        );
    }

    /// 만료 레코드가 섞여 있어도 **살아있는 짧은 prefix** 를 가리지 않는다 — 만료분을 먼저
    /// 걸러내지 않고 최장 prefix 를 고르면 "가장 잘 맞는 승인이 죽었다"가 곧 전면 거부가 된다.
    #[test]
    fn expired_longest_prefix_does_not_shadow_live_shorter() {
        let mut dead = rec_ttl(&["git", "push", "origin"], Some("/x"), 1000.0);
        dead.id = "ap-dead".into();
        dead.sign(SECRET);
        let mut live = rec(&["git", "push"], Some("/x"), &[]);
        live.id = "ap-live".into();
        live.sign(SECRET);
        let recs = vec![dead, live];
        let best =
            best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], 5000.0, false)
                .expect("살아있는 짧은 prefix 가 선택돼야");
        assert_eq!(best.id, "ap-live", "만료된 최장 prefix 가 살아있는 승인을 가렸다");
    }

    // ── ★서명 세탁·만료 연장 (codex gpt-6-astra 위임 작성 · 전 줄 검토 후 채택) ──
    // 채택 시 수정 1건: T2 의 마지막 단언 문안이 "TTL 필수 여부와 무관하게"라고 말하면서 실제로는
    // `require_ttl=false` 만 재고 있었다 — 말과 측정이 어긋나면 그 문장이 다음 사람을 속인다.
    // 두 모드를 **둘 다 재도록** 고쳐서 문안을 사실로 만들었다.

    /// 같은 id의 위조본을 앞에 삽입하면 id 재검색으로 재서명 대상을 바꿀 수 있다.
    /// 더 긴 prefix와 먼 만료 시각을 가진 위조본도 검증된 인덱스를 대신해서는 안 된다.
    /// 이 검사가 실패하면 approval.check의 재서명이 위조본을 유효한 승인으로 세탁할 수 있다.
    #[test]
    fn duplicate_id_forgery_is_never_the_selected_index() {
        let forged = rec_ttl(&["git", "push", "origin"], None, 1_000_000_000.0);
        let mut valid = rec_ttl(&["git", "push"], None, 2000.0);
        valid.sign(SECRET);

        assert_eq!(forged.id, "ap-test-1", "위조본의 id가 대조 조건과 다릅니다");
        assert_eq!(forged.id, valid.id, "두 검체의 id가 같아야 공격을 재현합니다");
        assert!(!forged.has_valid_signature(SECRET), "위조본은 서명이 무효여야 합니다");
        let records = vec![forged, valid];

        for require_ttl in [false, true] {
            let selected = best_match_index_at(
                &records, SECRET, "git push origin main", None, &[], 1500.0, require_ttl,
            );
            assert_eq!(
                selected,
                Some(1),
                "검증된 두 번째 레코드를 선택해야 합니다: TTL 필수={require_ttl}"
            );
            let idx = selected.expect("유효한 레코드의 인덱스가 누락되었습니다");
            assert!(
                records[idx].has_valid_signature(SECRET),
                "선택된 레코드의 서명이 무효입니다: TTL 필수={require_ttl}"
            );
            assert_eq!(
                records[idx].command_prefix,
                vec!["git".to_string(), "push".to_string()],
                "선택된 prefix가 유효본과 다릅니다: TTL 필수={require_ttl}"
            );
            assert_eq!(
                best_match_index_at(
                    &records[..1], SECRET, "git push origin main", None, &[], 1500.0, require_ttl,
                ),
                None,
                "위조본만 남아도 무효 서명을 선택해서는 안 됩니다: TTL 필수={require_ttl}",
            );
        }
    }

    /// 승인 사용 시 updated_at 갱신과 재서명이 절대 만료 시각을 연장해서는 안 된다.
    /// 만료 직전 사용을 반복하는 공격자가 승인을 계속 살려 둘 수 있는지 검사한다.
    /// 이 검사가 실패하면 처음 정한 TTL 이후에도 같은 승인이 재사용될 수 있다.
    #[test]
    fn resigning_updated_at_preserves_absolute_expiry() {
        let mut record = rec_ttl(&["git", "push"], None, 2000.0);
        let initial_expiry = record.expires_at;
        record.sign(SECRET);
        assert!(record.has_valid_signature(SECRET), "초기 TTL 레코드의 서명이 무효입니다");
        assert_eq!(record.expires_at, initial_expiry, "초기 서명이 만료 시각을 변경했습니다");

        for updated_at in [1200.0, 1600.0, 1999.0] {
            record.updated_at = updated_at;
            record.sign(SECRET);
            assert!(
                record.has_valid_signature(SECRET),
                "updated_at={updated_at} 갱신 후 재서명이 무효입니다"
            );
            assert_eq!(
                record.expires_at, initial_expiry,
                "updated_at={updated_at} 갱신 후 절대 만료 시각이 변경되었습니다"
            );
        }

        let records = [record];
        assert_eq!(
            best_match_index_at(&records, SECRET, "git push origin main", None, &[], 1999.0, false),
            Some(0),
            "재서명된 승인은 초기 만료 시각 전에는 선택되어야 합니다",
        );
        for require_ttl in [false, true] {
            assert_eq!(
                best_match_index_at(
                    &records, SECRET, "git push origin main", None, &[], 2001.0, require_ttl,
                ),
                None,
                "재서명을 반복해도 초기 만료 시각 이후에는 거부해야 합니다: TTL 필수={require_ttl}",
            );
        }
    }

    #[test]
    fn b64_roundtrip() {
        for s in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            b"\x00\xff\x10",
        ] {
            let enc = b64_encode(s);
            assert_eq!(b64_decode(&enc).as_deref(), Some(s), "base64 라운드트립 실패");
        }
        // 알려진 벡터(RFC 4648)
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(b64_decode("Zm9vYmFy").unwrap(), b"foobar");
    }

    /// ★(R2F-DM · 성찰 2회차 A4 M1 ⓒ) **`HOME` 만 돌리는 검체 금지**(소스 전수 핀) — 윈도우의 `dirs::home_dir()` 은 `HOME` 을 보지 않아, `HOME` 만 임시 폴더로 돌린 검체는 임시 폴더가 아니라 러너의 **실제 프로필**을 읽고 쓰며
    /// (`~\.cys\approvals*.json` 오염 + 찾는 파일이 없어 실패) 맥에서는 우연히 통한다. 그래서 데몬 소스 전체(디렉터리 스캔 · 목록 밖 파일 포함)에서 프로세스 `HOME` 을 바꾸는 검체는 **같은 함수 안에서**
    /// 저장소 루트 이음매(`with_store_root(`) 나 스레드 지역 홈 이음매(`test_home::set(`)를 함께 쓴다. 이 검체가 붉으면 새 검체가 `HOME` 에만 기대는 것이다 — 이음매로 옮겨라.
    #[test]
    fn r2f_dm_no_daemon_test_turns_only_the_home_env_without_a_seam() {
        let needle = concat!("set_var(\"HO", "ME\"");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cysd");
        let (mut scanned, mut hits) = (0usize, 0usize);
        for e in std::fs::read_dir(&dir).expect("cysd 소스 디렉터리") {
            let file = e.expect("항목").file_name().to_string_lossy().to_string();
            if !file.ends_with(".rs") {
                continue;
            }
            let text = std::fs::read_to_string(dir.join(&file)).expect("소스 읽기");
            scanned += 1;
            // 함수 단위로 자른다 — 4칸 들여쓴 `fn`(검체 모듈 안의 함수·도우미) 머리에서 나눈다.
            let mut cuts: Vec<usize> = text.match_indices("\n    fn ").chain(text.match_indices("\n    pub(crate) fn ")).map(|(i, _)| i).collect();
            cuts.sort_unstable();
            let mut chunks: Vec<&str> = Vec::new();
            let mut start = 0usize;
            for i in cuts {
                chunks.push(&text[start..i]);
                start = i;
            }
            chunks.push(&text[start..]);
            for c in chunks {
                if !c.contains(needle) {
                    continue;
                }
                hits += 1;
                assert!(
                    c.contains("with_store_root(") || c.contains("test_home::set("),
                    "{file}: `HOME` 만 돌리는 검체가 있다(이음매 없음) — 윈도우에서는 러너의 실제 프로필을 건드린다:\n{}",
                    c.lines().take(6).collect::<Vec<_>>().join("\n")
                );
            }
        }
        assert!(scanned >= 20, "스캔이 공허하다({scanned}개)");
        assert!(hits >= 6, "`HOME` 을 바꾸는 검체가 {hits}곳뿐이다 — 판정 대상이 사라졌다면 핀을 점검하라(기대 ≥ 6: handlers 승인 RPC 검체 등)");
    }

    // ── ★(R2F-DM · 성찰 2회차) 승인 레코드 시각의 **마이크로초 양자화** — 윈도우 서명 불일치(JSON f64 왕복)의 수리 검체 ────────────────────────────────────
    // 서명 재료의 `createdAt={f64}`·`updatedAt={f64}`·`expiresAt={f64}` 는 `Display` 문자열이고 레코드는 `serde_json` 으로 저장·재독한다. 윈도우 시계는 100ns(리눅스는 1ns)라 시각이 유효숫자
    // 17자리가 되는데 `serde_json` 기본 파서는 그런 값의 약 10 % 를 같은 값으로 돌려주지 못한다 → 서명 불일치 → 승인 거부(`cys approval check` · guard 훅의 LOOSE 우회가 먹지 않는다).
    // 수리 = 시각을 만드는 두 곳(`approval.sign`·`approval.check` 의 재서명)이 `record_now()`(마이크로초 양자화)를 쓴다. `float_roundtrip` 기능은 0.14.43 K1 설계상 켜지 않는다(`quantize_epoch_us` 의 doc).
    // 맥의 실제 시계는 1µs 라 양자화 없이도 어긋나지 않는다 → 윈도우·리눅스 시계는 시계 이음매(`with_raw_clock_script`)와 결정론 격자로 재현한다. 시계·난수 없이 결정론이다.

    // ── 시계 이음매(§`raw_epoch_now`) — `cfg(test)` 한정 · 스레드 지역 ───────────────────────────────────────────────────────────────
    thread_local! {
        /// 검체가 주입한 "원시 벽시계 판독" 대기열(앞에서부터 하나씩 소비). 비어 있으면 실제 시계다.
        static RAW_CLOCK_SCRIPT: std::cell::RefCell<std::collections::VecDeque<f64>> =
            std::cell::RefCell::new(std::collections::VecDeque::new());
    }

    pub(crate) fn raw_clock_next() -> Option<f64> {
        RAW_CLOCK_SCRIPT.with(|s| s.borrow_mut().pop_front())
    }

    /// 아직 소비되지 않은 판독 수 — 검체가 "두 생산자가 모두 이 시계를 썼다"를 단언하는 데 쓴다.
    pub(crate) fn raw_clock_remaining() -> usize {
        RAW_CLOCK_SCRIPT.with(|s| s.borrow().len())
    }

    /// 원시 시계를 판독 목록으로 갈아 끼운다 — 가드가 살아 있는 동안만(드롭하면 비운다 · 이 스레드 한정).
    #[must_use]
    pub(crate) fn with_raw_clock_script(readings: &[f64]) -> RawClockGuard {
        RAW_CLOCK_SCRIPT.with(|s| *s.borrow_mut() = readings.iter().copied().collect());
        RawClockGuard
    }

    pub(crate) struct RawClockGuard;

    impl Drop for RawClockGuard {
        fn drop(&mut self) {
            RAW_CLOCK_SCRIPT.with(|s| s.borrow_mut().clear());
        }
    }

    /// 결정론 시각 격자 — 고정 시작(2026-10-04T08:20:00Z 근방) · 고정 보폭(7,919,113 칸) · 칸 크기 `unit_ns`(100 = 윈도우 `SystemTime`, 1 = 리눅스 `clock_gettime`). 값은 `Duration::as_secs_f64` 라 제품의 `now_epoch()` 와 같은 산출식이다.
    fn r2f_grid_epoch(unit_ns: u64, i: u64) -> f64 {
        let total_ns: u128 = 1_791_102_000u128 * 1_000_000_000 + (i as u128) * 7_919_113u128 * (unit_ns as u128);
        std::time::Duration::new((total_ns / 1_000_000_000) as u64, (total_ns % 1_000_000_000) as u32).as_secs_f64()
    }

    /// 위 격자에서 **기본 파서가 같은 값으로 돌려주지 못하는** 17자리 값(탐침 실측) — 100ns 격자 i=3·14·46 · 1ns 격자 i=2·3.
    pub(crate) const R2F_BAD_100NS: [f64; 3] = [1791102002.3757339, 1791102011.0867581, 1791102036.4279199];
    pub(crate) const R2F_BAD_1NS: [f64; 2] = [1791102000.0158381, 1791102000.0237575];

    /// **지금 시각 근처**에서 기본 파서가 같은 값으로 돌려주지 못하는 17자리 "원시 판독" `n` 개 — 100ns 칸 · 약 0.8ms 보폭(시작 시각만 실제 시계 · 나머지는 결정론).
    /// RPC 검체는 만료·신선도를 **실제 시계**로 판정하므로 합성 시계도 지금 근처여야 한다(고정한 과거 시각이면 TTL 이 이미 지나 승인이 거부된다).
    pub(crate) fn r2f_lossy_readings_near_now(n: usize) -> Vec<f64> {
        let now_ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("시스템 시계").as_nanos();
        let base_ns = now_ns / 100 * 100;
        let mut out = Vec::with_capacity(n);
        for i in 0..50_000u128 {
            let total = base_ns + i * 7_919 * 100;
            let x = std::time::Duration::new((total / 1_000_000_000) as u64, (total % 1_000_000_000) as u32).as_secs_f64();
            if r2f_lossy(x) {
                out.push(x);
                if out.len() == n {
                    break;
                }
            }
        }
        assert_eq!(out.len(), n, "지금 근처에서 기본 파서가 틀리는 값을 {n}개 찾지 못했다({}개) — 이 빌드의 파서가 정확해졌나(`float_roundtrip`?)", out.len());
        out
    }

    /// `x` 를 JSON 으로 쓰고 읽는다 — `load_records_from` 이 실제로 타는 두 길(`from_str::<f64>` · `from_str::<Value> → as_f64`)과 쓴 문자열.
    fn r2f_json_roundtrip(x: f64) -> (f64, f64, String) {
        let text = serde_json::to_string(&x).expect("직렬화");
        let via_f64: f64 = serde_json::from_str(&text).expect("f64 판독");
        let via_value = serde_json::from_str::<serde_json::Value>(&text).expect("Value 판독").as_f64().expect("수치");
        (via_f64, via_value, text)
    }

    /// 두 길 가운데 하나라도 `to_bits()` 까지 같지 않으면 참.
    fn r2f_lossy(x: f64) -> bool {
        let (a, b, _) = r2f_json_roundtrip(x);
        a.to_bits() != x.to_bits() || b.to_bits() != x.to_bits()
    }

    /// ① 양자화한 시각은 100ns 격자·1ns 격자 각 20,000 값에서 **두 판독 길 모두 비트까지 정확히** 돌아온다(불일치 0) — 만료 시각(양자화한 지금 + 정수 TTL 을 다시 양자화 = 핸들러가 하는 식)도 같다.
    /// 양자화는 멱등이고 · 값을 1µs 넘게 옮기지 않으며 · 유효숫자 16자리 이하다.
    /// 공허 방지: 같은 격자의 **원시** 값은 기본 파서에서 1,000건 이상 어긋난다(탐침 실측 100ns 격자 2076건 · 1ns 격자 2456건) — 양자화가 항등이면 아래 단언이 붉다.
    #[test]
    fn r2f_dm_quantized_record_clock_survives_json_exactly_on_the_100ns_and_1ns_grids() {
        for (label, unit) in [("100ns(윈도우 시계)", 100u64), ("1ns(리눅스 시계)", 1u64)] {
            let (mut raw_lossy, mut q_lossy, mut sum_lossy, mut not_idem) = (0u32, 0u32, 0u32, 0u32);
            let (mut max_digits, mut max_shift) = (0usize, 0f64);
            let mut first_bad: Option<f64> = None;
            for i in 0..20_000u64 {
                let raw = r2f_grid_epoch(unit, i);
                if r2f_lossy(raw) {
                    raw_lossy += 1;
                }
                let q = quantize_epoch_us(raw);
                max_shift = max_shift.max((q - raw).abs());
                if quantize_epoch_us(q).to_bits() != q.to_bits() {
                    not_idem += 1;
                }
                let (_, _, text) = r2f_json_roundtrip(q);
                max_digits = max_digits.max(text.bytes().filter(u8::is_ascii_digit).count());
                if r2f_lossy(q) {
                    q_lossy += 1;
                    first_bad.get_or_insert(raw);
                }
                for ttl in [1u64, 60, 3_600, 86_400, 2_592_000] {
                    if r2f_lossy(quantize_epoch_us(q + ttl as f64)) {
                        sum_lossy += 1;
                        first_bad.get_or_insert(raw);
                    }
                }
            }
            assert!(
                raw_lossy >= 1_000,
                "{label}: 원시 값이 기본 파서에서 {raw_lossy}/20000 건만 어긋난다 — 이 검체의 전제가 사라졌다(`serde_json` 의 `float_roundtrip` 이 켜졌나? 0.14.43 K1 검체도 함께 점검하라)"
            );
            assert_eq!(
                (q_lossy, sum_lossy, not_idem),
                (0, 0, 0),
                "{label}: 양자화한 시각이 JSON 왕복에서 바뀌었다(지금 {q_lossy}건 · 만료 {sum_lossy}건 · 멱등 위반 {not_idem}건 · 첫 불일치 원시값 {first_bad:?})"
            );
            assert!(max_digits <= 16, "{label}: 양자화한 값의 유효숫자가 {max_digits}자리다(기대 ≤ 16)");
            // 정확 산술이면 ≤ 0.5µs 지만 `f64` 곱(`t * 1e6` — 1.8e15 부근 칸 0.25)과 가장 가까운 `f64` 선택(칸 2.4e-7 초)이 각각 반올림을 더해 실제 상한은 약 0.75µs 다 — 1µs 이내면 충분하다(마이크로초 아래는 의미가 없다).
            assert!(max_shift <= 1e-6, "{label}: 양자화가 값을 {max_shift:e} 초 옮겼다(기대 ≤ 1µs)");
        }
        // 표: 마이크로초 아래는 반올림(작은 크기라 리터럴이 정확하다) · 영·무한·NaN 은 그대로 · 음수는 대칭.
        assert_eq!(quantize_epoch_us(1.000_000_4).to_bits(), 1.0f64.to_bits());
        assert_eq!(quantize_epoch_us(1.000_000_6).to_bits(), 1.000_001f64.to_bits());
        assert_eq!(quantize_epoch_us(-1.000_000_6).to_bits(), (-1.000_001f64).to_bits());
        assert_eq!(quantize_epoch_us(0.0).to_bits(), 0.0f64.to_bits());
        assert!(quantize_epoch_us(f64::INFINITY).is_infinite() && quantize_epoch_us(f64::NAN).is_nan());
    }

    /// ② 실제 `save_records → load_records`(`with_store_root` 이음매) — **원시** 17자리 시각으로 서명한 레코드는 저장→읽기 뒤 서명이 죽는다(수리 전 윈도우 사고의 재현 · 음성 대조),
    /// 같은 판독을 **양자화한 값**으로 서명하면 무TTL·TTL 모두 세 값 `to_bits()` 동일 + 서명 유효, 그리고 **사용 1회**(`approval.check` 가 하는 일 = `updated_at` 새 시각으로 재서명·저장) 뒤에도 같다.
    #[test]
    fn r2f_dm_approvals_signed_on_the_quantized_clock_survive_save_load_and_one_use() {
        for (v, unit, i) in [(R2F_BAD_100NS[0], 100u64, 3u64), (R2F_BAD_100NS[1], 100, 14), (R2F_BAD_100NS[2], 100, 46), (R2F_BAD_1NS[0], 1, 2), (R2F_BAD_1NS[1], 1, 3)] {
            assert_eq!(r2f_grid_epoch(unit, i).to_bits(), v.to_bits(), "고정한 값이 격자의 i={i} 값과 다르다({v:?})");
            assert!(r2f_lossy(v), "{v:?}: 이 빌드의 기본 파서가 이 값을 정확히 돌려준다 — 전제가 사라졌다(`float_roundtrip` 이 켜졌나? 0.14.43 K1 검체도 함께 점검하라)");
        }
        let root = std::env::temp_dir().join(format!(
            "cys-r2f-f64-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        std::fs::create_dir_all(&root).expect("임시 루트");
        let _guard = with_store_root(&root);
        let secret: &[u8] = b"r2f-f64-roundtrip-secret-32-bytes-xx";
        let rec = |id: &str, created: f64, updated: f64, expires: Option<f64>| {
            let mut r = ApprovalRecord {
                version: 1,
                id: id.into(),
                command_prefix: vec!["echo".into(), "hi".into()],
                cwd: Some("/tmp".into()),
                environment: vec![],
                created_at: created,
                updated_at: updated,
                expires_at: expires,
                signature: String::new(),
            };
            r.sign(secret);
            r
        };
        let same_bits = |a: &ApprovalRecord, b: &ApprovalRecord, what: &str| {
            assert_eq!(a.created_at.to_bits(), b.created_at.to_bits(), "{what}: created_at 이 왕복에서 바뀌었다({:?} → {:?})", b.created_at, a.created_at);
            assert_eq!(a.updated_at.to_bits(), b.updated_at.to_bits(), "{what}: updated_at 이 왕복에서 바뀌었다({:?} → {:?})", b.updated_at, a.updated_at);
            assert_eq!(a.expires_at.map(f64::to_bits), b.expires_at.map(f64::to_bits), "{what}: expires_at 이 왕복에서 바뀌었다({:?} → {:?})", b.expires_at, a.expires_at);
            assert!(a.has_valid_signature(secret), "{what}: 저장→읽기 뒤 서명이 무효다 — 시각 값이 달라졌다");
        };
        let combos = [(R2F_BAD_100NS[0], R2F_BAD_100NS[1], R2F_BAD_100NS[2]), (R2F_BAD_1NS[0], R2F_BAD_1NS[1], R2F_BAD_100NS[2]), (R2F_BAD_100NS[2], R2F_BAD_1NS[1], R2F_BAD_1NS[0])];
        // 음성 대조 — 원시 시각 그대로 서명하면 저장→읽기 뒤 서명이 죽는다(수리 전 윈도우).
        let mut raw_dead = 0u32;
        let mut raw_total = 0u32;
        for ttl in [false, true] {
            for (n, (c, u, e)) in combos.into_iter().enumerate() {
                let written = rec(&format!("r2f-raw-{ttl}-{n}"), c, u, ttl.then_some(e));
                save_records(std::slice::from_ref(&written)).expect("저장");
                let back = load_records();
                assert_eq!(back.len(), 1, "원시 ttl={ttl} 조합{n}: 읽은 레코드 수");
                raw_total += 1;
                if !back[0].has_valid_signature(secret) {
                    raw_dead += 1;
                }
            }
        }
        assert_eq!(raw_dead, raw_total, "원시 17자리 시각으로 서명한 레코드 {raw_total}건 가운데 {raw_dead}건만 죽었다 — 음성 대조의 전제가 사라졌다(기본 파서가 정확해졌나?)");
        // 수리 — 같은 판독을 양자화(`record_now` 가 하는 일)한 값으로 서명하면 모두 산다.
        let q = quantize_epoch_us;
        for ttl in [false, true] {
            for (n, (c, u, _)) in combos.into_iter().enumerate() {
                let what = format!("ttl={ttl} 조합{n}");
                let written = rec(&format!("r2f-{ttl}-{n}"), q(c), q(u), ttl.then(|| q(q(c) + 3600.0)));
                save_records(std::slice::from_ref(&written)).expect("저장");
                let back = load_records();
                assert_eq!(back.len(), 1, "{what}: 읽은 레코드 수");
                same_bits(&back[0], &written, &what);
                // 사용 1회 — `approval.check` 가 하는 일: 읽은 레코드의 `updated_at` 을 새 시각(양자화)으로 바꿔 재서명·저장.
                let mut used = back[0].clone();
                used.updated_at = q(if n % 2 == 0 { R2F_BAD_100NS[1] } else { R2F_BAD_1NS[0] });
                used.sign(secret);
                save_records(std::slice::from_ref(&used)).expect("재저장");
                let again = load_records();
                assert_eq!(again.len(), 1, "{what}: 사용 뒤 읽은 레코드 수");
                same_bits(&again[0], &used, &format!("{what}(사용 1회 뒤)"));
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ③ 원문 핀 — 승인 레코드의 시각을 만드는 **두 곳**(`approval.sign` 의 `created_at`·`updated_at`·`expires_at` · `approval.check` 의 재서명 `updated_at`)이 양자화한 시계를 쓴다.
    /// 둘 중 하나가 원시 `state::now_epoch()` 로 돌아가면 윈도우에서 승인이 다시 죽는다(맥은 1µs 시계라 어떤 검체도 못 잡는다 — 그래서 원문으로 지킨다).
    /// 그리고 `ApprovalRecord` 리터럴을 만드는 곳이 데몬 전체에서 그 한 곳뿐이다(새 생산자가 생기면 이 핀이 붉어 시계 규칙을 상기시킨다).
    #[test]
    fn r2f_dm_record_producers_use_the_quantized_clock() {
        let strip = |src: &str| -> String {
            src.lines()
                .map(|l| match l.find("//") {
                    Some(i) if !l[..i].contains('"') => &l[..i],
                    _ => l,
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let handlers_full = include_str!("handlers.rs");
        let prod = strip(handlers_full.split("\n#[cfg(test)]\nmod tests {").next().expect("handlers.rs 운영부"));
        assert!(prod.len() < handlers_full.len(), "운영부 경계(`#[cfg(test)] mod tests`)를 못 찾았다");
        // 있어야 하는 것
        let sign_now = concat!("let now = crate::approval::", "record_now();");
        let sign_exp = concat!("expires_at: ttl_secs.map(|t| crate::approval::", "quantize_epoch_us(now + t as f64)),");
        let check_upd = concat!("r.updated_at = crate::approval::", "record_now();");
        for (what, needle) in [("sign 의 now", sign_now), ("sign 의 expires_at", sign_exp), ("check 재서명의 updated_at", check_upd)] {
            assert_eq!(prod.matches(needle).count(), 1, "handlers.rs 운영부에 `{needle}`({what})가 정확히 한 번 있어야 한다");
        }
        // 있으면 안 되는 것 — 원시 시계로 되돌린 꼴
        let raw_sign_now = concat!("let now = crate::state::", "now_epoch();\n            let mut rec = crate::approval::ApprovalRecord");
        let raw_sign_exp = concat!("expires_at: ttl_secs.map(|t| ", "now + t as f64),");
        let raw_check_upd = concat!("r.updated_at = crate::state::", "now_epoch();");
        for (what, needle) in [("sign 의 now", raw_sign_now), ("sign 의 expires_at", raw_sign_exp), ("check 재서명의 updated_at", raw_check_upd)] {
            assert_eq!(prod.matches(needle).count(), 0, "handlers.rs 운영부에 원시 시계 꼴 `{needle}`({what})가 남았다 — 윈도우 승인 서명이 다시 죽는다(`approval::record_now`)");
        }
        // 생산자 센서스 — 데몬 전체(디렉터리 스캔)에서 `ApprovalRecord { … }` 리터럴을 만드는 운영 코드는 handlers.rs 의 그 한 곳뿐이다.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cysd");
        let (mut scanned, mut literals) = (0usize, 0usize);
        for entry in std::fs::read_dir(&dir).expect("cysd 소스 폴더") {
            let path = entry.expect("항목").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("소스 읽기");
            let body = strip(text.split("\n#[cfg(test)]").next().unwrap_or(""));
            scanned += 1;
            literals += body
                .lines()
                .filter(|l| l.contains("ApprovalRecord {") && !l.contains("struct ApprovalRecord") && !l.contains("impl ApprovalRecord") && !l.contains("-> ApprovalRecord"))
                .count();
        }
        assert!(scanned >= 20, "스캔이 공허하다({scanned}개)");
        assert_eq!(literals, 1, "`ApprovalRecord {{ … }}` 리터럴을 만드는 운영 코드가 {literals}곳이다(기대 1 = approval.sign) — 새 생산자는 `approval::record_now()` 로 시각을 만들어야 한다");
    }

    // ── ★(0.14.31 · WP-4 R2 · codex major) TTL 저장소 분리의 **호환성 검체** ──────────────
    // 위임 작성: codex(gpt-6-astra) · 전 줄 검토 후 채택(R4-WP4-r2-tests-prompt.md).
    // 이 두 검체가 재는 사실은 하나다: **구 데몬은 TTL 레코드를 파괴할 수 없다.** 공용 파일에
    // TTL 이 남아 있던 종전 배치에서는 구 데몬이 다른 승인 하나를 정상 사용하기만 해도
    // `expires_at` 이 사라지고, 서명에 그 값이 묶여 있으므로 그 승인은 **영구 거부**된다.
    // (음성 대조가 그 사고를 같은 파일에서 실제로 재현한다 — 수리를 지우면 검체가 빨강이 된다.)
    // ★HOME 을 프로세스 전역으로 바꾸므로 두 검체는 하나의 잠금으로 직렬화한다. 같은 파일의
    //   다른 검체는 HOME 을 만지지 않는다(handlers.rs 의 승인 RPC 검체는 자기 HOME 을 쓰는
    //   기존 관례를 따르며, 그 관례의 프로세스 전역 경합은 이 커밋의 범위 밖이다).
    // approval.rs의 mod tests 안에 그대로 삽입한다. 외부 crate나 기존 테스트 헬퍼는 필요 없다.
    // 함수별 static은 서로 다른 잠금이므로 두 테스트가 공유할 static 하나를 둔다.
    static TTL_STORE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn ttl_record_survives_an_old_daemon_rewriting_the_shared_store() {
        // HOME과 secret은 프로세스 전역이다. 두 테스트가 반드시 같은 잠금을 사용한다.
        // 다른 HOME/env 접근 테스트까지 보호하려면 그 테스트도 이 잠금을 공유하거나
        // 테스트 실행기를 --test-threads=1로 실행해야 한다.
        let _lock = TTL_STORE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // 잠금보다 나중에 선언하므로 환경 복원/정리가 끝난 뒤 잠금이 해제된다.
        // OsString으로 보관하여 비유니코드 값과 원래 미설정 상태까지 보존한다.
        struct RestoreHome {
            home: Option<std::ffi::OsString>,
            secret: Option<std::ffi::OsString>,
            temporary: std::path::PathBuf,
        }
        impl Drop for RestoreHome {
            fn drop(&mut self) {
                for (key, old) in [
                    ("HOME", &self.home),
                    ("CYS_APPROVAL_SECRET_B64", &self.secret),
                ] {
                    match old {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
                let result = std::fs::remove_dir_all(&self.temporary);
                // 이미 실패하여 unwind 중이면 이중 panic으로 프로세스를 종료하지 않는다.
                if !std::thread::panicking() {
                    result.expect("테스트 종료 후 임시 HOME을 삭제하지 못했습니다");
                }
            }
        }
        let temporary = std::env::temp_dir().join(format!(
            "cys-ttl-old-writer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("고유 임시 HOME 이름을 만들 시스템 시각이 잘못되었습니다")
                .as_nanos()
        ));
        // create_dir은 기존 경로를 재사용하지 않으므로 타인의 파일을 정리하지 않는다.
        std::fs::create_dir(&temporary).expect("격리된 임시 HOME을 만들지 못했습니다");
        let restore = RestoreHome {
            home: std::env::var_os("HOME"),
            secret: std::env::var_os("CYS_APPROVAL_SECRET_B64"),
            temporary,
        };
        let secret: &[u8] = b"ttl-store-compatibility-secret-32-bytes";
        // ★(R2F-DM · ⓒ) 저장소 루트는 `HOME` 이 아니라 **이음매**로 옮긴다 — 윈도우의 `dirs::home_dir()` 은 `HOME` 을 보지 않아(`store_root` 의 doc) `HOME` 만 바꾼 이 검체는
        //   임시 폴더가 아니라 러너의 **실제 프로필** `~\.cys\approvals*.json` 을 읽고 썼다(실측: 윈도우 진단 잡 `37188821194` — 두 검체가 임시 폴더의 파일을 찾지 못해 실패). 가드는 `restore` 보다
        //   뒤에 선언되어 임시 폴더를 지우기 전에 풀린다. `HOME` 은 더 이상 바꾸지 않는다(복원 목록의 항목은 그대로 무해하다).
        let _root = with_store_root(&restore.temporary);
        std::env::set_var("CYS_APPROVAL_SECRET_B64", b64_encode(secret));
        let shared_path = restore.temporary.join(".cys/approvals.json");
        let ttl_path = restore.temporary.join(".cys/approvals-ttl.json");

        let mut legacy = ApprovalRecord {
            version: 1,
            id: "L".into(),
            command_prefix: vec!["git".into(), "status".into()],
            cwd: None,
            environment: vec![("CI".into(), "1".into())],
            created_at: 1000.0,
            updated_at: 1000.0,
            expires_at: None,
            signature: String::new(),
        };
        legacy.sign(secret);
        let mut ttl = ApprovalRecord {
            id: "T".into(),
            expires_at: Some(4_000_000_000.0),
            ..legacy.clone()
        };
        ttl.sign(secret);

        // 구 데몬의 모르는 필드 유실을 JSON 값 편집으로 재현한다.
        // 두 경로 중 공용 파일만 접근하며, 정상/음성 대조 모두 같은 writer를 쓴다.
        let old_daemon_rewrite = || {
            let mut value: serde_json::Value = serde_json::from_slice(
                &std::fs::read(&shared_path).expect("구 데몬이 읽을 공용 승인 파일이 없습니다"),
            )
            .expect("공용 승인 파일이 올바른 JSON이 아닙니다");
            for record in value["records"]
                .as_array_mut()
                .expect("공용 파일에 records 배열이 없습니다")
            {
                record
                    .as_object_mut()
                    .expect("승인 레코드가 JSON 객체가 아닙니다")
                    .remove("expires_at");
            }
            std::fs::write(
                &shared_path,
                serde_json::to_vec(&value).expect("구 데몬 재저장 JSON 생성 실패"),
            )
            .expect("구 데몬이 공용 승인 파일을 덮어쓰지 못했습니다");
        };
        // 반환형이 ()인 배경 API와 Result인 현재 구현 모두에서 파일 내용으로 저장을 검증한다.
        let _ = save_records(&[legacy.clone(), ttl.clone()]);
        let ttl_before = std::fs::read(&ttl_path).expect("TTL 승인은 전용 파일에 저장되어야 합니다");
        old_daemon_rewrite();
        assert_eq!(
            std::fs::read(&ttl_path).expect("TTL 전용 파일이 사라졌습니다"),
            ttl_before,
            "구 데몬의 공용 파일 재저장이 TTL 전용 파일을 변경했습니다"
        );
        let loaded = load_records();
        assert_eq!(
            loaded.len(),
            2,
            "구 데몬 재저장 후 L과 T가 모두 남아야 합니다"
        );
        for expected in [&legacy, &ttl] {
            let actual = loaded
                .iter()
                .find(|r| r.id == expected.id)
                .expect("구 데몬 재저장 후 기존 승인 id가 사라졌습니다");
            assert_eq!(
                actual.expires_at, expected.expires_at,
                "승인 {}의 만료 시각이 구 데몬 재저장으로 변경되었습니다",
                expected.id
            );
            assert!(
                actual.has_valid_signature(secret),
                "승인 {}의 서명이 구 데몬 재저장으로 손상되었습니다",
                expected.id
            );
            assert_eq!(
                actual.signature, expected.signature,
                "승인 {}의 기존 서명이 보존되지 않았습니다",
                expected.id
            );
        }

        // 음성 대조: 분리 전처럼 동일한 서명 검체 T를 공용 파일에 강제로 넣는다.
        // load_records()는 정상 TTL 판을 우선하므로 손상본을 공용 파일에서 직접 읽는다.
        assert!(
            ttl.has_valid_signature(secret),
            "음성 대조의 원본 T부터 서명이 무효라면 유실 사고를 입증할 수 없습니다"
        );
        std::fs::write(
            &shared_path,
            serde_json::to_vec(&serde_json::json!({
                "records": [&legacy, &ttl]
            }))
            .expect("음성 대조 JSON 생성 실패"),
        )
        .expect("음성 대조를 공용 파일에 쓰지 못했습니다");
        old_daemon_rewrite();
        let damaged: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&shared_path).expect("음성 대조 공용 파일 읽기 실패"),
        )
        .expect("음성 대조 공용 파일 JSON 해석 실패");
        let damaged: Vec<ApprovalRecord> = serde_json::from_value(damaged["records"].clone())
            .expect("구 데몬 재저장 결과를 승인 레코드로 읽지 못했습니다");
        assert_eq!(
            damaged.len(),
            2,
            "음성 대조에서 필드 제거가 레코드 삭제로 바뀌었습니다"
        );
        let damaged_ttl = damaged
            .iter()
            .find(|r| r.id == "T")
            .expect("음성 대조에서 T 자체가 사라졌습니다");
        assert_eq!(
            damaged_ttl.expires_at, None,
            "구 데몬 재저장 흉내가 T의 expires_at을 제거하지 않았습니다"
        );
        assert_eq!(
            damaged_ttl.signature, ttl.signature,
            "음성 대조는 서명 자체를 바꾸지 않고 만료 필드만 제거해야 합니다"
        );
        assert!(!damaged_ttl.has_valid_signature(secret),
            "공용 파일의 T가 만료 필드를 잃고도 서명 검증을 통과했습니다: 분리 전 사고가 재현되지 않았습니다");
        let intact_legacy = damaged
            .iter()
            .find(|r| r.id == "L")
            .expect("음성 대조에서 L이 사라졌습니다");
        assert!(
            intact_legacy.has_valid_signature(secret),
            "무기한 L까지 손상되었다면 TTL 필드 유실만을 재현한 검체가 아닙니다"
        );
    }

    #[test]
    fn saving_migrates_ttl_records_out_of_the_shared_store() {
        // HOME과 secret은 프로세스 전역이다. 두 테스트가 반드시 같은 잠금을 사용한다.
        // 다른 HOME/env 접근 테스트까지 보호하려면 그 테스트도 이 잠금을 공유하거나
        // 테스트 실행기를 --test-threads=1로 실행해야 한다.
        let _lock = TTL_STORE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // 잠금보다 나중에 선언하므로 환경 복원/정리가 끝난 뒤 잠금이 해제된다.
        // OsString으로 보관하여 비유니코드 값과 원래 미설정 상태까지 보존한다.
        struct RestoreHome {
            home: Option<std::ffi::OsString>,
            secret: Option<std::ffi::OsString>,
            temporary: std::path::PathBuf,
        }
        impl Drop for RestoreHome {
            fn drop(&mut self) {
                for (key, old) in [
                    ("HOME", &self.home),
                    ("CYS_APPROVAL_SECRET_B64", &self.secret),
                ] {
                    match old {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
                let result = std::fs::remove_dir_all(&self.temporary);
                // 이미 실패하여 unwind 중이면 이중 panic으로 프로세스를 종료하지 않는다.
                if !std::thread::panicking() {
                    result.expect("테스트 종료 후 임시 HOME을 삭제하지 못했습니다");
                }
            }
        }
        let temporary = std::env::temp_dir().join(format!(
            "cys-ttl-migration-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("고유 임시 HOME 이름을 만들 시스템 시각이 잘못되었습니다")
                .as_nanos()
        ));
        // create_dir은 기존 경로를 재사용하지 않으므로 타인의 파일을 정리하지 않는다.
        std::fs::create_dir(&temporary).expect("격리된 임시 HOME을 만들지 못했습니다");
        let restore = RestoreHome {
            home: std::env::var_os("HOME"),
            secret: std::env::var_os("CYS_APPROVAL_SECRET_B64"),
            temporary,
        };
        let secret: &[u8] = b"ttl-store-compatibility-secret-32-bytes";
        // ★(R2F-DM · ⓒ) 저장소 루트는 `HOME` 이 아니라 **이음매**로 옮긴다 — 윈도우의 `dirs::home_dir()` 은 `HOME` 을 보지 않아(`store_root` 의 doc) `HOME` 만 바꾼 이 검체는
        //   임시 폴더가 아니라 러너의 **실제 프로필** `~\.cys\approvals*.json` 을 읽고 썼다(실측: 윈도우 진단 잡 `37188821194` — 두 검체가 임시 폴더의 파일을 찾지 못해 실패). 가드는 `restore` 보다
        //   뒤에 선언되어 임시 폴더를 지우기 전에 풀린다. `HOME` 은 더 이상 바꾸지 않는다(복원 목록의 항목은 그대로 무해하다).
        let _root = with_store_root(&restore.temporary);
        std::env::set_var("CYS_APPROVAL_SECRET_B64", b64_encode(secret));
        let shared_path = restore.temporary.join(".cys/approvals.json");
        let ttl_path = restore.temporary.join(".cys/approvals-ttl.json");

        let mut legacy = ApprovalRecord {
            version: 1,
            id: "L".into(),
            command_prefix: vec!["git".into(), "status".into()],
            cwd: None,
            environment: vec![("CI".into(), "1".into())],
            created_at: 1000.0,
            updated_at: 1000.0,
            expires_at: None,
            signature: String::new(),
        };
        legacy.sign(secret);
        let mut ttl = ApprovalRecord {
            id: "T".into(),
            expires_at: Some(4_000_000_000.0),
            ..legacy.clone()
        };
        ttl.sign(secret);

        // 분리 저장 이전 배치: 공용 파일 하나에 L, T 순서로 유효한 승인을 둔다.
        std::fs::create_dir_all(
            shared_path
                .parent()
                .expect("공용 파일의 상위 경로가 없습니다"),
        )
        .expect("임시 HOME 안에 승인 디렉터리를 만들지 못했습니다");
        std::fs::write(
            &shared_path,
            serde_json::to_vec(&serde_json::json!({
                "records": [&legacy, &ttl]
            }))
            .expect("이전 배치 JSON 생성 실패"),
        )
        .expect("이전 배치 공용 파일 생성 실패");
        assert!(
            !ttl_path.exists(),
            "이전 배치 검체에는 TTL 전용 파일이 없어야 합니다"
        );
        let loaded = load_records();
        assert_eq!(
            loaded.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["L", "T"],
            "분리 전 공용 파일을 읽을 때 승인 id 또는 L, T 순서가 바뀌었습니다"
        );
        for record in &loaded {
            assert!(
                record.has_valid_signature(secret),
                "마이그레이션 전 승인 {}의 서명이 무효입니다",
                record.id
            );
        }
        let before = serde_json::to_value(&loaded).expect("마이그레이션 전 검체 스냅샷 생성 실패");
        let _ = save_records(&loaded);

        let shared: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&shared_path).expect("마이그레이션 후 공용 파일이 없습니다"),
        )
        .expect("마이그레이션 후 공용 파일 JSON 해석 실패");
        let shared_records = shared["records"]
            .as_array()
            .expect("공용 파일에 records 배열이 없습니다");
        assert_eq!(
            shared_records
                .iter()
                .filter(|r| r.get("expires_at").is_some())
                .count(),
            0,
            "공용 파일에 expires_at 키가 남아 있어 구 데몬 재저장으로 TTL 승인이 손상될 수 있습니다"
        );
        assert_eq!(
            shared_records.len(),
            1,
            "마이그레이션 후 공용 파일에는 무기한 L 하나만 있어야 합니다"
        );
        assert_eq!(
            shared_records[0]["id"], "L",
            "공용 파일에서 무기한 L이 사라지거나 T가 남았습니다"
        );
        let dedicated: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&ttl_path).expect("마이그레이션이 TTL 전용 파일을 만들지 않았습니다"),
        )
        .expect("TTL 전용 파일 JSON 해석 실패");
        let dedicated: Vec<ApprovalRecord> = serde_json::from_value(dedicated["records"].clone())
            .expect("TTL 전용 파일의 records를 승인 목록으로 읽지 못했습니다");
        assert_eq!(
            dedicated.len(),
            1,
            "TTL 전용 파일에는 T 하나만 있어야 합니다"
        );
        assert_eq!(
            dedicated[0].id, "T",
            "마이그레이션 후 TTL 전용 파일에 T가 없습니다"
        );
        assert_eq!(
            dedicated[0].expires_at, ttl.expires_at,
            "TTL 전용 파일에서 T의 만료 시각이 변경되었습니다"
        );
        assert!(
            dedicated[0].has_valid_signature(secret),
            "TTL 전용 파일의 T 서명이 마이그레이션 중 손상되었습니다"
        );
        let reloaded = load_records();
        assert_eq!(
            reloaded.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["L", "T"],
            "분리 저장 후 다시 읽을 때 승인 id 또는 L, T 순서가 보존되지 않았습니다"
        );
        for record in &reloaded {
            assert!(
                record.has_valid_signature(secret),
                "분리 저장 후 승인 {}의 서명이 무효입니다",
                record.id
            );
        }
        // 재서명 등 우연한 복구로 검사를 통과하지 못하게 모든 필드와 기존 서명도 비교한다.
        assert_eq!(
            serde_json::to_value(&reloaded).expect("마이그레이션 후 스냅샷 생성 실패"),
            before,
            "마이그레이션은 저장 위치만 바꿔야 하는데 승인 내용, 서명 또는 순서가 변경되었습니다"
        );
    }

}
