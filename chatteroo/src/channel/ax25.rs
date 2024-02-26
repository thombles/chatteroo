//! Chatteroo over AX.25

use std::str::FromStr;

use ax25::frame::{Address, Ax25Frame, FrameContent, ProtocolIdentifier, UnnumberedInformation};
use crc32fast::Hasher;
use thiserror::Error;

use crate::protocol::{
    global::{
        BucketContentRequest, BucketContentResponse, ChatterooVersion, Command, ContiguousRange,
        EpochRequest, EpochResponse, FrameDefinition, FrameRequest, FrameWithMetadata, InsertFrame,
        PingRequest, PingResponse, QuickEpochResponse, Range, StationDataRequest,
        StationDataResponse, StationHeard, StationSparse, StationSummary, Status, Transmission,
    },
    network::Network,
    station::Station,
};

use super::{ChannelError, ChannelRx, ChannelTx};

pub struct Ax25Channel {}

pub struct Ax25Tx {}

pub struct Ax25Rx {}

#[derive(Error, Debug)]
pub enum Ax25Error {
    #[error("Not a Chatteroo packet")]
    NotChatteroo,

    #[error("Invalid Chatteroo version {0}")]
    InvalidChatterooVersion(u8),

    #[error("Chatteroo protocol error {0:?}")]
    ProtocolError(crate::error::Error),

    #[error("Invalid Command")]
    InvalidCommand,

    #[error("Invalid station")]
    InvalidStation,

    #[error("Truncated message")]
    Truncated,

    #[error("Invalid UTF-8")]
    InvalidUtf8,

    #[error("Packet CRC did not match content")]
    CrcMismatch,
}

impl ChannelTx for Ax25Tx {
    fn send(&self, t: Transmission) -> Result<(), ChannelError> {
        let _packet = encode_transmission(&t);

        // TODO: actually send
        Ok(())
    }
}

impl ChannelRx for Ax25Rx {
    fn recv(&self) -> Result<Transmission, ChannelError> {
        unimplemented!();
    }
}

fn encode_transmission(t: &Transmission) -> Ax25Frame {
    let version = ssid_version(&t.version);
    let dest_addr_str = format!("CHT{}-{}", t.network.id(), version);
    let src_addr_str = t.sender.to_string();
    let pid = ProtocolIdentifier::None;
    let info = encode_command(&t.command, t.network.id());
    // Take the src, dest and info so far and add a 4-byte CRC
    // AX.25 frequently lets corrupt packets through and Chatteroo will be really
    // sensitive to any errors since it caches aggressively, so let's spend the bytes.
    let mut packet_hash = Hasher::new();
    packet_hash.update(&src_addr_str.as_bytes());
    packet_hash.update(&dest_addr_str.as_bytes());
    packet_hash.update(&info);
    let packet_hash = packet_hash.finalize();
    let info = info
        .into_iter()
        .chain(packet_hash.to_be_bytes().into_iter())
        .collect();
    let ui = UnnumberedInformation {
        pid,
        info,
        poll_or_final: false,
    };
    Ax25Frame {
        source: Address::from_str(&src_addr_str).unwrap(),
        destination: Address::from_str(&dest_addr_str).unwrap(),
        route: vec![],
        command_or_response: None,
        content: ax25::frame::FrameContent::UnnumberedInformation(ui),
    }
}

