use crate::{
    codecs::BitcoinCodec, errors::CrawlerError, MAINNET_PORT, SEED_NODE, TIMEOUT_CONNECTION,
};
use std::{collections::HashSet, net::SocketAddr, time::Duration};
use tokio::{
    net::{lookup_host, TcpStream},
    time::timeout,
};
use tokio_util::codec::Framed;

pub async fn connect_node(
    remote_address: &SocketAddr,
) -> Result<Framed<TcpStream, BitcoinCodec>, CrawlerError> {
    let connection = TcpStream::connect(remote_address);

    let stream = timeout(Duration::from_millis(TIMEOUT_CONNECTION), connection)
        .await
        .map_err(CrawlerError::Timeout)?
        .map_err(CrawlerError::ConnectionError)?;

    Ok(Framed::new(stream, BitcoinCodec {}))
}

pub async fn get_seed_nodes() -> Result<HashSet<SocketAddr>, CrawlerError> {
    let lookup_host = lookup_host((SEED_NODE, MAINNET_PORT))
        .await
        .map_err(CrawlerError::LookupError)?;
    Ok(HashSet::from_iter(lookup_host))
}
