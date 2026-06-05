//! Error structs

use crate::common::MIN_BIT_LENGTH;
use core::{fmt, result};

/// Default result struct
pub type Result = result::Result<num_bigint::BigUint, Error>;

/// Error struct
#[derive(Debug)]
pub enum Error {
    /// Handles when the bit sizes are too small
    BitLength(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::BitLength(length) => write!(
                f,
                "The given bit length is too small; must be at least {}: {}",
                MIN_BIT_LENGTH, length
            ),
        }
    }
}

impl core::error::Error for Error {}
