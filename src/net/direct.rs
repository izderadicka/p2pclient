use std::{
    collections::{HashMap, HashSet, VecDeque},
    task::Poll,
};

use handler::{DirectHandler, DirectHandlerEvent};
use libp2p::{
    swarm::{DialPeerCondition, NetworkBehaviour, NetworkBehaviourAction},
    PeerId,
};
use protocol::{DirectProtocolSend, Message};
use smallvec::SmallVec;

use crate::error::Error;

pub mod handler;
pub mod protocol;

pub struct Direct {
    connected: HashSet<PeerId>,
    events: VecDeque<NetworkBehaviourAction<DirectProtocolSend, DirectEvent>>,
    pending_sends: HashMap<PeerId, SmallVec<[Message; 4]>>,
}

impl Direct {
    pub fn new() -> Self {
        Direct {
            connected: HashSet::new(),
            events: VecDeque::new(),
            pending_sends: HashMap::new(),
        }
    }
    pub fn send(&mut self, to: &PeerId, msg: Message) {
        if self.connected.contains(to) {
            self.send_connected(to, msg)
        } else {
            self.pending_sends.entry(to.clone()).or_default().push(msg);
            self.events.push_back(NetworkBehaviourAction::DialPeer {
                peer_id: to.clone(),
                condition: DialPeerCondition::Disconnected,
            })
        }
    }

    fn send_connected(&mut self, to: &PeerId, msg: Message) {
        self.events
            .push_back(NetworkBehaviourAction::NotifyHandler {
                event: DirectProtocolSend::new(msg),
                peer_id: to.clone(),
                handler: libp2p::swarm::NotifyHandler::Any,
            })
    }
}

pub enum DirectEvent {
    Message { message: Message, peer_id: PeerId },
    MessageSent { peer_id: PeerId },
    Error { error: Error, peer_id: PeerId },
    Timeout { peer_id: PeerId },
}

impl NetworkBehaviour for Direct {
    type ProtocolsHandler = DirectHandler;

    type OutEvent = DirectEvent;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        DirectHandler::new()
    }

    fn addresses_of_peer(&mut self, _peer_id: &PeerId) -> Vec<libp2p::Multiaddr> {
        Vec::new()
    }

    fn inject_connected(&mut self, peer_id: &PeerId) {
        if let Some(pending_messages) = self.pending_sends.remove(peer_id) {
            for msg in pending_messages {
                self.send_connected(peer_id, msg)
            }
        }
        self.connected.insert(peer_id.clone());
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        self.connected.remove(peer_id);
    }

    fn inject_dial_failure(&mut self, peer: &PeerId) {
        if let Some(pending_messages) = self.pending_sends.remove(peer) {
            for _msg in pending_messages {
                self.events
                    .push_back(NetworkBehaviourAction::GenerateEvent(DirectEvent::Error {
                        error: Error::msg(format!("Message dropped, cannot dial peer {}", peer)),
                        peer_id: peer.clone(),
                    }))
            }
        }
    }

    fn inject_event(
        &mut self,
        peer_id: PeerId,
        _connection: libp2p::core::connection::ConnectionId,
        event: DirectHandlerEvent,
    ) {
        let evt = match event {
            DirectHandlerEvent::Message(m) => DirectEvent::Message {
                message: m,
                peer_id,
            },
            DirectHandlerEvent::MessageSent => DirectEvent::MessageSent { peer_id },
            DirectHandlerEvent::Error(e) => DirectEvent::Error { error: e, peer_id },
            DirectHandlerEvent::Timeout => DirectEvent::Timeout { peer_id },
        };
        self.events
            .push_back(NetworkBehaviourAction::GenerateEvent(evt))
    }

    fn poll(
        &mut self,
        _cx: &mut std::task::Context<'_>,
        _params: &mut impl libp2p::swarm::PollParameters,
    ) -> Poll<NetworkBehaviourAction<DirectProtocolSend, Self::OutEvent>> {
        if let Some(evt) = self.events.pop_front() {
            return Poll::Ready(evt);
        }
        Poll::Pending
    }
}
