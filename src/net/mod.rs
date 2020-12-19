use crate::{error, utils};
use error::*;
use libp2p::{
    gossipsub::{
        Gossipsub, GossipsubConfigBuilder, GossipsubEvent, MessageAuthenticity, MessageId, Topic,
        ValidationMode,
    },
    identify::{Identify, IdentifyEvent},
    identity::Keypair,
    kad::{
        kbucket, record::store::MemoryStore, record::Key, store::RecordStore, AddProviderOk,
        Kademlia, KademliaEvent, PeerRecord, PutRecordOk, QueryResult, Quorum, Record,
    },
    mdns::{Mdns, MdnsEvent},
    ping::PingConfig,
    ping::{Ping, PingEvent},
    swarm::{NetworkBehaviour, NetworkBehaviourEventProcess, SwarmEvent},
    NetworkBehaviour, PeerId,
};
use std::{
    collections::{hash_map::DefaultHasher, HashSet},
    convert::TryInto,
    fmt::Debug,
    hash::Hash,
    hash::Hasher,
    time::Duration,
};
pub use transport::build_transport;
use utils::*;

pub mod transport;

#[derive(NetworkBehaviour)]
pub struct OurNetwork {
    topics: Gossipsub,
    #[behaviour(ignore)]
    topic: Topic,
    dns: Mdns,
    kad: Kademlia<MemoryStore>,
    ping: Ping,
    #[behaviour(ignore)]
    peers: HashSet<PeerId>,
    #[behaviour(ignore)]
    my_id: PeerId,
    #[behaviour(ignore)]
    kad_boostrap_started: bool,
    identify: Identify,
}

impl NetworkBehaviourEventProcess<IdentifyEvent> for OurNetwork {
    fn inject_event(&mut self, evt: IdentifyEvent) {
        if let IdentifyEvent::Received {
            peer_id,
            info,
            observed_addr,
        } = evt
        {
            debug!(
                "Identified peer {} info {:?}, it sees us on addr {}",
                peer_id, info, observed_addr
            );
        }
    }
}

impl NetworkBehaviourEventProcess<KademliaEvent> for OurNetwork {
    fn inject_event(&mut self, evt: KademliaEvent) {
        match evt {
            KademliaEvent::QueryResult { result, .. } => match result {
                QueryResult::GetProviders(Ok(ok)) => {
                    debug!("Got providers {:?}", ok);
                    println!(
                        "Key {} is provided by ({})",
                        ok.key.printable(),
                        ok.providers.printable_list()
                    );
                }
                QueryResult::GetProviders(Err(err)) => {
                    error!("Failed to get providers: {:?}", err);
                }
                QueryResult::GetRecord(Ok(ok)) => {
                    debug!("Got record: {:?}", ok);
                    for PeerRecord {
                        record: Record { key, value, .. },
                        ..
                    } in ok.records
                    {
                        println!("Record {:?} = {:?}", key.printable(), value.printable(),);
                    }
                }
                QueryResult::GetRecord(Err(err)) => {
                    error!("Failed to get record: {:?}", err);
                }
                QueryResult::PutRecord(Ok(PutRecordOk { key })) => {
                    debug!("Successfully put record {:?}", key.printable());
                }
                QueryResult::PutRecord(Err(err)) => {
                    error!("Failed to put record: {:?}", err);
                }
                QueryResult::StartProviding(Ok(AddProviderOk { key })) => {
                    debug!("Successfully put provider record {:?}", key.printable());
                }
                QueryResult::StartProviding(Err(err)) => {
                    error!("Failed to put provider record: {:?}", err);
                }

                QueryResult::GetClosestPeers(Ok(res)) => {
                    debug!("Got closest peers {:?}", res);
                    println!("Closest peers for {} are:", res.key.printable());
                    let search_key = kbucket::Key::new(res.key);
                    for peer in res.peers {
                        let other_key: kbucket::Key<_> = peer.clone().into();
                        let distance = search_key.distance(&other_key);
                        let addresses = self.kad.addresses_of_peer(&peer);
                        println!(
                            "{} {} {}",
                            peer,
                            distance.ilog2().unwrap_or(0),
                            addresses.printable_list()
                        );
                    }
                }
                QueryResult::GetClosestPeers(Err(e)) => {
                    error!("Error getting closest peers: {:?}", e);
                }
                QueryResult::Bootstrap(Ok(boot)) => {
                    debug!("Boostrap done: {:?}", boot)
                }
                QueryResult::Bootstrap(Err(e)) => {
                    error!("Boostrap error: {:?}", e);
                }
                QueryResult::RepublishProvider(res) => match res {
                    Ok(r) => debug!("Republished provider: {:?}", r),
                    Err(e) => error!("Error republishing provider: {:?}", e),
                },
                QueryResult::RepublishRecord(res) => match res {
                    Ok(r) => debug!("Republished record: {:?}", r),
                    Err(e) => error!("Error republishing record: {:?}", e),
                },
            },
            KademliaEvent::RoutablePeer { peer, address } => {
                debug!(
                    "Peer discovered {} at {}, but is not added to table",
                    peer, address
                )
            }
            KademliaEvent::PendingRoutablePeer { peer, address } => {
                debug!(
                    "Peer discovered {} at {}, and might be added to table later",
                    peer, address
                )
            }
            KademliaEvent::RoutingUpdated {
                peer,
                addresses,
                old_peer,
            } => {
                debug!(
                    "Routing table updated by {} at {}, replacing {:?}",
                    peer,
                    addresses.into_vec().printable_list(),
                    old_peer
                );
                if !self.kad_boostrap_started {
                    self.kad.bootstrap().unwrap();
                    self.kad_boostrap_started = true;
                }
            }
            KademliaEvent::UnroutablePeer { peer } => {
                debug!("Unroutable peer {}", peer)
            }
        }
    }
}

