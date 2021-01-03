use std::{collections::VecDeque, fmt::{Debug, Display}, task::Poll, time::Duration};

use libp2p::swarm::{
    KeepAlive, ProtocolsHandler, ProtocolsHandlerEvent, ProtocolsHandlerUpgrErr, SubstreamProtocol,
};

use super::protocol::{DirectProtocolReceive, DirectProtocolSend, Message};
use crate::{error::Error as CrateError, };

const TIMEOUT: u64 = 10; // outbound timeout in secs
const QUEUE_SIZE: usize = 100; // max capacity of queue 

pub type Error = DirectError<CrateError>;
#[derive(Debug)]
pub struct DirectError<E: Debug>(E);
impl DirectError<CrateError> {
    // pub fn new<E:Into<CrateError>>(e:E) -> Self {
    //     DirectError(e.into())
    // }
    // pub fn msg<E: Into<String>>(msg: E) -> Self {
    //     DirectError(CrateError::msg(msg.into()))
    // }
}
impl<E: Debug + Display> Display for DirectError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Direct error: {}", self.0)
    }
}
impl<E: Display + Debug> std::error::Error for DirectError<E> {}

pub enum DirectHandlerEvent {
    Message(Message),
    MessageSent,
    Error(CrateError),
    Timeout,
}

pub struct DirectHandler {
    events: VecDeque<DirectHandlerEvent>,
    outgoing: VecDeque<DirectProtocolSend>,
    keep_alive: KeepAlive,
    num_outgoing: usize
}

impl DirectHandler {
    pub fn new() -> Self {
        DirectHandler {
            events: VecDeque::new(),
            outgoing: VecDeque::new(),
            keep_alive: KeepAlive::No,
            num_outgoing: 0,
        }
    }

    fn got_outbound(&mut self) {
        debug_assert!(self.num_outgoing>0);
        self.num_outgoing = self.num_outgoing.saturating_sub(1);
        // we do not have any outbound substream requests so connection is safe to close 
        if self.num_outgoing == 0 && self.outgoing.len() == 0 {
            self.keep_alive = KeepAlive::No
        }
    }
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

    fn inject_fully_negotiated_inbound(&mut self, msg: Message, _info: Self::InboundOpenInfo) {
        self.events.push_back(DirectHandlerEvent::Message(msg))
    }

    fn inject_fully_negotiated_outbound(&mut self, _protocol: (), _info: Self::OutboundOpenInfo) {
        self.got_outbound();
        self.events.push_back(DirectHandlerEvent::MessageSent);
    }

    fn inject_event(&mut self, msg: DirectProtocolSend) {
       self.keep_alive = KeepAlive::Yes;
       self.num_outgoing += 1;
        self.outgoing.push_back(msg);
    }

    fn inject_dial_upgrade_error(
        &mut self,
        _info: Self::OutboundOpenInfo,
        error: ProtocolsHandlerUpgrErr<crate::error::Error>,
    ) {
        self.got_outbound();
        match error {
            ProtocolsHandlerUpgrErr::Timeout => self.events.push_back(DirectHandlerEvent::Timeout),
            ProtocolsHandlerUpgrErr::Timer => self
                .events
                .push_back(DirectHandlerEvent::Error(CrateError::msg("Outbound Timer"))),
            ProtocolsHandlerUpgrErr::Upgrade(e) => self
                .events
                .push_back(DirectHandlerEvent::Error(CrateError::msg(e))),
        }
    }

    fn connection_keep_alive(&self) -> KeepAlive {
        self.keep_alive
    }

    fn poll(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<
        ProtocolsHandlerEvent<
            Self::OutboundProtocol,
            Self::OutboundOpenInfo,
            Self::OutEvent,
            Self::Error,
        >,
    > {
        if let Some(evt) = self.events.pop_front() {
            if self.events.capacity() > QUEUE_SIZE {
                self.events.shrink_to_fit()
            }
            return Poll::Ready(ProtocolsHandlerEvent::Custom(evt));

        }
        if let Some(msg) = self.outgoing.pop_front() {
            if self.outgoing.capacity() > QUEUE_SIZE {
                self.outgoing.shrink_to_fit();
            }
            return Poll::Ready(ProtocolsHandlerEvent::OutboundSubstreamRequest {
                protocol: SubstreamProtocol::new(msg, ()).with_timeout(Duration::from_secs(TIMEOUT)),
            });
        }
        Poll::Pending
    }
}
