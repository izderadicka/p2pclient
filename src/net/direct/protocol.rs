use std::iter;

use futures::{AsyncRead, AsyncWrite, future::BoxFuture, prelude::*};
use libp2p::{InboundUpgrade, OutboundUpgrade, core::{UpgradeInfo, upgrade}};

use crate::error::Error;

const PROTOCOL_NAME: &[u8] = b"/direct/0.0.1";
const MAX_MESSAGE_SIZE: usize = 16_384;

pub type Message = String;

pub struct DirectProtocolSend {
    msg: Message
}

impl DirectProtocolSend {
    pub fn new(msg: Message) -> Self {
        DirectProtocolSend{msg}
    }
}

impl UpgradeInfo for DirectProtocolSend {
    type Info = &'static [u8];

    type InfoIter = iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        iter::once(PROTOCOL_NAME)
    }
}

impl <C> OutboundUpgrade<C> for DirectProtocolSend 
where C: AsyncWrite+Send+Unpin+'static
{
    type Output = ();

    type Error = Error;

    type Future = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, mut socket: C, info: Self::Info) -> Self::Future {
        async move {
            upgrade::write_one(&mut socket, self.msg).await?;
            socket.close().await?;
            Ok(())
        }.boxed()
    }
}

#[derive(Debug, Clone)]
pub struct DirectProtocolReceive;

impl DirectProtocolReceive {
    pub fn new() -> Self {
        DirectProtocolReceive {
            
        }
    }
}

impl UpgradeInfo for DirectProtocolReceive {
    type Info = &'static [u8];

    type InfoIter = iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        iter::once(PROTOCOL_NAME)
    }
}

impl <C> InboundUpgrade<C> for DirectProtocolReceive 
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Output = Message;

    type Error = Error;

    type Future = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, mut socket: C, _info: Self::Info) -> Self::Future {
        async move {
        socket.close().await?;
        let data = upgrade::read_one(&mut socket, MAX_MESSAGE_SIZE).await?;
        let msg = String::from_utf8(data)?;
        Ok(msg)
        }.boxed()
    }
}