impl NetworkBehaviourEventProcess<PingEvent> for OurNetwork {
    fn inject_event(&mut self, evt: PingEvent) {
        trace!("Got ping event {:?}", evt);
    }
}

impl NetworkBehaviourEventProcess<MdnsEvent> for OurNetwork {
    fn inject_event(&mut self, evt: MdnsEvent) {
        match evt {
            MdnsEvent::Discovered(peers) => {
                for (peer, addr) in peers {
                    if !self.peers.contains(&peer) {
                        debug!("Discovered peer {} on {}", peer, addr);

                        self.kad.add_address(&peer, addr);
                        self.peers.insert(peer);
                    } else {
                        trace!("mDNS: found known peer{} on {}", peer, addr);
                    }
                }
            }
            MdnsEvent::Expired(peers) => {
                for (peer, addr) in peers {
                    debug!("Peer {} on {} expired and removed", peer, addr);
                }
            }
        }
    }
}

impl NetworkBehaviourEventProcess<GossipsubEvent> for OurNetwork {
    fn inject_event(&mut self, evt: GossipsubEvent) {
        match evt {
            GossipsubEvent::Message(peer, msg_id, m) => {
                debug!("Received message {:?}", m);
                println!(
                    "({:?} through {}; id:{})->{}",
                    m.source,
                    peer,
                    msg_id,
                    m.data.printable()
                )
            }
            GossipsubEvent::Subscribed { peer_id, topic } => {
                debug!("Peer {} subscribed to {:?}", peer_id, topic)
            }
            GossipsubEvent::Unsubscribed { peer_id, topic } => {
                debug!("Peer {} unsubscribed from {:?}", peer_id, topic)
            }
        }
    }
}

