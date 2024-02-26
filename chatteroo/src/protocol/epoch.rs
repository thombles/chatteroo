//! Week-long blocks of time within which data is synced

use crate::error::Error;
use time::macros::datetime;
use time::OffsetDateTime;

/// Beginning of time in the chatteroo universe
const START: OffsetDateTime = datetime!(2020-01-01 0:00 UTC);

/// A particular week, used to specify regions of time that can come into sync and fall out of
/// sync in a coordinated manner across the network. When new frames are created they implicitly
/// belong to current epoch. The receive time of frames is also tracked in terms of epochs/blocks.
///
/// Officially an epoch is an absolute monotonically-increasing index beginning from 0. This will
/// probably be abbreviated in binary payloads.
///
/// * Epoch 0 lasts from 2020-01-01 00:00:00 to 2020-01-07 23:59:59.
/// * Epoch 1 lasts from 2020-01-08 00:00:00 to 2020-01-14 23:59:59.
/// * And so on.
#[derive(Debug, Eq, PartialEq)]
pub struct Epoch {
    abs: u32,
}

impl Epoch {
    /// Returns the current epoch
    pub fn now() -> Self {
        Self::at(OffsetDateTime::now_utc())
    }

    /// Restore an `Epoch` from the abbreviated mod-8 format.
    ///
    /// If an epoch is converted to mod-8 form then it is intended to be converted back shortly
    /// thereafter - for example if it's transmitted over a packet radio channel, and the receiver
    /// wants to get the absolute form back again. The receiver will come up with the correct
    /// absolute value provided both stations' clocks are within one week of each other.
    ///
    /// On the sending side, `index_mod8`, only the current epoch and previous 4 epochs may be used.
    /// This represents 5 of the possible mod-8 values. Due to clock skew, the receiver may
    /// perceive it as one week earlier or later than that range. For example
    ///
    /// * Receiver is one hour behind sender. Sender ticks over to a new epoch and sends a message.
    ///   The receiver still thinks "now" is the previous epoch, so it will receive a message from
    ///   +1 epoch into the future. This is okay.
    /// * Receiver is one hour ahead of sender. Receiver ticks over to a new epoch and receives a
    ///   message where the sender was referring to epoch (now - 4). From the receiver's perspective
    ///   that is actually epoch (now - 5). This is also okay.
    ///
    /// With this extra +1 and -1 to support these skew cases, 7 out of the 8 possible values are
    /// now accounted for. What of the 8th? Well logically we could extend it a bit further and
    /// guess that it means (now - 6), or (now + 2). Instead we will treat it as an indication that
    /// something has gone terribly wrong with our relative clocks since it should never happen.
    /// Therefore if we hit this "dead value", `from_mod8` will return an error.
    pub fn from_mod8(mod8: u8) -> Result<Self, Error> {
        let now_abs = Self::now().abs;
        let curr_candidate = (now_abs & 0xfffffff8) + mod8 as u32;
        let upper_candidate = curr_candidate + 8;
        let lower_candidate = curr_candidate - 8;
        if curr_candidate >= (now_abs - 5) && curr_candidate <= (now_abs + 1) {
            Ok(Self {
                abs: curr_candidate,
            })
        } else if lower_candidate >= (now_abs - 5) && lower_candidate <= (now_abs + 1) {
            Ok(Self {
                abs: lower_candidate,
            })
        } else if upper_candidate >= (now_abs - 5) && upper_candidate <= (now_abs + 1) {
            Ok(Self {
                abs: upper_candidate,
            })
        } else {
            Err(Error::UnreadableEpoch)
        }
    }

    /// Absolute numeric index of this `Epoch`
    pub fn index_abs(&self) -> u32 {
        self.abs
    }

    /// The last 3 bits of this `Epoch`'s index.
    ///
    /// This can be restored to an absolute epoch using context (the current time) via `from_mod8`.
    ///
    /// For correct decoding and protection against clock skew, this must only be called on the
    /// current epoch or on the 4 epochs immediately before the current epoch.
    pub fn index_mod8(&self) -> u8 {
        (self.abs % 8) as u8
    }

    /// How many weeks old this epoch is, relative to now.
    ///
    /// May be negative if this epoch is from the future - particularly possible if talking to
    /// another station with some clock skew.
    pub fn age(&self) -> i32 {
        let now = Self::now();
        now.abs as i32 - self.abs as i32
    }

    /// Returns an Epoch for a particular given time
    fn at(dt: OffsetDateTime) -> Self {
        let diff = dt - START;
        Self {
            abs: diff.whole_weeks() as u32,
        }
    }
}

/// A particular hour, used to specify regions of time during which messages were received.
///
/// As a station receives frames, the time of receipt assigns that frame locally to a particular
/// block, the one in progress. These time ranges form part of the slow-fill sync algorithm.
///
/// Blocks are a subdivision of epochs - in a given epoch (week) there are 168 hours, so the block
/// index can be from 0 to 167 inclusive.
pub struct Block {
    epoch: Epoch,
    index: u32,
}

impl Block {
    pub fn epoch(&self) -> &Epoch {
        &self.epoch
    }

    pub fn index(&self) -> u32 {
        self.index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_edges() {
        let epoch0_start = datetime!(2020-01-01 00:00:00 UTC);
        let epoch0_end = datetime!(2020-01-07 23:59:59 UTC);
        let epoch1_start = datetime!(2020-01-08 00:00:00 UTC);
        let epoch1_end = datetime!(2020-01-14 23:59:59 UTC);
        let epoch2_start = datetime!(2020-01-15 00:00:00 UTC);
        let one_year_later = datetime!(2021-01-01 12:00:00 UTC);

        assert_eq!(Epoch::at(epoch0_start).index_abs(), 0);
        assert_eq!(Epoch::at(epoch0_end).index_abs(), 0);
        assert_eq!(Epoch::at(epoch1_start).index_abs(), 1);
        assert_eq!(Epoch::at(epoch1_end).index_abs(), 1);
        assert_eq!(Epoch::at(epoch2_start).index_abs(), 2);
        assert_eq!(Epoch::at(one_year_later).index_abs(), 52);
    }

    #[test]
    fn epoch_mod8_now_restore() {
        let now = Epoch::now();
        let abbrev = now.index_mod8();
        let restored = Epoch::from_mod8(abbrev).unwrap();
        assert_eq!(now, restored);
    }

    #[test]
    fn epoch_mod8_all_values() {
        let mut err_count = 0;
        let mut past_count = 0;
        let mut curr_count = 0;
        let mut future_count = 0;

        let now_abs = Epoch::now().index_abs();
        for mod8 in 0u8..=7 {
            match Epoch::from_mod8(mod8) {
                Ok(e) if e.index_abs() < now_abs => past_count += 1,
                Ok(e) if e.index_abs() == now_abs => curr_count += 1,
                Ok(e) if e.index_abs() > now_abs => future_count += 1,
                _ => err_count += 1,
            }
        }

        assert_eq!(err_count, 1);
        assert_eq!(past_count, 5);
        assert_eq!(curr_count, 1);
        assert_eq!(future_count, 1);
    }
}
