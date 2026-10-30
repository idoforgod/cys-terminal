//! 세션 트랜스크립트 영속·검색 (자가개선 루프의 '기억' 절반).
//! 모든 surface의 stripped 출력을 SQLite FTS5(trigram)에 영속 — 어떤 에이전트(claude/gemini/
//! codex)의 터미널 활동이든 통합 검색된다. Hermes session_search의 대응이자 확장.

use crate::state::{state_dir, Daemon};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

pub struct LineRecord {
    pub ts: f64,
    pub surface_id: u64,
    pub role: Option<String>,
    pub title: String,
    pub line: String,
}

fn db_path(daemon: &Daemon) -> PathBuf {
    state_dir(&daemon.socket_path).join("transcripts.db")
}

fn open_db(path: &PathBuf) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS lines(
           id INTEGER PRIMARY KEY,
           ts REAL NOT NULL,
           surface_id INTEGER NOT NULL,
           role TEXT,
           title TEXT,
           line TEXT NOT NULL
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS lines_fts USING fts5(
           line, content='lines', content_rowid='id',
           tokenize='trigram case_sensitive 0'
         );
         CREATE TRIGGER IF NOT EXISTS lines_ai AFTER INSERT ON lines BEGIN
           INSERT INTO lines_fts(rowid, line) VALUES (new.id, new.line);
         END;
         CREATE TRIGGER IF NOT EXISTS lines_ad AFTER DELETE ON lines BEGIN
           INSERT INTO lines_fts(lines_fts, rowid, line) VALUES ('delete', old.id, old.line);
         END;
         CREATE TABLE IF NOT EXISTS chains(
           surface_id INTEGER PRIMARY KEY,
           line_count INTEGER NOT NULL,
           hash TEXT NOT NULL,
           anchor_count INTEGER NOT NULL DEFAULT 0,
           anchor_hash TEXT NOT NULL DEFAULT ''
         );",
    )?;
    Ok(conn)
}

const GENESIS: [u8; 32] = [0u8; 32];

fn hash_step(prev: &[u8; 32], line: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(prev);
    h.update(line.as_bytes());
    h.update(b"\n");
    h.finalize().into()
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// 전용 writer 스레드: 채널로 받은 라인을 1초 배치로 insert (데몬 핫패스 비블로킹).
/// T4-18: 삽입 순서대로 surface별 해시체인을 갱신 — 저장 트랜스크립트의 변조 증거성.
/// T4-19: 주기 보존 정리(prune) — 무한 성장 차단. 정리된 prefix는 anchor로 봉인되어
/// attest 검증 지평(retention 창) 너머의 pin은 명시적으로 검증 불가 처리된다.
pub fn spawn_writer(daemon_socket: PathBuf) -> Sender<LineRecord> {
    let (tx, rx): (Sender<LineRecord>, Receiver<LineRecord>) = std::sync::mpsc::channel();
    let path = state_dir(&daemon_socket).join("transcripts.db");
    std::thread::spawn(move || {
      // ★P0-5(D3/W5): recall 쓰기 스레드 본문을 catch_unwind 로 감싼다 — 어떤 panic 도 삼키지 않고 stderr 에
      //   기록하고 스레드 자연 종료(무한 재스폰 금지). last_prune Instant 언더플로(위 수리)처럼 부트 스레드가
      //   침묵사(P0-5)하던 클래스의 구조 방어. AssertUnwindSafe: 패닉 후 공유상태 재사용 없음(로그만).
      let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let Ok(conn) = open_db(&path) else {
            eprintln!("[recall] cannot open {}", path.display());
            return;
        };
        // 체인 상태 로드: surface_id → (line_count, hash)
        let mut chains: std::collections::HashMap<u64, (u64, [u8; 32])> = conn
            .prepare("SELECT surface_id, line_count, hash FROM chains")
            .and_then(|mut stmt| {
                let rows = stmt
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, i64>(0)? as u64,
                            r.get::<_, i64>(1)? as u64,
                            r.get::<_, String>(2)?,
                        ))
                    })?
                    .filter_map(|r| r.ok())
                    .filter_map(|(sid, count, h)| unhex(&h).map(|hh| (sid, (count, hh))))
                    .collect();
                Ok(rows)
            })
            .unwrap_or_default();
        let mut last_prune: Option<std::time::Instant> = None; // ★None=즉시 due(Instant 뺄셈 언더플로 panic 제거)
        let mut buf: Vec<LineRecord> = Vec::new();
        loop {
            // 첫 레코드는 블로킹 대기, 이후 1초 또는 200건까지 모아 배치
            match rx.recv_timeout(Duration::from_secs(600)) {
                Ok(first) => buf.push(first),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    maybe_prune(&conn, &mut chains, &mut last_prune);
                    continue;
                }
                Err(_) => return, // 데몬 종료
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while buf.len() < 200 {
                let remain = deadline.saturating_duration_since(std::time::Instant::now());
                if remain.is_zero() {
                    break;
                }
                match rx.recv_timeout(remain) {
                    Ok(r) => buf.push(r),
                    Err(_) => break,
                }
            }
            let mut touched: std::collections::HashSet<u64> = std::collections::HashSet::new();
            let _ = conn.execute_batch("BEGIN");
            {
                let mut stmt = conn
                    .prepare_cached(
                        "INSERT INTO lines(ts, surface_id, role, title, line) VALUES (?1,?2,?3,?4,?5)",
                    )
                    .unwrap();
                for r in buf.drain(..) {
                    let ok = stmt
                        .execute(rusqlite::params![
                            r.ts,
                            r.surface_id as i64,
                            r.role,
                            r.title,
                            r.line
                        ])
                        .is_ok();
                    if ok {
                        // 체인은 '실제 삽입된' 라인만 따라간다 (insert 실패 시 미반영)
                        let entry = chains.entry(r.surface_id).or_insert((0, GENESIS));
                        entry.1 = hash_step(&entry.1, &r.line);
                        entry.0 += 1;
                        touched.insert(r.surface_id);
                    }
                }
            }
            // 체인 upsert를 같은 트랜잭션에 — 라인과 체인의 원자적 동행
            {
                let mut up = conn
                    .prepare_cached(
                        "INSERT INTO chains(surface_id, line_count, hash) VALUES (?1,?2,?3)
                         ON CONFLICT(surface_id) DO UPDATE SET line_count=?2, hash=?3",
                    )
                    .unwrap();
                for sid in &touched {
                    if let Some((count, h)) = chains.get(sid) {
                        let _ = up.execute(rusqlite::params![*sid as i64, *count as i64, hex(h)]);
                    }
                }
            }
            let _ = conn.execute_batch("COMMIT");
            maybe_prune(&conn, &mut chains, &mut last_prune);
        }
      }));
      if let Err(panic) = outcome {
          let msg = panic
              .downcast_ref::<&str>()
              .map(|s| (*s).to_string())
              .or_else(|| panic.downcast_ref::<String>().cloned())
              .unwrap_or_else(|| "unknown panic payload".to_string());
          eprintln!("[recall] ★쓰기 스레드 panic 포착(P0-5 침묵사 차단·재스폰 안 함): {msg}");
      }
    });
    tx
}

