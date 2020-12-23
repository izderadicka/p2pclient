use crate::{error, utils};
use error::*;
use libp2p::{NetworkBehaviour, PeerId, gossipsub::{
        Gossipsub, GossipsubConfigBuilder, GossipsubEvent, MessageAuthenticity, MessageId, Topic,
        ValidationMode,
    }, identify::{Identify, IdentifyEvent}, identity::Keypair, kad::{
        kbucket, record::store::MemoryStore, record::Key, store::RecordStore, AddProviderOk,
        Addresses, Kademlia, KademliaEvent, PeerRecord, PutRecordOk, QueryResult, Quorum, Record,
    }, mdns::{Mdns, MdnsEvent}, ping::PingConfig, ping::{Ping, PingEvent}, request_response::{ProtocolSupport, RequestResponse, RequestResponseConfig, RequestResponseEvent, RequestResponseMessage}, swarm::{NetworkBehaviour, NetworkBehaviourEventProcess, SwarmEvent}};
use proto::ProtocolCodec;
use std::{
    collections::{hash_map::DefaultHasher, HashSet},
    convert::TryInto,
    fmt::Debug,
    hash::Hash,
    hash::Hasher,
    iter::once,
    time::Duration,
};
pub use transport::build_transport;
use utils::*;

pub mod proto;
pub mod transport;

#[derive(NetworkBehaviour)]
pub struct OurNetwork {
    // Network Behaviours used
    pubsub: Gossipsub,
    dns: Mdns,
    kad: Kademlia<MemoryStore>,
    ping: Ping,
    identify: Identify,
    rpc: RequestResponse<ProtocolCodec>,

    // other properties
    #[behaviour(ignore)]
    topic: Topic,
    #[behaviour(ignore)]
    peers: HashSet<PeerId>,
    #[behaviour(ignore)]
    my_id: PeerId,
    #[behaviour(ignore)]
    kad_boostrap_started: bool,
}

impl NetworkBehaviourEventProcess<RequestResponseEvent<String, String>> for OurNetwork {
    fn inject_event(&mut self, evt: RequestResponseEvent<String, String>) {
        if let RequestResponseEvent::Message{message, peer} = evt {
            match message {
                RequestResponseMessage::Response{response, request_id} =>  
                    println!("Got response message id {} from {}: {}", request_id, peer, response), 

                RequestResponseMessage::Request{request_id, request, channel} => {
                    debug!("Got request message id {} from {}: {}", request_id, peer, request);
                    let reply = "Confirm ".to_string() + &request;
                    self.rpc.send_response(channel, reply).unwrap_or_else(|e| error!("Error sending reply {}", e));
                }

            }

           

        } else {
            debug!("RPC event {:?}", evt)
        }
    }
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
            pubsub,
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
            rpc: RequestResponse::new(
                ProtocolCodec,
                once((proto::PROTOCOL_NAME.to_string(), ProtocolSupport::Full)),
                RequestResponseConfig::default(),
            ),
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

    //usage methods for convenience

    pub fn put_record<K, V>(&mut self, key: K, value: V) -> Result<()>
    where
        K: AsRef<[u8]>,
        V: Into<Vec<u8>>,
    {
        let record = Record {
            key: Key::new(&key),
            value: value.into(),
            publisher: None,
            expires: None,
        };
        self.kad
            .put_record(record, Quorum::One)
            .map_err(|e| Error::msg(format!("Store error {:?}", e)))?;

        Ok(())
    }

    pub fn get_record<K>(&mut self, key: K)
    where
        K: AsRef<[u8]>,
    {
        let key = Key::new(&key);
        self.kad.get_record(&key, Quorum::One);
    }

    pub fn publish<T>(&mut self, msg: T) -> Result<()>
    where
        T: Into<Vec<u8>>,
    {
        self.pubsub
            .publish(&self.topic, msg)
            .map_err(|e| Error::msg(format!("Publish error{:?}", e)))
    }

    pub fn start_providing<K>(&mut self, key: K) -> Result<()>
    where
        K: AsRef<[u8]>,
    {
        let key = Key::new(&key);
        self.kad
            .start_providing(key)
            .map_err(|e| Error::msg(format!("Store error {:?}", e)))?;

        Ok(())
    }

    pub fn stop_providing<K>(&mut self, key: K)
    where
        K: AsRef<[u8]>,
    {
        let key = Key::new(&key);
        self.kad.stop_providing(&key);
    }

    pub fn get_providers<K>(&mut self, key: K) -> bool
    where
        K: AsRef<[u8]>,
    {
        let key = Key::new(&key);
        let providers = self.kad.store_mut().providers(&key);
        debug!("Locally known providers {:?}", providers);
        if providers
            .iter()
            .find(|r| r.provider == self.my_id)
            .is_some()
        {
            false
        } else {
            self.kad.get_providers(key);
            true
        }
    }

    pub fn get_closest_peers<K>(&mut self, key: K)
    where
        K: AsRef<[u8]>,
    {
        if let Ok(peer) = std::str::from_utf8(key.as_ref())
            .map_err(|_| ())
            .and_then(|s| s.parse::<PeerId>().map_err(|_| ()))
        {
            self.kad.get_closest_peers(peer)
        } else {
            self.kad.get_closest_peers(key.as_ref())
        };
    }

    pub fn buckets(
        &mut self,
    ) -> impl Iterator<Item = kbucket::KBucketRef<'_, kbucket::Key<PeerId>, Addresses>> {
        self.kad.kbuckets()
    }

    pub fn send_message(&mut self, peer: PeerId, message: String) {
        let id = self.rpc.send_request(&peer, message);
        debug!("Send request id {}", id);
    }
}
