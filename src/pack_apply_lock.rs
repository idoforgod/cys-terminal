//! The pack-apply lock (`~/.cys/.pack-apply.lock`) — moved here from the CLI binary by a PURE MOVE
//! (RULINGS B7, contract O-C2-4): same lock file path (built by the caller as
//! `pack_state_base().join(".pack-apply.lock")`), same protocol (unix `flock(LOCK_EX)` on a
//! `create(true)` file, parent directory created first, unlock after the closure), no timeout (the
//! flock blocks), a no-op on Windows. The CLI keeps the names through a `use` (its callers and its
//! test `apply_lock_creates_missing_parent_dir` are unedited). The daemon's revert / apply take the
//! SAME lock file, so `cys pack-update` and a revert run one after the other (C2 2.8 — the only
//! lock the repair mechanism takes).

/// flock(LOCK_EX) 임계영역에서 f를 실행(§7-⑧ 폴백 apply-lock — per-file write_atomic + writer 배타).
/// non-unix는 잠금 없이 실행.
///
/// ⚠보장 범위(정직 명시 · 층위 분리):
/// 1) 이 락은 **writer 측 상호배제(serialization)만** 제공한다 — 동시 writer가 같은 pack_dir를
///    겹쳐 쓰는 것을 직렬화할 뿐이다.
/// 2) **트랜잭션 rollback/commit marker는 이 락의 책임이 아니라 apply_pack_transactional의 책임이다**
///    — backup journal + `.pack-version` hard commit marker로 부분커밋 0(all-or-nothing)을 보장한다
///    (pack-update 경로). 이 락은 그 트랜잭션을 writer 배타 창 안에서 단독 실행시키는 역할만 한다.
/// 3) 그러나 §6-4 심링크(pack_dir) 1회 마이그레이션이 보류된 현재(디렉터리 일괄 atomic 스왑 미구현),
///    **외부 동시 live READER의 snapshot atomic(multi-file SET 일관성·torn-read)은 여전히 보장되지
///    않는다.** §7-⑧ 폴백이 요구한 reader-측 차단(공유 flock)을 load-bearing 리더(compose_directive —
///    MASTER_DIRECTIVE/soul.md/MEMORY.md/각 SKILL.md 순차 읽기 · Tauri read_board_catalog)가 취하지
///    않기 때문이다. 그 결과 apply 창 동안 외부 리더는 신규-directive + 구-soul 같은 혼재(torn) 집합을
///    관측할 수 있다. pack-update 자신의 reinject는 apply 이후 실행되어 안전하고, 노출 대상은 외부 동시
///    리더뿐이다. 진짜 reader 집합 원자성은 §6-4 심링크 스왑 도입 시 확보된다.
pub fn with_apply_lock<T>(lock_path: &std::path::Path, f: impl FnOnce() -> T) -> Result<T, String> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // CI fresh 환경엔 ~/.cys/ 가 없어 lock 파일 open이 ENOENT로 실패한다.
        // 락 파일 열기 직전 부모 디렉토리를 보장한다(이미 있으면 무해).
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("apply-lock 부모 디렉토리 생성 실패 {}: {e}", parent.display())
            })?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock_path)
            .map_err(|e| format!("apply-lock 열기 실패 {}: {e}", lock_path.display()))?;
        let fd = file.as_raw_fd();
        if unsafe { libc::flock(fd, libc::LOCK_EX) } != 0 {
            return Err(format!("flock 실패: {}", std::io::Error::last_os_error()));
        }
        let out = f();
        unsafe {
            libc::flock(fd, libc::LOCK_UN);
        }
        Ok(out)
    }
    #[cfg(not(unix))]
    {
        let _ = lock_path;
        Ok(f())
    }
}

/// ~/.cys (pack_dir의 부모) — 무중단 채널 상태파일(.pack-staging·.pack-apply.lock·.pack-accepted.json) 루트.
pub fn pack_state_base() -> std::path::PathBuf {
    crate::pack::pack_dir()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// `<pack_state_base>/.pack-apply.lock` — the lock file `cys pack-update` takes (cys.rs builds the
/// same path from [`pack_state_base`]).
pub fn pack_apply_lock_path() -> std::path::PathBuf {
    pack_state_base().join(".pack-apply.lock")
}

/// The lock file of the lane whose pack is `pack`: `<parent of pack>/.pack-apply.lock` — for the
/// HQ pack and every `~/.cys/pack-dept-*` lane that is `~/.cys/.pack-apply.lock` (the same file
/// [`pack_apply_lock_path`] gives when `pack` is this process's pack).
pub fn pack_apply_lock_path_for(pack: &std::path::Path) -> std::path::PathBuf {
    pack.parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".pack-apply.lock")
}

/// The SAME lock (same file, same `flock`), tried WITHOUT blocking — a new sibling for the daemon's
/// request handlers, which must never block for the length of a pack update (the CLI waits 40 s for
/// a reply): `Ok(None)` = held by someone else right now (nothing ran). `with_apply_lock` itself is
/// unchanged. Windows: a no-op like `with_apply_lock`.
pub fn try_with_apply_lock<T>(
    lock_path: &std::path::Path,
    f: impl FnOnce() -> T,
) -> Result<Option<T>, String> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("apply-lock 부모 디렉토리 생성 실패 {}: {e}", parent.display())
            })?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock_path)
            .map_err(|e| format!("apply-lock 열기 실패 {}: {e}", lock_path.display()))?;
        let fd = file.as_raw_fd();
        if unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Ok(None);
            }
            return Err(format!("flock 실패: {e}"));
        }
        let out = f();
        unsafe {
            libc::flock(fd, libc::LOCK_UN);
        }
        Ok(Some(out))
    }
    #[cfg(not(unix))]
    {
        let _ = lock_path;
        Ok(Some(f()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B7: a CLI-side holder and a daemon-side caller exclude each other on the same lock file —
    /// while one process holds it (flock from a child `cys`-like holder: here a second open file
    /// description in another thread), the try-variant refuses and `with_apply_lock` waits.
    #[cfg(unix)]
    #[test]
    fn cli_and_daemon_exclude_each_other_on_the_same_lock_file() {
        use std::sync::mpsc;
        let base = std::env::temp_dir().join(format!(
            "cys-applylock-excl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let pack = base.join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        let lock = pack_apply_lock_path_for(&pack);
        assert_eq!(lock, base.join(".pack-apply.lock"));
        let (held_tx, held_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let l2 = lock.clone();
        let holder = std::thread::spawn(move || {
            with_apply_lock(&l2, || {
                held_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
            .unwrap();
        });
        held_rx.recv().unwrap();
        // a request handler does not block: the lock is busy
        assert_eq!(try_with_apply_lock(&lock, || 7).unwrap(), None);
        // a blocking taker waits until the holder releases
        let (got_tx, got_rx) = mpsc::channel();
        let l3 = lock.clone();
        let waiter = std::thread::spawn(move || {
            with_apply_lock(&l3, || got_tx.send(()).unwrap()).unwrap();
        });
        assert!(got_rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "did not wait");
        release_tx.send(()).unwrap();
        got_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("never got the lock");
        holder.join().unwrap();
        waiter.join().unwrap();
        assert_eq!(try_with_apply_lock(&lock, || 7).unwrap(), Some(7));
        let _ = std::fs::remove_dir_all(&base);
    }
}
