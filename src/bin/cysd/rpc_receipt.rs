//! Application full-frame receipt / expiry ordering.
//! No caller, binding, writer or submission authority is issued by this module.
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

#[derive(Default)]
pub(crate) struct ReceiptRegistry {
    state: Mutex<State>,
    hooks: ReceiptTestHooks,
}
#[derive(Default)]
struct State {
    unparsed: Vec<(Weak<FrameIdentity>, Instant)>,
}
struct FrameIdentity;

/// Exists only between full-frame observation and synchronous parse/routing.
/// !Send prevents a spawned connection future from retaining this GLOBAL barrier
/// over an await. It must narrow/release before the dispatch semaphore wait.
#[must_use]
pub(crate) struct PendingFrame {
    registry: Arc<ReceiptRegistry>,
    identity: Arc<FrameIdentity>,
    at: Instant,
    registered: bool,
    _synchronous: PhantomData<Rc<()>>,
}

/// A timestamp plus optional TARGET-LOCAL residency guards. Neither grants execution.
#[must_use]
pub(crate) struct ReceivedFrame<T> {
    at: Instant,
    retained: T,
}
impl<T> ReceivedFrame<T> {
    pub(crate) fn at(&self) -> Instant { self.at }
    pub(crate) fn into_parts(self) -> (Instant, T) { (self.at, self.retained) }
}

impl ReceiptRegistry {
    pub(crate) fn note_narrowed(&self, id: &serde_json::Value, at: Instant) { self.hooks.note("narrowed", id, at); }
    pub(crate) fn note_dispatch_started(&self, id: &serde_json::Value, at: Instant) { self.hooks.started(id, at); }
    pub(crate) fn note_dispatch_finished(&self, id: &serde_json::Value, at: Instant) { self.hooks.note("finished", id, at); }
    /// Called INSIDE the framer immediately after fill_buf, with a bounded in-memory
    /// newline/EOF classifier. Clock and full-frame recognition share the expiry lock.
    /// Never call after a completed frame has already escaped the framer.
    pub(crate) fn recognize<C>(self: &Arc<Self>, classify: impl FnOnce() -> Option<C>)
        -> Option<(C, PendingFrame)>
    {
        self.recognize_with(|| self.hooks.now(), classify)
    }

    fn recognize_with<C>(self: &Arc<Self>, clock: impl FnOnce() -> Instant,
        classify: impl FnOnce() -> Option<C>) -> Option<(C, PendingFrame)>
    {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let at = clock();
        let complete = classify()?;
        let identity = Arc::new(FrameIdentity);
        state.unparsed.push((Arc::downgrade(&identity), at));
        Some((complete, PendingFrame { registry: self.clone(), identity, at,
            registered: true, _synchronous: PhantomData }))
    }

    /// Lock order: registry -> association table. Callers must hold no Surface map,
    /// native/input/store/table lock before entering. Closure performs only bounded
    /// in-memory expiry; never IO, discovery, waiting or event publication.
    pub(crate) fn with_expiry_clock<T>(&self, now: Instant, expire: impl FnOnce(Instant) -> T) -> T {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let cutoff = state.unparsed.iter().map(|(_, at)| *at).min().map_or(now, |at| at.min(now));
        expire(cutoff)
    }
}
impl PendingFrame {
    pub(crate) fn at(&self) -> Instant { self.at }

    /// Resolve the target outside this lock while PendingFrame remains live. Then
    /// acquire its association table inside this closure, retain eligible original
    /// entries, and remove the global barrier atomically before releasing the lock.
    pub(crate) fn narrow<T>(mut self, retain: impl FnOnce(Instant) -> T) -> ReceivedFrame<T> {
        let registry = self.registry.clone();
        let mut state = registry.state.lock().unwrap_or_else(|e| e.into_inner());
        let retained = retain(self.at);
        self.remove(&mut state);
        drop(state);
        ReceivedFrame { at: self.at, retained }
    }
    pub(crate) fn release(self) -> ReceivedFrame<()> { self.narrow(|_| ()) }

    fn remove(&mut self, state: &mut State) {
        let key = Arc::downgrade(&self.identity);
        state.unparsed.retain(|(id, _)| !Weak::ptr_eq(id, &key));
        self.registered = false;
    }
}
impl Drop for PendingFrame {
    fn drop(&mut self) {
        if self.registered {
            let registry = self.registry.clone();
            let mut state = registry.state.lock().unwrap_or_else(|e| e.into_inner());
            self.remove(&mut state);
        }
    }
}

// All production callbacks inline to clock/no-op; test controls contain no admission/native authority.
#[cfg(not(test))]
#[derive(Default)]
struct ReceiptTestHooks;
#[cfg(not(test))]
impl ReceiptTestHooks {
    #[inline] fn now(&self) -> Instant { Instant::now() }
    #[inline] fn note(&self, _: &'static str, _: &serde_json::Value, _: Instant) {}
    #[inline] fn started(&self, _: &serde_json::Value, _: Instant) {}
}

