#[macro_use]
extern crate log;

use input::{InputOutputSwitch, OutputId, OutputSwitch};
use libp2p::{
    core::connection::ConnectionLimits, identity::Keypair, swarm::NetworkBehaviour,
    swarm::SwarmBuilder, PeerId, Swarm,
};

use args::Args;
use async_std::task;
use error::{Error, Result};
use futures::prelude::*;
use net::{build_transport, OurNetwork};
use std::time::Duration;

use utils::*;

mod args;
mod error;
mod input;
mod net;
mod utils;

const ADDR: &str = "/ip4/127.0.0.1/tcp/0";
const TIMEOUT_SECS: u64 = 20;

async fn handle_input_checked(
    swarm: &mut Swarm<OurNetwork>,
    line: String,
    output_id: OutputId,
    mut out: OutputSwitch,
) {
    if let Err(e) = handle_input(swarm, line, output_id, &mut out).await {
        error!("Input error: {}", e);
        out.aprintln(output_id, format!("Command failed : {}", e))
            .await;
    }
}

async fn handle_input(
    swarm: &mut Swarm<OurNetwork>,
    line: String,
    output_id: OutputId,
    out: &mut OutputSwitch,
) -> Result<()> {
    macro_rules! outln {
        ($($p:expr),+) => {
            out.aprintln(output_id, format!($($p),+)).await
        }
    }
    let mut items = line.split(' ').filter(|s| !s.is_empty());

    let cmd = next_item(&mut items, "command")?.to_ascii_uppercase();
    match cmd.as_str() {
        "PUT" => {
            let key = next_item(&mut items, "key")?;
            let value = rest_of(items)?;
            swarm.put_record(key, value, output_id).await?;
        }
        "GET" => {
            swarm
                .get_record(next_item(&mut items, "key")?, output_id)
                .await;
        }
        "SAY" | "PUBLISH" => {
            swarm.publish(rest_of(items)?, output_id).await?;
        }
        "SEND" | "REQ" => {
            let peer = next_item(&mut items, "peer id")?.parse()?;
            let message = rest_of(items)?;
            swarm.send_message(peer, message, output_id).await;
        }
        "PROVIDE" => {
            let key = next_item(&mut items, "key")?;
            swarm.start_providing(key, output_id).await?;
        }
        "STOP_PROVIDE" => {
            let key = next_item(&mut items, "key")?;
            swarm.stop_providing(key, output_id).await;
        }
        "GET_PROVIDERS" => {
            let key = next_item(&mut items, "key")?;
            if !swarm.get_providers(key, output_id).await {
                out.println(output_id, format!("Key {} is provided locally", key));
            }
        }
        "GET_PEERS" => {
            let key = next_item(&mut items, "key or peer_id")?;
            swarm.get_closest_peers(key, output_id).await;
        }
        "BUCKETS" => {
            for b in swarm.buckets() {
                let (start, end) = b.range();
                let start = start.ilog2().unwrap_or(0);
                let end = end.ilog2().unwrap_or(0);
                outln!("({:X} - {:X}) => {}", start, end, b.num_entries())
            }
        }
        "ADDR" => {
            let peer: PeerId = next_item(&mut items, "peer_id")?.parse()?;
            let addrs = swarm.addresses_of_peer(&peer);
            outln!(
                "Peer {} is known to have these addresses {}",
                peer,
                addrs.printable_list()
            );
        }
        "INFO" => {
            let net_info = Swarm::network_info(&swarm);
            let conns = net_info.connection_counters();
            let mut t = swarm.online_time().as_secs();
            let hours = t / 3600;
            t -= hours * 3600;
            let mins = t / 60;
            let secs = t % 60;

            outln!("Running duration: {}:{:02}:{:02}", hours, mins, secs);
            outln!("Connected peers: {}", net_info.num_peers());
            outln!("Connections: {}", conns.num_established());
            outln!("Pending connections: {}", conns.num_pending());
        }
        "MY_ID" => {
            outln!("My ID is {}", Swarm::local_peer_id(&swarm))
        }
        "HELP" => outln!(
            "\
PUT <KEY> <VALUE>                   Puts value into Kademlia DHT under given key
GET <KEY>                           Gets value from Kademlia DHT from given key 
SAY|PUBLISH <MESSAGE>               Publishes message to all peers 
PROVIDE <KEY>                       Makes itself a provider of the key
STOP_PROVIDE <KEY>                  Stops itself providing the key
GET_PROVIDERS <KEY>                 Gets all providers for given key
GET_PEERS <KEY or PEER_ID>          Gets closest peers for given peer_id or arbitrary key
SEND|REQ <PEER_ID> <MESSAGE>        Sends request message to given peer (peers echos back)
ADDR <PEER_ID>                      Prints known addresses for given peer_id
BUCKETS                             Lists local Kademlia buckets (peers routing)
INFO                                Prints some client info
MY_ID                               Prints local peer_id\
        "
        ),
        _ => {
            error!("Invalid command {}", cmd);
            return Err(Error::msg("Invalid command"));
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    env_logger::try_init()?;
    let args = Args::from_args();
    info!("Started");
    let key = Keypair::generate_ed25519();
    let my_id = key.public().into_peer_id();
    println!("My id is {}", my_id.to_base58());
    info!("My id is {}", my_id.to_base58());

    let transport = build_transport(key.clone(), Duration::from_secs(TIMEOUT_SECS))?;
    let mut input = task::block_on(InputOutputSwitch::new(!args.no_input, args.control))?;
    let net = task::block_on(OurNetwork::new(
        my_id.clone(),
        key,
        "test_chat".into(),
        input.outputs(),
    ))?;

    let mut swarm = SwarmBuilder::new(transport, net, my_id)
        .connection_limits(ConnectionLimits::default())
        .build();

    for peer_addr in args.peers.iter() {
        let addr = peer_addr.parse()?;
        Swarm::dial_addr(&mut swarm, addr)?;
    }

    let _listener_id = Swarm::listen_on(&mut swarm, ADDR.parse().unwrap())?;

    task::block_on(async move {
        loop {
            futures::select! {
                line = input.next().fuse() => {
                    match line {
                        Some((line, input_id)) => {

                            handle_input_checked(&mut swarm, line, input_id, input.outputs()).await;
                        },
                        None => {
                            warn!("End of input");
                            break
                        }
                    }
                }
                event = swarm.next_event().fuse() => {
                    swarm.handle_event(event)
                }
            }
        }
    });

    info!("Finished");
    Ok(())
}