/// T4-19 보존 정리: CYS_RECALL_RETAIN_DAYS(기본 30, 0=비활성)보다 오래된 prefix를
/// surface별로 anchor에 봉인한 뒤 삭제한다 (체인 정합 유지 — id 순 prefix 삭제).
fn maybe_prune(
    conn: &Connection,
    chains: &mut std::collections::HashMap<u64, (u64, [u8; 32])>,
    last_prune: &mut Option<std::time::Instant>,
) {
    // ★P0-5(D3/W5·CI 28780215417 실증): 과거 `Instant::now() - Duration::from_secs(86400)`(24h 전) 초기화는
    //   Windows CI VM 처럼 **부팅<24h** 이면 Instant(부팅 원점 단조시계) 언더플로로 panic("overflow when
    //   subtracting duration from instant") → recall 부트 스레드 침묵사. Option<Instant> 로 대체 — None=한 번도
    //   prune 안 함=즉시 due(원래 "첫 prune 즉시 발동" 의도 보존)·Instant 뺄셈 0(non-panicking).
    if let Some(t) = last_prune {
        if t.elapsed() < Duration::from_secs(6 * 3600) {
            return;
        }
    }
    *last_prune = Some(std::time::Instant::now());
    let days: f64 = std::env::var("CYS_RECALL_RETAIN_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30.0);
    if days <= 0.0 {
        return;
    }
    let cutoff = crate::state::now_epoch() - days * 86400.0;
    let sids: Vec<u64> = match conn
        .prepare("SELECT DISTINCT surface_id FROM lines WHERE ts < ?1")
        .and_then(|mut s| {
            let rows = s
                .query_map([cutoff], |r| r.get::<_, i64>(0))?
                .filter_map(|r| r.ok())
                .map(|v| v as u64)
                .collect::<Vec<_>>();
            Ok(rows)
        }) {
        Ok(v) => v,
        Err(_) => return,
    };
    for sid in sids {
        // anchor 로드 (없으면 genesis)
        let (mut anchor_count, mut anchor_hash) = conn
            .query_row(
                "SELECT anchor_count, anchor_hash FROM chains WHERE surface_id=?1",
                [sid as i64],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)? as u64,
                        r.get::<_, String>(1)?,
                    ))
                },
            )
            .ok()
            .map(|(c, h)| (c, unhex(&h).unwrap_or(GENESIS)))
            .unwrap_or((0, GENESIS));
        // 삭제 대상 prefix를 id 순으로 해시하며 봉인
        let rows: Vec<(i64, String)> = match conn
            .prepare("SELECT id, line FROM lines WHERE surface_id=?1 AND ts<?2 ORDER BY id")
            .and_then(|mut s| {
                let rows = s
                    .query_map(rusqlite::params![sid as i64, cutoff], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                    })?
                    .filter_map(|r| r.ok())
                    .collect::<Vec<_>>();
                Ok(rows)
            }) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if rows.is_empty() {
            continue;
        }
        let max_id = rows.last().map(|(id, _)| *id).unwrap_or(0);
        for (_, line) in &rows {
            anchor_hash = hash_step(&anchor_hash, line);
            anchor_count += 1;
        }
        let _ = conn.execute_batch("BEGIN");
        let _ = conn.execute(
            "DELETE FROM lines WHERE surface_id=?1 AND id<=?2",
            rusqlite::params![sid as i64, max_id],
        );
        let _ = conn.execute(
            "UPDATE chains SET anchor_count=?2, anchor_hash=?3 WHERE surface_id=?1",
            rusqlite::params![sid as i64, anchor_count as i64, hex(&anchor_hash)],
        );
        let _ = conn.execute_batch("COMMIT");
        // 메모리 체인 상태는 total 기준이라 prune과 무관 (anchor만 전진)
        let _ = chains;
    }
}

/// T4-18 attest pin: 평가자(master)가 외부(SESSION_STATE 등)에 보관할 (count, hash).
/// 커밋된 상태 기준 — 버퍼 중(최대 1초)의 라인은 다음 pin에 반영된다.
pub fn attest_pin(daemon: &Daemon, surface_id: u64) -> Result<Value, String> {
    let conn = open_db(&db_path(daemon)).map_err(|e| e.to_string())?;
    let row = conn
        .query_row(
            "SELECT line_count, hash, anchor_count FROM chains WHERE surface_id=?1",
            [surface_id as i64],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )
        .map_err(|_| format!("no transcript chain for surface {surface_id}"))?;
    Ok(serde_json::json!({
        "surface_id": surface_id, "count": row.0, "hash": row.1,
        "verification_horizon": {"anchor_count": row.2,
            "note": "pin with count <= anchor_count is beyond the retention window"},
    }))
}

/// T4-18 attest verify: 평가자가 보관한 pin(count, hash)과 저장 트랜스크립트를 대조.
/// producer(워커)가 DB를 사후 수정하면 체인이 깨져 여기서 드러난다.
pub fn attest_verify(
    daemon: &Daemon,
    surface_id: u64,
    pin_hash: &str,
    pin_count: u64,
) -> Result<Value, String> {
    let mut conn = open_db(&db_path(daemon)).map_err(|e| e.to_string())?;
    // anchor 읽기와 lines 읽기를 하나의 deferred 읽기 트랜잭션으로 묶어 단일 스냅샷에서
    // 본다. 둘을 분리된 autocommit statement로 읽으면 그 사이에 writer 스레드의 maybe_prune
    // 커밋(prefix 삭제 + anchor 전진을 한 트랜잭션)이 끼어 prune 전 anchor와 prune 후 lines를
    // 섞어 읽고, 정직한 pin을 'insufficient rows'로 거짓 거부한다(TOCTOU). 첫 SELECT가
    // 스냅샷을 고정하므로 그 뒤의 prune 커밋은 두 읽기 모두에 보이지 않는다.
    let tx = conn
        .transaction()
        .map_err(|e| e.to_string())?;
    let (anchor_count, anchor_hash) = tx
        .query_row(
            "SELECT anchor_count, anchor_hash FROM chains WHERE surface_id=?1",
            [surface_id as i64],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?)),
        )
        .ok()
        .map(|(c, h)| (c, unhex(&h).unwrap_or(GENESIS)))
        .unwrap_or((0, GENESIS));
    if pin_count < anchor_count {
        return Ok(serde_json::json!({
            "match": false, "reason": "pin is beyond the retention window (pruned)",
            "anchor_count": anchor_count, "pin_count": pin_count,
        }));
    }
    let need = pin_count - anchor_count;
    // need가 i64::MAX를 넘는 거대 u64면 `need as i64`가 음수로 래핑되고, SQLite는 음수 LIMIT을
    // '무제한'으로 해석해 surface 전 행을 스캔한다(자원 결함). 어떤 surface도 i64::MAX 행을
    // 가질 수 없으므로 LIMIT 바인딩만 i64::MAX로 클램프한다 — used < need 비교는 클램프 안 된
    // need로 그대로 수행돼 fail-closed('insufficient rows') 의미는 불변.
    let limit = need.min(i64::MAX as u64) as i64;
    let mut h = anchor_hash;
    let mut used: u64 = 0;
    {
        let mut stmt = tx
            .prepare("SELECT line FROM lines WHERE surface_id=?1 ORDER BY id LIMIT ?2")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![surface_id as i64, limit], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        for line in rows.filter_map(|r| r.ok()) {
            h = hash_step(&h, &line);
            used += 1;
        }
    }
    if used < need {
        return Ok(serde_json::json!({
            "match": false, "reason": "insufficient rows (transcript rows missing/deleted)",
            "rows_needed": need, "rows_used": used,
        }));
    }
    let computed = hex(&h);
    Ok(serde_json::json!({
        "match": computed == pin_hash,
        "computed_hash": computed, "pin_hash": pin_hash,
        "count": pin_count, "rows_used": used,
    }))
}

/// 데몬 재시작 시 surface_id 연속성 seed: 영속 트랜스크립트의 최대 id.
/// (재시작마다 1부터 재발급하면 무관 세션이 같은 id로 recall에 합쳐진다)
/// lines만 보면 prune이 한 surface의 라인을 전부 삭제했을 때(전체가 보존창 밖) 그
/// surface_id가 빠져 seed가 낮아진다 → 새 surface가 옛 chains 행과 같은 id를 공유하는
/// 체인 오염이 발생한다. chains는 prune에서 삭제되지 않는 영구 권위 레지스트리이므로
/// 두 테이블의 MAX를 함께 취해 high-water mark가 prune에도 단조 유지되게 한다.
pub fn max_surface_id(socket_path: &std::path::Path) -> u64 {
    let path = state_dir(socket_path).join("transcripts.db");
    if !path.exists() {
        return 0;
    }
    Connection::open(&path)
        .ok()
        .and_then(|c| {
            c.query_row(
                "SELECT MAX(m) FROM (
                   SELECT COALESCE(MAX(surface_id), 0) AS m FROM lines
                   UNION ALL
                   SELECT COALESCE(MAX(surface_id), 0) AS m FROM chains
                 )",
                [],
                |r| r.get::<_, i64>(0),
            )
            .ok()
        })
        .map(|v| v.max(0) as u64)
        .unwrap_or(0)
}

