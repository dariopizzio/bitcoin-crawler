mod codecs;
mod errors;
mod messages;
mod tcp;
mod workers;

use errors::CrawlerError;
use std::{
    collections::HashSet,
    io,
    sync::{Arc, Mutex},
};
use tcp::get_seed_nodes;
use workers::ThreadPool;

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
