use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Tried to restore a mod-8 epoch value that makes no sense - likely clock skew")]
    UnreadableEpoch,

    #[error("Unable to parse a station identifier")]
    InvalidStationIdentifier,

    #[error("Callsign is not uppercase ASCII")]
    InvalidCallsign,

    #[error("Network name is not an ASCII identifier of appropriate length")]
    InvalidNetwork,

    #[error("SSID is not between 0 and 9")]
    InvalidSsid,
}