/// FTS 검색 (RPC recall.search). 쿼리는 phrase로 quoting해 FTS 구문 주입을 차단.
/// trigram 토크나이저는 3자 미만에 항상 빈 결과 — 짧은 쿼리는 LIKE 스캔으로 폴백한다.
pub fn search(
    daemon: &Daemon,
    query: &str,
    role: Option<String>,
    surface_id: Option<u64>,
    days: Option<f64>,
    limit: u64,
) -> Result<Value, String> {
    let conn = open_db(&db_path(daemon)).map_err(|e| e.to_string())?;
    let use_fts = query.chars().count() >= 3;
    let mut sql;
    let mut params: Vec<Box<dyn rusqlite::ToSql>>;
    if use_fts {
        let phrase = format!("\"{}\"", query.replace('"', "\"\""));
        sql = String::from(
            "SELECT l.ts, l.surface_id, l.role, l.title, l.line
             FROM lines l JOIN lines_fts f ON l.id = f.rowid
             WHERE lines_fts MATCH ?1",
        );
        params = vec![Box::new(phrase)];
    } else {
        let like = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        sql = String::from(
            "SELECT l.ts, l.surface_id, l.role, l.title, l.line
             FROM lines l
             WHERE l.line LIKE ?1 ESCAPE '\\'",
        );
        params = vec![Box::new(like)];
    }
    if let Some(r) = role {
        sql.push_str(&format!(" AND l.role = ?{}", params.len() + 1));
        params.push(Box::new(r));
    }
    if let Some(s) = surface_id {
        sql.push_str(&format!(" AND l.surface_id = ?{}", params.len() + 1));
        params.push(Box::new(s as i64));
    }
    if let Some(d) = days {
        let cutoff = crate::state::now_epoch() - d * 86400.0;
        sql.push_str(&format!(" AND l.ts >= ?{}", params.len() + 1));
        params.push(Box::new(cutoff));
    }
    sql.push_str(&format!(" ORDER BY l.ts DESC LIMIT {}", limit.min(500)));

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(json!({
                "ts": row.get::<_, f64>(0)?,
                "surface_id": row.get::<_, i64>(1)?,
                "role": row.get::<_, Option<String>>(2)?,
                "title": row.get::<_, Option<String>>(3)?,
                "line": row.get::<_, String>(4)?,
            }))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect::<Vec<_>>();
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM lines", [], |r| r.get(0))
        .unwrap_or(0);
    Ok(json!({"matches": rows, "indexed_lines": total}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_unhex_roundtrip() {
        let g = hex(&GENESIS);
        assert_eq!(g, "0".repeat(64));
        assert_eq!(unhex(&g), Some(GENESIS));
        // 임의 바이트 왕복
        let mut b = [0u8; 32];
        for (i, x) in b.iter_mut().enumerate() {
            *x = (i as u8).wrapping_mul(7).wrapping_add(3);
        }
        assert_eq!(unhex(&hex(&b)), Some(b));
    }

    #[test]
    fn unhex_rejects_bad_input() {
        // 길이 != 64
        assert_eq!(unhex(""), None);
        assert_eq!(unhex(&"0".repeat(63)), None);
        assert_eq!(unhex(&"0".repeat(65)), None);
        // 비-16진 문자 (길이는 64지만 'g' 포함)
        let mut bad = "0".repeat(62);
        bad.push_str("gg");
        assert_eq!(unhex(&bad), None);
        // 대문자 16진도 허용 (from_str_radix는 대소문자 무관)
        let upper = "AB".repeat(32);
        assert!(unhex(&upper).is_some());
    }

    #[test]
    fn unhex_is_non_canonical_but_byte_comparison_is_safe() {
        // ★R3 발견(보안 무해 확인): u8::from_str_radix("+a",16)=Ok(10)이라 unhex는
        // '+' 접두 바이트 윈도를 받아들인다 → 같은 바이트로 디코드되는 비정규 인코딩 존재.
        // 그러나 attest 비교는 '디코드된 [u8;32]'끼리이므로 위조 위협이 아니다:
        //   "+a"+"0"*62 와 "0a"+"0"*62 는 다른 문자열이지만 같은 바이트로 디코드되고,
        //   여전히 재계산된 정규 체인 해시 바이트와 '같아야만' 통과한다.
        // 즉 비정규 입력이 통과해도 '엉뚱한 해시를 정답으로 둔갑'시키지 못한다.
        let plus = format!("+a{}", "0".repeat(62));
        let canon = format!("0a{}", "0".repeat(62));
        assert_eq!(plus.len(), 64);
        // 두 인코딩이 동일 바이트로 디코드됨 (canonicity 부재의 정확한 표현)
        assert_eq!(unhex(&plus), unhex(&canon));
        // 핵심 안전성: 정규 hex() 출력은 항상 [0-9a-f]만 — round-trip은 정규형으로 닫힌다
        let bytes = unhex(&canon).unwrap();
        assert_eq!(hex(&bytes), canon); // 재인코딩은 '0a…' 정규형
        assert_ne!(hex(&bytes), plus); // '+a…'로 되돌아가지 않음
    }

    #[test]
    fn hash_step_is_deterministic_and_order_sensitive() {
        // 같은 입력 → 같은 출력
        let h1 = hash_step(&GENESIS, "hello");
        let h2 = hash_step(&GENESIS, "hello");
        assert_eq!(h1, h2);
        // 다른 라인 → 다른 해시
        assert_ne!(hash_step(&GENESIS, "a"), hash_step(&GENESIS, "b"));
        // prev가 다르면 같은 라인이어도 다른 해시 (체이닝)
        assert_ne!(hash_step(&GENESIS, "x"), hash_step(&h1, "x"));
        // 순서 의존: a→b 와 b→a 는 다른 최종 해시
        let ab = hash_step(&hash_step(&GENESIS, "a"), "b");
        let ba = hash_step(&hash_step(&GENESIS, "b"), "a");
        assert_ne!(ab, ba);
    }

    #[test]
    fn hash_step_includes_newline_delimiter() {
        // "ab" 한 줄 != "a","b" 두 줄 — 줄 경계가 해시에 반영돼
        // 라인 병합/분할로 같은 체인이 나오지 않아야 한다 (변조 증거성의 핵심).
        let one_line = hash_step(&GENESIS, "ab");
        let two_lines = hash_step(&hash_step(&GENESIS, "a"), "b");
        assert_ne!(one_line, two_lines);
    }

    /// spawn_writer/attest_verify가 의존하는 체인 정합 불변식을 실제 DB로 검증한다.
    /// (라인 삽입 순서대로 GENESIS부터 hash_step을 누적하면 chains.hash와 일치하고,
    ///  라인 하나라도 변조되면 재계산 해시가 어긋난다 — attest_verify의 핵심 보증.)
    #[test]
    fn chain_matches_recomputation_and_detects_tamper() {
        let dir = std::env::temp_dir().join(format!("cys_recall_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("chain_{}.db", line!()));
        let _ = std::fs::remove_file(&path);
        let conn = open_db(&path).expect("open temp db");

        let sid: i64 = 7;
        let lines = ["alpha", "beta", "gamma"];
        // 삽입 + 체인 누적 (writer 루프 본문과 동일한 순서·연산)
        let mut h = GENESIS;
        let mut count: i64 = 0;
        for line in lines {
            conn.execute(
                "INSERT INTO lines(ts, surface_id, role, title, line) VALUES (1.0,?1,NULL,NULL,?2)",
                rusqlite::params![sid, line],
            )
            .unwrap();
            h = hash_step(&h, line);
            count += 1;
        }
        conn.execute(
            "INSERT INTO chains(surface_id, line_count, hash) VALUES (?1,?2,?3)",
            rusqlite::params![sid, count, hex(&h)],
        )
        .unwrap();

        // 재계산: GENESIS부터 저장된 라인을 id 순으로 hash_step → chains.hash와 일치
        let stored_hash: String = conn
            .query_row(
                "SELECT hash FROM chains WHERE surface_id=?1",
                [sid],
                |r| r.get(0),
            )
            .unwrap();
        let recompute = |conn: &Connection| -> String {
            let mut hh = GENESIS;
            let mut stmt = conn
                .prepare("SELECT line FROM lines WHERE surface_id=?1 ORDER BY id")
                .unwrap();
            let rows = stmt
                .query_map([sid], |r| r.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();
            for l in rows {
                hh = hash_step(&hh, &l);
            }
            hex(&hh)
        };
        assert_eq!(recompute(&conn), stored_hash, "정상 체인은 재계산과 일치");

        // 변조: 한 라인 내용을 사후 수정하면 재계산 해시가 어긋난다
        conn.execute(
            "UPDATE lines SET line='TAMPERED' WHERE surface_id=?1 AND line='beta'",
            [sid],
        )
        .unwrap();
        assert_ne!(
            recompute(&conn),
            stored_hash,
            "변조된 라인은 체인 불일치로 드러나야 한다"
        );

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// CYS_RECALL_RETAIN_DAYS는 프로세스 전역 — maybe_prune 테스트가 cargo 병렬 러너에서
    /// 서로의 env를 덮어쓰지 않게 직렬화한다 (env 경합 flakiness 차단).
    static RETAIN_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn temp_db(tag: u32) -> (PathBuf, Connection) {
        let dir = std::env::temp_dir().join(format!("cys_recall_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("attest_{tag}.db"));
        let _ = std::fs::remove_file(&path);
        let conn = open_db(&path).expect("open temp db");
        (path, conn)
    }

    /// attest_verify의 검증 계산을 프로덕션 코드와 동일한 쿼리 경로로 재현한다.
    /// (recall.rs:296 attest_verify 본문과 1:1 대응 — anchor 로드 → 윈도우 게이트 →
    ///  need = pin_count - anchor_count → ORDER BY id LIMIT need → hash_step 누적 → 비교.
    ///  분기 reason 문자열까지 동일하게 미러링해 프로덕션 분기 회귀를 잡는다.)
    fn verify_mirror(conn: &Connection, sid: i64, pin_hash: &str, pin_count: u64) -> Value {
        let (anchor_count, anchor_hash) = conn
            .query_row(
                "SELECT anchor_count, anchor_hash FROM chains WHERE surface_id=?1",
                [sid],
                |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?)),
            )
            .ok()
            .map(|(c, h)| (c, unhex(&h).unwrap_or(GENESIS)))
            .unwrap_or((0, GENESIS));
        if pin_count < anchor_count {
            return json!({"match": false, "reason": "pin is beyond the retention window (pruned)"});
        }
        let need = pin_count - anchor_count;
        let mut h = anchor_hash;
        let mut used: u64 = 0;
        {
            let mut stmt = conn
                .prepare("SELECT line FROM lines WHERE surface_id=?1 ORDER BY id LIMIT ?2")
                .unwrap();
            let rows = stmt
                .query_map(rusqlite::params![sid, need as i64], |r| r.get::<_, String>(0))
                .unwrap();
            for line in rows.filter_map(|r| r.ok()) {
                h = hash_step(&h, &line);
                used += 1;
            }
        }
        if used < need {
            return json!({"match": false, "reason": "insufficient rows (transcript rows missing/deleted)"});
        }
        json!({"match": hex(&h) == pin_hash, "rows_used": used})
    }

    /// 평가자가 보관한 정직한 pin(count, hash)은 검증을 통과하고,
    /// 잘못된 hash는 불일치로 떨어진다 (attest의 1차 보증).
    #[test]
    fn attest_verify_honest_pin_matches_and_wrong_hash_fails() {
        let (path, conn) = temp_db(line!());
        let sid: i64 = 3;
        let lines = ["one", "two", "three", "four"];
        let mut h = GENESIS;
        for line in lines {
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (1.0,?1,NULL,NULL,?2)",
                rusqlite::params![sid, line],
            )
            .unwrap();
            h = hash_step(&h, line);
        }
        conn.execute(
            "INSERT INTO chains(surface_id,line_count,hash) VALUES (?1,?2,?3)",
            rusqlite::params![sid, lines.len() as i64, hex(&h)],
        )
        .unwrap();

        // 정직한 pin: count=4, hash=정상 → match true
        let pin_hash = hex(&h);
        let r = verify_mirror(&conn, sid, &pin_hash, 4);
        assert_eq!(r["match"].as_bool(), Some(true));
        assert_eq!(r["rows_used"].as_u64(), Some(4));

        // 잘못된 hash(다른 라인 집합) → match false (위조 보관 pin 거부)
        let wrong = hex(&hash_step(&GENESIS, "forged"));
        assert_eq!(verify_mirror(&conn, sid, &wrong, 4)["match"].as_bool(), Some(false));

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// 라인이 사후 삭제되면 used < need 로 "insufficient rows" — producer의 증거 인멸 탐지.
    #[test]
    fn attest_verify_detects_deleted_rows() {
        let (path, conn) = temp_db(line!());
        let sid: i64 = 5;
        let mut h = GENESIS;
        for line in ["a", "b", "c"] {
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (1.0,?1,NULL,NULL,?2)",
                rusqlite::params![sid, line],
            )
            .unwrap();
            h = hash_step(&h, line);
        }
        conn.execute(
            "INSERT INTO chains(surface_id,line_count,hash) VALUES (?1,3,?2)",
            rusqlite::params![sid, hex(&h)],
        )
        .unwrap();
        // pin은 3줄을 주장하나 한 줄을 삭제 → 검증 시 행 부족으로 거부
        conn.execute("DELETE FROM lines WHERE surface_id=?1 AND line='c'", [sid])
            .unwrap();
        let r = verify_mirror(&conn, sid, &hex(&h), 3);
        assert_eq!(r["match"].as_bool(), Some(false));
        assert_eq!(
            r["reason"].as_str(),
            Some("insufficient rows (transcript rows missing/deleted)")
        );

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// prune로 봉인된 anchor 너머의 pin(pin_count < anchor_count)은 검증 지평 밖 →
    /// 명시적 거부. anchor 경계의 pin(pin_count == anchor_count)은 봉인 해시와 비교 가능.
    #[test]
    fn attest_verify_anchor_window_boundary() {
        let (path, conn) = temp_db(line!());
        let sid: i64 = 9;
        // 5줄을 가정한 뒤 앞 2줄을 prune해 anchor=2로 봉인, 남은 3줄은 lines에 보존.
        let pruned = ["g0", "g1"];
        let retained = ["g2", "g3", "g4"];
        let mut anchor_h = GENESIS;
        for line in pruned {
            anchor_h = hash_step(&anchor_h, line);
        }
        // 보존 라인만 lines 테이블에 (prune이 id<=max를 삭제한 상태를 모사)
        for line in retained {
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (1.0,?1,NULL,NULL,?2)",
                rusqlite::params![sid, line],
            )
            .unwrap();
        }
        // total 체인 해시(참고)와 anchor(count=2, hash) 기록
        let mut total_h = anchor_h;
        for line in retained {
            total_h = hash_step(&total_h, line);
        }
        conn.execute(
            "INSERT INTO chains(surface_id,line_count,hash,anchor_count,anchor_hash)
             VALUES (?1,5,?2,2,?3)",
            rusqlite::params![sid, hex(&total_h), hex(&anchor_h)],
        )
        .unwrap();

        // pin_count=1 (< anchor_count 2) → retention 창 너머로 명시적 거부
        let beyond = verify_mirror(&conn, sid, &"0".repeat(64), 1);
        assert_eq!(beyond["match"].as_bool(), Some(false));
        assert_eq!(
            beyond["reason"].as_str(),
            Some("pin is beyond the retention window (pruned)")
        );

        // pin_count=2 (== anchor_count) → need=0, 봉인 해시와 직접 비교 (경계 검증 가능)
        let at_anchor = verify_mirror(&conn, sid, &hex(&anchor_h), 2);
        assert_eq!(at_anchor["match"].as_bool(), Some(true));
        assert_eq!(at_anchor["rows_used"].as_u64(), Some(0));

        // pin_count=5 (전체) → anchor부터 보존 3줄 재계산 = total 해시와 일치
        let full = verify_mirror(&conn, sid, &hex(&total_h), 5);
        assert_eq!(full["match"].as_bool(), Some(true));
        assert_eq!(full["rows_used"].as_u64(), Some(3));

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// 거대 pin_count(i64::MAX 초과)는 need = pin_count - anchor_count를 i64::MAX 너머로 키운다.
    /// `need as i64`는 음수로 래핑되고 SQLite는 음수 LIMIT을 '무제한'으로 해석 → surface 전 행
    /// 스캔(자원 결함). 수정은 LIMIT 바인딩을 i64::MAX로 클램프한다. 이 테스트는 ①음수 LIMIT의
    /// 무제한 스캔을 실증하고 ②클램프한 LIMIT이 스캔을 실제 행 수로 묶으면서도 used < need로
    /// fail-closed('insufficient rows')를 유지함을 박제한다 — attest_verify(recall.rs:326)와 미러.
    #[test]
    fn attest_verify_huge_count_clamps_limit_no_full_scan() {
        let (path, conn) = temp_db(line!());
        let sid: i64 = 88;
        // surface에 3줄만 보관 (anchor=0, 미prune).
        let lines = ["a", "b", "c"];
        let mut h = GENESIS;
        for line in lines {
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (1.0,?1,NULL,NULL,?2)",
                rusqlite::params![sid, line],
            )
            .unwrap();
            h = hash_step(&h, line);
        }
        conn.execute(
            "INSERT INTO chains(surface_id,line_count,hash) VALUES (?1,3,?2)",
            rusqlite::params![sid, hex(&h)],
        )
        .unwrap();

        // 공격 입력: param_u64는 u64 전 범위를 허용 → pin_count = u64::MAX.
        let pin_count: u64 = u64::MAX;
        let anchor_count: u64 = 0; // chains 기본값
        let need = pin_count - anchor_count;

        // ① 근본원인 실증: 래핑된 음수 LIMIT은 SQLite에서 '무제한' = 전 행 스캔.
        let wrapped_limit = need as i64; // = -1
        assert!(wrapped_limit < 0, "거대 need는 i64로 음수 래핑되어야 한다");
        let scanned_with_wrap: u64 = {
            let mut stmt = conn
                .prepare("SELECT line FROM lines WHERE surface_id=?1 ORDER BY id LIMIT ?2")
                .unwrap();
            stmt.query_map(rusqlite::params![sid, wrapped_limit], |r| r.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .count() as u64
        };
        assert_eq!(
            scanned_with_wrap, 3,
            "음수 LIMIT은 무제한으로 해석돼 surface 전 행을 스캔한다(버그 증명)"
        );

        // ② 수정 검증: LIMIT을 i64::MAX로 클램프하면 음수 래핑이 사라진다.
        //    (실제 행 수가 i64::MAX보다 작으므로 스캔 행 수는 동일하지만, LIMIT은 더 이상
        //     '무제한 음수'가 아니라 양의 상한이다 — 음수 LIMIT 우발경로 제거가 핵심.)
        let clamped_limit = need.min(i64::MAX as u64) as i64;
        assert!(clamped_limit > 0, "클램프된 LIMIT은 양수여야 한다");
        assert_eq!(clamped_limit, i64::MAX);
        let used: u64 = {
            let mut stmt = conn
                .prepare("SELECT line FROM lines WHERE surface_id=?1 ORDER BY id LIMIT ?2")
                .unwrap();
            stmt.query_map(rusqlite::params![sid, clamped_limit], |r| r.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .count() as u64
        };

        // ③ 안전성 불변식: 클램프 안 된 need로 비교 → used(3) < need(거대) → fail-closed.
        assert!(used < need, "used는 거대 need에 한참 못 미쳐 'insufficient rows'로 거부");

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// maybe_prune 실함수 end-to-end: 보존창보다 오래된 prefix를 anchor로 봉인한 뒤 삭제하고,
    /// 봉인 anchor(count,hash) + 남은 라인 재계산이 prune 전 total 체인과 정확히 일치하는지 검증.
    /// (hand-build anchor가 아니라 maybe_prune가 실제로 계산한 anchor의 정합성을 박제 —
    ///  봉인 누락·순서 오류·off-by-one이 있으면 attest가 영구 거짓이 되는 핵심 불변식.)
    #[test]
    fn maybe_prune_seals_anchor_consistent_with_full_chain() {
        let (path, conn) = temp_db(line!());
        let sid: i64 = 11;
        let now = crate::state::now_epoch();
        // 5줄: 앞 3줄은 보존창 밖(오래됨, prune 대상), 뒤 2줄은 최근(보존).
        // RETAIN_DAYS=30 기준 cutoff = now - 30일. old_ts는 그보다 더 과거.
        let old_ts = now - 100.0 * 86400.0; // 100일 전 → 확실히 prune
        let recent_ts = now; // 지금 → 보존
        let old_lines = ["o0", "o1", "o2"];
        let recent_lines = ["r0", "r1"];
        for l in old_lines {
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (?1,?2,NULL,NULL,?3)",
                rusqlite::params![old_ts, sid, l],
            )
            .unwrap();
        }
        for l in recent_lines {
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (?1,?2,NULL,NULL,?3)",
                rusqlite::params![recent_ts, sid, l],
            )
            .unwrap();
        }
        // total 체인 (prune 전 진실): GENESIS부터 5줄 누적
        let mut total_h = GENESIS;
        for l in old_lines.iter().chain(recent_lines.iter()) {
            total_h = hash_step(&total_h, l);
        }
        conn.execute(
            "INSERT INTO chains(surface_id,line_count,hash) VALUES (?1,5,?2)",
            rusqlite::params![sid, hex(&total_h)],
        )
        .unwrap();

        // maybe_prune 실행: last_prune을 6h+ 과거로 둬 debounce 게이트를 통과시킨다.
        let mut chains: std::collections::HashMap<u64, (u64, [u8; 32])> =
            std::collections::HashMap::new();
        let mut last_prune: Option<std::time::Instant> = None; // ★prune 발동(즉시 due·Instant 뺄셈 제거)
        {
            let _g = RETAIN_ENV_LOCK.lock().unwrap();
            let retain_env = crate::governance::ReapEnvGuard::set(&[("CYS_RECALL_RETAIN_DAYS", "30")]);
            maybe_prune(&conn, &mut chains, &mut last_prune);
            drop(retain_env);
        }

        // 1) 오래된 3줄은 lines에서 삭제, 최근 2줄만 남는다
        let remaining: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT line FROM lines WHERE surface_id=?1 ORDER BY id")
                .unwrap();
            stmt.query_map([sid], |r| r.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert_eq!(remaining, vec!["r0".to_string(), "r1".to_string()]);

        // 2) anchor_count=3, anchor_hash=앞 3줄 체인과 일치
        let (anchor_count, anchor_hash_s): (i64, String) = conn
            .query_row(
                "SELECT anchor_count, anchor_hash FROM chains WHERE surface_id=?1",
                [sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(anchor_count, 3);
        let mut expect_anchor = GENESIS;
        for l in old_lines {
            expect_anchor = hash_step(&expect_anchor, l);
        }
        assert_eq!(unhex(&anchor_hash_s), Some(expect_anchor));

        // 3) 핵심 불변식: 봉인 anchor부터 남은 2줄을 재계산하면 prune 전 total 해시와 동일.
        //    (즉 prune은 검증 가능한 진실을 보존한다 — attest가 prune 후에도 정확)
        let mut recomputed = expect_anchor;
        for l in &remaining {
            recomputed = hash_step(&recomputed, l);
        }
        assert_eq!(
            recomputed, total_h,
            "anchor + 남은 라인 재계산이 prune 전 total 체인과 일치해야 한다"
        );

        // 4) attest_verify 미러로 prune 후 검증: anchor 경계 너머는 거부, 전체 pin은 통과.
        // pin_count=2 (< anchor_count 3) → retention 창 밖 거부
        let beyond = verify_mirror(&conn, sid, &"0".repeat(64), 2);
        assert_eq!(beyond["match"].as_bool(), Some(false));
        assert_eq!(
            beyond["reason"].as_str(),
            Some("pin is beyond the retention window (pruned)")
        );
        // pin_count=5 (전체) → anchor(3) + 남은 2줄 재계산 = total → match
        let full = verify_mirror(&conn, sid, &hex(&total_h), 5);
        assert_eq!(full["match"].as_bool(), Some(true));
        assert_eq!(full["rows_used"].as_u64(), Some(2));

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// search()의 짧은-쿼리 LIKE 폴백(recall.rs:391)이 '%'·'_'·'\'를 리터럴로 escape하는지
    /// 실제 SQLite LIKE ... ESCAPE '\' 의미로 검증한다. escape가 빠지면 'a%'가 와일드카드로
    /// 새어 무관 라인이 검색되거나(정보 누출) 구문이 깨진다 — FTS 주입 차단 불변식.
    /// (search 본문의 escape 식 + LIKE 절을 1:1 미러링; <3자 쿼리 = trigram 미적용 경로)
    fn like_search(conn: &Connection, query: &str) -> Vec<String> {
        let like = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let mut stmt = conn
            .prepare("SELECT line FROM lines WHERE line LIKE ?1 ESCAPE '\\' ORDER BY id")
            .unwrap();
        stmt.query_map([like], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    #[test]
    fn search_like_fallback_treats_wildcards_as_literal() {
        let (path, conn) = temp_db(line!());
        let sid: i64 = 21;
        // 리터럴 '%'·'_'를 포함한 라인과, 그것이 와일드카드로 새면 잘못 매칭될 라인을 함께 넣는다.
        for l in [
            "progress 50% done", // '%' 리터럴 포함
            "progress 50X done", // '50%'가 와일드카드면 여기도 매칭됨 (오검출 후보)
            "a_b snake",         // '_' 리터럴 포함
            "axb other",         // 'a_b'가 와일드카드면 매칭됨 (오검출 후보)
            "back\\slash",       // '\' 리터럴 포함
        ] {
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (1.0,?1,NULL,NULL,?2)",
                rusqlite::params![sid, l],
            )
            .unwrap();
        }
        // '%'는 리터럴 — "50%"는 "50% done"만 매칭, "50X done"은 제외
        let r = like_search(&conn, "50%");
        assert_eq!(r, vec!["progress 50% done".to_string()]);
        // '_'는 리터럴 — "a_b"는 "a_b snake"만, "axb other"는 제외
        let r = like_search(&conn, "a_b");
        assert_eq!(r, vec!["a_b snake".to_string()]);
        // '\'는 리터럴 — "back\slash" 매칭 (escape 시퀀스로 깨지지 않음)
        let r = like_search(&conn, "back\\slash");
        assert_eq!(r, vec!["back\\slash".to_string()]);
        // 평범한 부분 문자열은 정상 매칭 (escape가 일반 검색을 망치지 않음)
        let r = like_search(&conn, "done");
        assert_eq!(r.len(), 2);

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// search()의 FTS 경로(recall.rs:383)가 쿼리를 phrase로 quoting하고 내부 '"'를
    /// 이중화(""")해 FTS5 MATCH 구문 주입을 차단하는지 실제 FTS5로 검증한다.
    /// escape가 빠지면 '"'가 포함된 쿼리가 FTS 구문으로 해석돼 에러나 의도치 않은 매칭을
    /// 일으킨다 — 3자+ 쿼리(trigram 적용)의 주입 차단 불변식.
    fn fts_search(conn: &Connection, query: &str) -> rusqlite::Result<Vec<String>> {
        let phrase = format!("\"{}\"", query.replace('"', "\"\""));
        let mut stmt = conn.prepare(
            "SELECT l.line FROM lines l JOIN lines_fts f ON l.id = f.rowid
             WHERE lines_fts MATCH ?1 ORDER BY l.id",
        )?;
        let rows = stmt
            .query_map([phrase], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    #[test]
    fn search_routing_threshold_short_queries_need_like_fallback() {
        // ★불변식 박제: search()의 use_fts = query.chars().count() >= 3 라우팅이 옳다.
        // 근거를 '실제 FTS5 trigram'으로 증명한다 — trigram 토크나이저는 3자 미만 쿼리에
        // 빈 결과를 내므로, 1~2자 쿼리를 FTS로 보내면 recall이 조용히 0건을 반환한다.
        // 따라서 데몬은 <3자를 반드시 LIKE로 폴백해야 한다. 이 테스트는 (a) FTS가
        // 2자에 빈 결과임을 확인하고 (b) 같은 2자가 LIKE로는 잡힘을 확인해 폴백 필요성을
        // 박제한다. (c) 라우팅 술어가 .len()(바이트)이 아니라 .chars()(문자)임도 박제.
        let (path, conn) = temp_db(line!());
        let sid: i64 = 31;
        for l in ["ok done", "hi there", "한글 출력 완료"] {
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (1.0,?1,NULL,NULL,?2)",
                rusqlite::params![sid, l],
            )
            .unwrap();
        }

        // (a) 2자 쿼리는 trigram FTS에서 빈 결과 — FTS 단독이면 recall 무음 실패
        let fts_2 = fts_search(&conn, "ok").expect("2자 FTS 쿼리는 에러 없이 빈 결과");
        assert!(fts_2.is_empty(), "trigram FTS는 2자에 0건: {fts_2:?}");
        // (b) 동일 2자가 LIKE 폴백으로는 정상 매칭 — 폴백이 무음 실패를 막는다
        let like_2 = like_search(&conn, "ok");
        assert_eq!(like_2, vec!["ok done".to_string()]);

        // 라우팅 술어를 search() 본문과 1:1 미러링 (use_fts 결정만 격리)
        let use_fts = |q: &str| q.chars().count() >= 3;
        // 경계: 2자=LIKE, 3자=FTS
        assert!(!use_fts("ok"), "2자는 LIKE 경로");
        assert!(use_fts("don"), "3자는 FTS 경로");
        assert!(!use_fts(""), "빈 쿼리도 LIKE 경로(FTS 미적용)");

        // (c) 멀티바이트: '한글'은 2 chars지만 6 bytes — .len()이면 6>=3로 FTS 오라우팅돼
        // trigram 빈 결과로 무음 실패한다. .chars().count()는 2라 올바르게 LIKE로 간다.
        assert_eq!("한글".len(), 6);
        assert_eq!("한글".chars().count(), 2);
        assert!(!use_fts("한글"), "2 chars 멀티바이트는 LIKE — .len()(6) 오라우팅 차단");
        // 그 LIKE 경로로 실제 매칭됨 (무음 실패 아님)
        let like_kr = like_search(&conn, "한글");
        assert_eq!(like_kr, vec!["한글 출력 완료".to_string()]);
        // 3 chars 멀티바이트는 FTS 경로 + trigram이 실제로 잡는다 (대조군)
        assert!(use_fts("출력완"));

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn search_fts_phrase_quoting_blocks_match_injection() {
        let (path, conn) = temp_db(line!());
        let sid: i64 = 23;
        for l in [
            r#"say "hello" now"#, // 리터럴 큰따옴표 포함 (3자+ trigram 대상)
            "say goodbye now",    // OR 주입이 성공하면 잘못 매칭될 후보
            "plain content here",
        ] {
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (1.0,?1,NULL,NULL,?2)",
                rusqlite::params![sid, l],
            )
            .unwrap();
        }
        // 내부 '"'가 이중화돼 phrase 리터럴로 매칭 — '"hello"'를 포함한 라인만 (에러 없이).
        let r = fts_search(&conn, r#""hello""#).expect("quoted query must not error");
        assert_eq!(r, vec![r#"say "hello" now"#.to_string()]);
        // FTS 구문 주입 시도("a" OR "b")도 phrase로 박제 → 그 리터럴이 없으면 빈 결과
        // (주입이 통하면 goodbye 라인이 새어나옴). 구문 에러 없이 안전히 비매칭이어야 한다.
        let r = fts_search(&conn, r#"hello" OR "goodbye"#)
            .expect("injection attempt must be quoted, not errored");
        assert!(
            !r.iter().any(|l| l.contains("goodbye")),
            "FTS 주입으로 goodbye가 새면 안 된다: {r:?}"
        );
        // 평범한 3자+ 쿼리는 정상 trigram 매칭
        let r = fts_search(&conn, "plain").expect("normal query ok");
        assert_eq!(r, vec!["plain content here".to_string()]);

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// maybe_prune의 debounce 게이트: 최근 6h 내에 이미 prune했으면 아무 것도 하지 않는다
    /// (오래된 라인이 있어도 삭제·봉인 없음 — 과도한 prune I/O 방지 불변식).
    #[test]
    fn maybe_prune_respects_six_hour_debounce() {
        let (path, conn) = temp_db(line!());
        let sid: i64 = 13;
        let old_ts = crate::state::now_epoch() - 100.0 * 86400.0;
        for l in ["x0", "x1"] {
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (?1,?2,NULL,NULL,?3)",
                rusqlite::params![old_ts, sid, l],
            )
            .unwrap();
        }
        let mut chains: std::collections::HashMap<u64, (u64, [u8; 32])> =
            std::collections::HashMap::new();
        // last_prune이 방금(now) → debounce 미경과 → 즉시 반환, 삭제 없음
        let mut last_prune: Option<std::time::Instant> = Some(std::time::Instant::now()); // ★최근 prune=미발동
        {
            let _g = RETAIN_ENV_LOCK.lock().unwrap();
            let retain_env = crate::governance::ReapEnvGuard::set(&[("CYS_RECALL_RETAIN_DAYS", "30")]);
            maybe_prune(&conn, &mut chains, &mut last_prune);
            drop(retain_env);
        }
        let cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM lines WHERE surface_id=?1", [sid], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(cnt, 2, "debounce 중에는 오래된 라인도 보존돼야 한다");

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// ★TOCTOU 회귀: attest_verify가 anchor와 lines를 같은 커넥션의 '분리된 autocommit
    /// statement' 두 번으로 읽으면(트랜잭션 미감싸기), 그 사이에 writer 스레드의 maybe_prune
    /// 커밋(prefix 삭제 + anchor 전진을 한 트랜잭션)이 끼어 prune '전' anchor와 prune '후'
    /// lines를 섞어 읽는다 → 정직한 pin을 'insufficient rows'로 거짓 거부한다.
    ///
    /// split_reads = 현재(버그) 시퀀스: read1(anchor) → [prune 끼어듦] → read2(lines).
    /// txn_reads  = 수정(BEGIN..COMMIT) 시퀀스: 두 읽기가 한 deferred 읽기 트랜잭션 = 단일 스냅샷.
    /// 같은 인터리빙에서 split은 거짓 거부, txn은 정직 통과해야 한다.
    #[test]
    fn attest_verify_split_reads_race_falsely_rejects_honest_pin() {
        let (path, mut conn) = temp_db(line!());
        let sid: i64 = 41;
        // 5줄을 삽입하고 total 체인(=정직한 pin)을 기록. anchor는 아직 0(미prune).
        let lines = ["p0", "p1", "p2", "p3", "p4"];
        let mut total_h = GENESIS;
        let old_ts = crate::state::now_epoch() - 100.0 * 86400.0; // 앞 3줄을 오래되게
        let recent_ts = crate::state::now_epoch();
        for (i, l) in lines.iter().enumerate() {
            let ts = if i < 3 { old_ts } else { recent_ts };
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (?1,?2,NULL,NULL,?3)",
                rusqlite::params![ts, sid, l],
            )
            .unwrap();
            total_h = hash_step(&total_h, l);
        }
        conn.execute(
            "INSERT INTO chains(surface_id,line_count,hash) VALUES (?1,5,?2)",
            rusqlite::params![sid, hex(&total_h)],
        )
        .unwrap();
        let pin_hash = hex(&total_h);
        let pin_count: u64 = 5;

        // 두 번째 커넥션(writer 역할) — 같은 DB에 별도 connection.
        let writer = open_db(&path).expect("second connection");

        // prune을 '실행하는' 클로저: 앞 3줄(오래된 prefix)을 anchor로 봉인+삭제(한 트랜잭션).
        let run_prune = |writer: &Connection| {
            let mut chains: std::collections::HashMap<u64, (u64, [u8; 32])> =
                std::collections::HashMap::new();
            let mut last_prune: Option<std::time::Instant> = None; // ★prune 발동(즉시 due·Instant 뺄셈 제거)
            let _g = RETAIN_ENV_LOCK.lock().unwrap();
            let retain_env = crate::governance::ReapEnvGuard::set(&[("CYS_RECALL_RETAIN_DAYS", "30")]);
            maybe_prune(writer, &mut chains, &mut last_prune);
            drop(retain_env);
        };

        // --- (A) split-reads(현재 버그 시퀀스): read1 → prune → read2 ---
        let split_result = {
            // read1: anchor 로드 (autocommit statement, 즉시 스냅샷 닫힘)
            let (anchor_count, anchor_hash) = conn
                .query_row(
                    "SELECT anchor_count, anchor_hash FROM chains WHERE surface_id=?1",
                    [sid],
                    |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?)),
                )
                .ok()
                .map(|(c, h)| (c, unhex(&h).unwrap_or(GENESIS)))
                .unwrap_or((0, GENESIS));
            // 두 읽기 사이에 prune 커밋이 끼어든다 (앞 3줄 삭제 + anchor=3 전진)
            run_prune(&writer);
            // read2: lines 로드 (새 스냅샷 — prune 후 상태를 본다)
            let need = pin_count.saturating_sub(anchor_count);
            let mut h = anchor_hash;
            let mut used: u64 = 0;
            {
                let mut stmt = conn
                    .prepare("SELECT line FROM lines WHERE surface_id=?1 ORDER BY id LIMIT ?2")
                    .unwrap();
                let rows = stmt
                    .query_map(rusqlite::params![sid, need as i64], |r| r.get::<_, String>(0))
                    .unwrap();
                for line in rows.filter_map(|r| r.ok()) {
                    h = hash_step(&h, &line);
                    used += 1;
                }
            }
            if used < need {
                ("insufficient", false)
            } else {
                ("computed", hex(&h) == pin_hash)
            }
        };
        // 버그 박제: split-reads는 정직한 pin을 거짓 거부한다 (insufficient rows).
        assert_eq!(
            split_result,
            ("insufficient", false),
            "split-reads는 prune 인터리빙에서 정직 pin을 거짓 거부해야 한다(버그 증명)"
        );

        // 상태 리셋: prune을 되돌려 같은 초기 조건에서 txn 경로를 검증한다.
        // (anchor=0, 5줄 모두 복원)
        conn.execute("DELETE FROM lines WHERE surface_id=?1", [sid])
            .unwrap();
        for (i, l) in lines.iter().enumerate() {
            let ts = if i < 3 { old_ts } else { recent_ts };
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (?1,?2,NULL,NULL,?3)",
                rusqlite::params![ts, sid, l],
            )
            .unwrap();
        }
        conn.execute(
            "UPDATE chains SET line_count=5, hash=?2, anchor_count=0, anchor_hash=?3 WHERE surface_id=?1",
            rusqlite::params![sid, hex(&total_h), hex(&GENESIS)],
        )
        .unwrap();

        // --- (B) txn-reads(수정 시퀀스): BEGIN(deferred) → read1 → prune → read2 → COMMIT ---
        // 읽기 트랜잭션이 첫 SELECT에서 스냅샷을 고정하므로, 그 사이 writer가 커밋해도
        // 같은 커넥션의 두 읽기는 동일 스냅샷(prune 전)을 본다 → 정직 pin 통과.
        let txn_result = {
            let tx = conn.transaction().unwrap();
            let (anchor_count, anchor_hash) = tx
                .query_row(
                    "SELECT anchor_count, anchor_hash FROM chains WHERE surface_id=?1",
                    [sid],
                    |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?)),
                )
                .ok()
                .map(|(c, h)| (c, unhex(&h).unwrap_or(GENESIS)))
                .unwrap_or((0, GENESIS));
            // 첫 읽기로 스냅샷 고정된 뒤 writer가 prune 커밋해도 tx는 못 본다
            run_prune(&writer);
            let need = pin_count.saturating_sub(anchor_count);
            let mut h = anchor_hash;
            let mut used: u64 = 0;
            {
                let mut stmt = tx
                    .prepare("SELECT line FROM lines WHERE surface_id=?1 ORDER BY id LIMIT ?2")
                    .unwrap();
                let rows = stmt
                    .query_map(rusqlite::params![sid, need as i64], |r| r.get::<_, String>(0))
                    .unwrap();
                for line in rows.filter_map(|r| r.ok()) {
                    h = hash_step(&h, &line);
                    used += 1;
                }
            }
            let out = if used < need {
                ("insufficient", false)
            } else {
                ("computed", hex(&h) == pin_hash)
            };
            let _ = tx.rollback();
            out
        };
        // 수정 박제: txn-reads는 같은 인터리빙에서도 정직 pin을 통과시킨다.
        assert_eq!(
            txn_result,
            ("computed", true),
            "txn-reads(읽기 트랜잭션)는 prune 인터리빙에서도 정직 pin을 통과시켜야 한다(수정 증명)"
        );

        drop(writer);
        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// RETAIN_DAYS=0(비활성)이면 아무리 오래된 라인도 prune하지 않는다 (보존 무한 모드).
    #[test]
    fn maybe_prune_disabled_when_retain_days_zero() {
        let (path, conn) = temp_db(line!());
        let sid: i64 = 17;
        let old_ts = crate::state::now_epoch() - 1000.0 * 86400.0;
        conn.execute(
            "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (?1,?2,NULL,NULL,'keep')",
            rusqlite::params![old_ts, sid],
        )
        .unwrap();
        let mut chains: std::collections::HashMap<u64, (u64, [u8; 32])> =
            std::collections::HashMap::new();
        let mut last_prune: Option<std::time::Instant> = None; // ★prune 발동(즉시 due·Instant 뺄셈 제거)
        {
            let _g = RETAIN_ENV_LOCK.lock().unwrap();
            let retain_env = crate::governance::ReapEnvGuard::set(&[("CYS_RECALL_RETAIN_DAYS", "0")]);
            maybe_prune(&conn, &mut chains, &mut last_prune);
            drop(retain_env);
        }
        let cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM lines WHERE surface_id=?1", [sid], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(cnt, 1, "RETAIN_DAYS=0이면 prune 비활성");

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    /// ★회귀: prune이 한 surface의 라인을 전부 삭제하면(전체가 보존창 밖) chains 행은
    /// 영구 잔존하지만 lines에서는 그 surface_id가 사라진다. max_surface_id가 lines만
    /// 보면 seed가 낮아져 재시작 후 새 surface가 옛 chains 행과 같은 surface_id를 재사용
    /// → 체인 오염·recall 병합(state.rs:323 주석의 불변식 위반). chains를 함께 봐야 한다.
    ///
    /// 이 테스트는 max_surface_id가 의존하는 쿼리를 프로덕션과 1:1로 미러링하되, 옛(lines만)
    /// 쿼리는 버그(낮은 seed)를, 수정(lines+chains MAX) 쿼리는 high-water mark 유지를 박제한다.
    #[test]
    fn max_surface_id_survives_full_surface_prune_via_chains() {
        let (path, conn) = temp_db(line!());

        // surface 50: 라인이 보존됨 (lines에 잔존) → max(lines)=50
        // surface 100: 전체 prune됨 → lines엔 없고 chains 행만 잔존 (anchor 봉인 상태)
        let kept_sid: i64 = 50;
        let pruned_sid: i64 = 100;

        // 보존 surface: lines + chains 모두 존재
        let mut h_keep = GENESIS;
        for l in ["k0", "k1"] {
            conn.execute(
                "INSERT INTO lines(ts,surface_id,role,title,line) VALUES (1.0,?1,NULL,NULL,?2)",
                rusqlite::params![kept_sid, l],
            )
            .unwrap();
            h_keep = hash_step(&h_keep, l);
        }
        conn.execute(
            "INSERT INTO chains(surface_id,line_count,hash) VALUES (?1,2,?2)",
            rusqlite::params![kept_sid, hex(&h_keep)],
        )
        .unwrap();

        // 전체 prune된 surface: lines에는 아무 행도 없고 chains 행만 anchor로 봉인되어 잔존.
        // (maybe_prune이 DELETE FROM lines만 하고 chains는 anchor만 UPDATE하므로 발생하는 상태)
        let mut anchor = GENESIS;
        for l in ["p0", "p1", "p2"] {
            anchor = hash_step(&anchor, l);
        }
        conn.execute(
            "INSERT INTO chains(surface_id,line_count,hash,anchor_count,anchor_hash)
             VALUES (?1,3,?2,3,?2)",
            rusqlite::params![pruned_sid, hex(&anchor)],
        )
        .unwrap();

        // ① 버그 재현: lines만 보는 옛 쿼리는 50을 반환 → seed=51 → chains 51..100과 충돌
        let lines_only: i64 = conn
            .query_row("SELECT COALESCE(MAX(surface_id), 0) FROM lines", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            lines_only, kept_sid,
            "lines만 보면 전체 prune된 surface 100을 놓쳐 seed가 낮아진다(버그 증명)"
        );

        // ② 수정 검증: lines+chains MAX는 전체 prune된 surface까지 보아 high-water mark 유지
        let lines_and_chains: i64 = conn
            .query_row(
                "SELECT MAX(m) FROM (
                   SELECT COALESCE(MAX(surface_id), 0) AS m FROM lines
                   UNION ALL
                   SELECT COALESCE(MAX(surface_id), 0) AS m FROM chains
                 )",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            lines_and_chains, pruned_sid,
            "lines+chains MAX는 전체 prune된 surface까지 보아 seed 충돌을 막아야 한다"
        );

        drop(conn);

        // ③ 실제 public 함수 경로 검증: max_surface_id는 같은 디렉터리의 transcripts.db를
        // 열어야 하므로, temp_db 파일을 state_dir 규약(socket 형제) 디렉터리의 transcripts.db로
        // 복사한 뒤 socket_path를 넘겨 end-to-end로 high-water mark가 100임을 박제한다.
        let e2e_dir = std::env::temp_dir().join(format!(
            "cys_recall_maxid_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::create_dir_all(&e2e_dir);
        let db_dst = e2e_dir.join("transcripts.db");
        let _ = std::fs::remove_file(&db_dst);
        std::fs::copy(&path, &db_dst).unwrap();
        let fake_socket = e2e_dir.join("cys.sock");
        assert_eq!(
            super::max_surface_id(&fake_socket),
            pruned_sid as u64,
            "max_surface_id는 전체 prune된 surface의 chains 행까지 보아 100을 반환해야 한다"
        );

        let _ = std::fs::remove_file(&db_dst);
        let _ = std::fs::remove_file(&path);
    }
}
