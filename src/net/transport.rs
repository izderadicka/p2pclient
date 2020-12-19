use std::time::Duration;

use libp2p::{PeerId, Transport, core::{muxing::StreamMuxerBox, transport::Boxed, upgrade}, identity::Keypair, noise, tcp, yamux};
use error::Result;

use crate::error;

pub fn build_transport(key: Keypair, timeout: Duration) -> Result<Boxed<(PeerId, StreamMuxerBox)>> {
    let transport = tcp::TokioTcpConfig::new().nodelay(true);
    let noise_key = noise::Keypair::<noise::X25519Spec>::new().into_authentic(&key)?;
    let noise = noise::NoiseConfig::xx(noise_key).into_authenticated();
    let mux = yamux::YamuxConfig::default();

    let proto = transport
        .upgrade(upgrade::Version::V1)
        .authenticate(noise)
        .multiplex(mux)
        .timeout(timeout)
        .boxed();

    Ok(proto)

}