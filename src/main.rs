
#[macro_use]
extern crate log;

use libp2p::{
    core::connection::ConnectionLimits,
    identity::Keypair,
    swarm::SwarmBuilder,
    Swarm,
};

use std::{
    time::Duration,
};
use error::Result;
use args::Args;
use tokio::{
    io::{self, AsyncBufReadExt, BufReader},
    stream::StreamExt,
};
use net::{OurNetwork, build_transport};

mod utils;
mod error;
mod args;
mod net;

const ADDR: &str = "/ip4/127.0.0.1/tcp/0";
const TIMEOUT_SECS: u64 = 20;


#[tokio::main]
async fn main() -> Result<()> {
    env_logger::try_init()?;
    let args = Args::from_args();
    info!("Started");
    let key = Keypair::generate_ed25519();
    let my_id = key.public().into_peer_id();
    println!("My id is {}", my_id.to_base58());
    info!("My id is {}", my_id.to_base58());

    let proto = build_transport(key.clone(), Duration::from_secs(TIMEOUT_SECS))?;
    let behaviour = OurNetwork::new(my_id.clone(), key, "test_chat".into()).await?;

    let mut swarm = SwarmBuilder::new(proto, behaviour, my_id)
        .executor(Box::new(|f| {
            tokio::spawn(f);
        }))
        .connection_limits(ConnectionLimits::default())
        .build();

    for peer_addr in args.peers {
        let addr = peer_addr.parse()?;
        Swarm::dial_addr(&mut swarm, addr)?;
    }

    let _listener_id = Swarm::listen_on(&mut swarm, ADDR.parse().unwrap())?;

    if args.no_input {
        loop {
            let evt = swarm.next_event().await;
            swarm.handle_event(evt)
        }
    } else {
        let mut input = BufReader::new(io::stdin()).lines();

        loop {
            tokio::select! {
                line = input.next() => {
                    match line {
                        Some(Ok(line)) => {

                            swarm.handle_input(line).unwrap_or_else(|e| error!("Input error: {}", e));
                        },
                        None => {
                            debug!("End of stdin");
                            break
                        }
                        Some(Err(e)) => {
                            error!("error reading stdin: {}", e);
                        }
                    }
                }
                event = swarm.next_event() => {
                    swarm.handle_event(event)
                }
            }
        }
    }

    info!("Finished");
    Ok(())
}
