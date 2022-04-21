#![warn(clippy::pedantic)]
extern crate core;

use crate::{
    codec::{
        ClientMessage, MessageCodec, MessageCodecError, PeerId, ServerMessage, MAX_STATE_SIZE_BYTES,
    },
    peer::{Peer, TaggedState},
    server::{ServerError, ServerState, Session},
};
use clap::Parser;
use futures::{future, SinkExt, StreamExt};
use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    fmt::Debug,
    hash::{Hash, Hasher},
    net::SocketAddr,
    pin::Pin,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::net::TcpStream;
use tokio::{
    io::{AsyncRead, AsyncWrite, BufReader, BufWriter},
    net::TcpListener,
    select,
    task::yield_now,
    time::interval,
};
use tokio_io_timeout::{TimeoutReader, TimeoutWriter};
use tokio_util::codec::{Decoder, Framed};
use tracing::{error, info, warn};

mod codec;
mod io_util;
mod peer;
mod server;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Listen address
    #[clap(
        short,
        long,
        default_values = &["127.0.0.1:1996", "[::1]:1996"],
        multiple_occurrences(true)
    )]
    listen: Vec<String>,

    /// Socket timeout
    #[clap(short, long, default_value = "5")]
    timeout_seconds: u64,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    let server_state = Arc::new(ServerState::new());
    let addrs: Vec<SocketAddr> = args
        .listen
        .iter()
        .map(|arg| {
            arg.parse()
                .unwrap_or_else(|_| panic!("invalid socket address {}", arg))
        })
        .collect();

    let mut handles = Vec::new();

    for addr in addrs {
        let server_state = server_state.clone();
        let handle = tokio::spawn(async move {
            run_server(
                Duration::from_secs(args.timeout_seconds),
                server_state,
                addr,
            )
            .await
            .expect("failed to run server");
        });
        handles.push(handle);
    }

    future::try_join_all(handles).await?;

    Ok(())
}

async fn run_server(
    socket_timeout: Duration,
    server_state: Arc<ServerState<BufWriter<TimeoutWriter<BufReader<TimeoutReader<TcpStream>>>>>>,
    addr: SocketAddr,
) -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind(addr).await?;

    info!("Listening on {:?}", addr);

    loop {
        let (stream, addr) = listener.accept().await?;
        info!("CONNECT {}", addr);

        let server_state = server_state.clone();
        stream.set_nodelay(true)?;

        let peer_id = server_state.peer_id_counter.fetch_add(1, Ordering::AcqRel);
        let peer = match Peer::new(peer_id, addr, &server_state).await {
            Ok(peer) => peer,
            Err(e) => {
                error!("{} peer creation failed: {:?}", addr, e);
                continue;
            }
        };

        let mut processor =
            Connection::new(peer.clone(), create_timeout_stream(socket_timeout, stream));

        tokio::spawn(async move { processor.process().await });
        yield_now().await;
    }
}

fn create_timeout_stream<S: AsyncRead + AsyncWrite>(
    timeout: Duration,
    stream: S,
) -> BufWriter<TimeoutWriter<BufReader<TimeoutReader<S>>>> {
    let mut timeout_stream = TimeoutReader::new(stream);
    timeout_stream.set_timeout(Some(timeout));
    let reader = BufReader::new(timeout_stream);
    let mut timeout_stream = TimeoutWriter::new(reader);
    timeout_stream.set_timeout(Some(timeout));
    BufWriter::new(timeout_stream)
}

struct Connection<S: AsyncRead + AsyncWrite> {
    peer: Arc<Peer<S>>,
    messages: Pin<Box<Framed<S, MessageCodec>>>,
}

/// Sets the peer's tagged state and notifies other peers in the same session for state broadcasting.
async fn set_peer_state<S: AsyncRead + AsyncWrite>(
    peer: &Arc<Peer<S>>,
    state: Arc<TaggedState>,
) -> Result<bool, ServerError> {
    if state.data.len() > MAX_STATE_SIZE_BYTES as usize {
        return Ok(false);
    }

    let session = match peer.session.read().await.clone().upgrade() {
        None => {
            warn!("peer {} has no associated session yet", *peer);
            return Ok(true);
        }
        Some(session) => session,
    };

    *peer.state.write().await = state;

    let peer_tag = peer.state.read().await.tag.clone();
    for session_peer in session.peers.read().await.values() {
        if session_peer.id == peer.id || session_peer.state.read().await.tag != peer_tag {
            // don't mark the peer dirty for itself or if the peer doesn't have a matching state tag
            continue;
        }

        session_peer
            .state_dirty
            .write()
            .await
            .insert(peer.id, peer.clone());
    }

    Ok(true)
}

/// Associate a peer with a session.
async fn join_peer_to_session<S: AsyncRead + AsyncWrite>(
    peer: Arc<Peer<S>>,
    session: &Arc<Session<S>>,
) -> Result<(), ServerError> {
    if peer.session.read().await.upgrade().is_some() {
        return Err(ServerError::new(format!(
            "peer {} is already associated with session {}",
            *peer, *session
        )));
    }

    *peer.session.write().await = Arc::downgrade(session);
    info!("JOIN {} to {}", *peer, *session);
    session.peers.write().await.insert(peer.id, peer);
    Ok(())
}

