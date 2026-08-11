use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}
