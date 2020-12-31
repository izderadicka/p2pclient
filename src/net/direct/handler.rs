use std::{collections::VecDeque, fmt::{Display, Debug}, task::Poll};

use libp2p::swarm::{KeepAlive, ProtocolsHandler, ProtocolsHandlerEvent, ProtocolsHandlerUpgrErr, SubstreamProtocol};

use super::protocol::{DirectProtocolReceive, DirectProtocolSend, Message};
use crate::error::Error as CrateError;

pub type Error = DirectError<CrateError>;
#[derive(Debug)]
pub struct DirectError<E:Debug>(E);
impl  DirectError<CrateError> {
    pub fn new<E:Into<CrateError>>(e:E) -> Self {
        DirectError(e.into())
    }
    pub fn msg<E: Into<String>>(msg: E) -> Self {
        DirectError(CrateError::msg(msg.into()))
    }
}
impl <E: Debug+Display> Display for DirectError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Direct error: {}", self.0)
    }
}
impl <E:Display+Debug> std::error::Error for DirectError<E> {}

pub enum DirectHandlerEvent {
    Message(Message),
    MessageSent,
    Error(CrateError),
    Timeout
}

pub struct DirectHandler{
    events: VecDeque<DirectHandlerEvent>,
    outgoing: VecDeque<DirectProtocolSend>,
}


impl ProtocolsHandler for DirectHandler {
    type InEvent = DirectProtocolSend;

    type OutEvent = DirectHandlerEvent;

    type Error = Error;

    type InboundProtocol = DirectProtocolReceive;

    type OutboundProtocol = DirectProtocolSend;

    type InboundOpenInfo = ();

    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(DirectProtocolReceive, ())
    }

    fn inject_fully_negotiated_inbound(
        &mut self,
        msg: Message,
        _info: Self::InboundOpenInfo
    ) {
        self.events.push_back(DirectHandlerEvent::Message(msg))
    }

    fn inject_fully_negotiated_outbound(
        &mut self,
        _protocol: (),
        _info: Self::OutboundOpenInfo
    ) {
        self.events.push_back(DirectHandlerEvent::MessageSent);
    }

    fn inject_event(&mut self, msg: DirectProtocolSend) {
        self.outgoing.push_back(msg);
    }

    fn inject_dial_upgrade_error(
        &mut self,
        _info: Self::OutboundOpenInfo,
        error: ProtocolsHandlerUpgrErr<
            crate::error::Error
        >
    ) {
        match error {
            ProtocolsHandlerUpgrErr::Timeout => {self.events.push_back(DirectHandlerEvent::Timeout)}
            ProtocolsHandlerUpgrErr::Timer => {self.events.push_back(DirectHandlerEvent::Error(CrateError::msg("Outbound Timer")))}
            ProtocolsHandlerUpgrErr::Upgrade(e) => {self.events.push_back(DirectHandlerEvent::Error(CrateError::msg(e)))}
        }
    }

    fn connection_keep_alive(&self) -> KeepAlive {
       KeepAlive::No
    }

    fn poll(&mut self, cx: &mut std::task::Context<'_>) -> Poll<
        ProtocolsHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::OutEvent, Self::Error>
    > {
        if let Some(evt) = self.events.pop_front() {
            return Poll::Ready(ProtocolsHandlerEvent::Custom(evt))
        }
        if let Some(msg) = self.outgoing.pop_front() {
            return Poll::Ready(ProtocolsHandlerEvent::OutboundSubstreamRequest{protocol: SubstreamProtocol::new(msg, ())})
        }
        Poll::Pending
    }
}