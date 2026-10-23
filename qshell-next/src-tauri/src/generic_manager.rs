use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::Mutex;
use crate::generic_terminal::generic_session::GenericSession;

#[derive(Clone)]
pub struct GenericStreamManager {
    pub sessions: Arc<Mutex<HashMap<String, Arc<GenericSession>>>>,
    instance_counter: Arc<AtomicU64>,
}

impl GenericStreamManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            instance_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn next_instance_id(&self) -> u64 {
        self.instance_counter.fetch_add(1, Ordering::SeqCst)
    }

    pub fn register_session(&self, sid: String, session: GenericSession) -> Arc<GenericSession> {
        let arc_sess = Arc::new(session);
        let mut map = self.sessions.lock();
        map.insert(sid, arc_sess.clone());
        arc_sess
    }

    pub fn cleanup_if_current(&self, sid: &str, instance_id: u64) -> bool {
        let mut map = self.sessions.lock();
        if let Some(existing) = map.get(sid) {
            if existing.instance_id == instance_id {
                map.remove(sid);
                return true;
            }
        }
        false
    }
}