#[allow(dead_code)]
fn decode_transmission(frame: &Ax25Frame, net_prefix: &str) -> Result<Transmission, Ax25Error> {
    if !frame.destination.callsign.starts_with("CHT") {
        return Err(Ax25Error::NotChatteroo);
    }
    let info = match &frame.content {
        FrameContent::UnnumberedInformation(ui) => ui.info.as_slice(),
        _ => return Err(Ax25Error::NotChatteroo),
    };
    let version = match frame.destination.ssid {
        0 => ChatterooVersion::Test,
        1 => ChatterooVersion::V1,
        n => return Err(Ax25Error::InvalidChatterooVersion(n)),
    };
    let network = frame.destination.callsign[3..].to_owned();
    let network = Network::new(network).unwrap();
    let sender = match Station::new(frame.source.callsign.to_owned(), frame.source.ssid) {
        Ok(s) => s,
        Err(e) => return Err(Ax25Error::ProtocolError(e)),
    };
    if info.len() < 4 {
        return Err(Ax25Error::Truncated);
    }
    let (info, crc) = info.split_at(info.len() - 4);
    let packet_hash = u32::from_be_bytes([crc[0], crc[1], crc[2], crc[3]]);
    let mut received_hash = Hasher::new();
    received_hash.update(&frame.source.to_string().as_bytes());
    received_hash.update(&frame.destination.callsign.as_bytes());
    received_hash.update(&[b'-', frame.destination.ssid + b'0']);
    received_hash.update(&info);
    let received_hash = received_hash.finalize();
    if packet_hash != received_hash {
        return Err(Ax25Error::CrcMismatch);
    }
    if info.is_empty() {
        return Err(Ax25Error::InvalidCommand);
    }
    let command: Command = match info[0] & 0b00011111 {
        0 => {
            // Status
            let epoch_now_mod8 = info[0] >> 5;
            let remaining = &info[1..];
            let (epoch_4_ago_crc, remaining) = take_crc(remaining)?;
            let (epoch_3_ago_crc, remaining) = take_crc(remaining)?;
            let (epoch_2_ago_crc, remaining) = take_crc(remaining)?;
            let (epoch_1_ago_crc, remaining) = take_crc(remaining)?;
            let (epoch_now_crc, remaining) = take_crc(remaining)?;
            let (epoch_next_crc, mut remaining) = take_crc(remaining)?;
            let mut recently_added = vec![];
            for _ in 0..4 {
                if remaining.is_empty() {
                    break;
                }
                let (station, r) = Station::try_parse(remaining, net_prefix)
                    .map_err(|_| Ax25Error::InvalidStation)?;
                let (top, bottom, r) = take_contiguous_range(r)?;
                remaining = r;
                recently_added.push(StationSparse {
                    station,
                    top,
                    bottom,
                });
            }
            Command::Status(Status {
                epoch_now_mod8,
                epoch_4_ago_crc,
                epoch_3_ago_crc,
                epoch_2_ago_crc,
                epoch_1_ago_crc,
                epoch_now_crc,
                epoch_next_crc,
                recently_added,
            })
        }
        1 => {
            // Range
            if info.len() < 3 {
                return Err(Ax25Error::Truncated);
            }
            let final_page = info[1] >> 4;
            let page = info[1] & 0x0f;
            let stations_len = info[2];
            let mut remaining = &info[3..];
            let mut stations = vec![];
            for _ in 0..stations_len {
                let (station, r) = Station::try_parse(remaining, net_prefix)
                    .map_err(|_| Ax25Error::InvalidStation)?;
                stations.push(StationHeard {
                    station,
                    is_mutual: false,
                });
                remaining = r;
            }
            for (i, station) in stations.iter_mut().enumerate() {
                let mutual_idx = (i / 8) as usize;
                if let Some(byte) = remaining.get(mutual_idx) {
                    station.is_mutual = byte & 1 << (7 - (i % 8)) > 0;
                } else {
                    return Err(Ax25Error::Truncated);
                }
            }
            Command::Range(Range {
                final_page,
                page,
                stations,
            })
        }
        2 => {
            // InsertFrame
            let frame = decode_frame_with_metadata(&info[1..])?;
            Command::InsertFrame(InsertFrame { frame })
        }
        3 => {
            // RepeatFrame
            let (station, remaining) = Station::try_parse(&info[1..], net_prefix)
                .map_err(|_| Ax25Error::InvalidStation)?;
            let frame = decode_frame_with_metadata(remaining)?;
            Command::RepeatFrame(FrameDefinition { station, frame })
        }
        4 => {
            // QuickSyncFrameRequest
            let request = decode_frame_request(&info[1..], net_prefix)?;
            Command::QuickSyncFrameRequest(request)
        }
        5 => {
            // QuickSyncFrameResponse
            let (station, remaining) = Station::try_parse(&info[1..], net_prefix)
                .map_err(|_| Ax25Error::InvalidStation)?;
            let frame = decode_frame_with_metadata(remaining)?;
            Command::QuickSyncFrameResponse(FrameDefinition { station, frame })
        }
        6 => {
            // BackfillFrameRequest
            let request = decode_frame_request(&info[1..], net_prefix)?;
            Command::BackfillFrameRequest(request)
        }
        7 => {
            // BackfillFrameResponse
            let (station, remaining) = Station::try_parse(&info[1..], net_prefix)
                .map_err(|_| Ax25Error::InvalidStation)?;
            let frame = decode_frame_with_metadata(remaining)?;
            Command::BackfillFrameResponse(FrameDefinition { station, frame })
        }
        8 => {
            // EpochRequest
            let epoch_mod8 = info[0] >> 5;
            let (target, _) = Station::try_parse(&info[1..], net_prefix)
                .map_err(|_| Ax25Error::InvalidStation)?;
            Command::EpochRequest(EpochRequest { target, epoch_mod8 })
        }
        9 => {
            // QuickEpochResponse
            let epoch_mod8 = info[0] >> 5;
            let mut stations = vec![];
            let mut remaining = &info[1..];
            while !remaining.is_empty() {
                let (station, r) = take_station_summary(remaining, net_prefix)?;
                stations.push(station);
                remaining = r;
            }
            Command::QuickEpochResponse(QuickEpochResponse {
                epoch_mod8,
                stations,
            })
        }
        10 => {
            // EpochResponse
            let epoch_mod8 = info[0] >> 5;
            let mut checksums = [0u32; 16];
            let mut remaining = &info[1..];
            for c in checksums.iter_mut() {
                let (crc, r) = take_crc(remaining)?;
                *c = crc;
                remaining = r;
            }
            Command::EpochResponse(EpochResponse {
                epoch_mod8,
                checksums,
            })
        }
        11 => {
            // BucketContentRequest
            let epoch_mod8 = info[0] >> 5;
            let (target, remaining) = Station::try_parse(&info[1..], net_prefix)
                .map_err(|_| Ax25Error::InvalidStation)?;
            if remaining.len() < 1 {
                return Err(Ax25Error::Truncated);
            }
            let bucket = remaining[0] >> 4;
            let page = remaining[0] & 0x0f;
            Command::BucketContentRequest(BucketContentRequest {
                target,
                epoch_mod8,
                bucket,
                page,
            })
        }
        12 => {
            // BucketContentResponse
            let epoch_mod8 = info[0] >> 5;
            if info.len() < 2 {
                return Err(Ax25Error::Truncated);
            }
            let final_page = info[1] >> 4;
            let page = info[1] & 0x0f;
            let mut remaining = &info[2..];
            let mut stations = vec![];
            while !remaining.is_empty() {
                let (ss, r) = take_station_summary(remaining, net_prefix)?;
                stations.push(ss);
                remaining = r;
            }
            Command::BucketContentResponse(BucketContentResponse {
                epoch_mod8,
                final_page,
                page,
                stations,
            })
        }
        13 => {
            // StationDataRequest
            let (target, remaining) = Station::try_parse(&info[1..], net_prefix)
                .map_err(|_| Ax25Error::InvalidStation)?;
            let (station, remaining) =
                Station::try_parse(remaining, net_prefix).map_err(|_| Ax25Error::InvalidStation)?;
            if remaining.len() < 2 {
                return Err(Ax25Error::Truncated);
            }
            let epoch_mod8 = remaining[0] >> 5;
            let from_index = u16::from_be_bytes([remaining[0], remaining[1]]) & 0x1fff;
            Command::StationDataRequest(StationDataRequest {
                target,
                station,
                epoch_mod8,
                from_index,
            })
        }
        14 => {
            // StationDataResponse
            let (station, remaining) = Station::try_parse(&info[1..], net_prefix)
                .map_err(|_| Ax25Error::InvalidStation)?;
            if remaining.is_empty() {
                return Err(Ax25Error::Truncated);
            }
            let epoch_mod8 = remaining[0] & 0b0000111;
            let end_of_data = (remaining[0] & 0b10000000) > 0;
            let mut ranges = vec![];
            let mut remaining = &remaining[1..];
            while !remaining.is_empty() {
                let (top, bottom, r) = take_contiguous_range(remaining)?;
                ranges.push(ContiguousRange { top, bottom });
                remaining = r;
            }
            Command::StationDataResponse(StationDataResponse {
                station,
                epoch_mod8,
                end_of_data,
                ranges,
            })
        }
        15 => {
            // PingRequest
            let (target, _) = Station::try_parse(&info[1..], net_prefix)
                .map_err(|_| Ax25Error::InvalidStation)?;
            Command::PingRequest(PingRequest { target })
        }
        16 => {
            // PingResponse
            let (target, remaining) = Station::try_parse(&info[1..], net_prefix)
                .map_err(|_| Ax25Error::InvalidStation)?;
            let diagnostic = std::str::from_utf8(remaining)
                .map_err(|_| Ax25Error::InvalidUtf8)?
                .to_string();
            Command::PingResponse(PingResponse { target, diagnostic })
        }
        _ => return Err(Ax25Error::InvalidCommand),
    };

    Ok(Transmission {
        version,
        network,
        sender,
        command,
    })
}

