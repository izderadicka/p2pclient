use crate::{
    error,
    input::{OutputId, OutputSwitch},
    utils,
};
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
        Addresses, Kademlia, KademliaEvent, PeerRecord, PutRecordOk, QueryResult, Quorum, Record,
    },
    mdns::{Mdns, MdnsEvent},
    ping::PingConfig,
    ping::{Ping, PingEvent},
    request_response::{
        ProtocolSupport, RequestResponse, RequestResponseConfig, RequestResponseEvent,
        RequestResponseMessage,
    },
    swarm::{NetworkBehaviour, NetworkBehaviourEventProcess, SwarmEvent},
    NetworkBehaviour, PeerId,
};
use proto::ProtocolCodec;
use std::{
    collections::{hash_map::DefaultHasher, HashSet},
    convert::TryInto,
    fmt::Debug,
    hash::Hash,
    hash::Hasher,
    iter::once,
    time::{Duration, Instant},
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
    #[behaviour(ignore)]
    started: Instant,
    #[behaviour(ignore)]
    outputs: OutputSwitch,
}

impl NetworkBehaviourEventProcess<RequestResponseEvent<String, String>> for OurNetwork {
    fn inject_event(&mut self, evt: RequestResponseEvent<String, String>) {
        match evt {
            RequestResponseEvent::Message { message, peer } => match message {
                RequestResponseMessage::Response {
                    response,
                    request_id,
                } => self.outputs.reply(
                    request_id,
                    format!(
                        "Got response message id {} from {}: {}",
                        request_id, peer, response
                    ),
                ),

                RequestResponseMessage::Request {
                    request_id,
                    request,
                    channel,
                } => {
                    debug!(
                        "Got request message id {} from {}: {}",
                        request_id, peer, request
                    );
                    let reply = "Confirm ".to_string() + &request;
                    self.rpc
                        .send_response(channel, reply)
                        .unwrap_or_else(|e| error!("Error sending reply {}", e));
                }
            },
            RequestResponseEvent::OutboundFailure {
                request_id, error, ..
            } => {
                self.outputs
                    .reply(request_id, format!("Error sending request {:?}", error));
            }
            _ => {
                debug!("RPC event {:?}", evt)
            }
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
            KademliaEvent::QueryResult {
                result,
                id: query_id,
                stats,
            } => {
                debug!(
                    "Got response for query id {:?}, stats {:?}",
                    query_id, stats
                );

                macro_rules! failure {
                    ($msg:literal, $err: ident) => {{
                        let msg = format!($msg, $err);
                        error!("{}", msg);
                        self.outputs.reply(query_id, msg);
                    }};
                }

                match result {
                    QueryResult::GetProviders(Ok(ok)) => {
                        debug!("Got providers {:?}", ok);
                        self.outputs.reply(
                            query_id,
                            format!(
                                "Key {} is provided by ({})",
                                ok.key.printable(),
                                ok.providers.printable_list()
                            ),
                        );
                    }
                    QueryResult::GetProviders(Err(err)) => {
                        failure!("Failed to get providers: {:?}", err);
                    }
                    QueryResult::GetRecord(Ok(ok)) => {
                        debug!("Got record: {:?}", ok);
                        let reply: String = ok
                            .records
                            .into_iter()
                            .map(|r| {
                                let PeerRecord {
                                    record: Record { key, value, .. },
                                    ..
                                } = r;
                                format!("Record {:?} = {:?}", key.printable(), value.printable(),)
                            })
                            .collect::<Vec<_>>()
                            .join("\n");

                        self.outputs.reply(query_id, reply);
                    }
                    QueryResult::GetRecord(Err(err)) => {
                        failure!("Failed to get record: {:?}", err)
                    }
                    QueryResult::PutRecord(Ok(PutRecordOk { key })) => {
                        debug!("Successfully put record {:?}", key.printable());
                    }
                    QueryResult::PutRecord(Err(err)) => {
                        error!("Failed to put record: {:?}", err);
                    }
                    QueryResult::StartProviding(Ok(AddProviderOk { key })) => {
                        let msg = format!("Successfully put provider record {:?}", key.printable());
                        debug!("{}", msg);
                        self.outputs.reply(query_id, msg);
                    }
                    QueryResult::StartProviding(Err(err)) => {
                        failure!("Failed to put provider record: {:?}", err);
                    }

                    QueryResult::GetClosestPeers(Ok(res)) => {
                        debug!("Got closest peers {:?}", res);
                        let mut msg = format!("Closest peers for {} are:", res.key.printable());
                        let search_key = kbucket::Key::new(res.key);
                        for peer in res.peers {
                            let other_key: kbucket::Key<_> = peer.clone().into();
                            let distance = search_key.distance(&other_key);
                            let addresses = self.kad.addresses_of_peer(&peer);
                            msg += &format!(
                                "{} {} {}",
                                peer,
                                distance.ilog2().unwrap_or(0),
                                addresses.printable_list()
                            );
                        }
                        self.outputs.reply(query_id, msg)
                    }
                    QueryResult::GetClosestPeers(Err(e)) => {
                        failure!("Error getting closest peers: {:?}", e);
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
                }
            }
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
                let msg = format!(
                    "({:?} through {}; id:{})->{}",
                    m.source,
                    peer,
                    msg_id,
                    m.data.printable()
                );
                self.outputs.send_to_all(msg);
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

    pub async fn new(
        my_id: PeerId,
        key: Keypair,
        topic: String,
        outputs: OutputSwitch,
    ) -> Result<Self> {
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
            started: Instant::now(),
            outputs,
        };
        Ok(behaviour)
    }

    //usage methods for convenience

    pub async fn put_record<K, V>(&mut self, key: K, value: V, output_id: OutputId) -> Result<()>
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
        let query_id = self
            .kad
            .put_record(record, Quorum::One)
            .map_err(|e| Error::msg(format!("Store error {:?}", e)))?;
        self.outputs.register_pending(output_id, query_id).await;

        Ok(())
    }

    pub async fn get_record<K>(&mut self, key: K, output_id: OutputId)
    where
        K: AsRef<[u8]>,
    {
        let key = Key::new(&key);
        let query_id = self.kad.get_record(&key, Quorum::One);
        self.outputs.register_pending(output_id, query_id).await
    }

    pub async fn publish<T>(&mut self, msg: T, output_id: OutputId) -> Result<()>
    where
        T: Into<Vec<u8>>,
    {
        self.pubsub
            .publish(&self.topic, msg)
            .map_err(|e| Error::msg(format!("Publish error{:?}", e)))?;
        self.outputs.aprintln(output_id, "Message published").await;
        Ok(())
    }

    pub async fn start_providing<K>(&mut self, key: K, output_id: OutputId) -> Result<()>
    where
        K: AsRef<[u8]>,
    {
        let key = Key::new(&key);
        let query_id = self
            .kad
            .start_providing(key)
            .map_err(|e| Error::msg(format!("Store error {:?}", e)))?;
        self.outputs.register_pending(output_id, query_id).await;

        Ok(())
    }

    pub async fn stop_providing<K>(&mut self, key: K, output_id: OutputId)
    where
        K: AsRef<[u8]>,
    {
        let key = Key::new(&key);
        self.kad.stop_providing(&key);
        self.outputs
            .aprintln(output_id, "Stopped providing key")
            .await;
    }

    pub async fn get_providers<K>(&mut self, key: K, output_id: OutputId) -> bool
    where
        K: AsRef<[u8]>,
    {
        let key = Key::new(&key);
        let providers = self.kad.store_mut().providers(&key);
        debug!("Locally known providers {:?}", providers);
        if providers.iter().any(|r| r.provider == self.my_id) {
            false
        } else {
            let query_id = self.kad.get_providers(key);
            self.outputs.register_pending(output_id, query_id).await;
            true
        }
    }

    pub async fn get_closest_peers<K>(&mut self, key: K, output_id: OutputId)
    where
        K: AsRef<[u8]>,
    {
        let query_id = if let Ok(peer) = std::str::from_utf8(key.as_ref())
            .map_err(|_| ())
            .and_then(|s| s.parse::<PeerId>().map_err(|_| ()))
        {
            self.kad.get_closest_peers(peer)
        } else {
            self.kad.get_closest_peers(key.as_ref())
        };

        self.outputs.register_pending(output_id, query_id).await;
    }

    pub fn buckets(
        &mut self,
    ) -> impl Iterator<Item = kbucket::KBucketRef<'_, kbucket::Key<PeerId>, Addresses>> {
        self.kad.kbuckets()
    }

    pub async fn send_message(&mut self, peer: PeerId, message: String, output_id: OutputId) {
        let id = self.rpc.send_request(&peer, message);
        debug!("Send request id {}", id);
        self.outputs.register_pending(output_id, id).await;
    }

    pub fn online_time(&self) -> Duration {
        Instant::now().duration_since(self.started)
    }
}
