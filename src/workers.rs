use crate::{
    errors::CrawlerError,
    messages::{perform_get_addr, perform_handshake},
    tcp::connect_node,
    LOCAL_ADDRESS, TIMEOUT_FUN,
};
use bitcoin::p2p::Address;
use std::{
    collections::HashSet,
    net::SocketAddr,
    str::FromStr,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{join, task::JoinHandle, time::timeout};

struct Worker {
    #[allow(dead_code)] // I'm using it only for printing
    id: usize,
    thread: JoinHandle<()>,
}

enum WorkerMessage {
    Message(SocketAddr),
    Kill,
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

pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: Sender<WorkerMessage>,
}

impl ThreadPool {
    pub fn new(size: usize, set_nodes: Arc<Mutex<HashSet<Address>>>) -> Self {
        let mut workers = Vec::with_capacity(size);

        let (sender, receiver) = mpsc::channel::<WorkerMessage>();

        let receiver = Arc::new(Mutex::new(receiver));

        for i in 0..size {
            workers.push(Worker::new(i, receiver.clone(), set_nodes.clone()))
        }

        Self { workers, sender }
    }

    pub fn execute(&self, seed_nodes: HashSet<SocketAddr>) {
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

    pub async fn join(self) {
        for worker in self.workers {
            let _ = join!(worker.thread);
        }
    }
}
