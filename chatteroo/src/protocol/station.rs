//! Station identifiers for participants in the Chatteroo network.
//!
//! A station identifier consists of their callsign and an SSID from 0-9.
//!
//! Chatteroo also defines a channel-agnostic compact binary encoding for
//! stations, which provides efficient packing for common callsign lengths. Note
//! that SSIDs do not extend to 15 as they do in AX.25 and those higher SSIDs
//! would be considered invalid in an AX.25 implementation of Chatteroo.
//!
//! The binary format has a 6-bit alphabet (values 0-63). The 6-bit values are
//! concatenated big-endian and a single value may span two bytes. A valid
//! station identifier will have at least one callsign letter or number and will
//! be terminated with an SSID value. When an SSID value is read, if the end of
//! the SSID value aligns with end of the end of a byte, then that is the end.
//! If the SSID value is split across two bytes, all remaining bits in the
//! second byte are to be ignored (and should be padded with 0 bits).
//!
//! The 6-bit values are as follows:
//!
//! * 0-25 are callsign letters from `A` to `Z`
//! * 26-35 are callsign numerals from `0` to `9`
//! * 36-45 are SSIDs from `0` to `9` (callsign complete)
//! * 46-55 are SSIDs from `0` to `9` (callsign requires net prefix)
//!
//! The difference between the two types of SSID is whether this is the callsign
//! in its entirety, or obtaining the true callsign requires the reader to add
//! the network name as a prefix.
//!
//! For example, all stations participating in the `VK7` network are aware of
//! this. The station `VK7XT-5` can transmit its name more efficiently by
//! sending `XT-5` plus an indication that the network name must be prefixed.

use crc32fast::Hasher;

use crate::error::Error;

/// Unique identifier for a participant in the chatteroo network.
///
/// Callsigns may only be ASCII uppercase and SSIDs must only be `0` to `9`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Station {
    callsign: String,
    ssid: u8,
}

impl Station {
    /// Construction a new Station from valid components.
    pub fn new(callsign: String, ssid: u8) -> Result<Station, Error> {
        if !callsign
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
        {
            return Err(Error::InvalidCallsign);
        }
        if ssid > 9 {
            return Err(Error::InvalidSsid);
        }
        Ok(Self { callsign, ssid })
    }

    /// Callsign part of station identifier, e.g. `VK7XT`.
    pub fn callsign(&self) -> &str {
        &self.callsign
    }

    /// Secondary Station Identifier (SSID), a number from `0` to `9`.
    pub fn ssid(&self) -> u8 {
        self.ssid
    }

    /// Stably allocate this station identifier into one of 16 buckets.
    ///
    /// Returns 0-15.
    pub fn bucket(&self) -> u8 {
        let mut hasher = Hasher::new();
        self.hash(&mut hasher);
        (hasher.finalize() % 16) as u8
    }

    /// Append this station identifier to a CRC32 hash state.
    pub fn hash(&self, hasher: &mut Hasher) {
        hasher.update(self.callsign.as_bytes());
        hasher.update(&[self.ssid]);
    }

    /// Produce compact binary encoding for this station identifier.
    ///
    /// `net_prefix` must be uppercase ASCII.
    pub fn encoded(&self, net_prefix: &str) -> Vec<u8> {
        let mut using_net_prefix = false;
        let mut callsign = self.callsign.to_string();
        if !net_prefix.is_empty() {
            if let Some(remainder) = callsign.strip_prefix(&net_prefix) {
                callsign = remainder.to_string();
                using_net_prefix = true;
            }
        }
        let values = callsign
            .chars()
            .map(|c| match c {
                'A'..='Z' => c as u8 - b'A',
                '0'..='9' => c as u8 - b'0' + 26,
                _ => {
                    unreachable!()
                }
            })
            .chain(std::iter::once(if using_net_prefix {
                self.ssid + 46
            } else {
                self.ssid + 36
            }));
        let mut out = vec![];
        for (i, value) in values.enumerate() {
            match i % 4 {
                0 => out.push(value << 2),
                1 => {
                    let last = out.last_mut().unwrap();
                    *last = *last | value >> 4;
                    out.push(value << 4);
                }
                2 => {
                    let last = out.last_mut().unwrap();
                    *last = *last | value >> 2;
                    out.push(value << 6);
                }
                3 => {
                    let last = out.last_mut().unwrap();
                    *last = *last | value;
                }
                _ => unreachable!(),
            }
        }
        out
    }

