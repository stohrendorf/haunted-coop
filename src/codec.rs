use crate::io_util::{ReadPascalExt, WritePascalExt};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use bytes::{buf::Reader, Buf, BufMut, BytesMut};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use std::{
    error::Error,
    fmt::{Debug, Display, Formatter},
    io::ErrorKind,
    sync::Arc,
};
use tokio_util::codec::{Decoder, Encoder};

/// The peer identifier.
pub type PeerId = u64;

/// The current message protocol version implemented by this server.
pub const PROTOCOL_VERSION: u16 = 0;

/// The maximum state data size, see [`ClientMessageTypeId::UpdateState`] and
/// [`ServerMessageTypeId::UpdateState`].
pub const MAX_STATE_SIZE_BYTES: u16 = 5000;

/// Client message IDs. Prefixed to every message sent from the client.
#[derive(Debug, Eq, PartialEq, TryFromPrimitive, IntoPrimitive)]
#[repr(u8)]
pub enum ClientMessageTypeId {
    /// # Description
    /// Login request. Must be the first message sent from the client.
    ///
    /// If the user can be authenticated, authorized and is allowed to join the session identified
    /// by `session_id`, the server sends a [`ServerMessageTypeId::ServerInfo`]. Otherwise, the
    /// server sends a [`ServerMessageTypeId::Failure`].
    ///
    /// # Body
    /// - `username: PString`
    /// - `auth_token: PString` - a revocable token used to authenticate and authorize the user
    /// - `session_id: PString` - the session the user wants to join
    Login = 0,

    /// # Description
    /// Client state to be broadcast to other peers in the same session.
    ///
    /// The server stores the sent data, existing state is replaced. All other peers in the same
    /// session as the client sending the message are marked to have the data delivered as a
    /// response to a [`ClientMessageTypeId::StateQuery`] message.
    ///
    /// May emit a [`ServerMessageTypeId::Failure`].
    ///
    /// # Body
    /// - `data: PBuffer` - the state to be broadcast
    UpdateState = 1,

    /// # Description
    /// Request to deliver all known states in the session. The response is a
    /// [`ServerMessageTypeId::FullSync`].
    ///
    /// # Body
    /// Empty
    StateQuery = 2,

    /// # Description
    /// Client error with a message containing diagnostic information for the server. The client
    /// may or may not close the connection after this message.
    ///
    /// # Body
    /// - `message: PString`
    Failure = 3,
}

/// Server message IDs. Prefixed to every message sent from the server.
#[derive(Debug, Eq, PartialEq, IntoPrimitive)]
#[repr(u8)]
pub enum ServerMessageTypeId {
    /// # Description
    /// Response for a successful [`ClientMessageTypeId::Login`]. Contains information about
    /// the server. End of the conversation.
    ///
    /// Marks the end of the "Login" conversation. Expected messages from now on are
    /// [`ClientMessageTypeId::UpdateState`] or [`ClientMessageTypeId::StateQuery`].
    ///
    /// # Body
    /// - `protocol_version: u16` - see [`PROTOCOL_VERSION`]
    /// - `max_state_size_bytes: u16` - maximum size of the `PBuffer` payload allowed in
    ///   [`ServerMessageTypeId::UpdateState`] or [`ClientMessageTypeId::UpdateState`] messages,
    ///   see [`MAX_STATE_SIZE_BYTES`]
    ServerInfo = 0,

    /// # Description
    /// An error message for the client. The connection may or may not be closed after this message.
    ///
    /// # Body
    /// - `message: PString`
    Failure = 1,

    /// # Description
    /// A state update for a specific peer. The `peer_id` is a server-defined abstract ID for each
    /// peer. Its endianness should not matter as long as it's used as a client-local identifier
    /// only. As the ID may be calculated from the peer through a hash algorithm, its uniqueness for
    /// each peer is highly probable, but not guaranteed.
    ///
    /// # Body
    /// - `peer_id: PeerId` - endianess is only important if you want to exchange the ID with other peers
    /// - `data: PBuffer`
    UpdateState = 2,

    /// # Description
    /// Contains all known states in the peer's session.
    ///
    /// # Body
    /// - `state_count: u16`
    /// - `state_count` times:
    ///   - `peer_id: PeerId`
    ///   - `data: PBuffer`
    FullSync = 3,
}

/// Message data containers to pass data to the [`MessageCodec`] encoder.
#[derive(Clone, Debug)]
pub enum ServerMessage {
    /// Server information, see [`ServerMessageTypeId::ServerInfo`].
    ServerInfo {},
    /// Failure message, see [`ServerMessageTypeId::Failure`].
    Failure {
        /// The message to be sent to the client.
        message: String,
    },
    /// State broadcast, see [`ServerMessageTypeId::UpdateState`].
    UpdateState {
        /// The peer states to be delivered.
        states: Vec<(PeerId, Arc<Vec<u8>>)>,
    },
    /// Complete state delivery, see [`ServerMessageTypeId::FullSync`].
    FullSync {
        /// The peer states to be delivered.
        states: Vec<(PeerId, Arc<Vec<u8>>)>,
    },
}

