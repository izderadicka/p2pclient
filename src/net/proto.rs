use std::io;

use async_trait::async_trait;
use libp2p::core::upgrade::{read_one, write_one, ReadOneError};
use libp2p::request_response::RequestResponseCodec;

pub const PROTOCOL_NAME: &str = "/demo/0.0.1";
pub const MAX_SIZE: usize = 10_000; //maximum size od message

macro_rules! read {
    ($io: expr) => {{
        let data = read_one($io, MAX_SIZE).await.map_err(|err| match err {
            ReadOneError::Io(e) => e,
            ReadOneError::TooLarge { requested, .. } => io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Request message too large {}", requested),
            ),
        })?;
        String::from_utf8(data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }};
}
#[derive(Debug, Clone)]
pub struct ProtocolCodec;

#[async_trait]
impl RequestResponseCodec for ProtocolCodec {
    type Protocol = String;

    type Request = String;

    type Response = String;

    async fn read_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> std::io::Result<Self::Request>
    where
        T: futures::AsyncRead + Unpin + Send,
    {
        read!(io)
    }

    async fn read_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> std::io::Result<Self::Response>
    where
        T: futures::AsyncRead + Unpin + Send,
    {
        read!(io)
    }

    async fn write_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> std::io::Result<()>
    where
        T: futures::AsyncWrite + Unpin + Send,
    {
        write_one(io, req).await
    }

    async fn write_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        res: Self::Response,
    ) -> std::io::Result<()>
    where
        T: futures::AsyncWrite + Unpin + Send,
    {
        write_one(io, res).await
    }
}
