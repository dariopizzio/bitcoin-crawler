use crate::io::Error;
use thiserror::Error;
use tokio::time::error::Elapsed;

#[derive(Error, Debug)]
pub enum CrawlerError {
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("There was a timeout: {0}")]
    Timeout(Elapsed),
    #[error("There was an error while performing a DNS resolution: {0}")]
    LookupError(Error),
    #[error("There was an error while connecting with the remote address: {0}")]
    ConnectionError(Error),
    #[error("There was a TCP error: {0}")]
    TcpError(Error),
}
