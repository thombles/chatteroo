//! Global messages related to data frame sync, not app-specific.

use super::{network::Network, station::Station};

/// Entire Chatteroo message sent or received on a radio channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transmission {
    pub version: ChatterooVersion,
    pub network: Network,
    pub sender: Station,
    pub command: Command,
}

/// Chatteroo protocol version.
///
/// This is intended to be used when breaking changes are made to the
/// protocol and it's important that nodes on the newer and older
/// versions do not interoperate. If and when that happens, there is
/// no intention to provide forwards or backwards compatibility at the
/// network layer. (If possible, the offline database will be rolled
/// forward however since it would be a shame to lose old messages.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatterooVersion {
    /// Development, experimentation, etc.
    Test,
    /// Placeholder for protocol version 1 when this is stable.
    V1,
}

/// Payload variant inside `Transmission`.
///
/// Note that some payloads are identical but have different semantic meanings
/// based on the command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Status(Status),

    Range(Range),

    InsertFrame(InsertFrame),
    RepeatFrame(FrameDefinition),

    QuickSyncFrameRequest(FrameRequest),
    QuickSyncFrameResponse(FrameDefinition),

    BackfillFrameRequest(FrameRequest),
    BackfillFrameResponse(FrameDefinition),

    EpochRequest(EpochRequest),
    QuickEpochResponse(QuickEpochResponse),
    EpochResponse(EpochResponse),

    BucketContentRequest(BucketContentRequest),
    BucketContentResponse(BucketContentResponse),

    StationDataRequest(StationDataRequest),
    StationDataResponse(StationDataResponse),

    PingRequest(PingRequest),
    PingResponse(PingResponse),
}

/// Station announces what data it has and recently-added frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// Current epoch from point of view of transmitting station. (0-7)
    pub epoch_now_mod8: u8,

    /// Checksum of epoch 4 weeks before current epoch.
    pub epoch_4_ago_crc: u32,

    /// Checksum of epoch 3 weeks before current epoch.
    pub epoch_3_ago_crc: u32,

    /// Checksum of epoch 2 weeks before current epoch.
    pub epoch_2_ago_crc: u32,

    /// Checksum of previous epoch.
    pub epoch_1_ago_crc: u32,

    /// Checksum of the epoch currently underway.
    pub epoch_now_crc: u32,

    /// Checksum of epoch after the current one.
    ///
    /// In an ideal world this would always be an empty checksum but
    /// if there is minor clock skew then a station who is "in the
    /// past" might start receiving data from a device "in the
    /// future". This data should still be flood-filled.
    ///
    /// Incorrect clocks will of course play havoc with application-
    /// level messages like chat histories, but so long as those
    /// timestamps are tied to the epoch of their underlying data
    /// frames then any errors will be constrained within an 8-week
    /// range then get flushed out.
    pub epoch_next_crc: u32,

    /// List of up to 4 stations whose data frames we received last.
    ///
    /// The idea is that a station who is mostly up-to-date will be
    /// able to pick off a precise station+frame combination that they
    /// don't have yet and request it directly, without having to go
    /// through the laborious backfill process. This opportunistic
    /// catch-up is called "quick sync".
    pub recently_added: Vec<StationSparse>,
}

/// A Station paired with a subset of the data frames we know from them.
///
/// It is implied that this refers to the current epoch, which must be
/// transmitted externally to this structure.
///
/// If we have complete knowledge of this station's frames for this epoch
/// then `bottom` will be 0. It is advisable to have an efficient encoding
/// for this case since it will be common.
///
/// If the range is broken up, we send the indices of the highest contiguous
/// range because the newest transmissions are the ones that stations around
/// us are least likely to have. The newest data is also the most interesting
/// from the user's perspective, so make it quick and easy to fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StationSparse {
    /// A station for which at least one frame is known.
    pub station: Station,

    /// End index (inclusive) of the highest contiguous block of frames.
    pub top: u16,

    /// Start index (inclusive) of the highest contiguous block of frames.
    pub bottom: u16,
}

/// Station is indicating which stations are in radio range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    /// Index (starting from 0) of the last page of data when spread across
    /// multiple `Range`s. In other words, (page count - 1).
    ///
    /// Must be in the range 0-15 (inclusive).
    pub final_page: u8,

    /// Which page this transmission represents.
    ///
    /// Must be in the range 0 to `page_count` (inclusive).
    pub page: u8,

    /// Stations that have been heard recently, and if they hear us.
    ///
    /// These have variable-length encoding and data should be paginated
    /// such that the encoded form of the `Range` is a maximum of
    /// approx 80 bytes.
    pub stations: Vec<StationHeard>,
}

/// A Station that we can hear, paired with whether we think they hear us.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StationHeard {
    /// Remote station that is in range (we can hear them)
    pub station: Station,

    /// Whether we have confirmation that they have heard us recently.
    ///
    /// If in doubt, false.
    pub is_mutual: bool,
}

/// Station is inserting a data frame of their own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InsertFrame {
    /// Frame inserted.
    pub frame: FrameWithMetadata,
}

/// Full information about a frame except who inserted it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameWithMetadata {
    /// Current epoch. (0-7)
    pub epoch_mod8: u8,

    /// Index of this frame within the epoch. (0-8191)
    pub index: u16,

    /// Is this frame the first in a higher-level message?
    pub start_of_message: bool,

    /// Is this frame the last in a higher-level message?
    pub end_of_message: bool,

    /// Which application will parse this message? (0-15)
    pub application: u8,

    /// Frame content. Maximum length is 80 bytes.
    pub data: Vec<u8>,
}

