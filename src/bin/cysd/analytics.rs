//! T7 E1-3 영속 분석 저장소 — cysd 내장 SQLite(`analytics.db`). recall.rs(transcripts.db)와
//! 별개 DB. 휘발성 in-memory `Consumption`을 재시작에도 보존한다(부트 시 최근 12h usage_records를
//! ts 순으로 리플레이해 오늘 비용·토큰·모델믹스·스파크라인 재구성). rusqlite(이미 의존).
//! 실패는 graceful — open이 None이면 영속 없이 데몬은 정상 동작(배지·실시간은 in-memory로 유지).
//! 스키마는 설계(docs/CONTROL_CENTER_DESIGN.md §2) 전체를 미리 만든다(events·messages 등은 E1-④/E3에서 사용).

use crate::state::{state_dir, Consumption, Daemon};
use rusqlite::Connection;
use std::path::Path;

const SPARK_SPAN_SECS: f64 = 43_200.0; // 12h — 부트 리플레이 창

/// analytics.db 열고 스키마 보장. 실패 시 None(graceful degrade).
pub fn open(socket_path: &Path) -> Option<Connection> {
    let path = state_dir(socket_path).join("analytics.db");
    let conn = Connection::open(&path).ok()?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS usage_records(
            id INTEGER PRIMARY KEY,
            session_id TEXT, role TEXT, agent TEXT, model TEXT,
            input_tokens INTEGER, output_tokens INTEGER,
            cache_creation INTEGER, cache_read INTEGER,
            cost_usd REAL, ts REAL);
         CREATE INDEX IF NOT EXISTS ix_usage_ts ON usage_records(ts);
         CREATE INDEX IF NOT EXISTS ix_usage_session ON usage_records(session_id);
         CREATE TABLE IF NOT EXISTS sessions(
            session_id TEXT PRIMARY KEY, role TEXT, agent TEXT, cwd TEXT,
            started_at REAL, ended_at REAL, title TEXT, summary TEXT, turn_count INTEGER);
         CREATE TABLE IF NOT EXISTS events(
            id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, agent TEXT,
            event_type TEXT, tool_name TEXT, is_skill INTEGER, skill_name TEXT, is_slash INTEGER,
            is_agent INTEGER, agent_type TEXT, agent_id TEXT,
            exit_code INTEGER, duration_ms INTEGER, ts REAL);
         CREATE INDEX IF NOT EXISTS ix_ev_skill ON events(is_skill, ts);
         CREATE INDEX IF NOT EXISTS ix_ev_agent ON events(is_agent, ts);
         CREATE TABLE IF NOT EXISTS messages(
            id INTEGER PRIMARY KEY, session_id TEXT, seq INTEGER, role TEXT,
            content TEXT, tool_name TEXT, tool_use_id TEXT, duration_ms INTEGER, ts REAL);
         CREATE TABLE IF NOT EXISTS daily_rollups(
            date TEXT PRIMARY KEY, payload TEXT, computed_at REAL);
         CREATE TABLE IF NOT EXISTS stars(session_id TEXT PRIMARY KEY, note TEXT, starred_at REAL);
         -- A-4: tail 오프셋 영속 — 재시작 시 마지막 256KB 재파싱→usage_records 중복 INSERT
         -- (UNIQUE 부재라 영구 잔존·리플레이 복리 부풀림)를 원천 차단하는 정확 재개점.
         CREATE TABLE IF NOT EXISTS tail_offsets(
            session_file TEXT PRIMARY KEY, off INTEGER, updated REAL);
         -- T2-3: append-only change-log + 단조 revn(낙관적 동시성) — ADDITIVE.
         -- SESSION_STATE.md 산문 복원 경로는 불변(이건 그 위에 얹는 결정론 change-replay 능력).
         -- penpot files_update.clj(:184-190 revn-conflict / :176-182 vern-conflict / :409 revn inc)의
         -- 개념만 클린룸 차용 — Clojure 코드복사 0(MPL-2.0 파일전염 회피, 일반 event-sourcing 패턴).
         CREATE TABLE IF NOT EXISTS state_scope(
            scope       TEXT PRIMARY KEY,  -- 예: 'SESSION_STATE', 'MASTER_TODO'
            revn        INTEGER NOT NULL,  -- 단조 ordering(penpot revn)
            vern        INTEGER NOT NULL,  -- restore/branch 마커(penpot vern)
            updated_at  REAL NOT NULL);
         CREATE TABLE IF NOT EXISTS change_log(
            seq           INTEGER PRIMARY KEY,  -- 전역 단조
            scope         TEXT NOT NULL,
            revn          INTEGER NOT NULL,     -- 이 변경이 만든 revn
            vern          INTEGER NOT NULL,
            kind          TEXT NOT NULL,        -- 변경 종류(자유 라벨)
            payload       TEXT NOT NULL,        -- IR(JSON 등) — 형식 버전드
            attest_hash   TEXT NOT NULL,        -- prev∥payload 해시체인(recall hash_step 동형)
            ts            REAL NOT NULL);
         CREATE INDEX IF NOT EXISTS ix_cl_scope_revn ON change_log(scope, revn);
         -- CC v2 WS-A: 계정 rate limit 시계열(추세·소진예측·부트 예열). ADDITIVE — 구버전 무시.
         CREATE TABLE IF NOT EXISTS rate_snapshots(
            ts REAL, provider TEXT, account TEXT, label TEXT,
            win TEXT, used_pct REAL, resets_at REAL);
         CREATE INDEX IF NOT EXISTS ix_rate_snap ON rate_snapshots(provider, account, win, ts);
         -- CC v2 WS-B: 스킬보드 실행 생애주기(산출물+데드라인 결정론 — skillrun.rs 헤더).
         -- exit_note: timeout_no_artifact|daemon_restart 등 판정 사유.
         CREATE TABLE IF NOT EXISTS skill_runs(
            run_id TEXT PRIMARY KEY, name TEXT, label TEXT, started_at REAL,
            deadline REAL, status TEXT,
            exit_note TEXT, artifact_dir TEXT, ended_at REAL);
         CREATE INDEX IF NOT EXISTS ix_skill_runs_started ON skill_runs(started_at);",
    )
    .ok()?;
    // fix-values-1 SP-1: rate_snapshots 에 출처 열(additive). 신규·구 DB 모두 이 ALTER 한 길로 붙는다 — 이미 있으면
    // 에러(duplicate column)를 `let _` 로 흡수해 멱등(channels.rs outbound 마이그레이션과 같은 방식). 구 cysd 는
    // 열 이름을 지정해 INSERT/SELECT 하므로 이 열을 무시한다(NULL). 출처 NULL = 이 열 이전의 행.
    let _ = conn.execute("ALTER TABLE rate_snapshots ADD COLUMN source TEXT", []);
    Some(conn)
}

// ── CC v2 WS-A: rate_snapshots ──

/// 계정 rate 스냅샷 1행 적재 — 스로틀·prune 판단은 호출부(accounts.rs). 실패 무해.
/// `source` = 그 관측의 계정 출처 라벨(accounts `AccountView.source` 와 같은 어휘) — 부트 복원이 경보 입력을
/// 창 밖(표시용) 값 없이 복원하는 근거다.
pub fn record_rate_snapshot(
    conn: &Connection,
    ts: f64,
    provider: &str,
    account: &str,
    label: &str,
    win: &str,
    used_pct: f64,
    resets_at: Option<f64>,
    source: &str,
) {
    let _ = conn.execute(
        "INSERT INTO rate_snapshots(ts, provider, account, label, win, used_pct, resets_at, source)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        rusqlite::params![ts, provider, account, label, win, used_pct, resets_at, source],
    );
}

/// 보존 창 밖 스냅샷 prune.
pub fn prune_rate_snapshots(conn: &Connection, before_ts: f64) {
    let _ = conn.execute("DELETE FROM rate_snapshots WHERE ts < ?1", [before_ts]);
}

/// (provider,account,win)별 **최신** 스냅샷 — 부트 예열용. since 이후만.
/// `exclude_source` 가 있으면 그 출처의 행을 **빼고** 최신을 고른다(경보 입력 복원 — 창 밖 값 제외).
/// 출처가 NULL 인 구 행(출처 열 이전 · 창 밖 값이 없던 시절)은 빼지 않는다.
/// 반환: (ts, provider, account, label, win, used_pct, resets_at)
pub fn last_rate_snapshots(
    conn: &Connection,
    since: f64,
    exclude_source: Option<&str>,
) -> Vec<(f64, String, String, String, String, f64, Option<f64>)> {
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT ts, provider, account, label, win, used_pct, resets_at FROM rate_snapshots
         WHERE ts >= ?1 AND (?2 IS NULL OR source IS NULL OR source <> ?2)
         GROUP BY provider, account, win HAVING ts = MAX(ts)",
    ) else {
        return out;
    };
    let rows = stmt.query_map(rusqlite::params![since, exclude_source], |r| {
        Ok((
            r.get::<_, f64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, f64>(5)?,
            r.get::<_, Option<f64>>(6)?,
        ))
    });
    if let Ok(rows) = rows {
        for r in rows.flatten() {
            out.push(r);
        }
    }
    out
}

/// 계정×창 최근 시계열(소진 예측용) — ts 오름차순 (ts, used_pct).
pub fn rate_series(
    conn: &Connection,
    provider: &str,
    account: &str,
    win: &str,
    since: f64,
) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT ts, used_pct FROM rate_snapshots
         WHERE provider=?1 AND account=?2 AND win=?3 AND ts>=?4 ORDER BY ts",
    ) else {
        return out;
    };
    let rows = stmt.query_map(rusqlite::params![provider, account, win, since], |r| {
        Ok((r.get::<_, f64>(0)?, r.get::<_, f64>(1)?))
    });
    if let Ok(rows) = rows {
        for r in rows.flatten() {
            out.push(r);
        }
    }
    out
}

// ── CC v2 WS-B: skill_runs ──

pub fn skill_run_insert(
    conn: &Connection,
    run_id: &str,
    name: &str,
    label: &str,
    now: f64,
    deadline: f64,
) {
    let _ = conn.execute(
        "INSERT OR REPLACE INTO skill_runs(run_id, name, label, started_at, deadline, status)
         VALUES(?1,?2,?3,?4,?5,'launched')",
        rusqlite::params![run_id, name, label, now, deadline],
    );
}

pub fn skill_run_update(
    conn: &Connection,
    run_id: &str,
    status: &str,
    exit_note: Option<&str>,
    artifact_dir: Option<&str>,
    ended_at: Option<f64>,
) {
    let _ = conn.execute(
        "UPDATE skill_runs SET status=?2,
            exit_note=COALESCE(?3, exit_note),
            artifact_dir=COALESCE(?4, artifact_dir),
            ended_at=COALESCE(?5, ended_at)
         WHERE run_id=?1",
        rusqlite::params![run_id, status, exit_note, artifact_dir, ended_at],
    );
}

/// 열린 run(launched) — 워처 전이 대상. (run_id, deadline)
pub fn skill_runs_open(conn: &Connection) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT run_id, deadline FROM skill_runs WHERE status='launched'",
    ) else {
        return out;
    };
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
    });
    if let Ok(rows) = rows {
        for r in rows.flatten() {
            out.push(r);
        }
    }
    out
}

/// 부트 reconcile — 열린 run 일괄 failed(사유 기록). 일회용 pane은 데몬 재시작을 넘어 살지 않는다.
pub fn skill_runs_fail_open(conn: &Connection, note: &str, now: f64) {
    let _ = conn.execute(
        "UPDATE skill_runs SET status='failed', exit_note=?1, ended_at=?2
         WHERE status='launched'",
        rusqlite::params![note, now],
    );
}

/// 최근 run 목록(UI 카드) — started_at 내림차순.
pub fn skill_runs_list(conn: &Connection, limit: u64) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT run_id, name, label, started_at, deadline, status, exit_note, artifact_dir, ended_at
         FROM skill_runs ORDER BY started_at DESC LIMIT ?1",
    ) else {
        return out;
    };
    let rows = stmt.query_map([limit], |r| {
        Ok(serde_json::json!({
            "run_id": r.get::<_, String>(0)?,
            "name": r.get::<_, String>(1)?,
            "label": r.get::<_, String>(2)?,
            "started_at": r.get::<_, f64>(3)?,
            "deadline": r.get::<_, Option<f64>>(4)?,
            "status": r.get::<_, String>(5)?,
            "exit_note": r.get::<_, Option<String>>(6)?,
            "artifact_dir": r.get::<_, Option<String>>(7)?,
            "ended_at": r.get::<_, Option<f64>>(8)?,
        }))
    });
    if let Ok(rows) = rows {
        for r in rows.flatten() {
            out.push(r);
        }
    }
    out
}