fn ssid_version(v: &ChatterooVersion) -> u8 {
    match v {
        ChatterooVersion::Test => 0,
        ChatterooVersion::V1 => 1,
    }
}

fn take_crc(buf: &[u8]) -> Result<(u32, &[u8]), Ax25Error> {
    if buf.len() < 4 {
        return Err(Ax25Error::Truncated);
    }
    let crc = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
    Ok((crc, &buf[4..]))
}

fn command_byte(c: &Command) -> u8 {
    match c {
        Command::Status(_) => 0,
        Command::Range(_) => 1,
        Command::InsertFrame(_) => 2,
        Command::RepeatFrame(_) => 3,
        Command::QuickSyncFrameRequest(_) => 4,
        Command::QuickSyncFrameResponse(_) => 5,
        Command::BackfillFrameRequest(_) => 6,
        Command::BackfillFrameResponse(_) => 7,
        Command::EpochRequest(_) => 8,
        Command::QuickEpochResponse(_) => 9,
        Command::EpochResponse(_) => 10,
        Command::BucketContentRequest(_) => 11,
        Command::BucketContentResponse(_) => 12,
        Command::StationDataRequest(_) => 13,
        Command::StationDataResponse(_) => 14,
        Command::PingRequest(_) => 15,
        Command::PingResponse(_) => 16,
    }
}

