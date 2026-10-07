//! 공유 JSON 설정 파일 외과 수술 도우미 — agy_statusline(상태줄 자동 연결)과
//! claude_tui(윈도우 classic 렌더러 기록)가 같은 판독·스캐너·쓰기 가능 확인을 쓴다.
//! 순수 이동(0.14.45 성찰 2회차 C1) · 동작 변경 0.

use serde_json::Value;
use std::path::Path;

/// 이보다 큰 settings.json 은 건드리지 않는다(정상 파일은 수백 바이트 — 2026-09-24 이 맥 316B).
const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

pub(crate) struct Member {
    pub(crate) key_start: usize,
    pub(crate) key: String,
    pub(crate) val_start: usize,
    pub(crate) val_end: usize,
}

pub(crate) struct RootScan {
    pub(crate) open: usize,
    pub(crate) close: usize,
    pub(crate) members: Vec<Member>,
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

fn skip_string(b: &[u8], mut i: usize) -> Result<usize, String> {
    if b.get(i) != Some(&b'"') {
        return Err("문자열 시작이 아니다".into());
    }
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Ok(i + 1),
            _ => i += 1,
        }
    }
    Err("문자열이 닫히지 않았다".into())
}

fn skip_value(b: &[u8], i: usize) -> Result<usize, String> {
    match b.get(i) {
        None => Err("값이 없다".into()),
        Some(b'"') => skip_string(b, i),
        Some(b'{') | Some(b'[') => {
            let mut depth = 0usize;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = skip_string(b, j)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth = depth.checked_sub(1).ok_or("괄호 짝이 맞지 않는다")?;
                        if depth == 0 {
                            return Ok(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            Err("괄호가 닫히지 않았다".into())
        }
        Some(_) => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                j += 1;
            }
            if j == i {
                Err("빈 값".into())
            } else {
                Ok(j)
            }
        }
    }
}

/// 최상위 객체의 멤버 위치를 잰다. 입력은 이미 serde_json 으로 유효성이 확인된 텍스트(BOM 제거 뒤)다.
pub(crate) fn scan_root(t: &str) -> Result<RootScan, String> {
    let b = t.as_bytes();
    let mut i = skip_ws(b, 0);
    if b.get(i) != Some(&b'{') {
        return Err("루트가 객체가 아니다".into());
    }
    let open = i;
    i = skip_ws(b, i + 1);
    let mut members = Vec::new();
    if b.get(i) == Some(&b'}') {
        return Ok(RootScan { open, close: i, members });
    }
    loop {
        let key_start = i;
        let key_end = skip_string(b, i)?;
        let key: String = serde_json::from_str(&t[key_start..key_end]).map_err(|e| format!("키 판독 실패: {e}"))?;
        i = skip_ws(b, key_end);
        if b.get(i) != Some(&b':') {
            return Err("콜론이 없다".into());
        }
        i = skip_ws(b, i + 1);
        let val_start = i;
        let val_end = skip_value(b, i)?;
        members.push(Member { key_start, key, val_start, val_end });
        i = skip_ws(b, val_end);
        match b.get(i) {
            Some(b',') => i = skip_ws(b, i + 1),
            Some(b'}') => return Ok(RootScan { open, close: i, members }),
            _ => return Err("멤버 구분자가 없다".into()),
        }
    }
}

pub(crate) fn line_indent(t: &str, pos: usize) -> Option<&str> {
    let line_start = t[..pos].rfind('\n').map_or(0, |i| i + 1);
    let ind = &t[line_start..pos];
    (!ind.is_empty() && ind.chars().all(|c| c == ' ' || c == '\t')).then_some(ind)
}

/// 파일 판독 결과: (원문 바이트, BOM 뗀 본문, BOM 여부, 파싱 값 — 빈 파일이면 None).
pub(crate) type Loaded = (Vec<u8>, String, bool, Option<Value>);

pub(crate) fn load(settings: &Path) -> Result<Option<Loaded>, String> {
    let meta = match std::fs::symlink_metadata(settings) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("설정 파일 상태를 읽지 못했다: {e}")),
    };
    if meta.file_type().is_symlink() {
        return Err("심볼릭 링크다(dotfile 관리 도구 등)".into());
    }
    if !meta.is_file() {
        return Err("일반 파일이 아니다".into());
    }
    if meta.len() > MAX_SETTINGS_BYTES {
        return Err(format!("파일이 너무 크다({}바이트)", meta.len()));
    }
    let raw = std::fs::read(settings).map_err(|e| format!("읽지 못했다: {e}"))?;
    let (bom, body) = match raw.strip_prefix(b"\xef\xbb\xbf") {
        Some(rest) => (true, rest),
        None => (false, raw.as_slice()),
    };
    let text = String::from_utf8(body.to_vec()).map_err(|_| "UTF-8 이 아니다".to_string())?;
    if text.trim().is_empty() {
        return Ok(Some((raw, text, bom, None)));
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("JSON 파싱 실패({e})"))?;
    if !v.is_object() {
        return Err("루트가 객체가 아니다".into());
    }
    Ok(Some((raw, text, bom, Some(v))))
}

/// 쓰기 가능 확인 — 추가 모드로 열어 보기만 한다(내용·mtime 무변경). 읽기 전용·권한 없음·(윈도우) 잠김이면 Err.
pub(crate) fn probe_writable(settings: &Path) -> Result<(), String> {
    if std::fs::metadata(settings).map(|m| m.permissions().readonly()).unwrap_or(false) {
        return Err("읽기 전용 파일이다".into());
    }
    std::fs::OpenOptions::new()
        .append(true)
        .open(settings)
        .map(|_| ())
        .map_err(|e| format!("쓰기로 열 수 없다(권한·잠김): {e}"))
}

#[cfg(unix)]
pub(crate) fn file_mode(settings: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(settings).ok().map(|m| m.permissions().mode() & 0o7777)
}
#[cfg(not(unix))]
pub(crate) fn file_mode(_settings: &Path) -> Option<u32> {
    None
}