/// Message data containers to pass data from the [`MessageCodec`] decoder.
#[derive(Clone, Debug)]
pub enum ClientMessage {
    /// Login request message, see [`ClientMessageTypeId::Login`].
    Login {
        username: String,
        auth_token: String,
        session_id: String,
    },
    /// State update message from a client, see [`ClientMessageTypeId::UpdateState`].
    UpdateState { data: Vec<u8> },
    /// Request to do a full sync, see [`ClientMessageTypeId::StateQuery`] and
    /// [`ServerMessageTypeId::FullSync`].
    StateQuery {},
    /// Failure message, see [`ClientMessageTypeId::Failure`].
    Failure {
        /// The message to be sent to the server.
        message: String,
    },
}

/// Simple container for a message codec error.
#[derive(Debug)]
pub struct MessageCodecError {
    message: String,
    pub is_connection_reset: bool,
}

impl MessageCodecError {
    pub fn new(message: String, is_connection_reset: bool) -> Self {
        Self {
            message,
            is_connection_reset,
        }
    }
}

impl Display for MessageCodecError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message.as_str())
    }
}

impl From<std::io::Error> for MessageCodecError {
    fn from(e: std::io::Error) -> Self {
        Self {
            message: format!("{e:?}"),
            is_connection_reset: e.kind() == ErrorKind::ConnectionReset,
        }
    }
}

impl Error for MessageCodecError {}

#[derive(Clone, Debug)]
#[allow(clippy::module_name_repetitions)]
pub struct MessageCodec {}

/// Tries to read a [`ClientMessageTypeId::Login`] from the client. Returns [`None`] if the data is
/// incomplete.
fn try_read_login(src: &mut Reader<BytesMut>) -> Result<Option<ClientMessage>, MessageCodecError> {
    let username = match src.read_pstring() {
        Ok(x) => x,
        Err(e) => {
            if e.kind() == ErrorKind::InvalidData || e.kind() == ErrorKind::UnexpectedEof {
                return Ok(None);
            }

            return Err(MessageCodecError::from(e));
        }
    };
    let Ok(auth_token) = src.read_pstring() else {
        return Ok(None);
    };
    let Ok(session_id) = src.read_pstring() else {
        return Ok(None);
    };
    Ok(Some(ClientMessage::Login {
        username,
        auth_token,
        session_id,
    }))
}

/// Tries to read a [`ClientMessageTypeId::UpdateState`] from the client. Returns [`None`] if the
/// data is incomplete.
fn try_read_update_state(
    src: &mut Reader<BytesMut>,
) -> Result<Option<ClientMessage>, MessageCodecError> {
    let data = match src.read_pbuffer(MAX_STATE_SIZE_BYTES) {
        Ok(x) => x,
        Err(e) => {
            if e.kind() == ErrorKind::InvalidData || e.kind() == ErrorKind::UnexpectedEof {
                return Ok(None);
            }

            return Err(MessageCodecError::from(e));
        }
    };
    Ok(Some(ClientMessage::UpdateState { data }))
}

/// Tries to read a [`ClientMessageTypeId::Failure`].
fn try_read_failure(
    src: &mut Reader<BytesMut>,
) -> Result<Option<ClientMessage>, MessageCodecError> {
    let message = match src.read_pstring() {
        Ok(x) => x,
        Err(e) => {
            if e.kind() == ErrorKind::InvalidData || e.kind() == ErrorKind::UnexpectedEof {
                return Ok(None);
            }

            return Err(MessageCodecError::from(e));
        }
    };
    Ok(Some(ClientMessage::Failure { message }))
}

impl MessageCodec {
    pub fn new() -> Self {
        Self {}
    }
}

/// Tries to apply `f` on the `reader` and return its result. Replaces `src` if `f` returns a
/// result, otherwise leaves it untouched.
fn try_read<F, R, E>(src: &mut BytesMut, mut reader: Reader<BytesMut>, f: F) -> Result<Option<R>, E>
where
    F: FnOnce(&mut Reader<BytesMut>) -> Result<Option<R>, E>,
{
    match f(&mut reader) {
        Ok(Some(result)) => {
            *src = reader.into_inner();
            Ok(Some(result))
        }
        other => other,
    }
}

impl Decoder for MessageCodec {
    type Item = ClientMessage;
    type Error = MessageCodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let mut reader = src.clone().reader();

        let id = match reader.read_u8() {
            Ok(x) => x,
            Err(e) => {
                if e.kind() == ErrorKind::UnexpectedEof {
                    return Ok(None);
                }

                return Err(MessageCodecError::from(e));
            }
        };

        match ClientMessageTypeId::try_from(id) {
            Ok(ClientMessageTypeId::Login) => try_read(src, reader, try_read_login),
            Ok(ClientMessageTypeId::UpdateState) => try_read(src, reader, try_read_update_state),
            Ok(ClientMessageTypeId::StateQuery) => {
                *src = reader.into_inner();
                Ok(Some(ClientMessage::StateQuery {}))
            }
            Ok(ClientMessageTypeId::Failure) => match try_read(src, reader, try_read_failure) {
                Ok(None) => Ok(None),
                Ok(Some(ClientMessage::Failure { message })) => {
                    Err(MessageCodecError::new(message, false))
                }
                Ok(Some(message)) => panic!("expected failure message, got {message:?}"),
                Err(e) => Err(e),
            },
            Err(_) => Err(MessageCodecError::new(
                format!("Invalid message ID `{id}`"),
                false,
            )),
        }
    }
}