fn encode_command(c: &Command, net_prefix: &str) -> Vec<u8> {
    let mut out = vec![];
    // Top 3 bits of command byte may be used for an epoch mod8 to save space
    let mut cmd_byte = command_byte(&c);
    match c {
        Command::Status(status) => {
            cmd_byte |= status.epoch_now_mod8 << 5;
            out.push(cmd_byte);
            out.extend(status.epoch_4_ago_crc.to_be_bytes().into_iter());
            out.extend(status.epoch_3_ago_crc.to_be_bytes().into_iter());
            out.extend(status.epoch_2_ago_crc.to_be_bytes().into_iter());
            out.extend(status.epoch_1_ago_crc.to_be_bytes().into_iter());
            out.extend(status.epoch_now_crc.to_be_bytes().into_iter());
            out.extend(status.epoch_next_crc.to_be_bytes().into_iter());
            for ss in &status.recently_added {
                out.extend(ss.station.encoded(net_prefix));
                encode_contiguous_range(ss.top, ss.bottom, &mut out);
            }
        }
        Command::Range(range) => {
            out.push(cmd_byte);
            let mut page_byte = range.page;
            page_byte |= range.final_page << 4;
            out.push(page_byte);
            out.push(range.stations.len() as u8);
            let mutual_len = if range.stations.len() % 8 > 0 {
                range.stations.len() / 8 + 1
            } else {
                range.stations.len()
            };
            let mut mutual = vec![0; mutual_len];
            for (i, sh) in range.stations.iter().enumerate() {
                out.extend(sh.station.encoded(net_prefix));
                if sh.is_mutual {
                    mutual[i / 8] |= mutual[i / 8] | (1 << (7 - (i % 8)));
                }
            }
            out.append(&mut mutual);
        }
        Command::InsertFrame(insert) => {
            out.push(cmd_byte);
            encode_frame_with_metadata(&insert.frame, &mut out);
        }
        Command::RepeatFrame(repeat) => {
            out.push(cmd_byte);
            out.extend(repeat.station.encoded(net_prefix));
            encode_frame_with_metadata(&repeat.frame, &mut out);
        }
        Command::QuickSyncFrameRequest(request) => {
            out.push(cmd_byte);
            encode_frame_request(request, net_prefix, &mut out);
        }
        Command::QuickSyncFrameResponse(response) => {
            out.push(cmd_byte);
            out.extend(response.station.encoded(net_prefix));
            encode_frame_with_metadata(&response.frame, &mut out);
        }
        Command::BackfillFrameRequest(request) => {
            out.push(cmd_byte);
            encode_frame_request(request, net_prefix, &mut out);
        }
        Command::BackfillFrameResponse(response) => {
            out.push(cmd_byte);
            out.extend(response.station.encoded(net_prefix));
            encode_frame_with_metadata(&response.frame, &mut out);
        }
        Command::EpochRequest(epoch) => {
            cmd_byte |= epoch.epoch_mod8 << 5;
            out.push(cmd_byte);
            out.extend(epoch.target.encoded(net_prefix));
        }
        Command::QuickEpochResponse(response) => {
            cmd_byte |= response.epoch_mod8 << 5;
            out.push(cmd_byte);
            for ss in &response.stations {
                encode_station_summary(ss, net_prefix, &mut out);
            }
        }
        Command::EpochResponse(response) => {
            cmd_byte |= response.epoch_mod8 << 5;
            out.push(cmd_byte);
            for crc in response.checksums {
                out.extend(crc.to_be_bytes().into_iter());
            }
        }
        Command::BucketContentRequest(request) => {
            cmd_byte |= request.epoch_mod8 << 5;
            out.push(cmd_byte);
            out.extend(request.target.encoded(net_prefix));
            let mut page = request.page;
            page |= request.bucket << 4;
            out.push(page);
        }
        Command::BucketContentResponse(response) => {
            cmd_byte |= response.epoch_mod8 << 5;
            out.push(cmd_byte);
            let mut page = response.page;
            page |= response.final_page << 4;
            out.push(page);
            for ss in &response.stations {
                encode_station_summary(ss, net_prefix, &mut out);
            }
        }
        Command::StationDataRequest(request) => {
            out.push(cmd_byte);
            out.extend(request.target.encoded(net_prefix));
            out.extend(request.station.encoded(net_prefix));
            let index = request.from_index | (request.epoch_mod8 as u16) << 13;
            out.extend(index.to_be_bytes().into_iter());
        }
        Command::StationDataResponse(response) => {
            out.push(cmd_byte);
            out.extend(response.station.encoded(net_prefix));
            let mut epoch = response.epoch_mod8;
            if response.end_of_data {
                epoch |= 1 << 7;
            }
            out.push(epoch);
            for r in &response.ranges {
                encode_contiguous_range(r.top, r.bottom, &mut out);
            }
        }
        Command::PingRequest(request) => {
            out.push(cmd_byte);
            out.extend(request.target.encoded(net_prefix));
        }
        Command::PingResponse(response) => {
            out.push(cmd_byte);
            out.extend(response.target.encoded(net_prefix));
            out.extend(response.diagnostic.as_bytes());
        }
    }
    out
}

