use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::Mutex;
use crate::generic_terminal::generic_session::GenericSession;

#[derive(Clone)]
pub struct GenericStreamManager {
    pub sessions: Arc<Mutex<HashMap<String, Arc<GenericSession>>>>,
    /// 「建连过程中就被关闭」的会话 id，注册时对照一次即可丢弃。
    /// 锁顺序统一为 sessions → pending_cancel，与 close_generic_session 保持一致。
    pub pending_cancel: Arc<Mutex<Vec<String>>>,
    instance_counter: Arc<AtomicU64>,
}

impl GenericStreamManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            pending_cancel: Arc::new(Mutex::new(Vec::new())),
            instance_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn next_instance_id(&self) -> u64 {
        self.instance_counter.fetch_add(1, Ordering::SeqCst)
    }

    /// 注册会话。返回 None 表示前端已在建连期间关闭该标签，
    /// 调用方必须放弃启动读线程（连接/子进程由调用方自行释放）。
    pub fn register_session(&self, sid: String, session: GenericSession) -> Option<Arc<GenericSession>> {
        let arc_sess = Arc::new(session);
        let previous = {
            let mut map = self.sessions.lock();
            let mut cancelled = self.pending_cancel.lock();
            if let Some(pos) = cancelled.iter().position(|id| id == &sid) {
                cancelled.remove(pos);
                return None;
            }
            map.insert(sid, arc_sess.clone())
        };
        // 同一个 id 重连时，旧实例的读线程与本地子进程必须主动结束：
        // 只把结构体替换掉的话，本地终端会留下一个仍在跑的 PowerShell/CMD，
        // Telnet/Raw 的读线程也要等对端超时才会退出。
        if let Some(previous) = previous {
            previous.stop();
        }
        Some(arc_sess)
    }

    /// 会话尚未注册时登记墓碑，等建连线程完成后丢弃。
    pub fn cancel_pending(&self, sid: String) {
        let mut cancelled = self.pending_cancel.lock();
        if !cancelled.iter().any(|id| id == &sid) {
            cancelled.push(sid);
            // 上限保护：正常路径下建连线程会及时取走，超出说明有异常残留
            if cancelled.len() > 128 {
                cancelled.remove(0);
            }
        }
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
