use crate::{
    codec::PeerId,
    server::{ServerError, Session},
    ServerState,
};
use parking_lot::RwLock;
use std::{
    collections::HashMap,
    fmt::{Display, Formatter},
    net::SocketAddr,
    sync::{atomic::AtomicBool, Arc, Weak},
};

pub struct Peer {
    pub id: PeerId,
    pub username: RwLock<Option<String>>,
    pub state: RwLock<Option<Arc<Vec<u8>>>>,
    /// The peers which didn't receive the current state yet.
    pub state_dirty: RwLock<HashMap<PeerId, Weak<Peer>>>,
    pub addr: SocketAddr,
    pub server_state: Arc<ServerState>,
    pub full_delivery: AtomicBool,
    pub session: RwLock<Weak<Session>>,
}

impl Display for Peer {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if let Some(username) = self.username.read().clone() {
            f.write_fmt(format_args!("{}(#{} `{}`)", self.addr, self.id, username))
        } else {
            f.write_fmt(format_args!("{}(#{})", self.addr, self.id))
        }
    }
}

impl PartialEq<Self> for Peer {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Peer {
    pub fn new(id: PeerId, peer_address: SocketAddr, server_state: &Arc<ServerState>) -> Self {
        Self {
            id,
            username: RwLock::new(None),
            state: RwLock::new(None),
            state_dirty: RwLock::new(HashMap::new()),
            addr: peer_address,
            server_state: server_state.clone(),
            full_delivery: AtomicBool::new(false),
            session: RwLock::new(Weak::new()),
        }
    }

    /// Get all peers in the same session of this peer that haven't received this peer's state yet.
    ///
    /// If `all_session_peers` is `true`, this will return all peers in the same session, excluding
    /// this peer.
    pub fn get_out_of_date_peers(
        &self,
        all_session_peers: bool,
    ) -> Result<HashMap<PeerId, Arc<Peer>>, ServerError> {
        let session = if let Some(session) = self.session.read().upgrade() {
            session
        } else {
            return Err(ServerError::new(format!(
                "{} has no associated session",
                *self
            )));
        };

        if all_session_peers {
            let mut peers = session.peers.read().clone();
            peers.remove(&self.id);
            Ok(peers)
        } else {
            let peers = self.state_dirty.read().clone();
            let mut result = HashMap::new();
            for (peer_id, peer) in peers {
                if let Some(peer) = peer.upgrade() {
                    result.insert(peer_id, peer);
                }
            }
            Ok(result)
        }
    }

    pub fn clear_dirty(&self) {
        self.state_dirty.write().clear();
    }
}