fn encode_frame_with_metadata(f: &FrameWithMetadata, out: &mut Vec<u8>) {
    let mut index = f.index;
    index |= (f.epoch_mod8 as u16) << 13;
    out.extend(index.to_be_bytes().into_iter());
    let mut application = f.application & 0x0f;
    if f.start_of_message {
        application |= 1 << 7;
    }
    if f.end_of_message {
        application |= 1 << 6;
    }
    out.push(application);
    out.extend(f.data.iter());
}

fn decode_frame_with_metadata(buf: &[u8]) -> Result<FrameWithMetadata, Ax25Error> {
    if buf.len() < 3 {
        return Err(Ax25Error::Truncated);
    }
    let epoch_mod8 = buf[0] >> 5;
    let index = u16::from_be_bytes([buf[0], buf[1]]) & 0x1fff;
    let application = buf[2] & 0x0f;
    let start_of_message = buf[2] & (1 << 7) > 0;
    let end_of_message = buf[2] & (1 << 6) > 0;
    let data = buf[3..].to_vec();
    Ok(FrameWithMetadata {
        epoch_mod8,
        index,
        start_of_message,
        end_of_message,
        application,
        data,
    })
}

fn encode_frame_request(fr: &FrameRequest, net_prefix: &str, out: &mut Vec<u8>) {
    out.extend(fr.target.encoded(net_prefix));
    out.extend(fr.inserter.encoded(net_prefix));
    let mut index = fr.index;
    index |= (fr.epoch_mod8 as u16) << 13;
    out.extend(index.to_be_bytes().into_iter());
}

