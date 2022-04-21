use crate::{
    codec::PeerId,
    server::{ServerError, Session},
    ServerState,
};
use std::{
    collections::HashMap,
    fmt::{Display, Formatter},
    io,
    net::SocketAddr,
    sync::{atomic::AtomicBool, Arc, Weak},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::RwLock,
    task::block_in_place,
};

#[derive(Debug, Clone)]
pub struct TaggedState {
    /// The current state payload of the peer.
    pub data: Vec<u8>,
    pub tag: String,
}

impl TaggedState {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            tag: String::new(),
        }
    }
}

pub struct Peer<S: AsyncRead + AsyncWrite> {
    pub id: PeerId,
    pub state: RwLock<Arc<TaggedState>>,
    /// The peers which didn't receive the current state yet.
    pub state_dirty: RwLock<HashMap<PeerId, Arc<Peer<S>>>>,
    pub addr: SocketAddr,
    pub server_state: Arc<ServerState<S>>,
    pub full_delivery: AtomicBool,
    pub session: RwLock<Weak<Session<S>>>,
}

impl<S: AsyncRead + AsyncWrite> Display for Peer<S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.addr.fmt(f)
    }
}

impl<S: AsyncRead + AsyncWrite> PartialEq<Self> for Peer<S> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<S: AsyncRead + AsyncWrite> Peer<S> {
    pub async fn new(
        id: PeerId,
        peer_address: SocketAddr,
        server_state: &Arc<ServerState<S>>,
    ) -> io::Result<Arc<Self>> {
        let peer = Arc::new(Self {
            id,
            state: RwLock::new(Arc::new(TaggedState::new())),
            state_dirty: RwLock::new(HashMap::new()),
            addr: peer_address,
            server_state: server_state.clone(),
            full_delivery: AtomicBool::new(false),
            session: RwLock::new(Weak::new()),
        });
        Ok(peer)
    }

    /// Get all peers in the same session of this peer with the same state tag that haven't received
    /// this peer's state yet.
    ///
    /// If `all_session_peers` is `true`, this will return all peers in the same session with the
    /// same state tag, excluding this peer.
    pub async fn get_out_of_date_peers(
        &self,
        all_session_peers: bool,
    ) -> Result<HashMap<PeerId, Arc<Peer<S>>>, ServerError> {
        let session = if let Some(session) = self.session.read().await.upgrade() {
            session
        } else {
            return Err(ServerError::new(format!(
                "{} has no associated session",
                *self
            )));
        };

        if all_session_peers {
            let mut peers = session.peers.read().await.clone();
            peers.remove(&self.id);
            let self_tag = self.state.read().await.tag.clone();
            block_in_place(|| peers.retain(|_, peer| peer.state.blocking_read().tag == self_tag));
            Ok(peers)
        } else {
            Ok(self.state_dirty.read().await.clone())
        }
    }

    /// Returns [`None`] if this peer's state tag doesn't match `tag`, otherwise the state.
    pub async fn state_if_tagged(&self, tag: &String) -> Option<Arc<TaggedState>> {
        let state = self.state.read().await.clone();
        if state.tag == *tag {
            Some(state)
        } else {
            None
        }
    }
}
