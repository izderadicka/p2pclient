#[macro_use]
extern crate log;

use libp2p::{PeerId, Swarm, swarm::NetworkBehaviour, core::connection::ConnectionLimits, identity::Keypair, 
    swarm::SwarmBuilder};

use args::Args;
use error::Result;
use net::{build_transport, OurNetwork};
use std::time::Duration;
use tokio::{
    io::{self, AsyncBufReadExt, BufReader},
    stream::StreamExt,
};
use utils::*;

mod args;
mod error;
mod net;
mod utils;

const ADDR: &str = "/ip4/127.0.0.1/tcp/0";
const TIMEOUT_SECS: u64 = 20;

fn handle_input(swarm: &mut Swarm<OurNetwork>, line: String) -> Result<()> {
    let mut items = line.split(' ').filter(|s| !s.is_empty());

    let cmd = next_item(&mut items, "command")?.to_ascii_uppercase();
    match cmd.as_str() {
        "PUT" => {
            let key = next_item(&mut items, "key")?;
            let value = rest_of(items)?;
            swarm.put_record(key, value)?;
        }
        "GET" => {
            swarm.get_record (next_item(&mut items, "key")?);
        }
        "SAY"|"PUBLISH" => {
            swarm.publish(rest_of(items)?)?;
        }
        "SEND"|"REQ" => {
            let peer = next_item(&mut items, "peer id")?.parse()?;
            let message = rest_of(items)?;
            swarm.send_message(peer, message);
        }
        "PROVIDE" => {
            let key = next_item(&mut items, "key")?;
            swarm.start_providing(key)?;
        }
        "STOP_PROVIDE" => {
            let key = next_item(&mut items, "key")?;
            swarm.stop_providing(key);
        }
        "GET_PROVIDERS" => {
            let key = next_item(&mut items, "key")?;
            if ! swarm.get_providers(key) {
                println!("Key {} is provided locally", key);
            }
        }
        "GET_PEERS" => {
            let key = next_item(&mut items, "key or peer_id")?;
                swarm.get_closest_peers(key);
        
        }
        "BUCKETS" => {
            for b in swarm.buckets() {
                let (start, end) = b.range();
                let start = start.ilog2().unwrap_or(0);
                let end = end.ilog2().unwrap_or(0);
                println!("({:X} - {:X}) => {}", start, end, b.num_entries())
            }
        }
        "ADDR" => {
            let peer: PeerId = next_item(&mut items, "peer_id")?.parse()?;
            let addrs = swarm.addresses_of_peer(&peer);
            println!(
                "Peer {} is known to have these addresses {}",
                peer,
                addrs.printable_list()
            );
        }
        "MY_ID" => {
            println!("My ID is {}", Swarm::local_peer_id(&swarm))
        }
        "HELP" => println!(
            "\
PUT <KEY> <VALUE>
GET <KEY>
SEND <MESSAGE>
PROVIDE <KEY>
STOP_PROVIDE <KEY>
GET_PROVIDERS <KEY>
GET_PEERS <KEY or PEER_ID>
BUCKETS
MY_ID\
        "
        ),
        _ => error!("Invalid command {}", cmd),
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::try_init()?;
    let args = Args::from_args();
    info!("Started");
    let key = Keypair::generate_ed25519();
    let my_id = key.public().into_peer_id();
    println!("My id is {}", my_id.to_base58());
    info!("My id is {}", my_id.to_base58());

    let transport = build_transport(key.clone(), Duration::from_secs(TIMEOUT_SECS))?;
    let net = OurNetwork::new(my_id.clone(), key, "test_chat".into()).await?;

    let mut swarm = SwarmBuilder::new(transport, net, my_id)
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

                            handle_input(&mut swarm, line).unwrap_or_else(|e| error!("Input error: {}", e));
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
