use super::*;
use crate::events::EventBus;

fn response(reply: Reply) -> Value {
    match reply { Reply::Single(value) => value, _ => panic!("create error must be a single reply") }
}

#[cfg(unix)]
#[test]
fn typed_allocator_busy_maps_to_distinct_readable_create_error() {
    let bus = EventBus::new(None);
    let message = crate::state::map_pty_open_error(&portable_pty::unix::PtyAllocatorBusy);
    let reply = response(surface_create_error(&bus, &json!(17), &message));
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["id"], 17);
    assert_eq!(reply["error"]["code"], "pty_allocator_busy");
    assert_eq!(reply["error"]["message"], "pty allocator busy");
}

#[test]
fn canonical_busy_error_publishes_exactly_one_finite_event_per_failed_request() {
    let bus = EventBus::new(None);
    let before = bus.latest_seq();
    let _ = surface_create_error(&bus, &json!(1), crate::state::PTY_ALLOCATOR_BUSY_MESSAGE);
    let rows = bus.replay_after(before);
    assert_eq!(rows.len(), 1, "one busy create must publish exactly one row");
    assert_eq!(rows[0]["name"], "surface.pty_allocator_busy");
    assert_eq!(rows[0]["payload"]["reason"], "pty_allocator_busy");
    assert_eq!(rows[0]["payload"]["wait_limit_ms"], 5000);
    assert!(!crate::alert_route::routable("surface.pty_allocator_busy"));
    let _ = surface_create_error(&bus, &json!(2), crate::state::PTY_ALLOCATOR_BUSY_MESSAGE);
    assert_eq!(bus.replay_after(before).len(), 2, "two failed requests are two events, never an internal retry");
}

#[test]
fn ordinary_errors_keep_spawn_failed_and_do_not_publish_busy_event() {
    let bus = EventBus::new(None);
    let before = bus.latest_seq();
    let error = std::io::Error::other("ordinary open failure");
    let message = crate::state::map_pty_open_error(&error);
    assert_eq!(message, "openpty failed: ordinary open failure");
    for message in [message.as_str(), "pty allocator busy extra", "spawn: bad command"] {
        let reply = response(surface_create_error(&bus, &json!(9), message));
        assert_eq!(reply["error"]["code"], "spawn_failed");
        assert_eq!(reply["error"]["message"], message);
    }
    assert!(bus.replay_after(before).is_empty());
}

#[test]
fn ordinary_error_text_cannot_impersonate_typed_allocator_busy() {
    let error = std::io::Error::other(crate::state::PTY_ALLOCATOR_BUSY_MESSAGE);
    assert_eq!(crate::state::map_pty_open_error(&error), "openpty failed: pty allocator busy");
}
