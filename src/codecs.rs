use bitcoin::{
    consensus::{deserialize_partial, serialize},
    p2p::message::RawNetworkMessage,
};
use std::io;
use tokio_util::{
    bytes::Buf,
    codec::{Decoder, Encoder},
};

pub struct BitcoinCodec {}

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
