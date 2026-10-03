//! cys↔cysd 와이어 무결성 가드 (leaf — socket/pty/governance/pack 무의존).
//!
//! producer 자기검증: cysd가 응답 `Value`를 NDJSON 줄로 직렬화할 때,
//! 같은 바이트를 즉시 재파싱한 `Value`가 선언과 `==`가 아니면 fail-loud(`Drift`).
//! (0.14.43) 단 어긋난 곳이 **부동소수 표기 정밀도뿐**이면 `FloatInexact`(Recoverable) — 와이어는 같고 판정만 가른다.
//! 디코더 대칭검증: 응답에 additive하게 부착된 `_flen`(payload 바이트 길이) 선언과
//! 실제 재직렬화 길이가 동일버전에서 어긋나면 트렁케이션(`LenMismatch`)으로 거부.
//! `_pv` 마이너 스큐는 무차별 kill이 아니라 graceful downgrade(`VersionSkew`).
//!
//! ★additive sibling-injection(§4.1): `_flen`·`_pv`를 **기존 top-level 응답 객체에
//! 형제 키로 추가**한다. `ok`/`result`/`id`/`error`는 제자리 그대로 → 구 디코더
//! (`serde_json::from_str` + `resp["ok"]`, deny_unknown_fields 없음)는 추가 키를
//! 무시 → 호환 깨짐 0. `{"frame":…}` 래핑은 top-level을 가려 호환을 깨므로 금지.
//!
//! penpot 클린룸 근거(개념만·코드복사 0): mem.cljs `assert-written`의
//! `(= expected actual)` 불변식 + mem.rs write_vec의 "first 4 bytes = size" 길이선언
//! + changes.cljc `verify?=true` 기본-ON 정신. penpot 식별자·코드는 옮기지 않으며
//! NDJSON·serde_json 환경의 독립 구현이다.

use serde_json::{Number, Value};

/// 와이어 마이너 버전 단일진실.
pub const PROTO_PV: u16 = 1;

/// 응답 객체에 additive하게 부착되는 메타 키.
const KEY_FLEN: &str = "_flen";
const KEY_PV: &str = "_pv";

/// cys↔cysd 와이어 무결성 위반 분류. T1-3 `Severity`로 사상된다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiError {
    /// 인코드 round-trip 불일치: 직렬화→재파싱이 선언 `Value`와 다름 → Critical.
    Drift,
    /// ★(0.14.43 · K1) 인코드 round-trip 이 **부동소수 표기 정밀도만큼만** 다름 — 구조·키·문자열·정수·불리언은 동일
    /// → 무결성 위반 아님(Recoverable). serde_json 기본 기능(`float_roundtrip` 꺼짐)의 부동소수 파싱은 best-effort 라
    /// 17자리 f64 가 1ulp 어긋나 재파싱될 수 있다(실측 값당 약 9~12%) — 이것을 `Drift` 로 보면 부동소수가 실린 모든
    /// 프레임이 값에 따라 Critical 로그를 쏟는다. 와이어는 `Drift` 와 같다(메타 없는 legacy 직렬화 폴백 — 구 클라이언트 호환).
    FloatInexact,
    /// 디코드 길이 불일치(동일버전): declared `_flen` != 실제 길이 → Critical(트렁케이션).
    LenMismatch,
    /// 디코드 불일치(버전 마이너 스큐): negotiated graceful downgrade(kill 아님).
    VersionSkew { peer_pv: u16, local_pv: u16 },
}

/// 인코드 자기검증 기본 ON. `CYS_ABI_VERIFY=0`로만 좁게 opt-out(debug-only 아님).
fn verify_on() -> bool {
    std::env::var("CYS_ABI_VERIFY").as_deref() != Ok("0")
}

/// 응답 payload(result 또는 error)의 canonical 직렬화 바이트 길이.
/// declared-len(`_flen`)의 선언값 — 둘 중 존재하는 쪽을 잰다(둘 다 없으면 0).
fn payload_len(resp: &Value) -> usize {
    let payload = resp.get("result").or_else(|| resp.get("error"));
    match payload {
        Some(v) => serde_json::to_string(v).map(|s| s.len()).unwrap_or(0),
        None => 0,
    }
}

