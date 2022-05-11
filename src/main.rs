#![warn(clippy::pedantic)]
extern crate core;

use crate::{
    codec::{
        ClientMessage, MessageCodec, MessageCodecError, PeerId, ServerMessage, MAX_STATE_SIZE_BYTES,
    },
    peer::Peer,
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
use tokio::{
    io::{AsyncRead, AsyncWrite, BufReader, BufWriter},
    net::TcpListener,
    runtime::Builder,
    select,
    time::interval,
};
use tokio_io_timeout::{TimeoutReader, TimeoutWriter};
use tokio_util::codec::{Decoder, Framed};
use tracing::{error, info, warn};

mod codec;
mod io_util;
mod peer;
mod server;
mod test_stream;

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

    /// Number of worker threads
    #[clap(short, long, default_value = "2")]
    worker_threads: usize,
}

fn main() -> Result<(), Box<dyn Error>> {
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

    info!(
        "Detected {} logical and {} physical CPU(s)",
        num_cpus::get(),
        num_cpus::get_physical()
    );

    if args.worker_threads > num_cpus::get() {
        warn!(
            "Requested number of worker threads ({}) exceeds logical CPU count ({})",
            args.worker_threads,
            num_cpus::get()
        );
    }

    let runtime = Builder::new_multi_thread()
        .worker_threads(args.worker_threads)
        .enable_io()
        .enable_time()
        .build()?;

    let mut handles = Vec::new();

    for addr in addrs {
        let server_state = server_state.clone();
        let handle = runtime.spawn(async move {
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

    runtime
        .block_on(future::try_join_all(handles))
        .expect("failed to run server");

    Ok(())
}

async fn run_server(
    socket_timeout: Duration,
    server_state: Arc<ServerState>,
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
        let peer = Arc::new(Peer::new(peer_id, addr, &server_state));
        let mut connection = Connection::new(peer, create_timeout_stream(socket_timeout, stream));

        tokio::spawn(async move { connection.process().await });
    }
}

fn create_timeout_stream<S: AsyncRead + AsyncWrite>(
    timeout: Duration,
    stream: S,
) -> impl AsyncRead + AsyncWrite {
    let mut timeout_stream = TimeoutReader::new(stream);
    timeout_stream.set_timeout(Some(timeout));
    let reader = BufReader::new(timeout_stream);
    let mut timeout_stream = TimeoutWriter::new(reader);
    timeout_stream.set_timeout(Some(timeout));
    BufWriter::new(timeout_stream)
}

struct Connection<S: AsyncRead + AsyncWrite> {
    peer: Arc<Peer>,
    messages: Pin<Box<Framed<S, MessageCodec>>>,
}

/// Sets the peer's state and notifies other peers in the same session for state broadcasting.
fn set_peer_state(peer: &Arc<Peer>, state: Arc<Vec<u8>>) -> bool {
    if state.len() > MAX_STATE_SIZE_BYTES as usize {
        return false;
    }

    let session = match peer.session.read().clone().upgrade() {
        None => {
            warn!("peer {} has no associated session yet", *peer);
            return true;
        }
        Some(session) => session,
    };

    *peer.state.write() = state;

    for session_peer in session.peers.read().values() {
        if session_peer.id == peer.id {
            // don't mark the peer dirty for itself
            continue;
        }

        session_peer
            .state_dirty
            .write()
            .insert(peer.id, Arc::downgrade(peer));
    }

    true
}

/// Associate a peer with a session.
fn join_peer_to_session(peer: Arc<Peer>, session: &Arc<Session>) -> Result<(), ServerError> {
    if peer.session.read().upgrade().is_some() {
        return Err(ServerError::new(format!(
            "peer {} is already associated with session {}",
            *peer, *session
        )));
    }

    *peer.session.write() = Arc::downgrade(session);
    info!("JOIN {} to {}", *peer, *session);
    session.peers.write().insert(peer.id, peer);
    Ok(())
}

impl<S: AsyncRead + AsyncWrite> Connection<S> {
    pub fn new(peer: Arc<Peer>, stream: S) -> Self {
        Self {
            peer,
            messages: Box::pin(MessageCodec::new().framed(stream)),
        }
    }

    async fn do_deliveries(&mut self) -> Result<(), MessageCodecError> {
        let session = self.peer.session.read().upgrade();
        if session.is_none() {
            return Ok(());
        }

        let full_delivery = self.peer.full_delivery.load(Ordering::Acquire);
        self.peer.full_delivery.store(false, Ordering::Release);
        self.send_states(full_delivery).await?;
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
                            error!("{} message handling failed: {:?}", self.peer, e);
                            break;
                        }
                    },
                    Some(Err(e)) => {
                        error!("{} message retrieval failed: {:?}", self.peer, e);
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
        }

        self.peer.server_state.drop_peer(&self.peer);

        info!("DISCONNECT {}", self.peer);
    }

    async fn handle_message(&mut self, msg: ClientMessage) -> Result<(), MessageCodecError> {
        match msg {
            ClientMessage::Login {
                username,
                auth_token: _,
                session_id,
            } => {
                let session = self.peer.server_state.get_or_create_session(&session_id);
                self.handle_login(username, &session).await?;
            }
            ClientMessage::UpdateState { data } => {
                self.handle_update_state(Arc::new(data)).await?;
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

    async fn send_states(&mut self, full_delivery: bool) -> Result<(), MessageCodecError> {
        let peers = match self.peer.get_out_of_date_peers(full_delivery) {
            Ok(peers) => peers,
            Err(e) => {
                return Err(MessageCodecError::new(e.message));
            }
        };

        self.peer.clear_dirty();

        let mut states: Vec<(PeerId, Arc<Vec<u8>>)> = Vec::new();
        states.reserve(peers.len());

        for (peer_id, peer) in peers {
            assert_ne!(peer_id, self.peer.id);

            let mut hasher = DefaultHasher::new();
            peer.addr.hash(&mut hasher);
            peer.id.hash(&mut hasher);
            states.push((hasher.finish(), peer.state.read().clone()));
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

    async fn handle_update_state(&mut self, state: Arc<Vec<u8>>) -> Result<(), MessageCodecError> {
        if set_peer_state(&self.peer, state) {
            return Ok(());
        }

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

    async fn handle_login(
        &mut self,
        username: String,
        session: &Arc<Session>,
    ) -> Result<(), MessageCodecError> {
        *self.peer.username.write() = Some(username);

        // TODO check auth token
        if true {
            info!("LOGIN {}", self.peer);
            if let Err(e) = join_peer_to_session(self.peer.clone(), session) {
                return Err(MessageCodecError::new(e.message));
            }
            {
                let dirty = match self.peer.get_out_of_date_peers(true) {
                    Ok(dirty) => dirty,
                    Err(e) => return Err(MessageCodecError::new(e.message)),
                };

                let mut write_lock = self.peer.state_dirty.write();
                write_lock.clear();
                for (peer_id, peer) in dirty {
                    write_lock.insert(peer_id, Arc::downgrade(&peer));
                }
            }
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
