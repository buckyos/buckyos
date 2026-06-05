//! Generates cryptographically secure prime numbers.

pub use crate::common::{
    gen_prime as from_rng, is_prime as check_with, is_prime_baillie_psw as strong_check_with,
};

#[cfg(feature = "getrandom")]
use crate::error::Result;

/// Constructs a new prime number with a size of `bit_length` bits.
///
/// This will initialize an `OsRng` instance and call the
/// `from_rng()` function.
///
/// Note: the `bit_length` MUST be at least 128-bits.
#[cfg(feature = "getrandom")]
pub fn new(bit_length: usize) -> Result {
    from_rng(bit_length, &mut rand_core::UnwrapErr(getrandom::SysRng))
}

/// Test if number is prime by
///
/// 1- Trial division by first 2048 primes
/// 2- Perform a Fermat Test
/// 3- Perform log2(bitlength) + 5 rounds of Miller-Rabin
///    depending on the number of bits
#[cfg(feature = "getrandom")]
pub fn check(candidate: &num_bigint::BigUint) -> bool {
    check_with(candidate, &mut rand_core::UnwrapErr(getrandom::SysRng))
}

/// Checks if number is a prime using the Baillie-PSW test
#[cfg(feature = "getrandom")]
pub fn strong_check(candidate: &num_bigint::BigUint) -> bool {
    strong_check_with(candidate, &mut rand_core::UnwrapErr(getrandom::SysRng))
}

#[cfg(test)]
mod tests {
    use super::{check, new, strong_check};

    #[test]
    fn tests() {
        for bits in &[128, 256, 512, 1024] {
            let n = new(*bits).unwrap();
            assert!(check(&n));
            assert!(strong_check(&n));
        }
    }
}
