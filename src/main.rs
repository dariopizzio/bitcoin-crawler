use std::{
    collections::HashSet,
    io,
    net::SocketAddr,
    str::FromStr,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    time::Duration,
};

use bitcoin::{
    consensus::{deserialize_partial, serialize},
    p2p::{
        message::{NetworkMessage, RawNetworkMessage},
        message_network::VersionMessage,
        Address, ServiceFlags,
    },
    Network,
};
use chrono::Utc;
use rand::{thread_rng, Rng};
use tokio::{
    join,
    net::{lookup_host, TcpStream},
    task::JoinHandle,
    time::{error::Elapsed, timeout},
};
use tokio_util::{
    bytes::Buf,
    codec::{Decoder, Encoder, Framed},
};

use futures::{SinkExt, StreamExt};

const TIMEOUT_FUN: u64 = 6_000;

#[tokio::main]
async fn main() {
    //let local_address = SocketAddr::from_str("127.0.0.1:8333").unwrap();

    let seed_nodes = get_seed_nodes().await;
    println!("Seeds: {seed_nodes:?}");

    let set_nodes: HashSet<Address> = HashSet::new();
    let set_nodes = Arc::new(Mutex::new(set_nodes));

    // Create threadpool
    let thread_pool = ThreadPool::new(5, set_nodes.clone()); // TODO size = user input?
    thread_pool.execute(seed_nodes);

    //thread_pool.kill();
    thread_pool.join().await;

    println!("#### set_nodes: {}", set_nodes.lock().unwrap().len());
}

async fn get_seed_nodes() -> HashSet<SocketAddr> {
    HashSet::from_iter(lookup_host(("seed.bitcoin.sipa.be", 8333)).await.unwrap())
}

async fn connect_node(remote_address: &SocketAddr) -> Framed<TcpStream, BitcoinCodec> {
    let connection = TcpStream::connect(remote_address);

    let stream = timeout(Duration::from_millis(500), connection)
        .await
        .unwrap()
        .unwrap();
    Framed::new(stream, BitcoinCodec {})
}

async fn perform_handshake(
    stream: &mut Framed<TcpStream, BitcoinCodec>,
    remote_address: &SocketAddr,
    local_address: &SocketAddr,
) {
    let version_message = RawNetworkMessage::new(
        Network::Bitcoin.magic(),
        NetworkMessage::Version(get_version_message(remote_address, local_address)),
    );

    stream.send(version_message).await.unwrap();

    while let Some(message) = stream.next().await {
        let message = message.unwrap();

        match message.payload() {
            NetworkMessage::Verack => {
                let verack_message =
                    RawNetworkMessage::new(Network::Bitcoin.magic(), NetworkMessage::Verack);

                stream.send(verack_message).await.unwrap();

                return;
            }
            _ => continue,
        }
    }
}

async fn perform_get_addr(stream: &mut Framed<TcpStream, BitcoinCodec>) -> HashSet<Address> {
    let mut set_nodes: HashSet<Address> = HashSet::new();

    let get_addr = RawNetworkMessage::new(Network::Bitcoin.magic(), NetworkMessage::GetAddr);

    stream.send(get_addr).await.unwrap();

    while let Some(message) = stream.next().await {
        let message = message.unwrap();

        match message.payload().clone() {
            NetworkMessage::Addr(nodes) => {
                set_nodes.extend(nodes.into_iter().map(|item| item.1));
                return set_nodes;
            }

            _ => continue,
        }
    }
    set_nodes
}

struct BitcoinCodec {}

impl Decoder for BitcoinCodec {
    type Item = RawNetworkMessage;
    type Error = io::Error;

    fn decode(
        &mut self,
        src: &mut tokio_util::bytes::BytesMut,
    ) -> Result<Option<Self::Item>, Self::Error> {
        if let Ok(item) = deserialize_partial::<RawNetworkMessage>(src) {
            src.advance(item.1);
            return Ok(Some(item.0));
        }
        Ok(None)
    }
}

