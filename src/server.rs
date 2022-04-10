use crate::{Peer, PeerId};
use std::{
    collections::HashMap,
    error::Error,
    fmt::{Debug, Display, Formatter},
    sync::Arc,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::RwLock,
};
use tracing::info;

pub struct Session<S: AsyncRead + AsyncWrite> {
    id: String,
    /// The peers within this session.
    pub peers: RwLock<HashMap<PeerId, Arc<Peer<S>>>>,
}

impl<S: AsyncRead + AsyncWrite> Session<S> {
    pub fn new(id: String) -> Arc<Self> {
        Arc::new(Self {
            id,
            peers: RwLock::new(HashMap::new()),
        })
    }
}

impl<S: AsyncRead + AsyncWrite> Display for Session<S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.id)
    }
}

pub struct ServerState<S: AsyncRead + AsyncWrite> {
    pub sessions: RwLock<HashMap<String, Arc<Session<S>>>>,
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

impl<S: AsyncRead + AsyncWrite> ServerState<S> {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Removes a peer from its session and purges the session if it becomes empty.
    pub async fn drop_peer(&self, peer: &Arc<Peer<S>>) -> Result<(), ServerError> {
        if let Some(session) = peer.session.read().await.upgrade() {
            let mut peers = session.peers.write().await;
            peers.remove(&peer.id);
            if peers.is_empty() {
                info!("purging empty session `{}`", session);
                self.sessions.write().await.remove(session.id.as_str());
            }
        } else {
            return Err(ServerError::new(format!(
                "{} has no associated session",
                *peer
            )));
        }

        Ok(())
    }
}
