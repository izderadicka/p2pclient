use handler::{DirectHandler, DirectHandlerEvent};
use libp2p::{PeerId, swarm::NetworkBehaviour};
use protocol::{DirectProtocolSend, Message};

pub mod protocol;
pub mod handler;


pub struct Direct{}

impl Direct {
    pub fn send(to: &PeerId, msg: Message) {

    }
}

pub enum DirectEvent{}

impl NetworkBehaviour for Direct {
    type ProtocolsHandler=DirectHandler;

    type OutEvent=DirectEvent;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        todo!()
    }

    fn addresses_of_peer(&mut self, peer_id: &PeerId) -> Vec<libp2p::Multiaddr> {
        todo!()
    }

    fn inject_connected(&mut self, peer_id: &PeerId) {
        todo!()
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        todo!()
    }

    fn inject_event(
        &mut self,
        peer_id: PeerId,
        connection: libp2p::core::connection::ConnectionId,
        event: DirectHandlerEvent
    ) {
        todo!()
    }

    fn poll(&mut self, cx: &mut std::task::Context<'_>, params: &mut impl libp2p::swarm::PollParameters)
        -> std::task::Poll<libp2p::swarm::NetworkBehaviourAction<DirectProtocolSend, Self::OutEvent>> {
        todo!()
    }
}