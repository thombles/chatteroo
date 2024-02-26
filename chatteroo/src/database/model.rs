//! Structs representing data in the database.

use time::OffsetDateTime;

pub struct Frame {
    id: i32,
    epoch: i32,
    inserter: String,
    index: i32,
    is_start: bool,
    is_end: bool,
    application: i32,
    data: Vec<u8>,
    inserted: OffsetDateTime,
}