// ── T2-3 append-only change-log + 단조 revn + 낙관적 동시성(restore-replay READER) ──
// ADDITIVE: 기존 SESSION_STATE.md 산문 복원은 손대지 않는다. 이건 그 위에 얹는,
// /clear 복원을 결정론 change-replay로 *추가* 재생할 수 있게 하는 능력층이다.

const CL_GENESIS: [u8; 32] = [0u8; 32];

/// change_log 해시체인 한 칸 — recall.rs hash_step과 동형(코드 재구현, 복사 아님).
fn change_hash_step(prev: &[u8; 32], payload: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(prev);
    h.update(payload.as_bytes());
    h.update(b"\n");
    h.finalize().into()
}

fn hex32(b: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for byte in b {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

/// append 결과 — penpot files_update.clj의 revn/vern 2차원 판정을 enum으로.
/// wire verb 노출(state.append)은 인-프로세스 편집기 배선(T2-1) 시점 — 현재는 능력층만 착륙.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendOutcome {
    /// 수용: 새 revn·vern·seq·해시체인 끝.
    Accepted { revn: u64, vern: u64, seq: u64, attest_hash: String },
    /// 낙관적 동시성 실패 — base_revn이 stored와 어긋남(다른 writer가 먼저 커밋).
    RevnConflict { stored_revn: u64 },
    /// restore-marker 충돌 — base_vern이 stored와 어긋남(다른 버전이 복원됨).
    VernConflict { stored_vern: u64 },
}

/// change_log 한 줄을 revn-check-and-append 한 트랜잭션으로 적재.
/// penpot의 Postgres MVCC 직렬화를 단일 cysd writer + 한 SQLite 트랜잭션으로 재현한다.
/// 낙관적 동시성: base_revn != stored_revn → RevnConflict, base_vern != stored_vern → VernConflict.
#[allow(dead_code)] // wire verb 노출은 T2-1 배선 시점 — 현재는 능력층 + 테스트만 소비.
pub fn change_log_append(
    conn: &mut Connection,
    scope: &str,
    base_revn: u64,
    base_vern: u64,
    kind: &str,
    payload: &str,
    ts: f64,
) -> rusqlite::Result<AppendOutcome> {
    let tx = conn.transaction()?; // BEGIN — check+append 원자성(인터리브 0)
    let (stored_revn, stored_vern): (u64, u64) = tx
        .query_row(
            "SELECT revn, vern FROM state_scope WHERE scope=?1",
            [scope],
            |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64)),
        )
        .unwrap_or((0, 0));
    // penpot :176-182 vern-conflict — restore/branch 마커 불일치(동시편집과 구별).
    if base_vern != stored_vern {
        return Ok(AppendOutcome::VernConflict { stored_vern });
    }
    // penpot :184-190 revn-conflict — base가 최신과 어긋나면 reject(단조 보장).
    if base_revn != stored_revn {
        return Ok(AppendOutcome::RevnConflict { stored_revn });
    }
    let new_revn = stored_revn + 1; // penpot :409 (update :revn inc)
    let prev = tx
        .query_row(
            "SELECT attest_hash FROM change_log WHERE scope=?1 ORDER BY seq DESC LIMIT 1",
            [scope],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|h| {
            let bytes = (0..32)
                .map(|i| u8::from_str_radix(h.get(i * 2..i * 2 + 2)?, 16).ok())
                .collect::<Option<Vec<u8>>>()?;
            let mut a = [0u8; 32];
            a.copy_from_slice(&bytes);
            Some(a)
        })
        .unwrap_or(CL_GENESIS);
    let attest = hex32(&change_hash_step(&prev, payload));
    tx.execute(
        "INSERT INTO change_log(scope, revn, vern, kind, payload, attest_hash, ts)
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![scope, new_revn as i64, stored_vern as i64, kind, payload, attest, ts],
    )?;
    let seq = tx.last_insert_rowid() as u64;
    tx.execute(
        "INSERT INTO state_scope(scope, revn, vern, updated_at) VALUES(?1,?2,?3,?4)
         ON CONFLICT(scope) DO UPDATE SET revn=?2, vern=?3, updated_at=?4",
        rusqlite::params![scope, new_revn as i64, stored_vern as i64, ts],
    )?;
    tx.commit()?;
    Ok(AppendOutcome::Accepted { revn: new_revn, vern: stored_vern, seq, attest_hash: attest })
}

/// restore-replay READER — /clear 복원을 결정론 재생으로.
/// FRESH WAL reader(stale connection 비사용)로 since_revn 이후 변경을 seq 순서대로 모은다.
/// 반환: (payloads in order, final_revn, vern, replayed_count). penpot :448-461 lagged-changes 동형.
#[allow(dead_code)] // /clear 복원 배선은 T2-1 시점 — 현재는 능력층 + 테스트만 소비.
pub fn change_log_restore(socket_path: &Path, scope: &str, since_revn: u64) -> Option<(Vec<String>, u64, u64, u64)> {
    let path = state_dir(socket_path).join("analytics.db");
    let conn = Connection::open(&path).ok()?; // FRESH reader — stale conn 안 잡음
    let (final_revn, vern): (u64, u64) = conn
        .query_row(
            "SELECT revn, vern FROM state_scope WHERE scope=?1",
            [scope],
            |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64)),
        )
        .unwrap_or((0, 0));
    let mut stmt = conn
        .prepare("SELECT payload FROM change_log WHERE scope=?1 AND revn>?2 ORDER BY seq ASC")
        .ok()?;
    let rows = stmt
        .query_map(rusqlite::params![scope, since_revn as i64], |row| row.get::<_, String>(0))
        .ok()?;
    let payloads: Vec<String> = rows.filter_map(|r| r.ok()).collect();
    let replayed = payloads.len() as u64;
    Some((payloads, final_revn, vern, replayed))
}

/// usage_record 1건 적재 — 수집기가 새 claude 메시지마다 호출. 실패는 무해히 무시.
#[allow(clippy::too_many_arguments)]
pub fn record_usage(
    conn: &Connection,
    session: &str,
    role: &str,
    agent: &str,
    model: &str,
    input: u64,
    output: u64,
    cache_creation: u64,
    cache_read: u64,
    cost: f64,
    ts: f64,
) {
    // D3: role(조직 단위 tier) 적재 — schema(:21)엔 role TEXT가 있으나 INSERT에서 누락돼 있었다.
    // 이것이 tier별 비용/재작업률 측정의 유일한 막힘이었다(by_tier eval baseline 전제).
    let _ = conn.execute(
        "INSERT INTO usage_records(session_id, role, agent, model, input_tokens, output_tokens, cache_creation, cache_read, cost_usd, ts)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        rusqlite::params![
            session, role, agent, model, input as i64, output as i64,
            cache_creation as i64, cache_read as i64, cost, ts
        ],
    );
}

/// 툴 호출의 파생 분류 — (is_skill, skill_name, is_agent, agent_type).
/// Skill 툴 → 스킬 호출, Task/Agent 툴 → 에이전트(서브에이전트) 호출. E3 스킬/에이전트 TOP의 키.
pub fn derive_tool(tool_name: &str, tool_input: &serde_json::Value) -> (bool, Option<String>, bool, Option<String>) {
    let is_skill = tool_name == "Skill";
    let skill_name = if is_skill {
        tool_input
            .get("skill")
            .or_else(|| tool_input.get("command"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim_start_matches('/').to_string())
    } else {
        None
    };
    let is_agent = tool_name == "Task" || tool_name == "Agent";
    let agent_type = if is_agent {
        tool_input.get("subagent_type").and_then(|v| v.as_str()).map(String::from)
    } else {
        None
    };
    (is_skill, skill_name, is_agent, agent_type)
}

/// events 테이블에 hook 이벤트 1건 적재 — usage.event RPC가 호출. 실패는 무해히 무시.
#[allow(clippy::too_many_arguments)]
pub fn record_event(
    conn: &Connection,
    session: &str,
    role: &str,
    agent: &str,
    event_type: &str,
    tool_name: &str,
    is_skill: bool,
    skill_name: Option<&str>,
    is_agent: bool,
    agent_type: Option<&str>,
    agent_id: Option<&str>,
    exit_code: Option<i64>,
    duration_ms: Option<i64>,
    ts: f64,
) {
    let _ = conn.execute(
        "INSERT INTO events(session_id, role, agent, event_type, tool_name, is_skill, skill_name,
            is_slash, is_agent, agent_type, agent_id, exit_code, duration_ms, ts)
         VALUES(?1,?2,?3,?4,?5,?6,?7,0,?8,?9,?10,?11,?12,?13)",
        rusqlite::params![
            session, role, agent, event_type, tool_name,
            is_skill as i64, skill_name, is_agent as i64, agent_type, agent_id, exit_code,
            duration_ms, ts
        ],
    );
}

/// (session, model, input_tokens, cache_creation, output, cost, ts)
type UsageRow = (String, String, u64, u64, u64, f64, f64);

