use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CopilotTurn {
    pub conversation_id: String,
    pub client_session_id: String,
    pub is_start_of_session: bool,
}

#[derive(Debug, Clone)]
pub struct PersistentSession {
    pub conversation_id: String,
    pub client_session_id: String,
    pub turn_count: u64,
    pub lock: Arc<Mutex<()>>,
}

impl PersistentSession {
    pub fn new() -> Self {
        Self {
            conversation_id: Uuid::new_v4().to_string(),
            client_session_id: Uuid::new_v4().to_string(),
            turn_count: 0,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn reserve_turn(&mut self) -> CopilotTurn {
        let turn = CopilotTurn {
            conversation_id: self.conversation_id.clone(),
            client_session_id: self.client_session_id.clone(),
            is_start_of_session: self.turn_count == 0,
        };
        self.turn_count += 1;
        turn
    }
}

#[derive(Debug)]
pub struct PersistentSessionStore {
    sessions: Mutex<HashMap<String, PersistentSession>>,
}

impl PersistentSessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, key: &str) -> PersistentSession {
        let mut map = self.sessions.lock().unwrap();
        map.entry(key.to_owned()).or_insert_with(PersistentSession::new).clone()
    }
}
