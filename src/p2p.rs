use std::collections::HashSet;

use super::chain::{Block, Chain};
use libp2p::{
    floodsub::{Floodsub, FloodsubEvent, Topic},
    identity::Keypair,
    mdns::{Mdns, MdnsEvent},
    swarm::{NetworkBehaviourEventProcess, Swarm},
    NetworkBehaviour, PeerId,
};
use log::{error, info};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json;
use tokio::sync::mpsc;

//  Topics for pub/sub protocol
//  Impairements: Broadcasts on each request thus extremely inefficient
pub static BLOCK_TOPIC: Lazy<Topic> = Lazy::new(|| Topic::new("blocks"));
pub static CHAIN_TOPIC: Lazy<Topic> = Lazy::new(|| Topic::new("chains"));

//  Key Pair for peer identification on network
pub static KEYS: Lazy<Keypair> = Lazy::new(Keypair::generate_ed25519);

//  Peer id for peer identification on network
pub static PEER_ID: Lazy<PeerId> = Lazy::new(|| PeerId::from(KEYS.public()));

//  Comprised of multiple NetworkBehavior to define the overall
//  NetworkBehaviour of the chain
#[derive(NetworkBehaviour)]
//#[behaviour(event_process = true)]
pub struct ChainBehaviour {
    //  Chain
    #[behaviour(ignore)]
    pub chain: Chain,

    //  Handles FloodSub protocol
    pub floodsub: Floodsub,

    //  Sends response to the UnboundedReceiver
    #[behaviour(ignore)]
    pub init_sender: mpsc::UnboundedSender<bool>,

    //  Handles automatic discovery of peers on the local network
    //  and adds them to the topology
    pub mdns: Mdns,

    //  Sends response to the UnboundedReceiver
    #[behaviour(ignore)]
    pub response_sender: mpsc::UnboundedSender<ChainResponse>,
}

//  Response when a peer sends their local blockchain
#[derive(Debug, Deserialize, Serialize)]
pub struct ChainResponse {
    pub blocks: Vec<Block>,
    pub receiver: String,
}

//  Triggers chain communication for requested ID
#[derive(Debug, Deserialize, Serialize)]
pub struct LocalChainRequest {
    pub from_peer_id: String,
}

//  Keep states for handling incoming messages, lazy init
//  and keyboard input by the client's user
pub enum EventType {
    LocalChainRequest(ChainResponse),
    Input(String),
    Init,
}

impl ChainBehaviour {
    //  Creates a new chain behaviour instance
    pub async fn new(
        chain: Chain,
        init_sender: mpsc::UnboundedSender<bool>,
        response_sender: mpsc::UnboundedSender<ChainResponse>,
    ) -> Self {
        let mut behaviour = Self {
            chain,
            floodsub: Floodsub::new(*PEER_ID),
            init_sender,
            mdns: Mdns::new(Default::default())
                .await
                .expect("can't create mdns"),
            response_sender,
        };

        //  Create topics in floodsub protocol
        behaviour.floodsub.subscribe(BLOCK_TOPIC.clone());
        behaviour.floodsub.subscribe(CHAIN_TOPIC.clone());

        behaviour
    }
}

//  Implemnt FloodsubEvent for ChainBehaviour
impl NetworkBehaviourEventProcess<FloodsubEvent> for ChainBehaviour {
    fn inject_event(&mut self, event: FloodsubEvent) {
        if let FloodsubEvent::Message(msg) = event {
            //  If message is of type ChainResponse and that the message is ours,
            //  we execute our consensus.
            if let Ok(response) = serde_json::from_slice::<ChainResponse>(&msg.data) {
                if response.receiver == PEER_ID.to_string() {
                    info!("Response from {}:", msg.source);
                    response.blocks.iter().for_each(|r| info!("{:?}", r));

                    self.chain.blocks = self
                        .chain
                        .choose(self.chain.blocks.clone(), response.blocks);
                }
            } else if let Ok(response) = serde_json::from_slice::<LocalChainRequest>(&msg.data) {
                //  If of type LocalChainRequest, we send ChainResponse to
                //  initiator
                info!("sending local chain to {}", msg.source.to_string());
                let peer_id = response.from_peer_id;

                if PEER_ID.to_string() == peer_id {
                    if let Err(e) = self.response_sender.send(ChainResponse {
                        blocks: self.chain.blocks.clone(),
                        receiver: msg.source.to_string(),
                    }) {
                        error!("error sending response via channel, {}", e);
                    };
                }
            } else if let Ok(block) = serde_json::from_slice::<Block>(&msg.data) {
                //  If of type Block, we try adding the block if valid
                info!("received new block from {}", msg.source.to_string());
                self.chain.try_add_block(block);
            }
        }
    }
}

//  Implement MdnsEvents for ChainBehaviour
impl NetworkBehaviourEventProcess<MdnsEvent> for ChainBehaviour {
    fn inject_event(&mut self, event: MdnsEvent) {
        match event {
            //  Add node to list of nodes when discovered
            MdnsEvent::Discovered(nodes) => {
                for (peer, _) in nodes {
                    self.floodsub.add_node_to_partial_view(peer)
                }
            }
            //  Remove node from list of nodes when TTL expires and
            //  address hasn't been refreshed
            MdnsEvent::Expired(nodes) => {
                for (peer, _) in nodes {
                    if !self.mdns.has_node(&peer) {
                        self.floodsub.remove_node_from_partial_view(&peer);
                    }
                }
            }
        }
    }
}

pub fn get_list_peers(swarm: &Swarm<ChainBehaviour>) -> Vec<String> {
    info!("Discovered Peers:");
    let nodes = swarm.behaviour().mdns.discovered_nodes();
    let mut unique_peers = HashSet::new();

    for peer in nodes {
        unique_peers.insert(peer);
    }

    unique_peers.iter().map(|peer| peer.to_string()).collect()
}

pub fn handle_print_peers(swarm: &Swarm<ChainBehaviour>) {
    let peers = get_list_peers(swarm);
    peers.iter().for_each(|peer| info!("{}", peer));
}

pub fn handle_print_chain(swarm: &Swarm<ChainBehaviour>) {
    info!("Local Blockchain:");
    let pretty_json =
        serde_json::to_string_pretty(&swarm.behaviour().chain.blocks).expect("can jsonify blocks");
    info!("{}", pretty_json);
}

pub fn handle_create_block(cmd: &str, swarm: &Swarm<ChainBehaviour>) {
    if let Some(data) = cmd.strip_prefix("create b") {
        let behaviour = swarm.behaviour_mut();
        let latest = behaviour
            .chain
            .blocks
            .last()
            .expect("there is at least one block");
        let block = Block::new(latest.id + 1, latest.hash.clone(), data.to_owned());

        let json = serde_json::to_string(&block).expect("cam jsnoify request");
        behaviour.chain.blocks.push(block);

        info!("broadcasting new block");
        behaviour
            .floodsub
            .publish(BLOCK_TOPIC.clone(), json.as_bytes());
    }
}
