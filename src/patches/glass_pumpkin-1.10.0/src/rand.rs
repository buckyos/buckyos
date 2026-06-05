use num_bigint::BigUint;
use num_traits::identities::Zero;
use rand_core::Rng;

/// Generate a random `BigUint` with up to `bits` bits (uniform over [0, 2^bits)).
pub fn gen_biguint(rng: &mut (impl Rng + ?Sized), bits: u64) -> BigUint {
    if bits == 0 {
        return BigUint::zero();
    }
    let bytes = bits.div_ceil(8) as usize;
    let mut buf = alloc::vec![0u8; bytes];
    rng.fill_bytes(&mut buf);
    // Mask the top byte so we get exactly `bits` bits max
    let rem_bits = (bits % 8) as u8;
    if rem_bits > 0 {
        buf[0] &= (1u8 << rem_bits) - 1;
    }
    BigUint::from_bytes_be(&buf)
}

/// Generate a random `BigUint` uniformly in [low, high) using rejection sampling.
pub fn gen_biguint_range(rng: &mut (impl Rng + ?Sized), low: &BigUint, high: &BigUint) -> BigUint {
    assert!(low < high);
    let range = high - low;
    let bits = range.bits();
    loop {
        let val = gen_biguint(rng, bits);
        if val < range {
            return val + low;
        }
    }
}

/// Iterator to generate a given amount of random numbers. For convenience of
/// use with miller_rabin tests, you can also append a specified number at the
/// end of the generated stream.
pub struct Randoms<'a, R> {
    appended: Option<BigUint>,
    lower_limit: &'a BigUint,
    upper_limit: &'a BigUint,
    amount: usize,
    rng: R,
}

impl<'a, R: Rng> Randoms<'a, R> {
    pub fn new(lower_limit: &'a BigUint, upper_limit: &'a BigUint, amount: usize, rng: R) -> Self {
        Self {
            appended: None,
            lower_limit,
            upper_limit,
            amount,
            rng,
        }
    }

    /// Append the number at the end to appear as if it was generated. This
    /// doesn't affect stream length. Only one number can be appended,
    /// subsequent calls will replace the previously appended number.
    pub fn with_appended(mut self, x: BigUint) -> Self {
        self.appended = Some(x);
        self
    }

    fn gen_biguint(&mut self) -> BigUint {
        gen_biguint_range(&mut self.rng, self.lower_limit, self.upper_limit)
    }
}

impl<R: Rng> Iterator for Randoms<'_, R> {
    type Item = BigUint;

    fn next(&mut self) -> Option<Self::Item> {
        if self.amount == 0 {
            None
        } else if self.amount == 1 {
            let r = match self.appended.take() {
                Some(x) => x,
                None => self.gen_biguint(),
            };
            self.amount -= 1;
            Some(r)
        } else {
            self.amount -= 1;
            Some(self.gen_biguint())
        }
    }
}

#[cfg(test)]
mod test {
    use alloc::vec::Vec;

    use super::Randoms;
    use num_bigint::BigUint;
    use rand::rng;

    #[test]
    fn generate_amount_test() {
        let amount = 3;
        let lo: BigUint = 0_u8.into();
        let hi: BigUint = 1_u8.into();
        let rands = Randoms::new(&lo, &hi, amount, rng());
        let generated = rands.collect::<Vec<_>>();
        assert_eq!(generated.len(), amount);

        let rands = Randoms::new(&lo, &hi, amount, rng()).with_appended(2_u8.into());
        let generated = rands.collect::<Vec<_>>();
        assert_eq!(generated.len(), amount);
    }
}
