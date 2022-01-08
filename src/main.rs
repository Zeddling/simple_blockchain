#[macro_use]
extern crate log;

mod chain;
mod p2p;

use libp2p::{
    core::transport::upgrade,
    mplex,
    noise::{Keypair, NoiseConfig, X25519Spec},
    swarm::SwarmBuilder,
    tcp::TokioTcpConfig,
    Swarm, Transport,
};
use tokio::{
    io::{stdin, AsyncBufReadExt, BufReader},
    select, spawn,
    sync::mpsc,
};

use crate::chain::Chain;

#[tokio::main]
async fn main() {
    //  Initialize entities for network
    pretty_env_logger::init();

    info!("Peer Id: {}", p2p::PEER_ID.clone());

    //  Create message passing channels for initialization and responses
    let (reponse_sender, mut response_receivier) = mpsc::unbounded_channel();
    let (init_sender, mut init_receiver) = mpsc::unbounded_channel();

    //  Init key pair
    let auth_keys = Keypair::<X25519Spec>::new()
        .into_authentic(&p2p::KEYS)
        .expect("can create auth keys");

    //  Init TCP/IP config for communication in the network
    let transport = TokioTcpConfig::new()
        .upgrade(upgrade::Version::V1)
        .authenticate(NoiseConfig::xx(auth_keys).into_authenticated())
        .multiplex(mplex::MplexConfig::new())
        .boxed();

    //  Init chain behaviour
    let behaviour = p2p::ChainBehaviour::new(Chain::new(), init_sender, reponse_sender).await;

    //  Init swarm
    //  A swarm is a system of p2p networked nodes that create a
    //  decentralized storage and communication service
    let mut swarm = SwarmBuilder::new(transport, behaviour, *p2p::PEER_ID)
        .executor(Box::new(|future| {
            spawn(future);
        }))
        .build();

    //  Safe reader for streams
    let mut stdin = BufReader::new(stdin()).lines();

    //  Start swarm
    Swarm::listen_on(
        &mut swarm,
        "/ip4/0.0.0.0/tcp/0"
            .parse()
            .expect("can get a local socket"),
    )
    .expect("swarm can be started");

    // Sends init trigger on the init channel
    spawn(async move {
        info!("sending init event");
        init_sender.send(true).expect("can send init event");
    });

    //  Handle keyboard events from user, incoming data and outgoing data
    loop {
        let event = {
            select! {
                line = stdin.next_line() => Some(
                    p2p::EventType::Input(
                        line.expect("can get line").expect("can read line from stdin")
                    )),
                response = response_receivier.recv() => {
                    Some(p2p::EventType::LocalChainRequest(response.expect("response exixts")))
                },
                _init = init_receiver.recv() => {
                    Some(p2p::EventType::Init)
                }
                e = swarm.select_next_some() => {
                    info!("Unhandled Swarm Event: {:?}", e);
                    None
                }
            };
        };

        if let Some(evt) = event {
            match evt {
                p2p::EventType::Init => {
                    let peers = p2p::get_list_peers(&swarm);
                    swarm.behaviour_mut().chain.genesis();

                    info!("connected nodes: {}", peers.len());

                    if !peers.is_empty() {
                        let request = p2p::LocalChainRequest {
                            from_peer_id: peers
                                .iter()
                                .last()
                                .expect("at least one peer")
                                .to_string(),
                        };

                        let json = serde_json::to_string(&request).expect("can jsonify request");
                        swarm
                            .behaviour_mut()
                            .floodsub
                            .publish(p2p::CHAIN_TOPIC.clone(), json.as_bytes());
                    }
                }
                p2p::EventType::LocalChainRequest(response) => {
                    let json = serde_json::to_string(&response).expect("can jsonify response");
                    swarm
                        .behaviour_mut()
                        .floodsub
                        .publish(p2p::CHAIN_TOPIC.clone(), json.as_bytes());
                }
                p2p::EventType::Input(line) => match line.as_str() {
                    "ls p" => p2p::handle_print_peers(&swarm),
                    cmd if cmd.starts_with("ls c") => p2p::handle_print_chain(&swarm),
                    cmd if cmd.starts_with("create b") => p2p::handle_create_block(cmd, &mut swarm),
                    _ => error!("unknown command"),
                },
            }
        }
    }
}