impl Encoder<ServerMessage> for MessageCodec {
    type Error = MessageCodecError;

    fn encode(&mut self, data: ServerMessage, buf: &mut BytesMut) -> Result<(), MessageCodecError> {
        match data {
            ServerMessage::FullSync { states } => {
                assert!(
                    u8::try_from(states.len()).is_ok(),
                    "Too many states for a full sync"
                );

                {
                    let mut w = buf.writer();
                    w.write_u8(ServerMessageTypeId::FullSync.into())?;
                    #[allow(clippy::cast_possible_truncation)]
                    w.write_u8(states.len() as u8)?;
                }

                for (peer_id, state) in states {
                    buf.reserve(1 + state.len());
                    let mut w = buf.writer();
                    w.write_u64::<LittleEndian>(peer_id)?;
                    w.write_pbuffer(&state)?;
                }
            }
            ServerMessage::UpdateState { states } => {
                for (peer_id, state) in states {
                    buf.reserve(1 + state.len());
                    let mut w = buf.writer();
                    w.write_u8(ServerMessageTypeId::UpdateState.into())?;
                    w.write_u64::<LittleEndian>(peer_id)?;
                    w.write_pbuffer(&state)?;
                }
            }
            ServerMessage::ServerInfo {} => {
                buf.reserve(4);
                let mut w = buf.writer();
                w.write_u8(ServerMessageTypeId::ServerInfo.into())?;
                w.write_u16::<LittleEndian>(PROTOCOL_VERSION)?;
                w.write_u16::<LittleEndian>(MAX_STATE_SIZE_BYTES)?;
            }
            ServerMessage::Failure { message } => {
                buf.reserve(1);
                let mut w = buf.writer();
                w.write_u8(ServerMessageTypeId::Failure.into())?;
                w.write_pstring(&message)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_stream::StreamBuilder;
    use crate::{
        codec::ClientMessageTypeId, io_util::WritePascalExt, test_stream::TestStream,
        ClientMessage, MessageCodec,
    };
    use byteorder::WriteBytesExt;
    use futures::StreamExt;
    use ntest::{test_case, timeout};
    use tokio::runtime::{Builder, Runtime};
    use tokio_util::codec::Decoder;

    #[test]
    #[timeout(20)]
    fn test_complete_body() {
        let mut data = Vec::new();
        data.write_u8(ClientMessageTypeId::UpdateState.into())
            .unwrap();
        data.write_pbuffer(&[1, 2, 3]).unwrap();
        let mut framed = MessageCodec::new().framed(TestStream::new(data));
        match create_runtime().block_on(framed.next()).unwrap().unwrap() {
            ClientMessage::UpdateState { data } => assert_eq!(data, [1, 2, 3]),
            _ => panic!(),
        }
    }

    #[test_case(0)]
    #[timeout(100)]
    #[test_case(2)]
    #[timeout(100)]
    fn test_incomplete_body(chunk_size: usize) {
        let mut data = Vec::new();
        data.write_u8(ClientMessageTypeId::UpdateState.into())
            .unwrap();
        data.write_pbuffer(&[1, 2, 3]).unwrap();
        data.resize(data.len() - 1, 0);
        expect_codec_failure(
            data,
            if chunk_size > 0 {
                Some(chunk_size)
            } else {
                None
            },
        );
    }

    #[test_case(0)]
    #[timeout(100)]
    #[test_case(2)]
    #[timeout(100)]
    fn test_missing_body(chunk_size: usize) {
        let mut data = Vec::new();
        data.write_u8(ClientMessageTypeId::UpdateState.into())
            .unwrap();
        expect_codec_failure(
            data,
            if chunk_size > 0 {
                Some(chunk_size)
            } else {
                None
            },
        );
    }

    #[test_case(0)]
    #[timeout(100)]
    #[test_case(2)]
    #[timeout(100)]
    fn test_invalid_message_id(chunk_size: usize) {
        let mut data = Vec::new();
        data.write_u8(99).unwrap();
        expect_codec_failure(
            data,
            if chunk_size > 0 {
                Some(chunk_size)
            } else {
                None
            },
        );
    }

    fn expect_codec_failure(data: Vec<u8>, chunk_size: Option<usize>) {
        let mut stream = StreamBuilder::new(data);
        if let Some(chunk_size) = chunk_size {
            stream.chunked(chunk_size);
        }
        let mut framed = MessageCodec::new().framed(stream.build());

        create_runtime()
            .block_on(framed.next())
            .unwrap()
            .expect_err("message codec must fail");
    }

    fn create_runtime() -> Runtime {
        Builder::new_multi_thread()
            .worker_threads(2)
            .enable_io()
            .enable_time()
            .build()
            .unwrap()
    }
}
