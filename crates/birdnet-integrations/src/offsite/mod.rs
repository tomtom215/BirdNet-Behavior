//! Sending backups somewhere other than the SD card they were written on.
//!
//! A station's rolling backups live beside its database, on the same card. That
//! covers the failure this project has always covered — a corrupt page, a bad
//! VACUUM, an interrupted write — and none of the ones that actually end a
//! station's records: the card wears out, the enclosure floods, the Pi is
//! stolen. This module is the other half.
//!
//! Everything here is off unless configured, and everything that leaves the
//! station goes through [`envelope`] first.

pub mod envelope;
pub mod s3;
pub mod sigv4;
