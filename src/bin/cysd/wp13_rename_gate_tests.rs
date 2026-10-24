// Included INSIDE handlers::tests (`include!` at the end of that module).
//
// ★WP13(0.14.53 · slot 13 · 헤드 결정 (2) 2026-10-24) — **M4 가 살아남은 자리의 검체**다.
//   측정(worker-9 · 2026-10-24): `handlers.rs` 의 `surface.rename` 거부 분기 897 B 를 **통째로 지워도**
//   cysd 전 검체(2,826)가 아무도 적색이 되지 않았다 ⇒ 덮고 있던 것은 `cys::title` 의 **순수 규칙**뿐이고
//   데몬 쪽 **호출 자리**(정화·신원 게이트)는 어떤 단언도 걸려 있지 않았다. 이 파일이 그 변이를 죽인다.
//   ⚠PTY 를 띄우므로 `cfg(unix)` 다 — 윈도우에서는 **존재하지 않는다**(`ignore` 속성이 아니라서
//   governance.rs 의 윈도우 `ignore` 전수 census 52건을 건드리지 않는다).
#[cfg(unix)]
mod wp13_rename_gate {
    use super::*;

    /// 좌석을 PTY 로 띄우고 **반드시 거둔다** — 검체가 끝나도 `sleep 30` 이 남으면 다음 검체의 환경이다.
    struct Rig {
        daemon: Arc<Daemon>,
        seats: Vec<Arc<crate::state::Surface>>,
    }
    impl Drop for Rig {
        fn drop(&mut self) {
            for seat in &self.seats {
                let mut child = seat.child.lock().unwrap_or_else(|e| e.into_inner());
                if matches!(child.try_wait(), Ok(None)) {
                    let _ = child.kill();
                }
            }
        }
    }

    fn rig(tag: &str) -> Rig {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cys-wp13-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).expect("시험 자리");
        Rig { daemon: Daemon::new(dir.join("cysd.sock")), seats: Vec::new() }
    }

    fn seat(rig: &mut Rig, role: &str) -> Arc<crate::state::Surface> {
        let seat = rig
            .daemon
            .create_surface(None, Some("sleep 30".into()), None, Some(role.to_string()), 24, 80)
            .expect("좌석");
        rig.daemon.surfaces.lock().unwrap().insert(seat.id, seat.clone());
        rig.seats.push(seat.clone());
        seat
    }

    /// 데몬의 **실제 처리 경로**로 보낸다(`handlers::dispatch` = 이미 파싱된 in-process 호출자용 입구).
    fn rename(daemon: &Arc<Daemon>, target: u64, title: &str, caller_pid: Option<u32>) -> Value {
        let req = Request {
            id: json!("wp13-rename"),
            method: "surface.rename".into(),
            params: json!({"surface_id": target, "title": title}),
        };
        let Reply::Single(resp) = dispatch(daemon, req, caller_pid) else {
            panic!("surface.rename 은 단일 응답이다")
        };
        resp
    }

    fn denied_events(daemon: &Arc<Daemon>) -> usize {
        daemon.bus.tail(300).iter().filter(|e| e["name"] == "surface.rename_denied").count()
    }

    #[test]
    fn wp13_surface_rename_refuses_a_foreign_seat_at_the_daemon() {
        let mut r = rig("gate");
        let caller = seat(&mut r, "worker-9");
        let target = seat(&mut r, "worker-7");
        *target.title.lock().unwrap() = "target's own title".to_string();

        // 발신 pane 해소: `resolve_caller_surface` 가 문서화한 **합성 주입**(start_time=None ⇒ 캐시 신뢰)으로
        // 이 pid 를 호출자 좌석에 묶는다. 커널 peer pid 를 흉내내는 것이 아니라 그 해소 결과를 고정하는 것이다.
        let pid = 900_013_u32;
        r.daemon.caller_cache.lock().unwrap().insert(
            pid,
            crate::state::CallerCacheEntry::new(Some(caller.id), crate::state::now_epoch(), None, 0),
        );

        // ① 남의 좌석 → 거부. 이름 있는 코드 · 버스 이벤트 · **제목 무변경** 셋이 다 단언된다.
        let before = denied_events(&r.daemon);
        let resp = rename(&r.daemon, target.id, "stolen", Some(pid));
        assert_eq!(resp["ok"], json!(false), "워커가 남의 좌석 이름을 바꾸는 것은 거부다: {resp}");
        assert_eq!(resp["error"]["code"], json!("rename_denied"), "거부는 이름 있는 코드다: {resp}");
        assert_eq!(*target.title.lock().unwrap(), "target's own title", "거부된 이름 변경은 제목을 쓰지 않는다");
        assert_eq!(denied_events(&r.daemon), before + 1, "거부는 surface.rename_denied 를 발행한다");

        // ② 자기 자신 → 통과(같은 경로의 양성 대조 — ①이 '전부 거부' 로 통과하지 않게 한다).
        let resp = rename(&r.daemon, caller.id, "my own tab", Some(pid));
        assert_eq!(resp["ok"], json!(true), "좌석은 자기 이름을 바꿀 수 있다: {resp}");
        assert_eq!(*caller.title.lock().unwrap(), "my own tab");

        // ③ master → 이 데몬의 어느 좌석이든 통과(헤드 결정 Q1 의 세 번째 가지).
        *caller.role.lock().unwrap() = Some("master".to_string());
        let resp = rename(&r.daemon, target.id, "head renamed it", Some(pid));
        assert_eq!(resp["ok"], json!(true), "master 는 이 데몬의 어느 좌석이든 바꾼다: {resp}");
        assert_eq!(*target.title.lock().unwrap(), "head renamed it");

        // ④ 정화도 이 경로에 걸려 있다 — 제어문자는 버리고(통과) 길이는 거절한다(쓰지 않는다).
        let resp = rename(&r.daemon, target.id, "a\nb\tc", Some(pid));
        assert_eq!(resp["ok"], json!(true), "{resp}");
        assert_eq!(*target.title.lock().unwrap(), "abc", "제어문자는 데몬 경로에서도 버린다");
        let resp = rename(&r.daemon, target.id, &"x".repeat(121), Some(pid));
        assert_eq!(resp["ok"], json!(false), "121자는 거절이다(자르지 않는다): {resp}");
        assert_eq!(resp["error"]["code"], json!("invalid_params"), "{resp}");
        assert_eq!(*target.title.lock().unwrap(), "abc", "거절된 제목은 쓰이지 않는다");
    }
}
