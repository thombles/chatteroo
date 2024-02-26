//! Chatteroo network identifier strings.

use crate::error::Error;

/// Chatteroo network identifier.
///
/// Nodes in different networks are completely independent and will
/// ignore each other. A network should be a "manageable" size, i.e.,
/// a community within which everybody will be interested in everyone
/// else's data, without overloading the channels.
///
/// The network identifier is a string containing up to 3 characters,
/// which may be uppercase ASCII or numerals from 0 to 9. While not
/// required, it is recommended to choose a network identifier which
/// is a prefix of the callsigns in use within that geographic region.
/// This is not only self-describing who the network is "for", but it
/// enables more efficient use of the channel since callsigns can be
/// transmitted in a more compact format when they are an extension of
/// the network identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network(String);

impl Network {
    pub fn new(network: String) -> Result<Self, Error> {
        if network
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            && network.len() <= 3
        {
            Ok(Self(network))
        } else {
            Err(Error::InvalidNetwork)
        }
    }

    pub fn id(&self) -> &str {
        &self.0
    }
}
