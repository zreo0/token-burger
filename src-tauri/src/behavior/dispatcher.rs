use std::collections::VecDeque;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use super::{tip_window, AgentBehaviorEvent, AgentBehaviorKind, BehaviorTip};

const MAX_QUEUE_LEN: usize = 10;
const MAIN_TRAY_ID: &str = "main";

/// 行为提示队列的展示状态
#[derive(Debug, Default)]
pub struct BehaviorQueue {
    current: Option<BehaviorTip>,
    queue: VecDeque<BehaviorTip>,
}

impl BehaviorQueue {
    /// 处理一个行为事件并返回当前展示状态
    pub fn handle_event(&mut self, event: AgentBehaviorEvent) -> Option<BehaviorTip> {
        match event.kind {
            AgentBehaviorKind::TurnStarted => {
                self.remove_session(&event.agent_name, &event.session_id);
            }
            AgentBehaviorKind::PermissionResolved => {
                self.remove_permission(&event);
            }
            AgentBehaviorKind::RunCompleted | AgentBehaviorKind::RunAborted => {
                self.remove_turn(&event);
                self.upsert_tip(event.into());
            }
            AgentBehaviorKind::PermissionRequested | AgentBehaviorKind::ToolError => {
                self.upsert_tip(event.into());
            }
        }

        self.current.clone()
    }

    /// 手动或自动关闭当前提示并返回下一条
    pub fn close_current(&mut self) -> Option<BehaviorTip> {
        self.current = self.queue.pop_front();
        self.current.clone()
    }

    /// 按 key 关闭提示，用于自动隐藏计时器精确命中
    pub fn close_key(&mut self, key: &str) -> Option<BehaviorTip> {
        if self.current.as_ref().is_some_and(|tip| tip.key == key) {
            return self.close_current();
        }

        self.queue.retain(|tip| tip.key != key);
        self.current.clone()
    }

    /// 读取当前提示
    #[cfg(test)]
    pub fn current(&self) -> Option<BehaviorTip> {
        self.current.clone()
    }

    /// 清空所有提示
    pub fn clear(&mut self) {
        self.current = None;
        self.queue.clear();
    }

    fn upsert_tip(&mut self, tip: BehaviorTip) {
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.key == tip.key)
        {
            self.current = Some(tip);
            return;
        }

        if let Some(existing) = self
            .queue
            .iter_mut()
            .find(|existing| existing.key == tip.key)
        {
            *existing = tip;
            return;
        }

        if self.current.is_none() {
            self.current = Some(tip);
            return;
        }

        self.queue.push_back(tip);
        while self.queue.len() > MAX_QUEUE_LEN {
            self.queue.pop_front();
        }
    }

    fn remove_session(&mut self, agent_name: &str, session_id: &str) {
        if self
            .current
            .as_ref()
            .is_some_and(|tip| tip.agent_name == agent_name && tip.session_id == session_id)
        {
            self.current = self.queue.pop_front();
        }

        self.queue
            .retain(|tip| !(tip.agent_name == agent_name && tip.session_id == session_id));
    }

    fn remove_turn(&mut self, event: &AgentBehaviorEvent) {
        if self
            .current
            .as_ref()
            .is_some_and(|tip| same_turn(tip, event))
        {
            self.current = self.queue.pop_front();
        }

        self.queue.retain(|tip| !same_turn(tip, event));
    }

    fn remove_permission(&mut self, event: &AgentBehaviorEvent) {
        if self
            .current
            .as_ref()
            .is_some_and(|tip| same_permission(tip, event))
        {
            self.current = self.queue.pop_front();
        }

        self.queue.retain(|tip| !same_permission(tip, event));
    }
}

/// 行为提示分发器，负责后端队列与窗口展示
pub struct BehaviorDispatcher {
    app: AppHandle,
    queue: Mutex<BehaviorQueue>,
    tray_rect: Mutex<Option<tip_window::TrayRect>>,
}

impl BehaviorDispatcher {
    /// 创建新的行为提示分发器
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            queue: Mutex::new(BehaviorQueue::default()),
            tray_rect: Mutex::new(None),
        }
    }

    /// 缓存最近一次主托盘位置
    pub fn cache_tray_rect(&self, rect: &tauri::Rect) {
        if let Ok(mut cached) = self.tray_rect.lock() {
            *cached = Some(tip_window::TrayRect::from_tauri_rect(rect));
        }
    }

    /// 处理行为事件并同步提示窗口
    pub fn handle_event(&self, event: AgentBehaviorEvent) {
        let current = {
            let Ok(mut queue) = self.queue.lock() else {
                return;
            };
            queue.handle_event(event)
        };

        self.sync_window(current);
    }

    /// 关闭当前提示
    pub fn close_current(&self) {
        let current = {
            let Ok(mut queue) = self.queue.lock() else {
                return;
            };
            queue.close_current()
        };

        self.sync_window(current);
    }

    /// 获取当前提示快照
    pub fn current_tip(&self) -> Option<BehaviorTip> {
        self.queue
            .lock()
            .ok()
            .and_then(|queue| queue.current.clone())
    }

    /// 清空提示队列并隐藏窗口
    pub fn clear(&self) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.clear();
        }
        let _ = tip_window::hide_tip_window(&self.app);
    }

    fn close_key(&self, key: &str) {
        let current = {
            let Ok(mut queue) = self.queue.lock() else {
                return;
            };
            queue.close_key(key)
        };

        self.sync_window(current);
    }

    fn sync_window(&self, current: Option<BehaviorTip>) {
        match current {
            Some(tip) => {
                let rect = self.current_tray_rect();
                if tip_window::show_tip_window(&self.app, rect, &tip).is_ok() {
                    self.start_auto_hide_timer(&tip);
                }
            }
            None => {
                let _ = tip_window::hide_tip_window(&self.app);
            }
        }
    }

    fn current_tray_rect(&self) -> Option<tip_window::TrayRect> {
        if let Some(rect) = self.read_main_tray_rect() {
            return Some(rect);
        }

        self.tray_rect.lock().ok().and_then(|rect| *rect)
    }

    fn read_main_tray_rect(&self) -> Option<tip_window::TrayRect> {
        let rect = self.app.tray_by_id(MAIN_TRAY_ID)?.rect().ok().flatten()?;
        let rect = tip_window::TrayRect::from_tauri_rect(&rect);
        if let Ok(mut cached) = self.tray_rect.lock() {
            *cached = Some(rect);
        }

        Some(rect)
    }

    fn start_auto_hide_timer(&self, tip: &BehaviorTip) {
        let Some(timeout_ms) = tip.auto_hide_ms else {
            return;
        };
        let key = tip.key.clone();
        let app = self.app.clone();

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(timeout_ms));
            let Some(state) = app.try_state::<crate::commands::AppState>() else {
                return;
            };
            state.behavior.close_key(&key);
        });
    }
}

