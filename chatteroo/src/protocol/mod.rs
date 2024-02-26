//! Definitions of messages sent between stations.
//!
//! All structs in this module are agnostic with respect to which
//! type of channel will be used to transmit it. That is, an AX.25
//! channel or an M17 channel or whatever else will provide a
//! translation to or from the messages defined here. However, the
//! first channel type to be supported is AX.25 and the messages
//! have been designed to be efficiently representable that way.

pub mod chat;
pub mod epoch;
pub mod forum;
pub mod global;
pub mod network;
pub mod station;