impl<S: AsyncRead + AsyncWrite> Connection<S> {
    pub fn new(peer: Arc<Peer<S>>, stream: S) -> Self {
        Self {
            peer,
            messages: Box::pin(MessageCodec::new().framed(stream)),
        }
    }

    async fn do_deliveries(&mut self) -> Result<(), MessageCodecError> {
        let session = self.peer.session.read().await.upgrade();
        if session.is_none() {
            return Ok(());
        }

        let tag = self.peer.state.read().await.tag.clone();
        let full_delivery = self.peer.full_delivery.load(Ordering::Acquire);
        self.peer.full_delivery.store(false, Ordering::Release);
        self.send_states(&tag, full_delivery).await?;
        Ok(())
    }

    async fn process(&mut self) {
        let mut delivery_ticker = interval(Duration::from_millis(1000 / 60));

        loop {
            select! {
                msg = self.messages.next() => match msg {
                    Some(Ok(msg)) => match self.handle_message(msg).await {
                        Ok(_) => {}
                        Err(e) => {
                            error!("message handling failed: {:?}", e);
                            break;
                        }
                    },
                    Some(Err(e)) => {
                        error!("{} {:?}", self.peer, e);
                        break;
                    }
                    None => continue,
                },
                _ = delivery_ticker.tick() => {
                    if let Err(e) = self.do_deliveries().await {
                        error!("{} state delivery failed: {:?}", self.peer, e);
                        break;
                    }
                },
            };

            if let Err(e) = self.messages.flush().await {
                error!("{} flush failed: {:?}", self.peer, e);
                break;
            }
            yield_now().await;
        }

        self.peer.server_state.drop_peer(&self.peer).await;

        info!("DISCONNECT {}", self.peer);
    }

    async fn handle_message(&mut self, msg: ClientMessage) -> Result<(), MessageCodecError> {
        match msg {
            ClientMessage::Login {
                username,
                auth_token: _,
                session_id,
            } => {
                let session = {
                    let mut sessions = self.peer.server_state.sessions.write().await;
                    sessions
                        .entry(session_id.clone())
                        .or_insert_with(|| Session::new(session_id.clone()))
                        .clone()
                };
                self.handle_login(username, &session).await?;
            }
            ClientMessage::UpdateState { tag, data } => {
                self.handle_update_state(Arc::new(TaggedState { data, tag }))
                    .await?;
            }
            ClientMessage::StateQuery {} => {
                self.peer.full_delivery.store(true, Ordering::Release);
            }
            ClientMessage::Failure { message } => {
                return Err(MessageCodecError::new(format!(
                    "{} sent unexpected failure: `{}`",
                    self.peer, message
                )))
            }
        }

        Ok(())
    }

    async fn send_states(
        &mut self,
        tag: &String,
        full_delivery: bool,
    ) -> Result<(), MessageCodecError> {
        let peers = match self.peer.get_out_of_date_peers(full_delivery).await {
            Ok(peers) => peers,
            Err(e) => {
                return Err(MessageCodecError::new(e.message));
            }
        };
        if peers.is_empty() {
            return Ok(());
        }
        self.peer.state_dirty.write().await.clear();

        let mut states: Vec<(PeerId, Arc<TaggedState>)> = Vec::new();
        states.reserve(peers.len());

        for (peer_id, peer) in peers {
            assert_ne!(peer_id, self.peer.id);

            let state = match peer.state_if_tagged(tag).await {
                Some(state) => state,
                None => continue,
            };
            let mut hasher = DefaultHasher::new();
            peer.addr.hash(&mut hasher);
            peer.id.hash(&mut hasher);
            states.push((hasher.finish(), state));
        }

        self.messages
            .send(if full_delivery {
                ServerMessage::FullSync { states }
            } else {
                ServerMessage::UpdateState { states }
            })
            .await?;
        Ok(())
    }

    async fn handle_update_state(
        &mut self,
        state: Arc<TaggedState>,
    ) -> Result<(), MessageCodecError> {
        match set_peer_state(&self.peer, state).await {
            Ok(true) => Ok(()),
            Ok(false) => {
                match self
                    .messages
                    .send(ServerMessage::Failure {
                        message: "failed to process state update".into(),
                    })
                    .await
                {
                    Ok(_) => Ok(()),
                    Err(e) => {
                        error!("failed to send failure message: {:?}", e);
                        Err(e)
                    }
                }
            }
            Err(e) => Err(MessageCodecError::new(e.message)),
        }
    }

    async fn handle_login(
        &mut self,
        username: String,
        session: &Arc<Session<S>>,
    ) -> Result<(), MessageCodecError> {
        *self.peer.username.write().await = Some(username);

        // TODO check auth token
        if true {
            info!("LOGIN {}", self.peer);
            match join_peer_to_session(self.peer.clone(), session).await {
                Ok(_) => {}
                Err(e) => return Err(MessageCodecError::new(e.message)),
            }
            let dirty = match self.peer.get_out_of_date_peers(true).await {
                Ok(dirty) => dirty,
                Err(e) => return Err(MessageCodecError::new(e.message)),
            };
            *self.peer.state_dirty.write().await = dirty;
            self.messages.send(ServerMessage::ServerInfo {}).await?;
            Ok(())
        } else {
            warn!("{} auth failed", self.peer);
            self.messages
                .send(ServerMessage::Failure {
                    message: "authentication failed".into(),
                })
                .await?;
            Err(MessageCodecError::new("invalid user credentials".into()))
        }
    }
}
