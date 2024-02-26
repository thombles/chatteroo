//! Sending and receiving Chatteroo messages on different radio types.

use thiserror::Error;

use crate::protocol::global::Transmission;

pub mod ax25;

pub trait ChannelTx {
    fn send(&self, t: Transmission) -> Result<(), ChannelError>;
}

pub trait ChannelRx {
    fn recv(&self) -> Result<Transmission, ChannelError>;
}

#[derive(Error, Debug)]
pub enum ChannelError {
    #[error("Channel closed")]
    Offline,
}