/// Station is sharing a data frame from someone else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameDefinition {
    /// Station which originally inserted this frame.
    pub station: Station,

    /// Frame which was inserted by `station`.
    pub frame: FrameWithMetadata,
}

/// Station requests another station to repeat a frame that they have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRequest {
    /// Station who is being asked to transmit the frame.
    ///
    /// Only this station may reply to the request.
    pub target: Station,

    /// Station who originally inserted the frame.
    pub inserter: Station,

    /// Epoch the frame is in.
    pub epoch_mod8: u8,

    /// Insertion index of the data frame within that epoch.
    pub index: u16,
}

/// Station requests another station to provide more detail about a
/// given epoch.
///
/// Responder will send an `QuickEpochResponse` if this is possible,
/// or else an `EpochResponse`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpochRequest {
    /// Station being asked about the data they have.
    ///
    /// Only this station may reply to the request.
    pub target: Station,

    /// Requested epoch.
    pub epoch_mod8: u8,
}

/// Station summarises the data in a given epoch by breaking it down into
/// the checksums associated with each station identifier.
///
/// This will be used in place of `EpochResponse` in smaller networks where all
/// of the station identifiers and their checksums will fit within a single
/// message. Once the number of stations means that is no longer possible, all
/// stations will start using `EpochResponse` and buckets to perform backfill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickEpochResponse {
    /// Epoch we're talking about.
    pub epoch_mod8: u8,

    /// Details about stations within this epoch, such that the entire payload
    /// of this part doesn't exceed 80 bytes.
    pub stations: Vec<StationSummary>,
}

/// Station summarises an epoch's data in by sorting station identifiers
/// into 16 buckets and checksumming the data within each bucket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpochResponse {
    /// Epoch whose data content is being summarised.
    pub epoch_mod8: u8,

    /// For each bucket of station identifiers, CRC of data frames in this epoch.
    ///
    /// Station identifiers are allocated to one of 16 buckets by suffix of CRC.
    pub checksums: [u32; 16],
}

/// Station requests another station to provide more detail about a
/// bucket within a given epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BucketContentRequest {
    /// Station being asked about data which they have.
    ///
    /// Only this station may reply to the request.
    pub target: Station,

    /// Requested epoch
    pub epoch_mod8: u8,

    /// Requested bucket of station identifiers. (0-15)
    pub bucket: u8,

    /// Requested page of station identifiers within the bucket. (0-15)
    pub page: u8,
}

/// Station reports the station identifiers in a specific bucket and the
/// checksum of the data associated with each station.
///
/// The bucket is implicit from the CRC of the station identifiers included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BucketContentResponse {
    /// Epoch we're talking about.
    pub epoch_mod8: u8,

    /// Index of the final page for stations in this bucket. (0-15)
    pub final_page: u8,

    /// Page number of this message. (0-15)
    pub page: u8,

    /// Details about stations within this page, such that the entire payload
    /// of this part doesn't exceed 80 bytes.
    pub stations: Vec<StationSummary>,
}

/// More detailed information about frames inserted by a particular station.
///
/// The epoch is implicit and must be specified separately from this struct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StationSummary {
    /// The station whose data frames we're talking about.
    pub station: Station,

    /// Highest index (inclusive) of the highest contiguous block of frames.
    pub top: u16,

    /// Lowest index (inclusive) of the highest contiguous block of frames.
    pub bottom: u16,

    /// CRC of all data frames known for this station in this epoch.
    pub epoch_crc: u32,
}

/// Station requests another station to list the frames it has which
/// were inserted by a given station during a given epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StationDataRequest {
    /// Station being asked about data which they have.
    ///
    /// Only this station may reply to the request.
    pub target: Station,

    /// Station who originally inserted the data being requested.
    pub station: Station,

    /// Requested epoch.
    pub epoch_mod8: u8,

    /// Start listing indices from the range that contains this value.
    ///
    /// Used to perform stable pagination. If a `StationDataResponse` indicates
    /// that it is not `end_of_data`, then the interrogating station will ask
    /// for the previous `top` + 1 as its next `from_index`.
    pub from_index: u16,
}

/// Station reports which frames exist for a given station identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StationDataResponse {
    /// Station who inserted these data frames.
    pub station: Station,

    /// Epoch we're talking about.
    pub epoch_mod8: u8,

    /// Does this command include the highest known index for this station?
    pub end_of_data: bool,

    /// Ordered list of known blocks for this station, expressed as a sequence
    /// of sparse contiguous ranges.
    ///
    /// Size of this payload should not exceed 80 bytes, beyond which an
    /// additional page should be used.
    pub ranges: Vec<ContiguousRange>,
}

/// Range of data frame indices known for a particular station.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContiguousRange {
    /// Top index (inclusive)
    pub top: u16,

    /// Bottom index (inclusive)
    pub bottom: u16,
}

/// Station requests a single diagnostic response from a target station.
///
/// Pings are to be used for manually testing to verify station liveness or
/// reachability, tune antennas, or check what version of software a station is
/// using.
///
/// Pings have no impact whatsoever on the regular functions of Chatteroo
/// (participating in a ping request or response does not count as a "heard
/// station"). It should only be used manually by operators and there is no
/// reason to make ping requests in normal network operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PingRequest {
    /// Station that is requested to respond to a ping.
    pub target: Station,
}

/// Stations responds to a `PingRequest`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PingResponse {
    /// The station to whose ping is being replied.
    pub target: Station,

    /// A short string (less than 80 UTF-8 bytes) decribing the software.
    ///
    /// It's recommended that this indicates a software version. For example:
    /// > `Chatteroo by VK7XT v1.5.0`
    pub diagnostic: String,
}