/// 부동소수 허용 폭 — 상대 `FLOAT_TOL * f64::EPSILON * max(|x|,|y|)`(최대 크기 값 기준 약 4~8ulp) · 0 근방은 절대차
/// `FLOAT_TOL * f64::MIN_POSITIVE`. best-effort 파서의 실측 최대 오차는 2ulp(전 지수 범위 무작위 40만 표본)라 여유가 있고,
/// 지수 한 칸(10배)이나 앞쪽 유효숫자 변질 같은 구조적 변질은 어림도 못 한다.
const FLOAT_TOL: f64 = 4.0;

/// 두 f64 가 "부동소수 표기 정밀도 차" 안인가 — 같거나, 상대 허용 폭 이내이거나, 0 근방 절대 허용 폭 이내.
/// 비유한(NaN·무한)은 `Value` 에 못 들어가지만, 들어와도 어느 비교도 참이 아니라 `false`(패닉 없음).
fn floats_close(x: f64, y: f64) -> bool {
    if x == y {
        return true;
    }
    let diff = (x - y).abs();
    diff <= FLOAT_TOL * f64::EPSILON * x.abs().max(y.abs()) || diff <= FLOAT_TOL * f64::MIN_POSITIVE
}

/// 숫자 비교 — 둘 다 정수 표현이면 `==`, 둘 다 실수 표현이면 [`floats_close`], 한쪽만 정수(표현이 바뀜)면 **다름**.
fn numbers_close(x: &Number, y: &Number) -> bool {
    let x_int = x.is_i64() || x.is_u64();
    let y_int = y.is_i64() || y.is_u64();
    match (x_int, y_int) {
        (true, true) => x == y,
        (false, false) => matches!((x.as_f64(), y.as_f64()), (Some(p), Some(q)) if floats_close(p, q)),
        _ => false,
    }
}

/// 두 `Value` 가 **부동소수 표기 정밀도 차를 빼면** 같은가(순수 · 재귀 깊이 = 입력 JSON 깊이 · 패닉·인덱싱 없음).
/// Null/Bool/String 은 `==` · 배열은 길이·원소별 · 객체는 키 집합·값별 · 숫자는 [`numbers_close`] · 종류가 다르면 다름.
fn equal_modulo_float(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => numbers_close(x, y),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| equal_modulo_float(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, p)| y.get(k).is_some_and(|q| equal_modulo_float(p, q)))
        }
        _ => false,
    }
}

/// round-trip 판정 — 재파싱 `reparsed` 가 선언 `original` 과 같으면 `Ok` · **부동소수 표기 정밀도만** 다르면
/// `FloatInexact` · 그 밖(구조·키·문자열·정수 …)은 `Drift`. 비-객체 응답은 부동소수 허용에서 뺀다 — 와이어 계약 위반
/// (메타를 달 자리가 없음)이라 `Drift` 가 불변이어야 하고, 안에 어긋난 부동소수가 있다고 조용히 삼키면 안 된다.
fn round_trip_verdict(reparsed: &Value, original: &Value) -> Result<(), AbiError> {
    if reparsed == original {
        Ok(())
    } else if original.is_object() && equal_modulo_float(reparsed, original) {
        Err(AbiError::FloatInexact)
    } else {
        Err(AbiError::Drift)
    }
}

/// producer 자기검증 프레이밍: 응답 `Value`를 declared-len 메타가 형제로 붙은 NDJSON 줄로.
///
/// (a) round-trip 동일성: 직렬화한 바이트를 즉시 재파싱해 선언 `Value`와 `==`가 아니면 `Err`.
///     부동소수 표기 정밀도만 다르면 `Err(FloatInexact)`(Recoverable), 그 밖은 `Err(Drift)`(Critical) —
///     [`round_trip_verdict`]. 어느 쪽이든 호출자는 메타 없는 legacy 직렬화로 폴백한다(와이어 바이트 동일).
///     `assert_eq!`/`debug_assert!`가 아니라 **명시 분기** — release에서도 발화.
/// (b) additive: 같은 top-level 객체에 `_flen`·`_pv`만 형제로 추가(중첩·래핑 없음).
pub fn frame_response(resp: &Value) -> Result<String, AbiError> {
    // (a) round-trip 자기검증 — 선언 == 실제 직렬화 결과.
    if verify_on() {
        let body = serde_json::to_string(resp).map_err(|_| AbiError::Drift)?;
        let reparsed: Value = serde_json::from_str(&body).map_err(|_| AbiError::Drift)?;
        round_trip_verdict(&reparsed, resp)?; // 같음 → 통과 · 부동소수 정밀도 차 → FloatInexact · 그 밖 → Drift(Critical)
    }
    // (b) additive sibling-injection — top-level은 보존, 메타만 형제로.
    let flen = payload_len(resp);
    let mut out = resp.clone();
    match out.as_object_mut() {
        Some(map) => {
            map.insert(KEY_FLEN.to_string(), Value::from(flen as u64));
            map.insert(KEY_PV.to_string(), Value::from(PROTO_PV as u64));
        }
        // 비-객체 응답은 메타를 달 자리가 없다 — 와이어 계약상 발생하지 않지만 fail-loud.
        None => return Err(AbiError::Drift),
    }
    let line = serde_json::to_string(&out).map_err(|_| AbiError::Drift)?;
    Ok(format!("{line}\n"))
}