#[cfg(test)]
#[derive(Default)]
struct ReceiptTestHooks {
    clock: Mutex<Option<Instant>>,
    observer: Mutex<Option<std::sync::mpsc::Sender<TestCheckpoint>>>,
    pause: Mutex<Option<(serde_json::Value, std::sync::mpsc::Receiver<()>)>>,
}
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct TestCheckpoint { pub kind: &'static str, pub id: serde_json::Value, pub at: Instant }
#[cfg(test)]
impl ReceiptTestHooks {
    fn now(&self) -> Instant { self.clock.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or_else(Instant::now) }
    fn note(&self, kind: &'static str, id: &serde_json::Value, at: Instant) {
        let tx = self.observer.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(tx) = tx { let _ = tx.send(TestCheckpoint { kind, id: id.clone(), at }); }
    }
    fn started(&self, id: &serde_json::Value, at: Instant) {
        self.note("started", id, at);
        let pause = {
            let mut held = self.pause.lock().unwrap_or_else(|e| e.into_inner());
            if held.as_ref().is_some_and(|(expected, _)| expected == id) { held.take() } else { None }
        };
        if let Some((_, rx)) = pause {
            rx.recv_timeout(std::time::Duration::from_secs(10)).expect("test dispatch barrier was not released");
        }
    }
}
#[cfg(test)]
impl ReceiptRegistry {
    pub(crate) fn test_clock(&self, at: Option<Instant>) { *self.hooks.clock.lock().unwrap() = at; }
    pub(crate) fn test_observe(&self, tx: std::sync::mpsc::Sender<TestCheckpoint>) { *self.hooks.observer.lock().unwrap() = Some(tx); }
    pub(crate) fn test_pause_dispatch(&self, id: serde_json::Value, rx: std::sync::mpsc::Receiver<()>) {
        *self.hooks.pause.lock().unwrap() = Some((id, rx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn unparsed_receipt_defers_expiry_but_release_does_not_renew_its_clock() {
        let registry = Arc::new(ReceiptRegistry::default());
        let at = Instant::now();
        let late = at + Duration::from_secs(61);
        let frame = registry.recognize_with(|| at, || Some(())).unwrap().1;
        assert_eq!(registry.with_expiry_clock(late, |t| t), at, "unparsed frame must hold the expiry cutoff");
        let received = frame.release();
        assert_eq!(received.at(), at);
        assert_eq!(registry.with_expiry_clock(late, |t| t), late);
    }
    #[test]
    fn narrowing_installs_local_retention_before_removing_the_global_barrier() {
        let registry = Arc::new(ReceiptRegistry::default());
        let at = Instant::now();
        let token = Arc::new(());
        let frame = registry.recognize_with(|| at, || Some(())).unwrap().1;
        let received = frame.narrow(|stamp| { assert_eq!(stamp, at); token.clone() });
        assert_eq!(Arc::strong_count(&token), 2);
        let late = at + Duration::from_secs(61);
        assert_eq!(registry.with_expiry_clock(late, |t| t), late, "narrowing must release the global expiry barrier");
        drop(received);
        assert_eq!(Arc::strong_count(&token), 1);
    }
    #[test]
    fn parse_or_narrowing_unwind_releases_the_global_barrier_even_after_poison() {
        let registry = Arc::new(ReceiptRegistry::default());
        let at = Instant::now();
        let result = std::panic::catch_unwind({ let registry = registry.clone(); move || {
            let frame = registry.recognize_with(|| at, || Some(())).unwrap().1;
            let _ = frame.narrow::<()>(|_| panic!("fixture parse/routing failure"));
        }});
        assert!(result.is_err());
        let later = at + Duration::from_secs(61);
        assert_eq!(registry.with_expiry_clock(later, |t| t), later);
        let frame = registry.recognize_with(|| later, || Some(())).unwrap().1;
        assert_eq!(registry.with_expiry_clock(later + Duration::from_secs(1), |t| t), later);
        drop(frame);
        assert_eq!(registry.with_expiry_clock(later + Duration::from_secs(1), |t| t), later + Duration::from_secs(1));
    }
    #[test]
    fn two_frames_release_independently_in_either_order() {
        let registry = Arc::new(ReceiptRegistry::default());
        let a = Instant::now(); let b = a + Duration::from_secs(1); let end = b + Duration::from_secs(61);
        let first = registry.recognize_with(|| a, || Some(())).unwrap().1; let second = registry.recognize_with(|| b, || Some(())).unwrap().1;
        assert_eq!(registry.with_expiry_clock(end, |t| t), a);
        drop(first);
        assert_eq!(registry.with_expiry_clock(end, |t| t), b);
        drop(second);
        assert_eq!(registry.with_expiry_clock(end, |t| t), end);
    }
}