fn same_turn(tip: &BehaviorTip, event: &AgentBehaviorEvent) -> bool {
    if tip.agent_name != event.agent_name || tip.session_id != event.session_id {
        return false;
    }

    event.turn_id.is_none() || tip.turn_id == event.turn_id
}

fn same_permission(tip: &BehaviorTip, event: &AgentBehaviorEvent) -> bool {
    if tip.agent_name != event.agent_name || tip.session_id != event.session_id {
        return false;
    }

    tip.call_id.is_some()
        && tip.call_id == event.call_id
        && (event.turn_id.is_none() || tip.turn_id == event.turn_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        kind: AgentBehaviorKind,
        session_id: &str,
        turn_id: Option<&str>,
        call_id: Option<&str>,
    ) -> AgentBehaviorEvent {
        AgentBehaviorEvent::new(
            "codex",
            session_id,
            kind,
            "2026-06-01T10:00:00Z",
            turn_id.map(ToString::to_string),
            call_id.map(ToString::to_string),
            "summary",
        )
    }

    #[test]
    fn queues_multiple_sessions_and_keeps_one_current() {
        let mut queue = BehaviorQueue::default();
        queue.handle_event(event(
            AgentBehaviorKind::PermissionRequested,
            "session-1",
            Some("turn-1"),
            Some("call-1"),
        ));
        queue.handle_event(event(
            AgentBehaviorKind::RunCompleted,
            "session-2",
            Some("turn-2"),
            None,
        ));

        assert_eq!(queue.current().unwrap().session_id, "session-1");
        assert_eq!(queue.queue.len(), 1);
        assert_eq!(queue.close_current().unwrap().session_id, "session-2");
    }

    #[test]
    fn drops_oldest_when_queue_exceeds_limit() {
        let mut queue = BehaviorQueue::default();
        queue.handle_event(event(
            AgentBehaviorKind::PermissionRequested,
            "current",
            Some("turn-current"),
            Some("call-current"),
        ));

        for idx in 0..11 {
            queue.handle_event(event(
                AgentBehaviorKind::PermissionRequested,
                &format!("session-{idx}"),
                Some("turn"),
                Some("call"),
            ));
        }

        assert_eq!(queue.queue.len(), 10);
        assert_eq!(queue.queue.front().unwrap().session_id, "session-1");
    }

    #[test]
    fn turn_started_clears_only_same_session() {
        let mut queue = BehaviorQueue::default();
        queue.handle_event(event(
            AgentBehaviorKind::PermissionRequested,
            "session-1",
            Some("turn-1"),
            Some("call-1"),
        ));
        queue.handle_event(event(
            AgentBehaviorKind::PermissionRequested,
            "session-2",
            Some("turn-2"),
            Some("call-2"),
        ));

        queue.handle_event(event(
            AgentBehaviorKind::TurnStarted,
            "session-1",
            Some("turn-3"),
            None,
        ));

        assert_eq!(queue.current().unwrap().session_id, "session-2");
    }

    #[test]
    fn permission_resolved_removes_matching_call() {
        let mut queue = BehaviorQueue::default();
        queue.handle_event(event(
            AgentBehaviorKind::PermissionRequested,
            "session-1",
            Some("turn-1"),
            Some("call-1"),
        ));
        queue.handle_event(event(
            AgentBehaviorKind::PermissionResolved,
            "session-1",
            Some("turn-1"),
            Some("call-1"),
        ));

        assert!(queue.current().is_none());
    }

    #[test]
    fn permission_resolved_can_match_without_turn_id() {
        let mut queue = BehaviorQueue::default();
        queue.handle_event(event(
            AgentBehaviorKind::PermissionRequested,
            "session-1",
            Some("turn-1"),
            Some("call-1"),
        ));
        queue.handle_event(event(
            AgentBehaviorKind::PermissionResolved,
            "session-1",
            None,
            Some("call-1"),
        ));

        assert!(queue.current().is_none());
    }
}