fn decode_frame_request(buf: &[u8], net_prefix: &str) -> Result<FrameRequest, Ax25Error> {
    let (target, remaining) =
        Station::try_parse(&buf, net_prefix).map_err(|_| Ax25Error::InvalidStation)?;
    let (inserter, remaining) =
        Station::try_parse(&remaining, net_prefix).map_err(|_| Ax25Error::InvalidStation)?;
    if remaining.len() < 2 {
        return Err(Ax25Error::Truncated);
    }
    let epoch_mod8 = remaining[0] >> 5;
    let index = u16::from_be_bytes([remaining[0], remaining[1]]) & 0x1fff;
    Ok(FrameRequest {
        target,
        inserter,
        epoch_mod8,
        index,
    })
}

fn encode_contiguous_range(top: u16, bottom: u16, out: &mut Vec<u8>) {
    let mut top = top;
    if bottom == 0 {
        top |= 1 << 15;
        out.extend(top.to_be_bytes().into_iter());
    } else {
        out.extend(top.to_be_bytes().into_iter());
        out.extend(bottom.to_be_bytes().into_iter());
    }
}

fn take_contiguous_range(buf: &[u8]) -> Result<(u16, u16, &[u8]), Ax25Error> {
    if buf.is_empty() {
        return Err(Ax25Error::Truncated);
    }
    if buf[0] & 0b10000000 > 0 {
        if buf.len() < 2 {
            return Err(Ax25Error::Truncated);
        }
        let top = u16::from_be_bytes([buf[0] & 0b01111111, buf[1]]);
        Ok((top, 0, &buf[2..]))
    } else {
        if buf.len() < 4 {
            return Err(Ax25Error::Truncated);
        }
        let top = u16::from_be_bytes([buf[0], buf[1]]);
        let bottom = u16::from_be_bytes([buf[2], buf[3]]);
        Ok((top, bottom, &buf[4..]))
    }
}

fn encode_station_summary(ss: &StationSummary, net_prefix: &str, out: &mut Vec<u8>) {
    out.extend(ss.station.encoded(net_prefix));
    encode_contiguous_range(ss.top, ss.bottom, out);
    out.extend(ss.epoch_crc.to_be_bytes().into_iter());
}

fn take_station_summary<'a, 'b>(
    buf: &'a [u8],
    net_prefix: &'b str,
) -> Result<(StationSummary, &'a [u8]), Ax25Error> {
    let (station, remaining) =
        Station::try_parse(buf, net_prefix).map_err(|_| Ax25Error::InvalidStation)?;
    let (top, bottom, remaining) = take_contiguous_range(remaining)?;
    let (epoch_crc, remaining) = take_crc(remaining)?;
    Ok((
        StationSummary {
            station,
            top,
            bottom,
            epoch_crc,
        },
        remaining,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_roundtrip() {
        let station = Station::new("VK7XT".to_owned(), 4).unwrap();
        let net_prefix = "VK7";
        let t = Transmission {
            version: ChatterooVersion::Test,
            network: Network::new(net_prefix.to_owned()).unwrap(),
            sender: station.clone(),
            command: Command::Status(Status {
                epoch_now_mod8: 1,
                epoch_4_ago_crc: 0xaaaaaaaa,
                epoch_3_ago_crc: 0xbbbbbbbb,
                epoch_2_ago_crc: 0xcccccccc,
                epoch_1_ago_crc: 0xdddddddd,
                epoch_now_crc: 0xeeeeeeee,
                epoch_next_crc: 0xffffffff,
                recently_added: vec![StationSparse {
                    station,
                    top: 50,
                    bottom: 0,
                }],
            }),
        };
        let encoded = encode_transmission(&t);
        let decoded = decode_transmission(&encoded, net_prefix).unwrap();
        assert_eq!(t, decoded);
    }
}
