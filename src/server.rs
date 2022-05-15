use crate::{Peer, PeerId};
use parking_lot::RwLock;
use std::{
    collections::HashMap,
    error::Error,
    fmt::{Debug, Display, Formatter},
    sync::{atomic::AtomicU64, Arc},
};
use tracing::{info, warn};

pub struct Session {
    id: String,
    /// The peers within this session.
    pub peers: RwLock<HashMap<PeerId, Arc<Peer>>>,
}

impl Session {
    pub fn new(id: String) -> Arc<Self> {
        Arc::new(Self {
            id,
            peers: RwLock::new(HashMap::new()),
        })
    }
}

impl Display for Session {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "`{}`(size {})",
            self.id,
            self.peers.read().len()
        ))
    }
}

pub struct ServerState {
    pub sessions: RwLock<HashMap<String, Arc<Session>>>,
    pub peer_id_counter: AtomicU64,
}

#[derive(Debug)]
pub struct ServerError {
    pub message: String,
}

impl ServerError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl Display for ServerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl Error for ServerError {}

impl ServerState {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            peer_id_counter: AtomicU64::new(0),
        }
    }

    /// Removes a peer from its session and purges the session if it becomes empty.
    pub fn drop_peer(&self, peer: &Arc<Peer>) {
        info!("{} DROP", peer);
        let session = peer.session.read().upgrade();
        if let Some(session) = session {
            let mut peers = session.peers.write();
            peers.remove(&peer.id);
            if peers.is_empty() {
                info!("purging empty session {}", session);
                self.sessions.write().remove(session.id.as_str());
            }
        } else {
            warn!("{} has no associated session", *peer);
        }
    }

    pub fn get_or_create_session(&self, session_id: &str) -> Arc<Session> {
        let mut sessions = self.sessions.write();
        sessions
            .entry(session_id.to_owned())
            .or_insert_with(|| Session::new(session_id.to_owned()))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use crate::{join_peer_to_session, Peer, ServerState};
    use std::sync::Arc;

    #[test]
    fn test_get_or_create_session() {
        let server: Arc<ServerState> = Arc::new(ServerState::new());
        let session_id: String = "abc".into();
        let session = server.get_or_create_session(&session_id);

        assert_eq!(server.sessions.read().len(), 1);
        assert!(server.sessions.read().contains_key(&session_id));
        assert!(Arc::ptr_eq(
            server.sessions.read().get(&session_id).unwrap(),
            &session
        ));

        let existing_session = server.get_or_create_session(&session_id);
        assert_eq!(server.sessions.read().len(), 1);
        assert!(server.sessions.read().contains_key(&session_id));
        assert!(Arc::ptr_eq(
            server.sessions.read().get(&session_id).unwrap(),
            &existing_session
        ));
    }

    #[test]
    fn test_join_peer_to_session() {
        let server: Arc<ServerState> = Arc::new(ServerState::new());
        let addr = "127.0.0.1:1234".parse().unwrap();
        let peer_id = 123u64;
        let peer = Arc::new(Peer::new(peer_id, addr, &server));
        let session_id: String = "abc".into();
        let session = server.get_or_create_session(&session_id);
        join_peer_to_session(peer.clone(), &session).unwrap();

        assert_eq!(server.sessions.read().len(), 1);
        assert!(server.sessions.read().contains_key(&session_id));
        assert!(Arc::ptr_eq(
            server.sessions.read().get(&session_id).unwrap(),
            &session
        ));
        assert_eq!(session.peers.read().len(), 1);
        assert!(Arc::ptr_eq(
            &peer,
            session.peers.read().get(&peer_id).unwrap()
        ));
    }

    #[test]
    fn test_drop_peer() {
        let server: Arc<ServerState> = Arc::new(ServerState::new());
        let addr = "127.0.0.1:1234".parse().unwrap();
        let peer_id = 123u64;
        let peer = Arc::new(Peer::new(peer_id, addr, &server));
        let session_id: String = "abc".into();
        let session = server.get_or_create_session(&session_id);
        join_peer_to_session(peer.clone(), &session).unwrap();
        server.drop_peer(&peer);

        assert!(server.sessions.read().is_empty());
    }
}