/// T4-5A(==T5-6 strand-3 == ONE guard): 단일 RPC 응답 페이로드 바이트 상한.
/// **프로세스 수명·load = 기존 watchdog(governance.rs)** / **단일 RPC 응답 바이트 = 이 신규
/// 직교 가드** — ADR 경계: 두 책임은 별개이며 이 가드는 watchdog와 중복이 아니다(한 곳에만 둔다).
/// cap 수치는 로컬 실측 기본값이며 `CYS_MAX_RESPONSE_BYTES`로 조정(penpot 호스티드 MCP 15MB는
/// 검증 상수 아님 — 상속 금지). screen-buffer FIFO `truncated`(handlers.rs:860)와 무관한 별 표면.
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// 실측 가능한 cap 노브 — 기본 `MAX_RESPONSE_BYTES`, env로만 좁게 조정.
fn max_response_bytes() -> usize {
    std::env::var("CYS_MAX_RESPONSE_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MAX_RESPONSE_BYTES)
}

/// cap 초과 응답을 **fail-loud + 트렁케이트**한 sentinel 응답으로 치환한다.
/// 거대한 단일 응답이 클라이언트 컨텍스트·메모리를 폭주시키는 경로를 차단한다(스트림 폴백은
/// 호출자 결정 — 여기선 결정론 트렁케이트 sentinel을 돌려 항상 한 줄은 내보낸다).
///
/// 반환: cap 이내면 `None`(원본 그대로 진행), 초과면 `Some(sentinel)` — 원본 `id`는 보존하고
/// `result`를 `{response_truncated, original_bytes, cap_bytes}` fail-loud 페이로드로 교체.
pub fn cap_response(resp: &Value) -> Option<Value> {
    let cap = max_response_bytes();
    let bytes = serde_json::to_string(resp).map(|s| s.len()).unwrap_or(0);
    if bytes <= cap {
        return None;
    }
    let id = resp.get("id").cloned().unwrap_or(Value::Null);
    Some(serde_json::json!({
        "id": id,
        "ok": false,
        "error": {
            "code": "response_truncated",
            "message": format!(
                "RPC response {bytes} bytes exceeds cap {cap} — truncated (set CYS_MAX_RESPONSE_BYTES or use streaming)"
            ),
            "original_bytes": bytes,
            "cap_bytes": cap,
        }
    }))
}

/// 디코더 대칭검증: declared `_flen` == 실제 payload 직렬화 길이.
///
/// 반환은 **top-level 응답 객체 그대로**(additive 계약 — 구 디코더처럼 `resp["ok"]`/
/// `resp["result"]`를 그대로 읽을 수 있다). 메타가 없으면 legacy peer로 보고 무검증 수용.
/// 동일 `_pv` + 길이 불일치 = `LenMismatch`(Critical); 마이너 스큐 = `VersionSkew`(graceful).
pub fn parse_frame(raw: &str) -> Result<Value, AbiError> {
    let resp: Value = serde_json::from_str(raw.trim_end()).map_err(|_| AbiError::LenMismatch)?;

    // 메타 부재 = legacy peer(또는 검증 off로 보낸 프레임) → graceful 수용.
    let declared = resp.get(KEY_FLEN).and_then(|v| v.as_u64());
    let Some(declared) = declared else {
        return Ok(resp);
    };
    let peer_pv = resp.get(KEY_PV).and_then(|v| v.as_u64()).unwrap_or(0) as u16;

    let actual = payload_len(&resp) as u64;
    if declared != actual {
        // 동일버전이면 트렁케이션(Critical); 마이너 스큐면 graceful downgrade.
        return Err(if peer_pv == PROTO_PV {
            AbiError::LenMismatch
        } else {
            AbiError::VersionSkew {
                peer_pv,
                local_pv: PROTO_PV,
            }
        });
    }
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_reply() -> Value {
        json!({"id": 1, "ok": true, "result": {"surface": "surface:7", "rows": [1, 2, 3]}})
    }

    /// frame_response 출력이 여전히 top-level ok/result/id를 같은 자리에 둔다 —
    /// 구 형태 디코드가 성공하고, _flen/_pv는 형제(래퍼 아님)임을 박제.
    #[test]
    fn wire_roundtrip_is_additive() {
        let reply = sample_reply();
        let line = frame_response(&reply).expect("frame_response ok");
        assert!(line.ends_with('\n'));

        // 구 디코더 시점: 그냥 from_str 후 top-level 키를 읽는다.
        let decoded: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(decoded["ok"].as_bool(), Some(true));
        assert_eq!(decoded["id"], json!(1));
        assert_eq!(decoded["result"]["surface"], json!("surface:7"));

        // _flen/_pv는 top-level 형제 키 — "frame" 래퍼는 존재하지 않는다.
        assert!(decoded.get("_flen").is_some(), "_flen must be a sibling");
        assert!(decoded.get("_pv").is_some(), "_pv must be a sibling");
        assert!(decoded.get("frame").is_none(), "must NOT wrap under 'frame'");
        assert_eq!(decoded["_pv"].as_u64(), Some(PROTO_PV as u64));

        // parse_frame 왕복: top-level 객체 그대로 + 검증 통과(추가 키 제외 원본 필드 보존).
        let reparsed = parse_frame(&line).expect("parse_frame ok");
        assert_eq!(reparsed["ok"].as_bool(), Some(true));
        assert_eq!(reparsed["result"], reply["result"]);
    }

    /// round-trip 불가능(비-객체) 응답 주입 시 Drift.
    #[test]
    fn wire_drift_detected() {
        // 와이어 계약 위반: 응답이 객체가 아니면 메타를 달 자리가 없어 fail-loud.
        let not_an_object = json!([1, 2, 3]);
        assert_eq!(frame_response(&not_an_object), Err(AbiError::Drift));

        // round-trip 자기검증 분기 자체의 박제: 정상 객체는 통과(대조군).
        assert!(frame_response(&sample_reply()).is_ok());
    }

    /// _flen을 조작(트렁케이션 모사)하면 동일버전에서 LenMismatch.
    #[test]
    fn wire_len_mismatch() {
        let line = frame_response(&sample_reply()).unwrap();
        let mut tampered: Value = serde_json::from_str(line.trim_end()).unwrap();
        // declared len을 거짓으로 늘림 = 실제 payload보다 길게 선언(트렁케이션 신호).
        tampered["_flen"] = json!(99999);
        let raw = serde_json::to_string(&tampered).unwrap();
        assert_eq!(parse_frame(&raw), Err(AbiError::LenMismatch));
    }

    /// peer _pv가 local과 다르고 len도 불일치면 Critical이 아니라 VersionSkew(graceful).
    #[test]
    fn wire_version_skew_graceful() {
        let line = frame_response(&sample_reply()).unwrap();
        let mut skewed: Value = serde_json::from_str(line.trim_end()).unwrap();
        skewed["_pv"] = json!(PROTO_PV as u64 + 1); // 마이너 스큐
        skewed["_flen"] = json!(99999); // len도 불일치
        let raw = serde_json::to_string(&skewed).unwrap();
        match parse_frame(&raw) {
            Err(AbiError::VersionSkew { peer_pv, local_pv }) => {
                assert_eq!(peer_pv, PROTO_PV + 1);
                assert_eq!(local_pv, PROTO_PV);
            }
            other => panic!("expected VersionSkew (graceful), got {other:?}"),
        }
    }

    /// 메타가 전혀 없는 legacy peer 프레임은 무검증 graceful 수용.
    #[test]
    fn wire_legacy_peer_accepted() {
        let legacy = r#"{"id":1,"ok":true,"result":{"x":1}}"#;
        let v = parse_frame(legacy).expect("legacy frame accepted");
        assert_eq!(v["ok"].as_bool(), Some(true));
    }

    /// T4-5A: cap 이내 응답은 통과(None), cap 초과 응답은 fail-loud sentinel로 트렁케이트.
    /// env 노브로 cap을 작게 핀해 실측 sleep 없이 결정론 검증.
    #[test]
    fn wire_byte_cap_truncates_oversize() {
        // cap을 512바이트로 좁혀 핀(테스트 전용 — sentinel(고정 메시지)은 이 안에 들어가되
        // 거대 배열은 초과하도록). 직렬화는 lazy라 set 후 즉시 호출.
        std::env::set_var("CYS_MAX_RESPONSE_BYTES", "512");

        // cap 이내: 통과(None — 원본 그대로 진행).
        let small = json!({"id": 1, "ok": true, "result": {"x": 1}});
        assert!(
            cap_response(&small).is_none(),
            "cap 이내 응답은 트렁케이트하지 않는다"
        );

        // cap 초과: 거대 배열 → fail-loud sentinel(id 보존, error.code=response_truncated).
        let big_rows: Vec<u64> = (0..1000).collect();
        let big = json!({"id": 7, "ok": true, "result": {"rows": big_rows}});
        let sentinel = cap_response(&big).expect("oversize → sentinel");
        assert_eq!(sentinel["id"], json!(7), "원본 id 보존");
        assert_eq!(sentinel["ok"].as_bool(), Some(false), "fail-loud");
        assert_eq!(
            sentinel["error"]["code"].as_str(),
            Some("response_truncated")
        );
        // sentinel 자체는 cap 이내라 무한 재트렁케이트 루프가 없다(고정 크기 sentinel).
        assert!(
            cap_response(&sentinel).is_none(),
            "sentinel은 cap 이내 — 재트렁케이트 안 함"
        );

        std::env::remove_var("CYS_MAX_RESPONSE_BYTES");
    }

    // ═══ ★(0.14.43 · K1) 부동소수 표기 정밀도 차는 Drift 가 아니다 ═══
    //
    // serde_json 기본 기능(`float_roundtrip` 꺼짐)의 부동소수 파싱은 best-effort 라 17자리 f64 가 1ulp 어긋나게 재파싱될 수
    // 있다(값당 약 9~12%). 종전에는 그것이 `Drift`(Critical)로 잡혀 부동소수가 실린 모든 프레임이 값에 따라 로그를 쏟았다.
    // 와이어는 그대로(메타 없는 폴백)이고 **판정만** `FloatInexact` 로 가른다 — 구조 불일치는 여전히 `Drift` 다.

    /// 이 빌드에서 round-trip 이 어긋나는 f64 하나 — 결정론 벡터 `92.2547419781119086` 를 먼저 쓰고(어긋남을 단언으로 확인),
    /// 어긋나지 않으면 `1.0 + k*EPSILON`(k < 2000) 후보에서 첫 값을 쓴다. 둘 다 없으면 계측 무효(패닉).
    fn inexact_f64() -> f64 {
        fn reparse_differs(x: f64) -> bool {
            let v = json!({"id": 1, "ok": true, "result": {"used_pct": x}});
            let back: Value = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
            back != v
        }
        let brief = 92.2547419781119086_f64;
        if reparse_differs(brief) {
            return brief;
        }
        (0..2000u32)
            .map(|k| 1.0 + f64::from(k) * f64::EPSILON)
            .find(|&x| reparse_differs(x))
            .expect("계측 무효 — 이 빌드의 serde_json 은 재파싱이 어긋나는 f64 를 만들지 못한다(float_roundtrip 이 켜졌나?)")
    }

    /// 양수 유한값의 바로 위 이웃(1ulp 위). 음수는 `-up_ulps(-x, n)` 로 크기를 키운다.
    fn up_ulps(x: f64, n: u64) -> f64 {
        f64::from_bits(x.to_bits() + n)
    }

    /// 결정론 의사난수(고정 시드 LCG) — 상위 32비트만 쓴다(하위 비트는 주기가 짧아 가수 하위 비트 분포를 망친다).
    struct Lcg(u64);
    impl Lcg {
        fn next32(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 32) as u32
        }
        fn next64(&mut self) -> u64 {
            (u64::from(self.next32()) << 32) | u64::from(self.next32())
        }
    }

    /// `equal_modulo_float` 진리표 — 모든 행은 **양방향(대칭)**으로 같은 답이어야 한다.
    /// 적색(돌연변이 M1 = 항상 true): "2배 차"·"문자열 1글자 차"·"키 누락"·"정수 1 vs 실수 1.0" 등 false 행이 전부 깨진다.
    #[test]
    fn k1_equal_modulo_float_table() {
        let x = 92.25474197811191_f64;
        let nested = |f: f64, s: &str, i: Value| {
            json!({"a": [1, {"b": [2.5, f, s]}], "c": null, "d": {"e": i, "f": true}})
        };
        let rows: Vec<(&str, Value, Value, bool)> = vec![
            ("같은 값(중첩 전 종류)", nested(x, "s", json!(7)), nested(x, "s", json!(7)), true),
            ("1ulp 차 f64", json!(x), json!(up_ulps(x, 1)), true),
            ("1ulp 차 음수 f64", json!(-x), json!(-up_ulps(x, 1)), true),
            ("3ulp 차 f64(허용 폭 안)", json!(x), json!(up_ulps(x, 3)), true),
            ("2배 차 f64", json!(1.5), json!(3.0), false),
            ("상대 1e-12 차 f64(허용 폭 밖)", json!(x), json!(x * (1.0 + 1e-12)), false),
            ("0 vs 0 근방 극소 f64(절대 허용)", json!(0.0), json!(5e-324), true),
            ("0 vs 1e-300 f64", json!(0.0), json!(1e-300), false),
            ("같은 문자열", json!("abc"), json!("abc"), true),
            ("문자열 1글자 차", json!("abc"), json!("abd"), false),
            ("같은 객체", json!({"a": 1, "b": 2}), json!({"a": 1, "b": 2}), true),
            ("키 누락", json!({"a": 1, "b": 2}), json!({"a": 1}), false),
            ("키 이름만 다름(개수 같음)", json!({"a": 1}), json!({"b": 1}), false),
            ("정수 1 vs 실수 1.0(표현 뒤바뀜)", json!(1), json!(1.0), false),
            ("정수 1 vs 정수 2", json!(1), json!(2), false),
            ("음수 정수 같음", json!(-5), json!(-5), true),
            ("u64 최대 같음", json!(u64::MAX), json!(u64::MAX), true),
            ("u64 최대 vs 실수 u64::MAX as f64", json!(u64::MAX), json!(u64::MAX as f64), false),
            ("불리언 다름", json!(true), json!(false), false),
            ("null vs false", Value::Null, json!(false), false),
            ("숫자 vs 문자열", json!(1), json!("1"), false),
            ("배열 vs 객체", json!([]), json!({}), false),
            ("같은 배열", json!([1.0, 2.0, "x"]), json!([1.0, 2.0, "x"]), true),
            ("배열 길이 다름", json!([1.0, 2.0]), json!([1.0]), false),
            ("배열 순서 다름", json!([1, 2]), json!([2, 1]), false),
            ("깊은 곳 부동소수 1ulp", nested(x, "s", json!(7)), nested(up_ulps(x, 1), "s", json!(7)), true),
            ("깊은 곳 부동소수 2배", nested(x, "s", json!(7)), nested(x * 2.0, "s", json!(7)), false),
            ("깊은 곳 문자열 차", nested(x, "s", json!(7)), nested(x, "t", json!(7)), false),
            ("깊은 곳 정수 값 차", nested(x, "s", json!(7)), nested(x, "s", json!(8)), false),
            ("깊은 곳 정수↔실수 뒤바뀜", nested(x, "s", json!(7)), nested(x, "s", json!(7.0)), false),
            ("부동소수 1ulp + 문자열 차(혼합)", nested(x, "s", json!(7)), nested(up_ulps(x, 1), "t", json!(7)), false),
        ];
        for (name, a, b, want) in &rows {
            assert_eq!(equal_modulo_float(a, b), *want, "{name}: a={a} b={b}");
            assert_eq!(equal_modulo_float(b, a), *want, "{name}(대칭): a={a} b={b}");
        }
    }

    /// `round_trip_verdict` — 같음→Ok · 부동소수 정밀도 차만→FloatInexact · 구조 불일치(키·문자열·정수·표현·길이)→Drift ·
    /// 비-객체는 부동소수 허용에서 제외(→Drift 불변).
    #[test]
    fn k1_round_trip_verdict_classifies_float_vs_structure() {
        let x = 92.25474197811191_f64;
        let orig = json!({"id": 1, "ok": true, "result": {"used_pct": x, "name": "w", "n": [1, 2]}});
        assert_eq!(round_trip_verdict(&orig.clone(), &orig), Ok(()));

        fn mutated(orig: &Value, f: impl FnOnce(&mut Value)) -> Value {
            let mut v = orig.clone();
            f(&mut v);
            v
        }
        let float_only = mutated(&orig, |v| v["result"]["used_pct"] = json!(up_ulps(x, 1)));
        assert_eq!(round_trip_verdict(&float_only, &orig), Err(AbiError::FloatInexact), "부동소수 1ulp 만 다르면 FloatInexact");

        let structural: Vec<(&str, Value)> = vec![
            ("키 소실", mutated(&orig, |v| { v["result"].as_object_mut().unwrap().remove("name"); })),
            ("문자열 변질", mutated(&orig, |v| v["result"]["name"] = json!("W"))),
            ("정수 값 변질", mutated(&orig, |v| v["id"] = json!(2))),
            ("정수→실수 표현 뒤바뀜", mutated(&orig, |v| v["id"] = json!(1.0))),
            ("배열 길이", mutated(&orig, |v| v["result"]["n"] = json!([1]))),
            ("부동소수 1ulp + 문자열 변질(혼합)", mutated(&orig, |v| {
                v["result"]["used_pct"] = json!(up_ulps(x, 1));
                v["result"]["name"] = json!("W");
            })),
        ];
        for (name, bad) in &structural {
            assert_eq!(round_trip_verdict(bad, &orig), Err(AbiError::Drift), "{name} 는 Drift");
        }

        // 비-객체: 안의 부동소수가 1ulp 어긋나도 Drift 는 불변(와이어 계약 위반을 조용히 삼키지 않는다).
        let arr = json!([x]);
        let arr_back = json!([up_ulps(x, 1)]);
        assert_eq!(round_trip_verdict(&arr_back, &arr), Err(AbiError::Drift));
    }

    /// 결정론 벡터 — 재파싱이 어긋나는 17자리 f64 가 실린 응답은 `Err(FloatInexact)` 이지 `Err(Drift)` 가 아니다.
    /// 적색(수정 전 = 어긋나면 무조건 Drift · 돌연변이 M2 = 허용 폭 0 · M3 = 종전 로직 복원): `Err(Drift)` 가 나와 깨진다.
    #[test]
    fn k1_inexact_float_frame_is_float_inexact_not_drift() {
        let x = inexact_f64();
        let v = json!({"id": 1, "ok": true, "result": {"used_pct": x}});
        let back: Value = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
        assert_ne!(back, v, "전제 — 이 값은 이 빌드의 best-effort 파서에서 재파싱이 어긋난다");
        assert_eq!(frame_response(&v), Err(AbiError::FloatInexact), "x={x:?}");

        // 중첩·다중 부동소수 프레임(control.hw 모양)도 같다 — 구조는 같고 부동소수만 어긋남.
        let hw = json!({"id": 3, "ok": true, "result": {
            "cpu_pct": [x, 1.5, 2.25], "mem": {"used_pct": x, "total": 17_179_869_184u64}, "temps": {"soc": 45.5}, "label": "hw"}});
        assert_eq!(frame_response(&hw), Err(AbiError::FloatInexact));

        // 깊이 100 중첩(재파싱 한도 128 안) — 비교가 입력 깊이만큼만 재귀한다.
        let mut deep = json!([x]);
        for _ in 0..100 {
            deep = json!([deep]);
        }
        assert_eq!(frame_response(&json!({"id": 4, "ok": true, "result": deep})), Err(AbiError::FloatInexact));
    }

    /// 폴백 프레임(메타 `_flen`·`_pv` 없음)은 클라이언트 `parse_frame` 이 무검증 수용한다 — 부동소수 프레임이 값에 따라
    /// `abi: LenMismatch` 로 간헐 실패하지 않는다(구 클라이언트도 같은 함수 · 메타 없는 legacy peer 경로).
    #[test]
    fn k1_float_inexact_fallback_frame_is_accepted_by_the_client_parser() {
        let x = inexact_f64();
        let v = json!({"id": 1, "ok": true, "result": {"used_pct": x}});
        assert_eq!(frame_response(&v), Err(AbiError::FloatInexact));
        // cysd `frame_line` 의 폴백 = 종전 Drift 폴백과 같은 바이트(`to_string + "\n"`).
        let fallback = format!("{}\n", serde_json::to_string(&v).unwrap());
        assert!(!fallback.contains("_flen") && !fallback.contains("_pv"), "폴백에 메타가 붙었다: {fallback}");
        let parsed = parse_frame(&fallback).expect("메타 없는 폴백 프레임은 무검증 수용");
        assert_eq!(parsed["ok"].as_bool(), Some(true));
        assert!(parsed.get("_flen").is_none());
    }

    /// 구조 Drift 는 여전히 Drift — 비-객체 응답은 안에 어긋나는 부동소수가 있어도(없어도) `Drift`.
    #[test]
    fn k1_non_object_response_stays_drift_even_with_inexact_float() {
        let x = inexact_f64();
        assert_eq!(frame_response(&json!([x])), Err(AbiError::Drift));
        assert_eq!(frame_response(&json!([1, 2, 3])), Err(AbiError::Drift));
        assert_eq!(frame_response(&json!(x)), Err(AbiError::Drift));
    }

    /// 성질 검체 — 고정 시드 LCG 로 f64 20만 개(짝수 번째는 f32 를 넓힌 값 = 절반, 홀수 번째는 전 지수 범위 무작위 비트)를
    /// `{"result":{"v":x}}` 로 넣는다: 결과는 항상 `Ok(_)` 또는 `Err(FloatInexact)` 이고 `Err(Drift)` 는 0건.
    /// 판별력: `FloatInexact` 가 실제로 나와야 한다(0건이면 전제가 사라진 계측 무효).
    #[test]
    fn k1_property_random_floats_never_drift() {
        const N: usize = 200_000;
        let mut rng = Lcg(0x9E37_79B9_7F4A_7C15);
        let (mut ok, mut inexact) = (0usize, 0usize);
        let mut bad: Vec<(f64, AbiError)> = Vec::new();
        let (mut done, mut tried) = (0usize, 0usize);
        while done < N && tried < 2 * N {
            tried += 1;
            let x = if done % 2 == 0 {
                f64::from(f32::from_bits(rng.next32()))
            } else {
                f64::from_bits(rng.next64())
            };
            if !x.is_finite() {
                continue; // NaN·무한은 `Value` 에 못 들어간다
            }
            done += 1;
            match frame_response(&json!({"result": {"v": x}})) {
                Ok(_) => ok += 1,
                Err(AbiError::FloatInexact) => inexact += 1,
                Err(e) => bad.push((x, e)),
            }
        }
        eprintln!("K1OBS property n={done} ok={ok} float_inexact={inexact} drift_or_other={}", bad.len());
        assert_eq!(done, N, "표본 수 부족(비유한 건너뛰기 한도 초과)");
        assert!(bad.is_empty(), "Drift 등 {}건 — 앞 5건: {:?}", bad.len(), &bad[..bad.len().min(5)]);
        assert!(
            inexact > N / 100,
            "FloatInexact {inexact}건/{N} — 너무 적다: 이 빌드의 부동소수 재파싱이 정확해졌다면 검체 전제가 사라진 것(float_roundtrip?)"
        );
        assert_eq!(ok + inexact, N);
    }

    /// ★와이어 바이트 불변 핀(Ok 경로) — 검증을 통과한 프레임은 종전(0.14.42)과 **한 바이트도 다르지 않다**:
    /// `_flen`(payload 직렬화 길이)·`_pv`(=1)가 형제 키로 붙고 키는 사전순(BTreeMap), 끝에 개행 하나.
    /// (리터럴은 판정 변경 전 코드에서 같은 입력으로 뽑은 값과 대조했다 — 차분 대조 로그 참조.)
    #[test]
    fn k1_wire_bytes_ok_path_golden() {
        let ok = json!({"id": 1, "ok": true, "result": {"used_pct": 92.5, "rows": [1, 2, 3]}});
        assert_eq!(
            frame_response(&ok).unwrap(),
            "{\"_flen\":32,\"_pv\":1,\"id\":1,\"ok\":true,\"result\":{\"rows\":[1,2,3],\"used_pct\":92.5}}\n"
        );
        let err = json!({"id": 2, "ok": false, "error": {"code": "x", "message": "m"}});
        assert_eq!(
            frame_response(&err).unwrap(),
            "{\"_flen\":26,\"_pv\":1,\"error\":{\"code\":\"x\",\"message\":\"m\"},\"id\":2,\"ok\":false}\n"
        );
    }
}