impl OurNetwork {
    pub fn handle_event<T: Debug, U: Debug>(&mut self, event: SwarmEvent<T, U>) {
        debug!("Swarm event {:?}", event);
        match event {
            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                if !self.peers.contains(&peer_id) {
                    self.kad
                        .add_address(&peer_id, endpoint.get_remote_address().clone());

                    self.peers.insert(peer_id);
                }
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                num_established,
                ..
            } => {
                if num_established == 0 && self.peers.contains(&peer_id) {
                    self.peers.remove(&peer_id);
                }
            }

            _ => {}
        }
    }

    pub fn handle_input(&mut self, line: String) -> Result<()> {
        let mut items = line.split(' ').filter(|s| !s.is_empty());

        let cmd = next_item(&mut items, "command")?.to_ascii_uppercase();
        match cmd.as_str() {
            "PUT" => {
                let key = next_item(&mut items, "key")?;
                let value = rest_of(items)?;
                let record = Record {
                    key: Key::new(&key),
                    value: value.into(),
                    publisher: None,
                    expires: None,
                };
                self.kad
                    .put_record(record, Quorum::One)
                    .map_err(|e| Error::msg(format!("Store error {:?}", e)))?;
            }
            "GET" => {
                let key = Key::new(&next_item(&mut items, "key")?);
                self.kad.get_record(&key, Quorum::One);
            }
            "SEND" => {
                self.topics
                    .publish(&self.topic, rest_of(items)?)
                    .map_err(|e| Error::msg(format!("Publish error{:?}", e)))?;
            }

            "PROVIDE" => {
                let key = Key::new(&next_item(&mut items, "key")?);
                self.kad
                    .start_providing(key)
                    .map_err(|e| Error::msg(format!("Store error {:?}", e)))?;
            }
            "STOP_PROVIDE" => {
                let key = Key::new(&next_item(&mut items, "key")?);
                self.kad.stop_providing(&key);
            }
            "GET_PROVIDERS" => {
                let key_value = next_item(&mut items, "key")?;
                let key = Key::new(&key_value);
                let providers = self.kad.store_mut().providers(&key);
                debug!("Locally known providers {:?}", providers);
                if providers
                    .iter()
                    .find(|r| r.provider == self.my_id)
                    .is_some()
                {
                    println!("Key {} is provided locally", key_value);
                } else {
                    self.kad.get_providers(key);
                }
            }
            "GET_PEERS" => {
                let key = next_item(&mut items, "key or peer_id")?;
                if key.starts_with("12D3KooW") {
                    let peer: PeerId = key.parse()?;
                    self.kad.get_closest_peers(peer);
                } else {
                    self.kad.get_closest_peers(key.as_bytes());
                };
            }
            "BUCKETS" => {
                for b in self.kad.kbuckets() {
                    let (start, end) = b.range();
                    let start = start.ilog2().unwrap_or(0);
                    let end = end.ilog2().unwrap_or(0);
                    println!("({:X} - {:X}) => {}", start, end, b.num_entries())
                }
            }
            "ADDR" => {
                let peer: PeerId = next_item(&mut items, "peer_id")?.parse()?;
                let addrs = self.addresses_of_peer(&peer);
                println!(
                    "Peer {} is known to have these addresses {}",
                    peer,
                    addrs.printable_list()
                );
            }
            "MY_ID" => {
                println!("My ID is {}", self.my_id)
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

    pub async fn new(my_id: PeerId, key: Keypair, topic: String) -> Result<Self> {
        let topic = Topic::new(topic);
        let cfg = GossipsubConfigBuilder::new()
            .message_id_fn(|msg| {
                let mut hasher = DefaultHasher::new();
                msg.data.hash(&mut hasher);
                MessageId::from(hasher.finish().to_string())
            })
            .validation_mode(ValidationMode::Strict)
            .build();

        let mut pubsub = Gossipsub::new(MessageAuthenticity::Signed(key.clone()), cfg);
        pubsub.subscribe(topic.clone());

        let kad = Kademlia::new(my_id.clone(), MemoryStore::new(my_id.clone()));

        let behaviour = OurNetwork {
            topics: pubsub,
            topic,
            dns: Mdns::new().await?,
            peers: HashSet::new(),
            ping: Ping::new(
                PingConfig::new()
                    .with_interval(Duration::from_secs(5))
                    .with_max_failures(3.try_into().unwrap())
                    .with_keep_alive(true),
            ),
            kad,
            kad_boostrap_started: false,
            my_id: my_id.clone(),
            identify: Identify::new(
                "ipfs/1.0.0".into(),
                env!("CARGO_PKG_NAME").to_string() + "/" + env!("CARGO_PKG_VERSION"),
                key.public(),
            ),
        };
        Ok(behaviour)
    }
}