/// cutoff 이후 usage_records를 ts 오름차순으로 읽는다(부트 리플레이용).
fn load_recent(conn: &Connection, cutoff: f64) -> Vec<UsageRow> {
    let mut stmt = match conn.prepare(
        "SELECT session_id, model, input_tokens, cache_creation, output_tokens, cost_usd, ts
         FROM usage_records WHERE ts >= ?1 ORDER BY ts ASC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![cutoff], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)? as u64,
            r.get::<_, i64>(3)? as u64,
            r.get::<_, i64>(4)? as u64,
            r.get::<_, f64>(5)?,
            r.get::<_, f64>(6)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// epoch초 → 로컬 "YYYY-MM-DD" (record_message의 날짜 리셋 키).
fn date_of(ts: f64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts as i64, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// 리플레이: 행 목록을 ts 순으로 record_message에 흘려 Consumption을 재구성(순수 — 테스트 가능).
fn replay(rows: &[UsageRow], c: &mut Consumption) {
    for (session, model, input_tokens, cache_creation, output, cost, ts) in rows {
        // 소비 input = input_tokens + cache_creation (수집기 적재와 동일 의미).
        c.record_message(session, input_tokens + cache_creation, *output, *cost, model, *ts, &date_of(*ts));
    }
}

/// tail 재개 오프셋 조회 — 없으면 None(호출부가 EOF−256KB 폴백).
pub fn load_offset(conn: &Connection, file: &str) -> Option<u64> {
    conn.query_row("SELECT off FROM tail_offsets WHERE session_file=?1", [file], |r| r.get::<_, i64>(0))
        .ok()
        .map(|v| v.max(0) as u64)
}

/// tail 오프셋 영속 — 소비 적재가 완료된 지점까지 기록(실패는 무해).
pub fn save_offset(conn: &Connection, file: &str, off: u64, now: f64) {
    let _ = conn.execute(
        "INSERT INTO tail_offsets(session_file, off, updated) VALUES(?1,?2,?3)
         ON CONFLICT(session_file) DO UPDATE SET off=?2, updated=?3",
        rusqlite::params![file, off as i64, now],
    );
}

/// 부트 시 1회 — usage_records로 in-memory Consumption을 재구성한다.
/// 창 = 로컬 자정과 12h 전 중 더 이른 쪽 — 구 12h 고정은 늦은 시각 재시작 때
/// 그날 오전분을 누락시켜 대시보드 today를 과소시켰다(전수조사 B-2 교정).
pub fn seed_consumption(daemon: &Daemon) {
    let now = crate::state::now_epoch();
    let since = window_since(now, "today").min(now - SPARK_SPAN_SECS);
    let rows = {
        let guard = daemon.analytics.lock().unwrap();
        match guard.as_ref() {
            Some(conn) => load_recent(conn, since),
            None => return,
        }
    };
    if rows.is_empty() {
        return;
    }
    let mut c = daemon.consumption.lock().unwrap();
    replay(&rows, &mut c);
}

// ───────────────────────── E2 비용·효율 집계 (control.analytics) ─────────────────────────

/// (agent, role, model, input, output, cache_creation, cache_read, cost, session, ts)
/// role(idx1)은 D3 by_tier 집계 키 — SELECT 컬럼 순서를 이 튜플 인덱스와 일치시킬 것.
pub type SummaryRow = (String, String, String, u64, u64, u64, u64, f64, String, f64);

/// window 문자열 → cutoff epoch. "today"(기본·로컬 자정)·"7d"·"all".
pub fn window_since(now: f64, window: &str) -> f64 {
    use chrono::{Local, TimeZone};
    match window {
        "all" => 0.0,
        "7d" => (now - 7.0 * 86_400.0).max(0.0),
        _ => Local
            .timestamp_opt(now as i64, 0)
            .single()
            .map(|dt| dt.date_naive())
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|naive| Local.from_local_datetime(&naive).single())
            .map(|dt| dt.timestamp() as f64)
            .unwrap_or((now - 86_400.0).max(0.0)),
    }
}

/// since 이후 usage_records 전 행(집계용 — agent·cache_read 포함). 실패는 빈 벡터.
fn load_summary_rows(conn: &Connection, since: f64) -> Vec<SummaryRow> {
    let mut stmt = match conn.prepare(
        "SELECT agent, role, model, input_tokens, output_tokens, cache_creation, cache_read, cost_usd, session_id, ts
         FROM usage_records WHERE ts >= ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![since], |r| {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),  // agent
            r.get::<_, String>(1).unwrap_or_default(),  // role (idx1)
            r.get::<_, String>(2).unwrap_or_default(),  // model
            r.get::<_, i64>(3)? as u64,
            r.get::<_, i64>(4)? as u64,
            r.get::<_, i64>(5)? as u64,
            r.get::<_, i64>(6)? as u64,
            r.get::<_, f64>(7)?,
            r.get::<_, String>(8).unwrap_or_default(),  // session
            r.get::<_, f64>(9)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// 행 목록을 토큰 4분해·모델믹스·에이전트믹스·캐시절감$·생산성으로 롤업(순수 — 테스트 가능).
/// 캐시절감$ = Σ cache_read × (input단가 − cache_read단가) — 풀 input 대비 컨텍스트 재사용 할인액.
pub fn summarize(rows: &[SummaryRow]) -> serde_json::Value {
    use serde_json::{json, Value};
    use std::collections::HashMap;
    let (mut t_in, mut t_out, mut t_cc, mut t_cr) = (0u64, 0u64, 0u64, 0u64);
    let (mut t_cost, mut savings) = (0.0f64, 0.0f64);
    let mut models: HashMap<String, [f64; 6]> = HashMap::new(); // [in,out,cc,cr,cost,msgs]
    let mut agents: HashMap<String, [f64; 3]> = HashMap::new(); // [tokens,cost,msgs]
    let mut tiers: HashMap<String, [f64; 3]> = HashMap::new(); // role(조직 tier)별 [tokens,cost,msgs] (D3)
    let mut sessions: HashMap<String, (f64, f64, u64, u64, f64)> = HashMap::new(); // (min_ts,max_ts,msgs,tokens,cost)
    for (agent, role, model, input, output, cc, cr, cost, session, ts) in rows {
        t_in += input;
        t_out += output;
        t_cc += cc;
        t_cr += cr;
        t_cost += cost;
        let p = crate::cost::pricing_for(model);
        savings += (*cr as f64 / 1_000_000.0) * (p.input_per_m - p.cache_read_per_m);
        let tokens = input + output + cc + cr;
        let m = models.entry(model.clone()).or_insert([0.0; 6]);
        m[0] += *input as f64;
        m[1] += *output as f64;
        m[2] += *cc as f64;
        m[3] += *cr as f64;
        m[4] += *cost;
        m[5] += 1.0;
        let akey = if agent.is_empty() { "unknown".to_string() } else { agent.clone() };
        let a = agents.entry(akey).or_insert([0.0; 3]);
        a[0] += tokens as f64;
        a[1] += *cost;
        a[2] += 1.0;
        // D3 by_tier: role(조직 단위 tier)별 집계. 빈 role → "unattributed"(by_agent의 "unknown" 동형).
        let tkey = if role.is_empty() { "unattributed".to_string() } else { role.clone() };
        let tr = tiers.entry(tkey).or_insert([0.0; 3]);
        tr[0] += tokens as f64;
        tr[1] += *cost;
        tr[2] += 1.0;
        let s = sessions.entry(session.clone()).or_insert((*ts, *ts, 0, 0, 0.0));
        s.0 = s.0.min(*ts);
        s.1 = s.1.max(*ts);
        s.2 += 1;
        s.3 += tokens;
        s.4 += *cost;
    }
    let msgs = rows.len() as f64;
    let nsess = sessions.len() as f64;
    let tokens_total = (t_in + t_out + t_cc + t_cr) as f64;
    let dur_sum: f64 = sessions.values().map(|(mn, mx, ..)| mx - mn).sum();
    let div = |num: f64, den: f64| if den > 0.0 { num / den } else { 0.0 };
    let mut by_model: Vec<Value> = models
        .into_iter()
        .map(|(model, v)| {
            json!({
                "model": model, "input": v[0] as u64, "output": v[1] as u64,
                "cache_creation": v[2] as u64, "cache_read": v[3] as u64,
                "tokens": (v[0] + v[1] + v[2] + v[3]) as u64, "cost_usd": round4(v[4]), "msgs": v[5] as u64,
                // B-4: 단가표 미적중 = DEFAULT(Sonnet) 폴백 사용 — UI가 "단가 미상" 표시
                "pricing_known": crate::cost::has_pricing(&model),
            })
        })
        .collect();
    by_model.sort_by(|a, b| {
        b["cost_usd"].as_f64().unwrap_or(0.0)
            .partial_cmp(&a["cost_usd"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a["model"].as_str().unwrap_or("").cmp(b["model"].as_str().unwrap_or("")))
    });
    let mut by_agent: Vec<Value> = agents
        .into_iter()
        .map(|(agent, v)| json!({"agent": agent, "tokens": v[0] as u64, "cost_usd": round4(v[1]), "msgs": v[2] as u64}))
        .collect();
    by_agent.sort_by(|a, b| {
        b["tokens"].as_u64().unwrap_or(0).cmp(&a["tokens"].as_u64().unwrap_or(0))
            .then_with(|| a["agent"].as_str().unwrap_or("").cmp(b["agent"].as_str().unwrap_or("")))
    });
    // D3 by_tier: cost desc, 동률은 tier asc (결정론 — by_model 패턴 동형)
    let mut by_tier: Vec<Value> = tiers
        .into_iter()
        .map(|(tier, v)| json!({
            "tier": tier, "tokens": v[0] as u64, "cost_usd": round4(v[1]), "msgs": v[2] as u64,
            "cost_per_msg": round4(div(v[1], v[2])),
        }))
        .collect();
    by_tier.sort_by(|a, b| {
        b["cost_usd"].as_f64().unwrap_or(0.0)
            .partial_cmp(&a["cost_usd"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a["tier"].as_str().unwrap_or("").cmp(b["tier"].as_str().unwrap_or("")))
    });
    // A-3: cache_roi_x 폐기 — savings/full = (입력단가−캐시단가)/입력단가로, 클로드 전 모델의
    // 캐시단가가 입력의 정확히 10%라 사용 패턴과 무관하게 항상 0.9가 나오는 무정보 지표였다.
    // 비용류는 round4/round2로 부동소수 원값 노출(0.9000000000000045류)도 함께 차단.
    json!({
        "totals": {
            "input": t_in, "output": t_out, "cache_creation": t_cc, "cache_read": t_cr,
            "tokens": tokens_total as u64, "cost_usd": round4(t_cost), "msgs": msgs as u64, "sessions": nsess as u64,
        },
        "cache_savings_usd": round4(savings),
        "cache_efficiency": round4(div(t_cr as f64, (t_in + t_cc + t_cr) as f64)),
        "by_model": by_model,
        "by_agent": by_agent,
        "by_tier": by_tier,
        "productivity": {
            "turns_per_session": round2(div(msgs, nsess)),
            "tokens_per_turn": round2(div(tokens_total, msgs)),
            "cost_per_session": round4(div(t_cost, nsess)),
            "avg_session_duration_secs": round2(div(dur_sum, nsess)),
        },
    })
}

/// 부동소수 노출 차단용 반올림(표시 정밀도) — 소액 비용 보존을 위해 비용은 4자리.
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// control.analytics 본체 — since 이후 usage_records를 롤업. conn 없으면 호출부가 빈 summarize 사용.
pub fn analytics_summary(conn: &Connection, since: f64) -> serde_json::Value {
    summarize(&load_summary_rows(conn, since))
}

// ───────────────────────── E3 스킬·에이전트 집계 (control.skills) ─────────────────────────

/// (event_type, role, tool_name, is_skill, skill_name, is_agent, agent_type, exit_code, ts, duration_ms)
pub type EventRow = (String, String, String, bool, String, bool, String, Option<i64>, f64, Option<i64>);

/// since 이후 툴 events 전 행. 실패는 빈 벡터.
fn load_event_rows(conn: &Connection, since: f64) -> Vec<EventRow> {
    let mut stmt = match conn.prepare(
        "SELECT event_type, role, tool_name, is_skill, skill_name, is_agent, agent_type, exit_code, ts, duration_ms
         FROM events WHERE ts >= ?1 AND event_type IN ('PRE_TOOL','POST_TOOL')",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![since], |r| {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
            r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
            r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(7)?,
            r.get::<_, f64>(8)?,
            r.get::<_, Option<i64>>(9)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// 이벤트 행을 스킬·툴·에이전트 호출 TOP과 🔥실패율·p50 실행시간으로 롤업(순수 — 테스트 가능).
/// calls = PRE_TOOL(호출 시도) 기준 · fail = POST_TOOL exit_code≠0 기준(둘은 PRE/POST 쌍으로 근사 정합).
/// duration_ms는 데몬 PRE→POST 페어링 산출값(B-9) — 미축적 행은 null이라 p50은 축적분부터 반영.
pub fn summarize_skills(rows: &[EventRow]) -> serde_json::Value {
    use serde_json::{json, Value};
    use std::collections::HashMap;
    // name → [calls, fail]; roles는 별도 맵
    let mut tools: HashMap<String, [u64; 2]> = HashMap::new();
    let mut skills: HashMap<String, [u64; 2]> = HashMap::new();
    let mut skill_roles: HashMap<String, HashMap<String, u64>> = HashMap::new();
    let mut agents: HashMap<String, u64> = HashMap::new();
    let mut agent_roles: HashMap<String, HashMap<String, u64>> = HashMap::new();
    let mut tool_durs: HashMap<String, Vec<i64>> = HashMap::new();
    let (mut tool_calls, mut skill_calls, mut agent_calls, mut fail_calls) = (0u64, 0u64, 0u64, 0u64);
    for (etype, role, tool, is_skill, skill, is_agent, atype, exit, _ts, dur) in rows {
        let role_key = if role.is_empty() { "?".to_string() } else { role.clone() };
        if etype == "PRE_TOOL" {
            if !tool.is_empty() {
                tools.entry(tool.clone()).or_insert([0, 0])[0] += 1;
                tool_calls += 1;
            }
            if *is_skill && !skill.is_empty() {
                skills.entry(skill.clone()).or_insert([0, 0])[0] += 1;
                *skill_roles.entry(skill.clone()).or_default().entry(role_key.clone()).or_insert(0) += 1;
                skill_calls += 1;
            }
            if *is_agent && !atype.is_empty() {
                *agents.entry(atype.clone()).or_insert(0) += 1;
                *agent_roles.entry(atype.clone()).or_default().entry(role_key).or_insert(0) += 1;
                agent_calls += 1;
            }
        } else if etype == "POST_TOOL" {
            if let Some(d) = dur {
                if !tool.is_empty() && *d >= 0 {
                    tool_durs.entry(tool.clone()).or_default().push(*d);
                }
            }
            if matches!(exit, Some(c) if *c != 0) {
                if !tool.is_empty() {
                    tools.entry(tool.clone()).or_insert([0, 0])[1] += 1;
                }
                if *is_skill && !skill.is_empty() {
                    skills.entry(skill.clone()).or_insert([0, 0])[1] += 1;
                }
                fail_calls += 1;
            }
        }
    }
    let p50 = |name: &str| -> Value {
        match tool_durs.get(name) {
            Some(v) if !v.is_empty() => {
                let mut s = v.clone();
                s.sort_unstable();
                json!(s[s.len() / 2])
            }
            _ => Value::Null,
        }
    };
    let rate = |fail: u64, calls: u64| if calls > 0 { fail as f64 / calls as f64 } else { 0.0 };
    let roles_val = |m: Option<&HashMap<String, u64>>| -> Value {
        match m {
            Some(rm) => {
                let mut v: Vec<Value> = rm.iter().map(|(r, c)| json!({"role": r, "count": c})).collect();
                v.sort_by(|a, b| b["count"].as_u64().unwrap_or(0).cmp(&a["count"].as_u64().unwrap_or(0)));
                Value::Array(v)
            }
            None => Value::Array(vec![]),
        }
    };
    // calls desc, 동률은 이름 asc (결정론)
    let sort_by_calls = |list: &mut Vec<Value>| {
        list.sort_by(|a, b| {
            b["calls"].as_u64().unwrap_or(0).cmp(&a["calls"].as_u64().unwrap_or(0))
                .then_with(|| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")))
        });
    };
    let mut by_skill: Vec<Value> = skills
        .iter()
        .map(|(name, v)| json!({
            "name": name, "calls": v[0], "fail": v[1], "fail_rate": rate(v[1], v[0]),
            "roles": roles_val(skill_roles.get(name)),
        }))
        .collect();
    sort_by_calls(&mut by_skill);
    let mut by_tool: Vec<Value> = tools
        .iter()
        .map(|(name, v)| json!({"name": name, "calls": v[0], "fail": v[1], "fail_rate": rate(v[1], v[0]), "p50_ms": p50(name)}))
        .collect();
    sort_by_calls(&mut by_tool);
    let mut by_agent: Vec<Value> = agents
        .iter()
        .map(|(name, c)| json!({"name": name, "calls": c, "by_role": roles_val(agent_roles.get(name))}))
        .collect();
    sort_by_calls(&mut by_agent);
    // 🔥반복실패 TOP — 툴 단위 fail>0, fail desc
    let mut failures: Vec<Value> = tools
        .iter()
        .filter(|(_, v)| v[1] > 0)
        .map(|(name, v)| json!({"name": name, "calls": v[0], "fail": v[1], "fail_rate": rate(v[1], v[0])}))
        .collect();
    failures.sort_by(|a, b| {
        b["fail"].as_u64().unwrap_or(0).cmp(&a["fail"].as_u64().unwrap_or(0))
            .then_with(|| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")))
    });
    json!({
        "totals": {
            "tool_calls": tool_calls, "skill_calls": skill_calls,
            "agent_calls": agent_calls, "fail_calls": fail_calls, "fail_rate": rate(fail_calls, tool_calls),
        },
        "by_skill": by_skill,
        "by_tool": by_tool,
        "by_agent": by_agent,
        "failures": failures,
    })
}

/// control.skills 본체 — since 이후 events를 롤업. conn 없으면 호출부가 빈 summarize_skills 사용.
pub fn skills_summary(conn: &Connection, since: f64) -> serde_json::Value {
    summarize_skills(&load_event_rows(conn, since))
}

// ───────────────────────── E4 세션 타임라인 (control.sessions / session_detail) ─────────────────────────

const RIBBON_BUCKETS: usize = 24; // 활동 리본 칸 수

/// 세션 집계용 usage 행 — (session, agent, tokens(4분해합), cost, ts)
type SessUsageRow = (String, String, u64, f64, f64);
/// 세션 집계용 event 행 — (session, role, tool, is_skill, skill_name, is_agent, exit_code, event_type, ts)
type SessEventRow = (String, String, String, bool, String, bool, Option<i64>, String, f64);

fn load_session_usage(conn: &Connection, since: f64) -> Vec<SessUsageRow> {
    let mut stmt = match conn.prepare(
        "SELECT session_id, agent, input_tokens, output_tokens, cache_creation, cache_read, cost_usd, ts
         FROM usage_records WHERE ts >= ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![since], |r| {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            (r.get::<_, i64>(2)? + r.get::<_, i64>(3)? + r.get::<_, i64>(4)? + r.get::<_, i64>(5)?) as u64,
            r.get::<_, f64>(6)?,
            r.get::<_, f64>(7)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

fn load_session_events(conn: &Connection, since: f64) -> Vec<SessEventRow> {
    let mut stmt = match conn.prepare(
        "SELECT session_id, role, tool_name, is_skill, skill_name, is_agent, exit_code, event_type, ts
         FROM events WHERE ts >= ?1 AND event_type IN ('PRE_TOOL','POST_TOOL')",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![since], |r| -> rusqlite::Result<SessEventRow> {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
            r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, String>(7).unwrap_or_default(),
            r.get::<_, f64>(8)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// 활동 리본 — [start,end]를 buckets칸으로 나눠 각 칸의 활동 수를 센다(순수).
fn ribbon(ts_list: &[f64], start: f64, end: f64, buckets: usize) -> Vec<u64> {
    let mut out = vec![0u64; buckets];
    let span = (end - start).max(1e-9);
    for &t in ts_list {
        let mut idx = (((t - start) / span) * buckets as f64) as isize;
        if idx < 0 {
            idx = 0;
        }
        if idx as usize >= buckets {
            idx = buckets as isize - 1;
        }
        out[idx as usize] += 1;
    }
    out
}

/// usage+event 행을 세션 단위로 병합·요약(순수·테스트). ended_at 내림차순(최신 먼저).
pub fn summarize_sessions(
    usage: &[SessUsageRow],
    events: &[SessEventRow],
    starred: &std::collections::HashSet<String>,
) -> serde_json::Value {
    use serde_json::json;
    use std::collections::HashMap;
    struct Agg {
        agent: String,
        role: String,
        tokens: u64,
        cost: f64,
        msgs: u64,
        tool_calls: u64,
        skill_calls: u64,
        agent_calls: u64,
        fail_calls: u64,
        min_ts: f64,
        max_ts: f64,
        ts_list: Vec<f64>,
        skills: HashMap<String, u64>,
    }
    let mut m: HashMap<String, Agg> = HashMap::new();
    let ensure = |m: &mut HashMap<String, Agg>, sid: &str, ts: f64| {
        m.entry(sid.to_string()).or_insert_with(|| Agg {
            agent: String::new(), role: String::new(), tokens: 0, cost: 0.0, msgs: 0,
            tool_calls: 0, skill_calls: 0, agent_calls: 0, fail_calls: 0,
            min_ts: ts, max_ts: ts, ts_list: Vec::new(), skills: HashMap::new(),
        });
    };
    for (sid, agent, tokens, cost, ts) in usage {
        ensure(&mut m, sid, *ts);
        let a = m.get_mut(sid).unwrap();
        if a.agent.is_empty() && !agent.is_empty() {
            a.agent = agent.clone();
        }
        a.tokens += tokens;
        a.cost += cost;
        a.msgs += 1;
        a.min_ts = a.min_ts.min(*ts);
        a.max_ts = a.max_ts.max(*ts);
        a.ts_list.push(*ts);
    }
    for (sid, role, _tool, is_skill, skill, is_agent, exit, etype, ts) in events {
        ensure(&mut m, sid, *ts);
        let a = m.get_mut(sid).unwrap();
        if a.role.is_empty() && !role.is_empty() {
            a.role = role.clone();
        }
        a.min_ts = a.min_ts.min(*ts);
        a.max_ts = a.max_ts.max(*ts);
        // PRE_TOOL/POST_TOOL 둘 다 ts_list엔 활동으로 — 리본은 활동 밀도이므로 무방.
        a.ts_list.push(*ts);
        // 호출 수는 PRE_TOOL(실제 호출 시도)만 — POST 중복 카운트 방지(control.skills와 일관).
        if etype == "PRE_TOOL" {
            a.tool_calls += 1;
            if *is_skill {
                a.skill_calls += 1;
                if !skill.is_empty() {
                    *a.skills.entry(skill.clone()).or_insert(0) += 1;
                }
            }
            if *is_agent {
                a.agent_calls += 1;
            }
        }
        // 실패는 POST_TOOL exit_code≠0.
        if matches!(exit, Some(c) if *c != 0) {
            a.fail_calls += 1;
        }
    }
    let mut sessions: Vec<serde_json::Value> = m
        .into_iter()
        .map(|(sid, a)| {
            let top_skill = a
                .skills
                .iter()
                .max_by(|x, y| x.1.cmp(y.1).then_with(|| y.0.cmp(x.0)))
                .map(|(k, _)| k.clone());
            json!({
                "session_id": sid,
                "agent": a.agent,
                "role": a.role,
                "started_at": a.min_ts,
                "ended_at": a.max_ts,
                "duration_secs": (a.max_ts - a.min_ts).max(0.0),
                "msgs": a.msgs,
                "tokens": a.tokens,
                "cost_usd": a.cost,
                "tool_activity": a.tool_calls,
                "skill_calls": a.skill_calls,
                "agent_calls": a.agent_calls,
                "fail_calls": a.fail_calls,
                "top_skill": top_skill,
                "ribbon": ribbon(&a.ts_list, a.min_ts, a.max_ts, RIBBON_BUCKETS),
                "starred": starred.contains(&sid),
            })
        })
        .collect();
    sessions.sort_by(|a, b| {
        b["ended_at"].as_f64().unwrap_or(0.0)
            .partial_cmp(&a["ended_at"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a["session_id"].as_str().unwrap_or("").cmp(b["session_id"].as_str().unwrap_or("")))
    });
    json!({ "sessions": sessions })
}

/// ⭐ 즐겨찾기 세션 집합.
pub fn starred_set(conn: &Connection) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    if let Ok(mut stmt) = conn.prepare("SELECT session_id FROM stars") {
        if let Ok(it) = stmt.query_map([], |r| r.get::<_, String>(0)) {
            out.extend(it.filter_map(|x| x.ok()));
        }
    }
    out
}

/// ⭐ 노트 맵 — session_id → note (B-8: note가 write-only였던 절반 구현을 완결).
pub fn starred_notes(conn: &Connection) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    if let Ok(mut stmt) = conn.prepare("SELECT session_id, note FROM stars WHERE note != ''") {
        if let Ok(it) =
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        {
            out.extend(it.filter_map(|x| x.ok()));
        }
    }
    out
}

/// ⭐ 토글 — starred=true면 upsert, false면 삭제. 실패는 무해히 무시.
/// 재스타 시 starred_at도 갱신한다(B-8: 구 구현은 최초 시각에 고정).
pub fn set_star(conn: &Connection, session_id: &str, starred: bool, note: &str, ts: f64) {
    if starred {
        let _ = conn.execute(
            "INSERT INTO stars(session_id, note, starred_at) VALUES(?1,?2,?3)
             ON CONFLICT(session_id) DO UPDATE SET note=?2, starred_at=?3",
            rusqlite::params![session_id, note, ts],
        );
    } else {
        let _ = conn.execute("DELETE FROM stars WHERE session_id = ?1", rusqlite::params![session_id]);
    }
}

/// control.sessions 본체 — since 이후 세션 목록 (+⭐노트 부착).
pub fn session_list(conn: &Connection, since: f64) -> serde_json::Value {
    let usage = load_session_usage(conn, since);
    let events = load_session_events(conn, since);
    let mut out = summarize_sessions(&usage, &events, &starred_set(conn));
    let notes = starred_notes(conn);
    if !notes.is_empty() {
        if let Some(list) = out.get_mut("sessions").and_then(|v| v.as_array_mut()) {
            for row in list {
                let sid = row["session_id"].as_str().unwrap_or("").to_string();
                if let Some(n) = notes.get(&sid) {
                    row["star_note"] = serde_json::json!(n);
                }
            }
        }
    }
    out
}

/// control.session_detail 본체 — 단일 세션의 이벤트 타임라인 + 토큰/비용/모델 요약.
/// ★전사 원문(HUMAN/ASSISTANT/TOOL 콘텐츠)은 미수집(messages 테이블 미적재) — 이벤트 타임라인으로 대체.
pub fn session_detail(conn: &Connection, session_id: &str) -> serde_json::Value {
    use serde_json::json;
    // 이벤트 타임라인 (ts 오름차순·원시 컬럼 그대로)
    let mut timeline: Vec<serde_json::Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT ts, event_type, tool_name, is_skill, skill_name, is_agent, agent_type, exit_code, role
         FROM events WHERE session_id = ?1 ORDER BY ts ASC LIMIT 2000",
    ) {
        if let Ok(it) = stmt.query_map(rusqlite::params![session_id], |r| {
            Ok(json!({
                "ts": r.get::<_, f64>(0)?,
                "event_type": r.get::<_, String>(1).unwrap_or_default(),
                "tool_name": r.get::<_, Option<String>>(2)?,
                "is_skill": r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                "skill_name": r.get::<_, Option<String>>(4)?,
                "is_agent": r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
                "agent_type": r.get::<_, Option<String>>(6)?,
                "exit_code": r.get::<_, Option<i64>>(7)?,
                "role": r.get::<_, Option<String>>(8)?,
            }))
        }) {
            timeline.extend(it.filter_map(|x| x.ok()));
        }
    }
    // 토큰/비용/모델 요약 — 해당 세션 usage_records를 summarize 재사용
    let urows = load_summary_rows_for_session(conn, session_id);
    let summary = summarize(&urows);
    json!({
        "session_id": session_id,
        "timeline": timeline,
        "summary": summary,
        // B-9(E4 최소구현): 전사 발췌 — DB 적재 대신 온디맨드 파일 꼬리 읽기(저장 비용 0)
        "transcript": transcript_excerpt(session_id),
    })
}

/// 전사 발췌 — session_id가 실제 트랜스크립트 경로면 꼬리 64KB에서 최근 30턴의
/// user/assistant 텍스트(턴당 400자)를 추출한다. 파일이 아니면 빈 배열(구 세션 호환).
fn transcript_excerpt(session_id: &str) -> Vec<serde_json::Value> {
    use serde_json::json;
    use std::io::{Read, Seek, SeekFrom};
    let p = std::path::Path::new(session_id);
    if !p.is_file() {
        return Vec::new();
    }
    let Ok(mut f) = std::fs::File::open(p) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(64 * 1024);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if f.take(64 * 1024).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf);
    let mut turns: Vec<serde_json::Value> = Vec::new();
    // start>0이면 첫 줄은 절단 가능성 — 스킵
    for line in text.lines().skip(if start > 0 { 1 } else { 0 }) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let ty = v["type"].as_str().unwrap_or("");
        if (ty != "user" && ty != "assistant") || v["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        let content = &v["message"]["content"];
        let mut body = String::new();
        if let Some(s) = content.as_str() {
            body = s.to_string();
        } else if let Some(arr) = content.as_array() {
            for c in arr {
                if c["type"].as_str() == Some("text") {
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    body.push_str(c["text"].as_str().unwrap_or(""));
                }
            }
        }
        if body.trim().is_empty() {
            continue;
        }
        let mut t: String = body.chars().take(400).collect();
        if body.chars().count() > 400 {
            t.push('…');
        }
        turns.push(json!({"role": ty, "text": t, "ts": v["timestamp"].as_str().unwrap_or("")}));
    }
    let n = turns.len();
    if n > 30 {
        turns.split_off(n - 30)
    } else {
        turns
    }
}

/// 단일 세션의 usage_records를 summarize 입력 형태로 로드.
fn load_summary_rows_for_session(conn: &Connection, session_id: &str) -> Vec<SummaryRow> {
    let mut stmt = match conn.prepare(
        "SELECT agent, role, model, input_tokens, output_tokens, cache_creation, cache_read, cost_usd, session_id, ts
         FROM usage_records WHERE session_id = ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![session_id], |r| {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),  // agent
            r.get::<_, String>(1).unwrap_or_default(),  // role (idx1)
            r.get::<_, String>(2).unwrap_or_default(),  // model
            r.get::<_, i64>(3)? as u64,
            r.get::<_, i64>(4)? as u64,
            r.get::<_, i64>(5)? as u64,
            r.get::<_, i64>(6)? as u64,
            r.get::<_, f64>(7)?,
            r.get::<_, String>(8).unwrap_or_default(),  // session
            r.get::<_, f64>(9)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

// ───────────────────────── E5 추세·주간 다이제스트 (control.weekly) ─────────────────────────

const WEEK_SECS: f64 = 7.0 * 86_400.0;

/// 주간 집계용 — (session, tokens, cost, ts)
type WeeklyUsageRow = (String, u64, f64, f64);
/// 주간 집계용 — (session, role, is_skill, skill, is_agent, event_type, ts)
type WeeklyEventRow = (String, String, bool, String, bool, String, f64);

fn load_weekly_usage(conn: &Connection, since: f64) -> Vec<WeeklyUsageRow> {
    let mut stmt = match conn.prepare(
        "SELECT session_id, input_tokens, output_tokens, cache_creation, cache_read, cost_usd, ts
         FROM usage_records WHERE ts >= ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![since], |r| {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),
            (r.get::<_, i64>(1)? + r.get::<_, i64>(2)? + r.get::<_, i64>(3)? + r.get::<_, i64>(4)?) as u64,
            r.get::<_, f64>(5)?,
            r.get::<_, f64>(6)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

fn load_weekly_events(conn: &Connection, since: f64) -> Vec<WeeklyEventRow> {
    let mut stmt = match conn.prepare(
        "SELECT session_id, role, is_skill, skill_name, is_agent, event_type, ts
         FROM events WHERE ts >= ?1 AND event_type IN ('PRE_TOOL','POST_TOOL')",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![since], |r| {
        Ok((
            r.get::<_, String>(0).unwrap_or_default(),
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0,
            r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            r.get::<_, Option<i64>>(4)?.unwrap_or(0) != 0,
            r.get::<_, String>(5).unwrap_or_default(),
            r.get::<_, f64>(6)?,
        ))
    });
    match rows {
        Ok(it) => it.filter_map(|x| x.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// 이번주(now-7d..now) vs 지난주(now-14d..now-7d)를 WoW·일별오버레이·효율리더·스킬자산으로 롤업(순수).
/// 토큰/비용은 session→role 귀속(events의 역할)으로 노드별 리더 산출. delta_pct: 지난주 0이면 null.
pub fn summarize_weekly(now: f64, usage: &[WeeklyUsageRow], events: &[WeeklyEventRow]) -> serde_json::Value {
    use serde_json::{json, Value};
    use std::collections::{HashMap, HashSet};
    let this_start = now - WEEK_SECS;
    let last_start = now - 2.0 * WEEK_SECS;
    let in_this = |ts: f64| ts >= this_start;
    let in_last = |ts: f64| (last_start..this_start).contains(&ts);

    // session → role (events에서 첫 비어있지 않은 역할)
    let mut sess_role: HashMap<String, String> = HashMap::new();
    for (sid, role, _, _, _, _, _) in events {
        if !role.is_empty() {
            sess_role.entry(sid.clone()).or_insert_with(|| role.clone());
        }
    }
    let role_of = |sid: &str| sess_role.get(sid).cloned().unwrap_or_else(|| "?".to_string());

    // WoW 글로벌 + 일별 오버레이(각 주 7칸) + 세션 집합
    let (mut t_tok, mut t_cost, mut t_msgs) = (0u64, 0.0f64, 0u64);
    let (mut l_tok, mut l_cost, mut l_msgs) = (0u64, 0.0f64, 0u64);
    let mut t_sess: HashSet<&str> = HashSet::new();
    let mut l_sess: HashSet<&str> = HashSet::new();
    let mut this_daily = vec![0u64; 7];
    let mut last_daily = vec![0u64; 7];
    // 역할별 토큰/비용/세션(이번주)
    let mut role_tok: HashMap<String, u64> = HashMap::new();
    let mut role_cost: HashMap<String, f64> = HashMap::new();
    let mut role_sess: HashMap<String, HashSet<String>> = HashMap::new();
    for (sid, tokens, cost, ts) in usage {
        if in_this(*ts) {
            t_tok += tokens;
            t_cost += cost;
            t_msgs += 1;
            t_sess.insert(sid);
            let d = (((*ts - this_start) / 86_400.0) as usize).min(6);
            this_daily[d] += tokens;
            let r = role_of(sid);
            *role_tok.entry(r.clone()).or_insert(0) += tokens;
            *role_cost.entry(r.clone()).or_insert(0.0) += cost;
            role_sess.entry(r).or_default().insert(sid.clone());
        } else if in_last(*ts) {
            l_tok += tokens;
            l_cost += cost;
            l_msgs += 1;
            l_sess.insert(sid);
            let d = (((*ts - last_start) / 86_400.0) as usize).min(6);
            last_daily[d] += tokens;
        }
    }

    // 역할별 활동(이번주) + 스킬 집합(이번주/지난주). 실패율은 E3(control.skills)·E6 담당이라 주간 리더엔 미포함.
    let mut role_skilldiv: HashMap<String, HashSet<String>> = HashMap::new();
    let mut role_tool_calls: HashMap<String, u64> = HashMap::new();
    let mut this_skills: HashMap<String, u64> = HashMap::new();
    let mut last_skills: HashSet<String> = HashSet::new();
    for (sid, role, is_skill, skill, _is_agent, etype, ts) in events {
        let r = if role.is_empty() { role_of(sid) } else { role.clone() };
        if in_this(*ts) {
            if etype == "PRE_TOOL" {
                *role_tool_calls.entry(r.clone()).or_insert(0) += 1;
                if *is_skill && !skill.is_empty() {
                    role_skilldiv.entry(r.clone()).or_default().insert(skill.clone());
                    *this_skills.entry(skill.clone()).or_insert(0) += 1;
                }
            }
        } else if in_last(*ts) && *is_skill && !skill.is_empty() && etype == "PRE_TOOL" {
            last_skills.insert(skill.clone());
        }
    }

    let delta = |t: f64, l: f64| -> Value {
        if l > 0.0 {
            json!(((t - l) / l * 100.0 * 10.0).round() / 10.0)
        } else {
            Value::Null
        }
    };
    let wow = json!({
        "tokens": {"this": t_tok, "last": l_tok, "delta_pct": delta(t_tok as f64, l_tok as f64)},
        "cost":   {"this": t_cost, "last": l_cost, "delta_pct": delta(t_cost, l_cost)},
        "sessions": {"this": t_sess.len(), "last": l_sess.len(), "delta_pct": delta(t_sess.len() as f64, l_sess.len() as f64)},
        "msgs":   {"this": t_msgs, "last": l_msgs, "delta_pct": delta(t_msgs as f64, l_msgs as f64)},
    });

    // 효율 리더: 역할별 토큰·세션·스킬다양성·간결도(토큰/턴)·실패. 토큰 desc 정렬.
    let mut roles: HashSet<String> = HashSet::new();
    roles.extend(role_tok.keys().cloned());
    roles.extend(role_tool_calls.keys().cloned());
    let mut leaders: Vec<Value> = roles
        .into_iter()
        .map(|r| {
            let tok = *role_tok.get(&r).unwrap_or(&0);
            let cost = *role_cost.get(&r).unwrap_or(&0.0);
            let calls = *role_tool_calls.get(&r).unwrap_or(&0);
            let sess = role_sess.get(&r).map(|s| s.len()).unwrap_or(0);
            let div = role_skilldiv.get(&r).map(|s| s.len()).unwrap_or(0);
            json!({
                "role": r, "tokens": tok, "cost_usd": cost, "sessions": sess, "tool_calls": calls,
                "skill_diversity": div,
                "tokens_per_session": if sess > 0 { tok / sess as u64 } else { 0 },
            })
        })
        .collect();
    leaders.sort_by(|a, b| {
        b["tokens"].as_u64().unwrap_or(0).cmp(&a["tokens"].as_u64().unwrap_or(0))
            .then_with(|| a["role"].as_str().unwrap_or("").cmp(b["role"].as_str().unwrap_or("")))
    });

    // 스킬 자산: 신규(이번주만)·휴면(지난주만)·최다(이번주 호출 TOP)
    let this_set: HashSet<&String> = this_skills.keys().collect();
    let mut new_skills: Vec<String> = this_set.iter().filter(|s| !last_skills.contains(**s)).map(|s| (*s).clone()).collect();
    new_skills.sort();
    let mut dormant: Vec<String> = last_skills.iter().filter(|s| !this_set.contains(s)).cloned().collect();
    dormant.sort();
    let mut top: Vec<Value> = this_skills.iter().map(|(k, v)| json!({"name": k, "calls": v})).collect();
    top.sort_by(|a, b| {
        b["calls"].as_u64().unwrap_or(0).cmp(&a["calls"].as_u64().unwrap_or(0))
            .then_with(|| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")))
    });

    json!({
        "wow": wow,
        "daily": {"this": this_daily, "last": last_daily},
        "leaders": leaders,
        "skill_asset": {"new": new_skills, "dormant": dormant, "top": top},
    })
}

/// control.weekly 본체 — 최근 14일 usage_records/events로 주간 다이제스트 집계.
pub fn weekly_summary(conn: &Connection, now: f64) -> serde_json::Value {
    let since = now - 2.0 * WEEK_SECS;
    summarize_weekly(now, &load_weekly_usage(conn, since), &load_weekly_events(conn, since))
}

// ───────────────────────── E9 RBAC: PII 차단(집계만 뷰어) ─────────────────────────

/// session_id(파일 경로=PII: 사용자 홈·프로젝트명 노출)를 안정적 해시로 가린다(순수).
/// 같은 입력→같은 출력(세션 구분 유지)·경로 미노출. 대시보드 공유·스크린샷 시 PII 차단.
pub fn redact_session_id(session_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(session_id.as_bytes());
    let hex = format!("{:x}", h.finalize());
    format!("sess-{}", &hex[..8])
}

/// control.sessions 결과의 session_id를 모두 가린다(집계 지표는 PII 아님 — 보존).
pub fn redact_sessions(mut v: serde_json::Value) -> serde_json::Value {
    if let Some(arr) = v.get_mut("sessions").and_then(|s| s.as_array_mut()) {
        for s in arr.iter_mut() {
            if let Some(sid) = s.get("session_id").and_then(|x| x.as_str()) {
                let r = redact_session_id(sid);
                s["session_id"] = serde_json::Value::String(r);
            }
        }
    }
    v
}

// ───────────────────────── D3 비용·효율 eval baseline (control.cost_baseline) ─────────────────────────

/// D3 재작업률(rework) — events에서 role(tier)별 calls/fail 집계. fail 정의=POST exit≠0(summarize_skills 동형).
/// rework_rate = fail/calls. producer≠evaluator 무결성: 검증된 fail_rate 정의를 baseline 분모로 재사용(새 정의 금지).
pub fn summarize_rework(rows: &[EventRow]) -> serde_json::Value {
    use serde_json::{json, Value};
    use std::collections::HashMap;
    let mut tiers: HashMap<String, [u64; 2]> = HashMap::new(); // role → [calls, fail]
    let (mut total_calls, mut total_fail) = (0u64, 0u64);
    for (etype, role, tool, _is_skill, _skill, _is_agent, _atype, exit, _ts, _dur) in rows {
        let key = if role.is_empty() { "unattributed".to_string() } else { role.clone() };
        if etype == "PRE_TOOL" && !tool.is_empty() {
            tiers.entry(key).or_insert([0, 0])[0] += 1;
            total_calls += 1;
        } else if etype == "POST_TOOL" && matches!(exit, Some(c) if *c != 0) {
            tiers.entry(key).or_insert([0, 0])[1] += 1;
            total_fail += 1;
        }
    }
    let rate = |fail: u64, calls: u64| if calls > 0 { fail as f64 / calls as f64 } else { 0.0 };
    let mut by_tier_rework: Vec<Value> = tiers
        .into_iter()
        .map(|(tier, v)| json!({"tier": tier, "calls": v[0], "fail": v[1], "rework_rate": rate(v[1], v[0])}))
        .collect();
    // calls desc, 동률은 tier asc (결정론)
    by_tier_rework.sort_by(|a, b| {
        b["calls"].as_u64().unwrap_or(0).cmp(&a["calls"].as_u64().unwrap_or(0))
            .then_with(|| a["tier"].as_str().unwrap_or("").cmp(b["tier"].as_str().unwrap_or("")))
    });
    json!({
        "by_tier_rework": by_tier_rework,
        "global_rework_rate": rate(total_fail, total_calls),
        "total_calls": total_calls,
        "total_fail": total_fail,
    })
}

/// D3 LOCKED baseline 합본 — by_tier(비용)+rework+cache_roi_x를 한 객체로(producer≠evaluator diff 단위).
/// tier 라우팅(R1) 도입 '전'에 cys cost-baseline lock으로 박제 → 도입 후 동일 eval 재실행·diff로 회귀 판정.
pub fn cost_baseline(conn: &Connection, since: f64) -> serde_json::Value {
    use serde_json::json;
    let cost = summarize(&load_summary_rows(conn, since)); // by_tier·cache_roi_x·productivity·totals
    let rework = summarize_rework(&load_event_rows(conn, since)); // by_tier_rework·global_rework_rate
    json!({
        "by_tier": cost["by_tier"],
        "cost_per_session": cost["productivity"]["cost_per_session"],
        "cache_roi_x": cost["cache_roi_x"],
        "cache_efficiency": cost["cache_efficiency"],
        "rework": rework,
        "totals": cost["totals"],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_load_replay_roundtrip() {
        let dir = std::env::temp_dir().join(format!("cys-analytics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join(format!("a-{}.db", line!()))).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_records(id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, agent TEXT,
             model TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_creation INTEGER,
             cache_read INTEGER, cost_usd REAL, ts REAL);",
        )
        .unwrap();
        let now = 2_000_000.0;
        record_usage(&conn, "/s/a.jsonl", "master", "claude", "claude-opus-4-8", 1000, 300, 2000, 50000, 0.42, now - 100.0);
        record_usage(&conn, "/s/a.jsonl", "master", "claude", "claude-opus-4-8", 500, 200, 0, 0, 0.01, now);
        // 12h 밖 — 제외돼야
        record_usage(&conn, "/s/old.jsonl", "worker", "claude", "claude-haiku-4-5", 10, 10, 0, 0, 0.5, now - 50_000.0);

        let rows = load_recent(&conn, now - SPARK_SPAN_SECS);
        assert_eq!(rows.len(), 2, "12h 안쪽 2건만(오래된 1건 제외)");

        let mut c = Consumption::default();
        replay(&rows, &mut c);
        assert_eq!(c.today_msgs, 2);
        assert_eq!(c.today_input, (1000 + 2000) + 500, "input+cache_creation 합");
        assert_eq!(c.today_tokens, (1000 + 2000 + 300) + (500 + 200));
        assert!((c.today_cost_usd - 0.43).abs() < 1e-9, "비용 보존 0.42+0.01");
        assert_eq!(c.model_tokens.get("claude-opus-4-8").copied(), Some((1000 + 2000 + 300) + (500 + 200)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn derive_and_record_event() {
        use serde_json::json;
        let (is_s, sn, is_a, at) = derive_tool("Skill", &json!({"skill": "commit"}));
        assert!(is_s && sn.as_deref() == Some("commit") && !is_a && at.is_none());
        let (is_s, _, is_a, at) = derive_tool("Task", &json!({"subagent_type": "Explore"}));
        assert!(!is_s && is_a && at.as_deref() == Some("Explore"));
        let (is_s, _, is_a, _) = derive_tool("Bash", &json!({"command": "ls"}));
        assert!(!is_s && !is_a, "일반 툴은 skill/agent 아님");
        let (_, sn, _, _) = derive_tool("Skill", &json!({"command": "/deep-research"}));
        assert_eq!(sn.as_deref(), Some("deep-research"), "/slash 접두 제거");

        let dir = std::env::temp_dir().join(format!("cys-ev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join(format!("e-{}.db", line!()))).unwrap();
        conn.execute_batch(
            "CREATE TABLE events(id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, agent TEXT,
             event_type TEXT, tool_name TEXT, is_skill INTEGER, skill_name TEXT, is_slash INTEGER,
             is_agent INTEGER, agent_type TEXT, agent_id TEXT, exit_code INTEGER, duration_ms INTEGER, ts REAL);",
        )
        .unwrap();
        record_event(&conn, "/s/a", "worker", "claude", "PRE_TOOL", "Skill", true, Some("commit"), false, None, None, None, None, 1000.0);
        record_event(&conn, "/s/a", "worker", "claude", "POST_TOOL", "Bash", false, None, false, None, None, Some(1), Some(850), 1001.0);
        let dur: i64 = conn
            .query_row("SELECT duration_ms FROM events WHERE event_type='POST_TOOL'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(dur, 850, "duration_ms가 적재되어야 skills p50 성립(B-9)");
        let skills: i64 = conn.query_row("SELECT COUNT(*) FROM events WHERE is_skill=1", [], |r| r.get(0)).unwrap();
        assert_eq!(skills, 1, "스킬 호출 1건");
        let fails: i64 = conn.query_row("SELECT COUNT(*) FROM events WHERE exit_code!=0", [], |r| r.get(0)).unwrap();
        assert_eq!(fails, 1, "실패(exit!=0) 1건 — E3 반복실패 토대");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summarize_costs_and_productivity() {
        // 세션 A: opus 2메시지(캐시 read 50000), 세션 B: haiku 1메시지. agent=claude/codex.
        let rows: Vec<SummaryRow> = vec![
            ("claude".into(), "master".into(), "claude-opus-4-8".into(), 1000, 300, 2000, 50000, 0.05, "/s/a".into(), 1000.0),
            ("claude".into(), "master".into(), "claude-opus-4-8".into(), 500, 200, 0, 0, 0.01, "/s/a".into(), 1100.0),
            ("codex".into(), "reviewer".into(), "claude-haiku-4-5".into(), 100, 50, 0, 0, 0.00035, "/s/b".into(), 1050.0),
        ];
        let s = summarize(&rows);
        let t = &s["totals"];
        assert_eq!(t["input"], 1600, "input 합 1000+500+100");
        assert_eq!(t["cache_read"], 50000);
        assert_eq!(t["msgs"], 3);
        assert_eq!(t["sessions"], 2, "세션 A·B");
        // 토큰 4분해 합 = 1600 + 550(out) + 2000(cc) + 50000(cr) = 54150
        assert_eq!(t["tokens"], 54150u64);
        // 캐시절감$ = 50000/1e6 × (opus input 5 − cache_read 0.5) = 0.05 × 4.5 = 0.225
        assert!((s["cache_savings_usd"].as_f64().unwrap() - 0.225).abs() < 1e-9, "{}", s["cache_savings_usd"]);
        // by_model: opus가 비용 우선 정렬 1위
        assert_eq!(s["by_model"][0]["model"], "claude-opus-4-8");
        assert_eq!(s["by_model"][0]["msgs"], 2);
        // 생산성: 턴/세션 = 3/2 = 1.5, 비용/세션 = (0.06035)/2
        let prod = &s["productivity"];
        assert!((prod["turns_per_session"].as_f64().unwrap() - 1.5).abs() < 1e-9);
        // 세션 A duration = 1100-1000 = 100, B = 0 → 평균 50
        assert!((prod["avg_session_duration_secs"].as_f64().unwrap() - 50.0).abs() < 1e-9);
        // 빈 입력 = 0 division 안전
        let empty = summarize(&[]);
        assert_eq!(empty["totals"]["msgs"], 0);
        assert_eq!(empty["productivity"]["tokens_per_turn"], 0.0);
    }

    #[test]
    fn record_usage_writes_role() {
        // D3: role 적재 — by_tier 집계의 유일한 전제(과거 INSERT 누락 회귀 방지).
        let dir = std::env::temp_dir().join(format!("cys-role-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join(format!("r-{}.db", line!()))).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_records(id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, agent TEXT,
             model TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_creation INTEGER,
             cache_read INTEGER, cost_usd REAL, ts REAL);",
        )
        .unwrap();
        record_usage(&conn, "/s/x", "cso", "claude", "claude-opus-4-8", 10, 5, 0, 0, 0.01, 1000.0);
        let role: String = conn
            .query_row("SELECT role FROM usage_records LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(role, "cso", "record_usage가 role을 적재해야 by_tier 집계 성립");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tail_offset_roundtrip() {
        // A-4: 오프셋 영속 — 재시작 시 정확 재개점(중복 INSERT 근절)의 토대
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tail_offsets(session_file TEXT PRIMARY KEY, off INTEGER, updated REAL);",
        )
        .unwrap();
        assert_eq!(load_offset(&conn, "/s/a.jsonl"), None);
        save_offset(&conn, "/s/a.jsonl", 12345, 1000.0);
        assert_eq!(load_offset(&conn, "/s/a.jsonl"), Some(12345));
        save_offset(&conn, "/s/a.jsonl", 99999, 1001.0); // upsert 갱신
        assert_eq!(load_offset(&conn, "/s/a.jsonl"), Some(99999));
    }

    #[test]
    fn summarize_by_tier_and_cache() {
        // master(opus) 2건·worker(haiku) 1건. cache_read 50000(opus)로 절감$ 산출.
        let rows: Vec<SummaryRow> = vec![
            ("claude".into(), "master".into(), "claude-opus-4-8".into(), 1000, 300, 2000, 50000, 0.05, "/s/a".into(), 1000.0),
            ("claude".into(), "master".into(), "claude-opus-4-8".into(), 500, 200, 0, 0, 0.01, "/s/a".into(), 1100.0),
            ("claude".into(), "worker".into(), "claude-haiku-4-5".into(), 100, 50, 0, 0, 0.001, "/s/b".into(), 1050.0),
        ];
        let s = summarize(&rows);
        let bt = s["by_tier"].as_array().unwrap();
        // cost desc → master(0.06) 먼저, worker(0.001) 뒤
        assert_eq!(bt[0]["tier"], "master");
        assert_eq!(bt[0]["msgs"], 2);
        assert!((bt[0]["cost_usd"].as_f64().unwrap() - 0.06).abs() < 1e-9);
        assert_eq!(bt[1]["tier"], "worker");
        // A-3: cache_roi_x는 폐기(전 클로드 모델 캐시단가=입력의 10%라 항상 0.9 — 무정보 상수)
        assert!(s.get("cache_roi_x").is_none(), "cache_roi_x는 제거되어야 한다");
        // 절감$ = 50000/1e6 × (5 − 0.5) = 0.225 (round4 보존)
        assert!((s["cache_savings_usd"].as_f64().unwrap() - 0.225).abs() < 1e-9);
        // cache_efficiency = cr/(in+cc+cr) = 50000/53600 → round4 = 0.9328
        assert!((s["cache_efficiency"].as_f64().unwrap() - 0.9328).abs() < 1e-9);
        // B-4: 단가표 적중 모델은 pricing_known=true
        assert_eq!(s["by_model"][0]["pricing_known"], true);
        // 빈 role → unattributed
        let r2: Vec<SummaryRow> = vec![
            ("claude".into(), "".into(), "claude-opus-4-8".into(), 10, 5, 0, 0, 0.01, "/s/c".into(), 1.0),
        ];
        assert_eq!(summarize(&r2)["by_tier"][0]["tier"], "unattributed");
        // 빈 입력 0-division 안전
        assert_eq!(summarize(&[])["cache_savings_usd"], 0.0);
        // B-4: 미상 모델은 pricing_known=false (Sonnet 폴백 추정 표시)
        let r3: Vec<SummaryRow> = vec![
            ("claude".into(), "w".into(), "future-model-9".into(), 10, 5, 0, 0, 0.01, "/s/d".into(), 1.0),
        ];
        assert_eq!(summarize(&r3)["by_model"][0]["pricing_known"], false);
    }

    #[test]
    fn summarize_rework_failrate() {
        // fail 정의=POST exit≠0 / calls=PRE(!tool.is_empty) — summarize_skills 동형(producer≠evaluator).
        let ev = |t: &str, role: &str, tool: &str, ex: Option<i64>| -> EventRow {
            (t.into(), role.into(), tool.into(), false, String::new(), false, String::new(), ex, 0.0, None)
        };
        let rows: Vec<EventRow> = vec![
            ev("PRE_TOOL", "worker", "Bash", None),
            ev("POST_TOOL", "worker", "Bash", Some(1)), // worker 1 fail
            ev("PRE_TOOL", "worker", "Edit", None),
            ev("POST_TOOL", "worker", "Edit", Some(0)), // 성공
            ev("PRE_TOOL", "master", "Read", None),
            ev("POST_TOOL", "master", "Read", Some(0)),
        ];
        let r = summarize_rework(&rows);
        // global: calls=3(PRE) fail=1(POST exit≠0) → 1/3
        assert!((r["global_rework_rate"].as_f64().unwrap() - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(r["total_calls"], 3);
        assert_eq!(r["total_fail"], 1);
        let bt = r["by_tier_rework"].as_array().unwrap();
        // worker calls=2(>master 1) → 먼저. rework_rate worker=1/2
        assert_eq!(bt[0]["tier"], "worker");
        assert_eq!(bt[0]["calls"], 2);
        assert!((bt[0]["rework_rate"].as_f64().unwrap() - 0.5).abs() < 1e-9);
        // 빈 입력 0-division 안전
        assert_eq!(summarize_rework(&[])["global_rework_rate"], 0.0);
    }

    #[test]
    fn summarize_skills_calls_and_failrate() {
        // Bash 2호출 1실패, Skill(commit) 1호출 PRE+POST 성공, Task→Explore 위임 1.
        let ev = |t: &str, role: &str, tool: &str, sk: bool, skn: &str, ag: bool, at: &str, ex: Option<i64>| -> EventRow {
            (t.into(), role.into(), tool.into(), sk, skn.into(), ag, at.into(), ex, 1000.0, if t == "POST_TOOL" { Some(500) } else { None })
        };
        let rows: Vec<EventRow> = vec![
            ev("PRE_TOOL", "worker", "Bash", false, "", false, "", None),
            ev("POST_TOOL", "worker", "Bash", false, "", false, "", Some(1)), // 실패
            ev("PRE_TOOL", "worker", "Bash", false, "", false, "", None),
            ev("POST_TOOL", "worker", "Bash", false, "", false, "", Some(0)), // 성공
            ev("PRE_TOOL", "master", "Skill", true, "commit", false, "", None),
            ev("POST_TOOL", "master", "Skill", true, "commit", false, "", Some(0)),
            ev("PRE_TOOL", "master", "Task", false, "", true, "Explore", None),
        ];
        let s = summarize_skills(&rows);
        let t = &s["totals"];
        assert_eq!(t["tool_calls"], 4, "PRE_TOOL: Bash2+Skill1+Task1");
        assert_eq!(t["skill_calls"], 1);
        assert_eq!(t["agent_calls"], 1);
        assert_eq!(t["fail_calls"], 1, "POST exit≠0 1건");
        assert!((t["fail_rate"].as_f64().unwrap() - 0.25).abs() < 1e-9, "1/4");
        // by_tool: Bash 1위(calls 2), fail 1, fail_rate 0.5
        assert_eq!(s["by_tool"][0]["name"], "Bash");
        assert_eq!(s["by_tool"][0]["calls"], 2);
        assert!((s["by_tool"][0]["fail_rate"].as_f64().unwrap() - 0.5).abs() < 1e-9);
        // by_skill: commit, 실패 0
        assert_eq!(s["by_skill"][0]["name"], "commit");
        assert_eq!(s["by_skill"][0]["fail"], 0);
        assert_eq!(s["by_skill"][0]["roles"][0]["role"], "master");
        // by_agent: Explore 위임
        assert_eq!(s["by_agent"][0]["name"], "Explore");
        assert_eq!(s["by_agent"][0]["by_role"][0]["role"], "master");
        // 🔥failures: Bash만(fail>0)
        assert_eq!(s["failures"].as_array().unwrap().len(), 1);
        assert_eq!(s["failures"][0]["name"], "Bash");
        // 빈 입력 안전
        let empty = summarize_skills(&[]);
        assert_eq!(empty["totals"]["tool_calls"], 0);
        assert_eq!(empty["totals"]["fail_rate"], 0.0);
    }

    #[test]
    fn ribbon_buckets_activity() {
        // [0,10] 10칸 — t=0→칸0, t=10(끝)→마지막 칸, t=5→중간
        let r = ribbon(&[0.0, 5.0, 10.0], 0.0, 10.0, 10);
        assert_eq!(r.len(), 10);
        assert_eq!(r[0], 1);
        assert_eq!(r[5], 1);
        assert_eq!(r[9], 1, "끝 ts는 마지막 칸으로 클램프");
        // span 0(단일 시점) 안전
        let r2 = ribbon(&[3.0, 3.0], 3.0, 3.0, 4);
        assert_eq!(r2.iter().sum::<u64>(), 2);
    }

    #[test]
    fn summarize_sessions_merges_usage_and_events() {
        let usage: Vec<SessUsageRow> = vec![
            ("/s/a".into(), "claude".into(), 1000, 0.05, 1000.0),
            ("/s/a".into(), "claude".into(), 500, 0.01, 1100.0),
            ("/s/b".into(), "codex".into(), 200, 0.001, 1050.0),
        ];
        let events: Vec<SessEventRow> = vec![
            ("/s/a".into(), "worker".into(), "Skill".into(), true, "commit".into(), false, None, "PRE_TOOL".into(), 1020.0),
            ("/s/a".into(), "worker".into(), "Bash".into(), false, "".into(), false, Some(1), "POST_TOOL".into(), 1040.0), // 실패
            ("/s/b".into(), "master".into(), "Task".into(), false, "".into(), true, None, "PRE_TOOL".into(), 1055.0),
        ];
        let mut starred = std::collections::HashSet::new();
        starred.insert("/s/a".to_string());
        let v = summarize_sessions(&usage, &events, &starred);
        let s = v["sessions"].as_array().unwrap();
        assert_eq!(s.len(), 2);
        // ended_at 내림차순 — /s/a(max 1100) 먼저, /s/b(max 1055)
        assert_eq!(s[0]["session_id"], "/s/a");
        assert_eq!(s[0]["agent"], "claude");
        assert_eq!(s[0]["role"], "worker");
        assert_eq!(s[0]["msgs"], 2);
        assert_eq!(s[0]["tokens"], 1500u64);
        assert!((s[0]["cost_usd"].as_f64().unwrap() - 0.06).abs() < 1e-9);
        assert_eq!(s[0]["skill_calls"], 1);
        assert_eq!(s[0]["fail_calls"], 1);
        assert_eq!(s[0]["top_skill"], "commit");
        assert_eq!(s[0]["starred"], true);
        assert!((s[0]["duration_secs"].as_f64().unwrap() - 100.0).abs() < 1e-9, "1100-1000");
        assert_eq!(s[0]["ribbon"].as_array().unwrap().len(), RIBBON_BUCKETS);
        // /s/b: codex·master·위임 1
        assert_eq!(s[1]["session_id"], "/s/b");
        assert_eq!(s[1]["agent_calls"], 1);
        assert_eq!(s[1]["starred"], false);
    }

    #[test]
    fn summarize_weekly_wow_and_assets() {
        let now = 2_000_000.0;
        let day = 86_400.0;
        // 이번주(now-1d): /s/a worker 2000토큰. 지난주(now-8d): /s/x worker 1000토큰.
        let usage: Vec<WeeklyUsageRow> = vec![
            ("/s/a".into(), 2000, 0.10, now - day),
            ("/s/x".into(), 1000, 0.04, now - 8.0 * day),
        ];
        // 이번주 스킬 commit·deep-research(신규), 지난주 commit·old-skill(→ old-skill 휴면)
        let events: Vec<WeeklyEventRow> = vec![
            ("/s/a".into(), "worker".into(), true, "commit".into(), false, "PRE_TOOL".into(), now - day),
            ("/s/a".into(), "worker".into(), true, "deep-research".into(), false, "PRE_TOOL".into(), now - day),
            ("/s/x".into(), "worker".into(), true, "commit".into(), false, "PRE_TOOL".into(), now - 8.0 * day),
            ("/s/x".into(), "worker".into(), true, "old-skill".into(), false, "PRE_TOOL".into(), now - 8.0 * day),
        ];
        let w = summarize_weekly(now, &usage, &events);
        // WoW: 토큰 2000 vs 1000 → +100%
        assert_eq!(w["wow"]["tokens"]["this"], 2000u64);
        assert_eq!(w["wow"]["tokens"]["last"], 1000u64);
        assert!((w["wow"]["tokens"]["delta_pct"].as_f64().unwrap() - 100.0).abs() < 1e-6);
        // 일별 오버레이 7칸
        assert_eq!(w["daily"]["this"].as_array().unwrap().len(), 7);
        assert_eq!(w["daily"]["last"].as_array().unwrap().len(), 7);
        // 리더: worker, 이번주 토큰 2000·스킬다양성 2
        assert_eq!(w["leaders"][0]["role"], "worker");
        assert_eq!(w["leaders"][0]["tokens"], 2000u64);
        assert_eq!(w["leaders"][0]["skill_diversity"], 2);
        // 스킬 자산: 신규=deep-research, 휴면=old-skill, 최다 포함 commit
        let new: Vec<&str> = w["skill_asset"]["new"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert!(new.contains(&"deep-research") && !new.contains(&"commit"), "{:?}", new);
        let dorm: Vec<&str> = w["skill_asset"]["dormant"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(dorm, vec!["old-skill"]);
        // 지난주 0 분모 가드: 빈 입력 delta null
        let empty = summarize_weekly(now, &[], &[]);
        assert!(empty["wow"]["tokens"]["delta_pct"].is_null());
    }

    #[test]
    fn redact_session_id_stable_and_pii_free() {
        let p = "/Users/user/.claude/projects/secret-proj/abc-123.jsonl";
        let r = redact_session_id(p);
        assert!(r.starts_with("sess-") && r.len() == 13, "{r}");
        assert!(!r.contains("cys") && !r.contains("secret") && !r.contains("/"), "PII 노출: {r}");
        assert_eq!(r, redact_session_id(p), "같은 입력 안정적");
        assert_ne!(r, redact_session_id("/other/path.jsonl"), "다른 입력 구분");
        // redact_sessions: 배열의 session_id만 가리고 집계는 보존
        let v = serde_json::json!({"sessions": [{"session_id": p, "tokens": 5000, "cost_usd": 0.1}]});
        let red = redact_sessions(v);
        assert_eq!(red["sessions"][0]["session_id"], serde_json::Value::String(r));
        assert_eq!(red["sessions"][0]["tokens"], 5000, "집계 보존");
    }

    // ── T2-3 append-only change-log + 단조 revn + 낙관적 동시성 ──

    /// 테스트용: open()의 실제 스키마로 analytics.db 생성(socket_path=dir/cys.sock → 부모 dir).
    /// tag로 테스트별 고유 디렉터리(병렬 실행 시 WAL 충돌 회피).
    fn open_change_db(tag: &str) -> (std::path::PathBuf, Connection) {
        let dir = std::env::temp_dir().join(format!("cys-cl-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("cys.sock");
        let conn = open(&socket).expect("open analytics.db");
        (socket, conn)
    }

    #[test]
    fn change_log_append_accepts_and_chains() {
        let (_socket, mut conn) = open_change_db("append");
        let out = change_log_append(&mut conn, "SESSION_STATE", 0, 0, "edit", "{\"a\":1}", 1000.0).unwrap();
        match out {
            AppendOutcome::Accepted { revn, vern, seq, attest_hash } => {
                assert_eq!(revn, 1, "첫 append revn=1");
                assert_eq!(vern, 0);
                assert_eq!(seq, 1);
                assert_eq!(attest_hash.len(), 64, "sha256 hex 64자");
            }
            other => panic!("기대 Accepted, 실제 {other:?}"),
        }
        // 두 번째 append는 base_revn=1로 수용, 해시체인 진행
        let out2 = change_log_append(&mut conn, "SESSION_STATE", 1, 0, "edit", "{\"a\":2}", 1001.0).unwrap();
        match out2 {
            AppendOutcome::Accepted { revn, attest_hash, .. } => {
                assert_eq!(revn, 2);
                // 체인: 첫 payload와 다른 입력 → 다른 해시
                let first: String = conn
                    .query_row("SELECT attest_hash FROM change_log WHERE revn=1", [], |r| r.get(0))
                    .unwrap();
                assert_ne!(attest_hash, first, "해시체인 칸마다 상이");
            }
            other => panic!("기대 Accepted, 실제 {other:?}"),
        }
    }

    #[test]
    fn revn_monotonic_rejects_stale_base() {
        let (_socket, mut conn) = open_change_db("monotonic");
        change_log_append(&mut conn, "S", 0, 0, "edit", "p0", 1.0).unwrap(); // revn→1
        change_log_append(&mut conn, "S", 1, 0, "edit", "p1", 2.0).unwrap(); // revn→2
        // stale base_revn=1(현재 stored=2) → RevnConflict, 단조 위반 0
        let out = change_log_append(&mut conn, "S", 1, 0, "edit", "px", 3.0).unwrap();
        assert_eq!(out, AppendOutcome::RevnConflict { stored_revn: 2 });
        // change_log는 2건만(거부된 건 미적재) → restore replay 결정론
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM change_log WHERE scope='S'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2, "거부된 append는 change_log에 들어가지 않음");
    }

    #[test]
    fn vern_conflict_distinguishes_restore_from_edit() {
        let (_socket, mut conn) = open_change_db("vern");
        change_log_append(&mut conn, "S", 0, 0, "edit", "p0", 1.0).unwrap();
        // stored_vern=0인데 base_vern=5(다른 버전 복원 가정) → VernConflict(RevnConflict와 구별)
        let out = change_log_append(&mut conn, "S", 1, 5, "edit", "p1", 2.0).unwrap();
        assert_eq!(out, AppendOutcome::VernConflict { stored_vern: 0 });
    }

    /// R6 핵심: 같은 base_revn으로 동시에 두 writer가 append → 정확히 1 Accepted, 1 VernConflict 아닌
    /// RevnConflict. 단일 cysd writer 직렬화(한 트랜잭션 check+append)가 단조성을 보장하므로,
    /// 같은 base에서 둘째는 stored가 이미 증가해 RevnConflict로 reject됨.
    #[test]
    fn concurrent_same_base_exactly_one_accepted() {
        let (_socket, mut conn) = open_change_db("concur");
        change_log_append(&mut conn, "S", 0, 0, "edit", "seed", 1.0).unwrap(); // stored revn→1
        // 두 writer가 둘 다 base_revn=1을 읽고(같은 base) 직렬로 cysd writer에 도달:
        let a = change_log_append(&mut conn, "S", 1, 0, "edit", "writerA", 2.0).unwrap();
        let b = change_log_append(&mut conn, "S", 1, 0, "edit", "writerB", 3.0).unwrap();
        let accepted = [&a, &b].iter().filter(|o| matches!(o, AppendOutcome::Accepted { .. })).count();
        let rejected = [&a, &b]
            .iter()
            .filter(|o| matches!(o, AppendOutcome::RevnConflict { stored_revn: 2 }))
            .count();
        assert_eq!(accepted, 1, "정확히 1건만 Accepted");
        assert_eq!(rejected, 1, "다른 1건은 RevnConflict(stale base)");
    }

    #[test]
    fn restore_replay_returns_changes_in_order() {
        let (socket, mut conn) = open_change_db("restore");
        for k in 0..5u64 {
            change_log_append(&mut conn, "S", k, 0, "edit", &format!("p{k}"), 1.0 + k as f64).unwrap();
        }
        drop(conn); // writer 닫고 FRESH WAL reader로 복원(stale conn 비사용 증명)
        let (payloads, revn, vern, replayed) = change_log_restore(&socket, "S", 2).unwrap();
        assert_eq!(revn, 5);
        assert_eq!(vern, 0);
        assert_eq!(replayed, 3, "revn>2 = p2,p3,p4");
        assert_eq!(payloads, vec!["p2".to_string(), "p3".to_string(), "p4".to_string()]);
    }

    // ── CC v2 WS-A: rate_snapshots ──

    #[test]
    fn rate_snapshots_record_last_and_prune() {
        let (_socket, conn) = open_change_db("ratesnap");
        // 같은 (계정,창)에 시계열 3점 + 다른 창 1점
        record_rate_snapshot(&conn, 100.0, "claude", "u1", "a@b.c", "5h", 10.0, Some(500.0), "statusline");
        record_rate_snapshot(&conn, 200.0, "claude", "u1", "a@b.c", "5h", 20.0, Some(500.0), "statusline");
        record_rate_snapshot(&conn, 300.0, "claude", "u1", "a@b.c", "5h", 30.0, Some(500.0), "statusline");
        record_rate_snapshot(&conn, 250.0, "claude", "u1", "a@b.c", "7d", 5.0, None, "statusline");
        // last: (u1,5h)=ts300, (u1,7d)=ts250 — 창별 최신 1행씩
        let last = last_rate_snapshots(&conn, 0.0, None);
        assert_eq!(last.len(), 2);
        let five = last.iter().find(|r| r.4 == "5h").unwrap();
        assert_eq!((five.0, five.5), (300.0, 30.0));
        // 시계열은 ts 오름차순
        let series = rate_series(&conn, "claude", "u1", "5h", 0.0);
        assert_eq!(series, vec![(100.0, 10.0), (200.0, 20.0), (300.0, 30.0)]);
        // prune: ts<250 삭제 → 5h는 300 한 점, 7d 250 한 점
        prune_rate_snapshots(&conn, 250.0);
        assert_eq!(rate_series(&conn, "claude", "u1", "5h", 0.0), vec![(300.0, 30.0)]);
    }

    /// fix-values-1 SP-1: 스냅샷에 출처를 싣는다 — 출처 열이 없던 구 DB 에도 열이 붙고(멱등), 구 행(출처 NULL ·
    /// 0.14.42 전에는 창 밖 값이 없었다)은 신뢰 행으로 남으며, `exclude_source` 로 창 밖 행을 빼고 최신을 고른다.
    #[test]
    fn rate_snapshots_carry_source_and_old_dbs_are_migrated() {
        let dir = std::env::temp_dir().join(format!("cys-cl-{}-ratesrc", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("cys.sock");
        let sd = state_dir(&socket);
        std::fs::create_dir_all(&sd).unwrap();
        {
            // 0.14.41 까지의 스키마(출처 열 없음) + 구 행 1개
            let old = Connection::open(sd.join("analytics.db")).unwrap();
            old.execute_batch(
                "CREATE TABLE rate_snapshots(ts REAL, provider TEXT, account TEXT, label TEXT,
                    win TEXT, used_pct REAL, resets_at REAL);
                 INSERT INTO rate_snapshots VALUES(100.0,'claude','u1','a@b.c','5h',90.0,NULL);",
            )
            .unwrap();
        }
        let conn = open(&socket).expect("구 DB 열기");
        record_rate_snapshot(&conn, 200.0, "claude", "u1", "a@b.c", "5h", 5.0, None, "statusline-outside");
        record_rate_snapshot(&conn, 150.0, "claude", "u2", "b@b.c", "5h", 40.0, None, "statusline");
        let pick = |ex: Option<&str>| {
            let mut v: Vec<(String, f64, f64)> =
                last_rate_snapshots(&conn, 0.0, ex).into_iter().map(|r| (r.2, r.0, r.5)).collect();
            v.sort_by(|a, b| a.0.cmp(&b.0));
            v
        };
        assert_eq!(pick(None), vec![("u1".into(), 200.0, 5.0), ("u2".into(), 150.0, 40.0)], "표시 복원 = 출처 무관 최신");
        assert_eq!(
            pick(Some("statusline-outside")),
            vec![("u1".into(), 100.0, 90.0), ("u2".into(), 150.0, 40.0)],
            "경보 복원 = 창 밖 행을 뺀 최신(구 NULL 행은 신뢰)"
        );
        drop(conn);
        let again = open(&socket).expect("두 번째 열기(마이그레이션 멱등)");
        assert_eq!(last_rate_snapshots(&again, 0.0, Some("statusline-outside")).len(), 2);
        drop(again);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── CC v2 WS-B: skill_runs 상태 전이 ──

    #[test]
    fn skill_runs_lifecycle_and_boot_reconcile() {
        let (_socket, conn) = open_change_db("skillruns");
        skill_run_insert(&conn, "r1", "skill-a", "라벨A", 100.0, 820.0);
        skill_run_insert(&conn, "r2", "skill-b", "라벨B", 110.0, 830.0);
        // 열린 run 2건(deadline 동봉)
        let open = skill_runs_open(&conn);
        assert_eq!(open.len(), 2);
        assert!(open.iter().any(|(id, d)| id == "r1" && *d == 820.0));
        // r1 done + 산출물 경로
        skill_run_update(&conn, "r1", "done", None, Some("/x/skill-out/r1"), Some(400.0));
        let open = skill_runs_open(&conn);
        assert_eq!(open.len(), 1, "done은 열린 run에서 제외");
        // 부트 reconcile: 남은 launched(r2) → failed(daemon_restart)
        skill_runs_fail_open(&conn, "daemon_restart", 500.0);
        assert!(skill_runs_open(&conn).is_empty());
        let runs = skill_runs_list(&conn, 10);
        assert_eq!(runs.len(), 2);
        let r2 = runs.iter().find(|r| r["run_id"] == "r2").unwrap();
        assert_eq!(r2["status"], "failed");
        assert_eq!(r2["exit_note"], "daemon_restart");
        let r1 = runs.iter().find(|r| r["run_id"] == "r1").unwrap();
        assert_eq!(r1["status"], "done");
        assert_eq!(r1["artifact_dir"], "/x/skill-out/r1");
    }
}