    /// Try to parse a station from the beginning of the encoded data.
    ///
    /// If successful, returns a `Station` instance and the remainder of
    /// `encoded` which follows. Otherwise returns an error.
    pub fn try_parse<'a, 'b>(
        mut encoded: &'a [u8],
        net_prefix: &'b str,
    ) -> Result<(Self, &'a [u8]), Error> {
        let mut values = vec![];
        while !encoded.is_empty() {
            let i = values.len();
            let value = match i % 4 {
                0 => encoded[0] >> 2,
                1 => {
                    if encoded.len() < 2 {
                        return Err(Error::InvalidStationIdentifier);
                    }
                    let value = (encoded[0] & 0b00000011) << 4 | (encoded[1] >> 4);
                    if encoded.len() < 2 {
                        return Err(Error::InvalidStationIdentifier);
                    }
                    encoded = &encoded[1..];
                    value
                }
                2 => {
                    if encoded.len() < 2 {
                        return Err(Error::InvalidStationIdentifier);
                    }
                    let value = (encoded[0] & 0b00001111) << 2 | (encoded[1] >> 6);
                    encoded = &encoded[1..];
                    value
                }
                3 => {
                    let value = encoded[0] & 0b00111111;
                    encoded = &encoded[1..];
                    value
                }
                _ => unreachable!(),
            };
            match value {
                v @ 0..=35 => values.push(v),
                v @ 36..=55 => {
                    // i=3 is the only case where `encoded` has already been moved on
                    // to "fresh" data. In other cases we must step past the padding
                    // before returning the remaining data.
                    if i != 3 && !encoded.is_empty() {
                        encoded = &encoded[1..];
                    }
                    values.push(v);
                    break;
                }
                _ => {
                    return Err(Error::InvalidStationIdentifier);
                }
            }
        }
        if let Some((ssid_value, callsign_values)) = values.split_last() {
            let mut callsign: String = callsign_values
                .iter()
                .map(|v| match v {
                    b @ 0..=25 => (b'A' + b) as char,
                    b @ 26..=35 => (b'0' + b - 26) as char,
                    _ => unreachable!(),
                })
                .collect();
            let ssid = match ssid_value {
                b @ 36..=45 => b - 36,
                b @ 46..=55 => {
                    callsign = format!("{}{}", net_prefix, callsign);
                    b - 46
                }
                _ => return Err(Error::InvalidStationIdentifier),
            };
            Ok((Station { callsign, ssid }, encoded))
        } else {
            Err(Error::InvalidStationIdentifier)
        }
    }
}

impl std::fmt::Display for Station {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.callsign, self.ssid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precise_test() {
        //                     < V  ><   K    ><   7    >< X  >    < T  >< SSID 5 >
        let expected_full = [0b01010100, 0b10101000, 0b01010111, 0b01001110, 0b10010000];
        let s = Station::new("VK7XT".to_owned(), 5).unwrap();
        let encoded = s.encoded("");
        assert_eq!(encoded, expected_full);

        // This time "SSID 5" is in the higher range that indicates a net prefix
        //                    < X  ><   T    >< SSID 5 >
        let expected_pre = [0b01011101, 0b00111100, 0b11000000];
        let encoded = s.encoded("VK7");
        assert_eq!(encoded, expected_pre);
    }

    #[test]
    fn expected_size() {
        let values = [
            // callsign, ssid, net prefix, expected bytes
            ("W1AW", 0, "", 4),
            ("VK7XT", 5, "", 5),
            ("VK7FDAE", 4, "", 6),
            // fewer bytes required when prefix is known
            ("VK7XT", 5, "VK7", 3),
            ("VK7NTK", 8, "VK7", 3),
            // but if prefix doesn't match get original result
            ("VK7XT", 5, "VK3", 5),
        ];
        for (callsign, ssid, prefix, expected_len) in values {
            let s = Station::new(callsign.to_owned(), ssid).unwrap();
            let encoded = s.encoded(prefix);
            assert_eq!(
                encoded.len(),
                expected_len,
                "{}-{} prefix {}",
                callsign,
                ssid,
                prefix
            );
        }
    }

    #[test]
    fn round_trip() {
        let callsigns = [("W1AW", 0), ("VK7XT", 5), ("VK7FDAE", 4), ("VK7NTK", 8)];
        let prefixes = ["", "VK7", "VK3"];
        for (c, ssid) in callsigns {
            for p in prefixes {
                let s = Station::new(c.to_owned(), ssid).unwrap();
                let encoded = s.encoded(p);
                let (decoded, _) = Station::try_parse(&encoded, p).unwrap();
                assert_eq!(s, decoded, "{}-{} prefix {}", c, ssid, p);
            }
        }
    }

    #[test]
    fn all_chars_and_sizes() {
        let full = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        for sub in 0..full.len() {
            let callsign = &full[sub..];
            for ssid in 0..=9 {
                let s = Station::new(callsign.to_owned(), ssid).unwrap();
                let encoded = s.encoded("");
                let (decoded, _) = Station::try_parse(&encoded, "").unwrap();
                assert_eq!(s, decoded, "{}-{}", callsign, ssid);
                // Make sure we don't panic if decoding truncated stations
                for i in 0..encoded.len() {
                    let _ = Station::try_parse(&encoded[i..], "");
                }
            }
        }
    }

    #[test]
    fn concatenated() {
        let s1 = Station::new("W1AW".to_owned(), 0).unwrap();
        let s2 = Station::new("VK7XT".to_owned(), 5).unwrap();
        let s3 = Station::new("VK7FDAE".to_owned(), 4).unwrap();

        let combined: Vec<u8> = s1
            .encoded("")
            .into_iter()
            .chain(s2.encoded("").into_iter())
            .chain(s3.encoded("").into_iter())
            .collect();

        let (read1, remainder) = Station::try_parse(&combined, "").unwrap();
        let (read2, remainder) = Station::try_parse(&remainder, "").unwrap();
        let (read3, remainder) = Station::try_parse(&remainder, "").unwrap();

        assert_eq!(read1, s1);
        assert_eq!(read2, s2);
        assert_eq!(read3, s3);
        assert!(remainder.is_empty());
    }

    #[test]
    fn buckets() {
        for (callsign, ssid) in [
            ("VK7XT", 5),
            ("W1AW", 0),
            ("VK7FDAE", 4),
            ("VK7NTK", 7),
            ("VK7NTK", 8),
            ("VK7NTK", 9),
        ] {
            let s = Station::new(callsign.to_owned(), ssid).unwrap();
            let mut hasher = Hasher::new();
            s.hash(&mut hasher);
            println!(
                "Call: {}\tHash: {:02X}\tBucket: {}",
                s,
                hasher.finalize(),
                s.bucket()
            );
        }
    }
}
