mod errors;

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
use errors::CrawlerError;
use rand::{thread_rng, Rng};
use tokio::{
    join,
    net::{lookup_host, TcpStream},
    task::JoinHandle,
    time::timeout,
};
use tokio_util::{
    bytes::Buf,
    codec::{Decoder, Encoder, Framed},
};

use futures::{SinkExt, StreamExt};

const TIMEOUT_FUN: u64 = 6_000;
const TIMEOUT_CONNECTION: u64 = 500;
const POOL_SIZE: usize = 5;
const LOCAL_ADDRESS: &str = "127.0.0.1:8333";
const SEED_NODE: &str = "seed.bitcoin.sipa.be";
const MAINNET_PORT: u16 = 8333;

#[tokio::main]
async fn main() -> Result<(), CrawlerError> {
    let seed_nodes = get_seed_nodes().await?;
    println!("Seeds: {seed_nodes:?}");

    let set_nodes = Arc::new(Mutex::new(HashSet::new()));

    let thread_pool = ThreadPool::new(POOL_SIZE, set_nodes.clone());
    thread_pool.execute(seed_nodes);

    thread_pool.join().await;

    println!("Nodes collected: {}", set_nodes.lock().unwrap().len());
    Ok(())
}

async fn get_seed_nodes() -> Result<HashSet<SocketAddr>, CrawlerError> {
    let lookup_host = lookup_host((SEED_NODE, MAINNET_PORT))
        .await
        .map_err(CrawlerError::LookupError)?;
    Ok(HashSet::from_iter(lookup_host))
}

async fn connect_node(
    remote_address: &SocketAddr,
) -> Result<Framed<TcpStream, BitcoinCodec>, CrawlerError> {
    let connection = TcpStream::connect(remote_address);

    let stream = timeout(Duration::from_millis(TIMEOUT_CONNECTION), connection)
        .await
        .map_err(CrawlerError::Timeout)?
        .map_err(CrawlerError::ConnectionError)?;

    Ok(Framed::new(stream, BitcoinCodec {}))
}

async fn perform_handshake(
    stream: &mut Framed<TcpStream, BitcoinCodec>,
    remote_address: &SocketAddr,
    local_address: &SocketAddr,
) -> Result<(), CrawlerError> {
    let version_message = RawNetworkMessage::new(
        Network::Bitcoin.magic(),
        NetworkMessage::Version(get_version_message(remote_address, local_address)),
    );

    stream
        .send(version_message)
        .await
        .map_err(CrawlerError::TcpError)?;

    while let Some(message) = stream.next().await {
        let message = message.map_err(CrawlerError::TcpError)?;

        match message.payload() {
            NetworkMessage::Verack => {
                let verack_message =
                    RawNetworkMessage::new(Network::Bitcoin.magic(), NetworkMessage::Verack);

                stream
                    .send(verack_message)
                    .await
                    .map_err(CrawlerError::TcpError)?;

                return Ok(());
            }
            _ => continue,
        }
    }

    Ok(())
}

async fn perform_get_addr(
    stream: &mut Framed<TcpStream, BitcoinCodec>,
) -> Result<HashSet<Address>, CrawlerError> {
    let mut set_nodes: HashSet<Address> = HashSet::new();

    let get_addr = RawNetworkMessage::new(Network::Bitcoin.magic(), NetworkMessage::GetAddr);

    stream
        .send(get_addr)
        .await
        .map_err(CrawlerError::TcpError)?;

    while let Some(message) = stream.next().await {
        let message = message.map_err(CrawlerError::TcpError)?;

        match message.payload().clone() {
            NetworkMessage::Addr(nodes) => {
                set_nodes.extend(nodes.into_iter().map(|item| item.1));
                return Ok(set_nodes);
            }

            _ => continue,
        }
    }
    Ok(set_nodes)
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
    #[allow(dead_code)] // I'm using it only for printing
    id: usize,
    thread: JoinHandle<()>,
}

impl Worker {
    fn new(
        id: usize,
        receiver: Arc<Mutex<Receiver<WorkerMessage>>>,
        set_nodes: Arc<Mutex<HashSet<Address>>>,
    ) -> Self {
        Self {
            id,
            thread: tokio::task::spawn(async move {
                loop {
                    let receiver_lock = receiver.clone();

                    let message = receiver.lock().unwrap().recv().unwrap();

                    match message {
                        WorkerMessage::Message(address) => {
                            println!("Thread: {id} - address: {address}");
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
        // TODO Change to &

        println!("Sending kill");
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
) -> Result<(), CrawlerError> {
    let local_address = SocketAddr::from_str(LOCAL_ADDRESS)
        .map_err(|_| CrawlerError::InvalidAddress(LOCAL_ADDRESS.to_string()))?;

    let mut stream = connect_node(&remote_address).await?;

    timeout(
        Duration::from_millis(TIMEOUT_FUN),
        perform_handshake(&mut stream, &remote_address, &local_address),
    )
    .await
    .map_err(CrawlerError::Timeout)??;

    let nodes = timeout(
        Duration::from_millis(TIMEOUT_FUN),
        perform_get_addr(&mut stream),
    )
    .await
    .map_err(CrawlerError::Timeout)??;

    set_nodes.lock().unwrap().extend(nodes.into_iter());

    Ok(())
}

enum WorkerMessage {
    Message(SocketAddr),
    Kill,
}
