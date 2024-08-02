use crate::{codecs::BitcoinCodec, errors::CrawlerError};
use bitcoin::{
    p2p::{
        message::{NetworkMessage, RawNetworkMessage},
        message_network::VersionMessage,
        Address, ServiceFlags,
    },
    Network,
};
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use rand::{thread_rng, Rng};
use std::{collections::HashSet, net::SocketAddr};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

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

pub async fn perform_handshake(
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

pub async fn perform_get_addr(
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