impl Encoder<RawNetworkMessage> for BitcoinCodec {
    type Error = io::Error;

    fn encode(
        &mut self,
        item: RawNetworkMessage,
        dst: &mut tokio_util::bytes::BytesMut,
    ) -> Result<(), Self::Error> {
        let data = serialize(&item);
        dst.extend_from_slice(&data);
        Ok(())
    }
}

fn get_version_message(remote_address: &SocketAddr, local_address: &SocketAddr) -> VersionMessage {
    let remote_address = Address::new(remote_address, ServiceFlags::NONE);
    let local_address = Address::new(local_address, ServiceFlags::NONE);
    let nonce = thread_rng().gen();

    VersionMessage::new(
        ServiceFlags::NONE,
        Utc::now().timestamp(),
        remote_address,
        local_address,
        nonce,
        "".to_string(), // TODO user_agent
        0,
    )
}

struct Worker {
    id: usize,
    thread: JoinHandle<()>,
}

impl Worker {
    fn new(
        id: usize,
        receiver: Arc<Mutex<Receiver<WorkerMessage>>>,
        set_nodes: Arc<Mutex<HashSet<Address>>>,
    ) -> Self {
        println!("worker: {id}");
        Self {
            id,
            thread: tokio::task::spawn(async move {
                loop {
                    let receiver_lock = receiver.clone();

                    let message = receiver.lock().unwrap().recv().unwrap();

                    match message {
                        WorkerMessage::Message(address) => {
                            println!("thread: {id} address: {address}");
                            let _ = worker_function(address, &set_nodes).await;
                            drop(receiver_lock);
                        }
                        WorkerMessage::Kill => {
                            println!("Killing thread: {id}");
                            drop(receiver_lock);
                            break;
                        }
                    }
                }
            }),
        }
    }
}

struct ThreadPool {
    workers: Vec<Worker>,
    sender: Sender<WorkerMessage>,
}

impl ThreadPool {
    fn new(size: usize, set_nodes: Arc<Mutex<HashSet<Address>>>) -> Self {
        let mut workers = Vec::with_capacity(size);

        let (sender, receiver) = mpsc::channel::<WorkerMessage>();

        let receiver = Arc::new(Mutex::new(receiver));

        for i in 0..size {
            workers.push(Worker::new(i, receiver.clone(), set_nodes.clone()))
        }

        Self { workers, sender }
    }

    fn execute(&self, seed_nodes: HashSet<SocketAddr>) {
        seed_nodes
            .iter()
            .for_each(|addr| self.sender.send(WorkerMessage::Message(*addr)).unwrap());
        // Change to &

        println!("sending kill");
        self.kill();
    }

    fn kill(&self) {
        self.workers.iter().for_each(|_w| {
            self.sender.send(WorkerMessage::Kill).unwrap();
        })
    }

    async fn join(self) {
        for worker in self.workers {
            let _ = join!(worker.thread);
        }
    }
}

async fn worker_function(
    remote_address: SocketAddr,
    set_nodes: &Arc<Mutex<HashSet<Address>>>,
) -> Result<(), Elapsed> {
    let local_address = SocketAddr::from_str("127.0.0.1:8333").unwrap();

    let mut stream = connect_node(&remote_address).await;
    println!("connect_node");

    timeout(
        Duration::from_millis(TIMEOUT_FUN),
        perform_handshake(&mut stream, &remote_address, &local_address),
    )
    .await?;

    println!("perform_handshake");

    let nodes = timeout(
        Duration::from_millis(TIMEOUT_FUN),
        perform_get_addr(&mut stream),
    )
    .await?;

    set_nodes.lock().unwrap().extend(nodes.into_iter());

    //println!("thread: {id} - nodes: {set_nodes:?}");

    Ok(())
}

enum WorkerMessage {
    Message(SocketAddr),
    Kill,
}
