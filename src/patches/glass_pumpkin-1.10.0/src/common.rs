use num_bigint::{BigInt, BigUint, Sign};
use num_integer::Integer;
use num_traits::identities::{One, Zero};
use num_traits::{Signed, ToPrimitive};

use crate::error::{Error, Result};
use crate::rand::{Randoms, gen_biguint, gen_biguint_range};
use once_cell::sync::Lazy;
use rand_core::Rng;

pub const MIN_BIT_LENGTH: usize = 128;

/// Generate a new prime number with size `bit_length`, sourced
/// from an already-initialized `Rng`
pub fn gen_prime<R: Rng + ?Sized>(bit_length: usize, rng: &mut R) -> Result {
    if bit_length < MIN_BIT_LENGTH {
        Err(Error::BitLength(bit_length))
    } else {
        let mut candidate;
        let checks = required_checks(bit_length);
        let size = bit_length as u64;

        loop {
            candidate = _prime_candidate(size, rng);

            if _is_prime_basic(&candidate, false, rng)
                && miller_rabin(&candidate, checks, true, rng)
                && lucas(&candidate)
            {
                return Ok(candidate);
            }
        }
    }
}

/// Generate a new safe prime number with size `bit_length`, sourced
/// from an already-initialized `Rng`.
pub fn gen_safe_prime<R: Rng + ?Sized>(bit_length: usize, rng: &mut R) -> Result {
    if bit_length < MIN_BIT_LENGTH {
        Err(Error::BitLength(bit_length))
    } else {
        let mut q;
        let mut p = BigUint::zero();
        let checks = required_checks(bit_length) - 5;
        let size_m1 = (bit_length - 1) as u64;

        loop {
            // Generate candidate for q
            q = _prime_candidate(size_m1, rng);

            // Check that q is congruent to 2 mod 3
            if (&q % 3u32).to_u64() == Some(2) {
                // Calculate p = 2q + 1
                p.clone_from(&q);
                p <<= 1;
                p.set_bit(0, true);

                // Check p is congruent to 2 mod 3, and check p and q are prime
                if (&p % 3u32).to_u64() == Some(2)
                    && _is_prime_basic(&q, true, rng)
                    && _is_prime_basic(&p, false, rng)
                    && miller_rabin(&q, checks, true, rng)
                    && miller_rabin(&p, checks, true, rng)
                    && lucas(&p)
                {
                    return Ok(p);
                }
            }
        }
    }
}

/// Checks if number is a prime using the Baillie-PSW test
pub fn is_prime_baillie_psw<R: Rng + ?Sized>(candidate: &BigUint, rng: &mut R) -> bool {
    _is_prime(
        candidate,
        required_checks(candidate.bits() as usize),
        true,
        false,
        rng,
    ) && lucas(candidate)
}

/// Checks if number is a safe prime using the Baillie-PSW test
pub fn is_safe_prime_baillie_psw<R: Rng + ?Sized>(candidate: &BigUint, rng: &mut R) -> bool {
    _is_safe_prime(
        candidate,
        required_checks(candidate.bits() as usize),
        true,
        rng,
    ) && lucas(candidate)
}

/// Checks if number is a safe prime
pub fn is_safe_prime<R: Rng + ?Sized>(candidate: &BigUint, rng: &mut R) -> bool {
    _is_safe_prime(
        candidate,
        required_checks(candidate.bits() as usize),
        false,
        rng,
    )
}

/// Common function for `is_safe_prime`
fn _is_safe_prime<R: Rng + ?Sized>(
    candidate: &BigUint,
    checks: usize,
    force2: bool,
    rng: &mut R,
) -> bool {
    // According to https://eprint.iacr.org/2003/186.pdf
    // a safe prime is congruent to 2 mod 3
    if (candidate % 3u32).to_u64() == Some(2) {
        // A safe prime satisfies (p-1)/2 is prime. Since a
        // prime is odd, We just need to divide by 2
        let p = &(candidate >> 1);
        return _is_prime(p, checks, force2, true, rng)
            && _is_prime(candidate, checks, force2, false, rng);
    }

    false
}

/// Test if number is prime by
///
/// 1- Trial division by first 2048 primes
/// 2- Perform a Fermat Test
/// 3- Perform log2(bitlength) + 5 rounds of Miller-Rabin
///    depending on the number of bits
pub fn is_prime<R: Rng + ?Sized>(candidate: &BigUint, rng: &mut R) -> bool {
    _is_prime(
        candidate,
        required_checks(candidate.bits() as usize),
        false,
        false,
        rng,
    )
}

/// Common function for `is_prime`
fn _is_prime<R: Rng + ?Sized>(
    candidate: &BigUint,
    checks: usize,
    force2: bool,
    q_check: bool,
    rng: &mut R,
) -> bool {
    if candidate.to_u64() == Some(2) {
        return true;
    }

    if candidate.is_even() || candidate.is_one() {
        return false;
    }

    if !_is_prime_basic(candidate, q_check, rng) {
        return false;
    }

    // Finally, do a Miller-Rabin test
    // See https://eprint.iacr.org/2018/749.pdf for good choices on appropriate number of tests
    if !miller_rabin(candidate, checks, force2, rng) {
        return false;
    }

    true
}

/// Generate a random candidate uint of the requested bit length
#[inline]
fn _prime_candidate<R: Rng + ?Sized>(bit_length: u64, rng: &mut R) -> BigUint {
    let mut candidate = gen_biguint(rng, bit_length);

    // Set lowest bit (ensure odd)
    candidate.set_bit(0, true);
    // Move left, setting the lowest bit until the size is sufficient
    let diff = bit_length - candidate.bits();
    if diff > 0 {
        candidate <<= diff;
        for bit in 0..diff {
            candidate.set_bit(bit, true);
        }
    }

    candidate
}

/// Compute `n mod m` from the little-endian u32 digits of `n`, without allocating.
#[inline]
fn mod_u32(digits: &[u32], m: u32) -> u32 {
    let m = m as u64;
    let mut rem = 0u64;
    for &d in digits.iter().rev() {
        rem = ((rem << 32) | d as u64) % m;
    }
    rem as u32
}

#[inline]
fn _is_prime_basic<R: Rng + ?Sized>(candidate: &BigUint, q_check: bool, rng: &mut R) -> bool {
    let digits = candidate.to_u32_digits();
    for r in PRIMES.iter().copied() {
        let rem = mod_u32(&digits, r);
        if rem == 0 {
            return candidate.to_u32() == Some(r);
        }
        // When checking safe primes, eliminate q congruent to (r - 1) / 2 modulo r
        if q_check && rem == (r - 1) / 2 {
            return false;
        }
    }

    fermat(candidate, rng)
}

/// Minimum checks to be considered okay
#[inline]
fn required_checks(bits: usize) -> usize {
    (bits.checked_ilog2().unwrap_or(1) as usize) + 5
}

/// Perform Fermat's little theorem on the candidate to determine probable
/// primality.
#[inline]
fn fermat<R: Rng + ?Sized>(candidate: &BigUint, rng: &mut R) -> bool {
    let random = gen_biguint_range(rng, &BigUint::one(), candidate);

    let result = random.modpow(&(candidate - 1_u8), candidate);

    result.is_one()
}

/// Perform miller rabin primality tests
fn miller_rabin<R: Rng + ?Sized>(
    candidate: &BigUint,
    limit: usize,
    force2: bool,
    rng: &mut R,
) -> bool {
    // Perform the Miller-Rabin test on the candidate, 'limit' times.
    let (trials, d) = rewrite(candidate);

    let cand_minus_one = candidate - 1_u32;

    let bases = Randoms::new(&TWO, candidate, limit, rng);
    let bases = if force2 {
        bases.with_appended((*TWO).clone())
    } else {
        bases
    };

    'nextbasis: for basis in bases {
        let mut test = basis.modpow(&d, candidate);

        if test.is_one() || test == cand_minus_one {
            continue;
        }
        for _ in 1..trials {
            test = (&test * &test) % candidate;
            if test.is_one() {
                return false;
            } else if test == cand_minus_one {
                break 'nextbasis;
            }
        }
        return false;
    }

    true
}

/// Compute `d` and `trials`
#[inline]
fn rewrite(candidate: &BigUint) -> (u64, BigUint) {
    let mut d = candidate - 1_u32;
    let trials = d.trailing_zeros().expect("n-1 is non-zero");

    if trials > 0 {
        d >>= trials;
    }

    (trials, d)
}

fn lucas(n: &BigUint) -> bool {
    // Baillie-OEIS "method C" for choosing D, P, Q,
    // as in https://oeis.org/A217719/a217719.txt:
    // try increasing P ≥ 3 such that D = P² - 4 (so Q = 1)
    // until Jacobi(D, n) = -1.
    // The search is expected to succeed for non-square n after just a few trials.
    // After more than expected failures, check whether n is square
    // (which would cause Jacobi(D, n) = 1 for all D not dividing n).
    let mut p = 3_u64;
    let n_int = BigInt::from_biguint(Sign::Plus, n.clone());

    loop {
        if p > 10000 {
            // This is widely believed to be impossible.
            // If we get a report, we'll want the exact number n.
            panic!("internal error: cannot find (D/n) = -1 for {:?}", n)
        }

        let j = jacobi(&BigInt::from(p * p - 4), &n_int);

        if j == -1 {
            break;
        }
        if j == 0 {
            // d = p²-4 = (p-2)(p+2).
            // If (d/n) == 0 then d shares a prime factor with n.
            // Since the loop proceeds in increasing p and starts with p-2==1,
            // the shared prime factor must be p+2.
            // If p+2 == n, then n is prime; otherwise p+2 is a proper factor of n.
            return n_int.to_u64() == Some(p + 2);
        }

        // We'll never find (d/n) = -1 if n is a square.
        // If n is a non-square we expect to find a d in just a few attempts on average.
        // After 40 attempts, take a moment to check if n is indeed a square.
        if p == 40 && n_int.sqrt().pow(2) == n_int {
            return false;
        }

        p += 1;
    }

    // Grantham definition of "extra strong Lucas pseudoprime", after Thm 2.3 on p. 876
    // (D, P, Q above have become Δ, b, 1):
    //
    // Let U_n = U_n(b, 1), V_n = V_n(b, 1), and Δ = b²-4.
    // An extra strong Lucas pseudoprime to base b is a composite n = 2^r s + Jacobi(Δ, n),
    // where s is odd and gcd(n, 2*Δ) = 1, such that either (i) U_s ≡ 0 mod n and V_s ≡ ±2 mod n,
    // or (ii) V_{2^t s} ≡ 0 mod n for some 0 ≤ t < r-1.
    //
    // We know gcd(n, Δ) = 1 or else we'd have found Jacobi(d, n) == 0 above.
    // We know gcd(n, 2) = 1 because n is odd.
    //
    // Arrange s = (n - Jacobi(Δ, n)) / 2^r = (n+1) / 2^r.
    let mut s = n + 1_u32;
    let r = s.trailing_zeros().expect("s should be non-zero");
    s >>= r;
    let nm2 = n - 2_u32; // n - 2

    // We apply the "almost extra strong" test, which checks the above conditions
    // except for U_s ≡ 0 mod n, which allows us to avoid computing any U_k values.
    // Jacobsen points out that maybe we should just do the full extra strong test:
    // "It is also possible to recover U_n using Crandall and Pomerance equation 3.13:
    // U_n = D^-1 (2V_{n+1} - PV_n) allowing us to run the full extra-strong test
    // at the cost of a single modular inversion. This computation is easy and fast in GMP,
    // so we can get the full extra-strong test at essentially the same performance as the
    // almost extra strong test."

    // Compute Lucas sequence V_s(b, 1), where:
    //
    //	V(0) = 2
    //	V(1) = P
    //	V(k) = P V(k-1) - Q V(k-2).
    //
    // (Remember that due to method C above, P = b, Q = 1.)
    //
    // In general V(k) = α^k + β^k, where α and β are roots of x² - Px + Q.
    // Crandall and Pomerance (p.147) observe that for 0 ≤ j ≤ k,
    //
    //	V(j+k) = V(j)V(k) - V(k-j).
    //
    // So in particular, to quickly double the subscript:
    //
    //	V(2k) = V(k)² - 2
    //	V(2k+1) = V(k) V(k+1) - P
    //
    // We can therefore start with k=0 and build up to k=s in log₂(s) steps.
    let mut vk = (*TWO).clone();
    let mut vk1 = BigUint::from(p);
    let n_minus_p = n - p;

    for i in (0..s.bits()).rev() {
        let mut t1 = (&vk * &vk1) + &n_minus_p;
        if s.bit(i) {
            // k' = 2k+1
            // V(k') = V(2k+1) = V(k) V(k+1) - P
            t1 %= n;
            vk = t1;
            // V(k'+1) = V(2k+2) = V(k+1)² - 2
            let mut t1 = (&vk1 * &vk1) + &nm2;
            t1 %= n;
            vk1 = t1;
        } else {
            // k' = 2k
            // V(k'+1) = V(2k+1) = V(k) V(k+1) - P
            t1 %= n;
            vk1 = t1;
            // V(k') = V(2k) = V(k)² - 2
            let mut t1 = (&vk * &vk) + &nm2;
            t1 %= n;
            vk = t1;
        }
    }

    // Now k=s, so vk = V(s). Check V(s) ≡ ±2 (mod n).
    if vk.to_u64() == Some(2) || vk == nm2 {
        // Check U(s) ≡ 0.
        // As suggested by Jacobsen, apply Crandall and Pomerance equation 3.13:
        //
        //	U(k) = D⁻¹ (2 V(k+1) - P V(k))
        //
        // Since we are checking for U(k) == 0 it suffices to check 2 V(k+1) == P V(k) mod n,
        // or P V(k) - 2 V(k+1) == 0 mod n.
        let mut t1 = &vk * p;
        let mut t2 = &vk1 << 1;

        if t1 < t2 {
            core::mem::swap(&mut t1, &mut t2);
        }

        t1 -= t2;

        t1 %= n;
        if t1.is_zero() {
            return true;
        }
    }

    // Check V(2^t s) ≡ 0 mod n for some 0 ≤ t < r-1.
    for _ in 0..r - 1 {
        if vk.is_zero() {
            return true;
        }

        // Optimization: V(k) = 2 is a fixed point for V(k') = V(k)² - 2,
        // so if V(k) = 2, we can stop: we will never find a future V(k) == 0.
        if vk.to_u64() == Some(2) {
            return false;
        }

        // k' = 2k
        // V(k') = V(2k) = V(k)² - 2
        vk = (&vk * &vk) - 2_u32;
        vk %= n;
    }

    false
}

/// Jacobi returns the Jacobi symbol (x/y), either +1, -1, or 0.
/// The y argument must be an odd integer.
#[allow(clippy::many_single_char_names)]
fn jacobi(x: &BigInt, y: &BigInt) -> isize {
    if !y.is_odd() {
        panic!(
            "invalid arguments, y must be an odd integer,but got {:?}",
            y
        );
    }

    let mut a = x.clone();
    let mut b = y.clone();
    let mut j = 1;

    if b.is_negative() {
        if a.is_negative() {
            j = -1;
        }
        b = -b;
    }

    loop {
        if b.is_one() {
            return j;
        }
        if a.is_zero() {
            return 0;
        }

        a = a.mod_floor(&b);

        let Some(s) = a.trailing_zeros() else {
            // a == 0
            return 0;
        };
        // a > 0

        // handle factors of 2 in a
        if s & 1 != 0 {
            let bmod8 = b.iter_u32_digits().next().unwrap_or(0) & 7;
            if bmod8 == 3 || bmod8 == 5 {
                j = -j;
            }
        }

        let c = &a >> s; // a = 2^s*c

        // swap numerator and denominator
        let b_low = b.iter_u32_digits().next().unwrap_or(0);
        let c_low = c.iter_u32_digits().next().unwrap_or(0);
        if b_low & c_low & 3 == 3 {
            j = -j
        }

        a = b;
        b = c;
    }
}

static PRIMES: &[u32] = &[
    3_u32, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71, 73, 79, 83, 89,
    97, 101, 103, 107, 109, 113, 127, 131, 137, 139, 149, 151, 157, 163, 167, 173, 179, 181, 191,
    193, 197, 199, 211, 223, 227, 229, 233, 239, 241, 251, 257, 263, 269, 271, 277, 281, 283, 293,
    307, 311, 313, 317, 331, 337, 347, 349, 353, 359, 367, 373, 379, 383, 389, 397, 401, 409, 419,
    421, 431, 433, 439, 443, 449, 457, 461, 463, 467, 479, 487, 491, 499, 503, 509, 521, 523, 541,
    547, 557, 563, 569, 571, 577, 587, 593, 599, 601, 607, 613, 617, 619, 631, 641, 643, 647, 653,
    659, 661, 673, 677, 683, 691, 701, 709, 719, 727, 733, 739, 743, 751, 757, 761, 769, 773, 787,
    797, 809, 811, 821, 823, 827, 829, 839, 853, 857, 859, 863, 877, 881, 883, 887, 907, 911, 919,
    929, 937, 941, 947, 953, 967, 971, 977, 983, 991, 997, 1009, 1013, 1019, 1021, 1031, 1033,
    1039, 1049, 1051, 1061, 1063, 1069, 1087, 1091, 1093, 1097, 1103, 1109, 1117, 1123, 1129, 1151,
    1153, 1163, 1171, 1181, 1187, 1193, 1201, 1213, 1217, 1223, 1229, 1231, 1237, 1249, 1259, 1277,
    1279, 1283, 1289, 1291, 1297, 1301, 1303, 1307, 1319, 1321, 1327, 1361, 1367, 1373, 1381, 1399,
    1409, 1423, 1427, 1429, 1433, 1439, 1447, 1451, 1453, 1459, 1471, 1481, 1483, 1487, 1489, 1493,
    1499, 1511, 1523, 1531, 1543, 1549, 1553, 1559, 1567, 1571, 1579, 1583, 1597, 1601, 1607, 1609,
    1613, 1619, 1621, 1627, 1637, 1657, 1663, 1667, 1669, 1693, 1697, 1699, 1709, 1721, 1723, 1733,
    1741, 1747, 1753, 1759, 1777, 1783, 1787, 1789, 1801, 1811, 1823, 1831, 1847, 1861, 1867, 1871,
    1873, 1877, 1879, 1889, 1901, 1907, 1913, 1931, 1933, 1949, 1951, 1973, 1979, 1987, 1993, 1997,
    1999, 2003, 2011, 2017, 2027, 2029, 2039, 2053, 2063, 2069, 2081, 2083, 2087, 2089, 2099, 2111,
    2113, 2129, 2131, 2137, 2141, 2143, 2153, 2161, 2179, 2203, 2207, 2213, 2221, 2237, 2239, 2243,
    2251, 2267, 2269, 2273, 2281, 2287, 2293, 2297, 2309, 2311, 2333, 2339, 2341, 2347, 2351, 2357,
    2371, 2377, 2381, 2383, 2389, 2393, 2399, 2411, 2417, 2423, 2437, 2441, 2447, 2459, 2467, 2473,
    2477, 2503, 2521, 2531, 2539, 2543, 2549, 2551, 2557, 2579, 2591, 2593, 2609, 2617, 2621, 2633,
    2647, 2657, 2659, 2663, 2671, 2677, 2683, 2687, 2689, 2693, 2699, 2707, 2711, 2713, 2719, 2729,
    2731, 2741, 2749, 2753, 2767, 2777, 2789, 2791, 2797, 2801, 2803, 2819, 2833, 2837, 2843, 2851,
    2857, 2861, 2879, 2887, 2897, 2903, 2909, 2917, 2927, 2939, 2953, 2957, 2963, 2969, 2971, 2999,
    3001, 3011, 3019, 3023, 3037, 3041, 3049, 3061, 3067, 3079, 3083, 3089, 3109, 3119, 3121, 3137,
    3163, 3167, 3169, 3181, 3187, 3191, 3203, 3209, 3217, 3221, 3229, 3251, 3253, 3257, 3259, 3271,
    3299, 3301, 3307, 3313, 3319, 3323, 3329, 3331, 3343, 3347, 3359, 3361, 3371, 3373, 3389, 3391,
    3407, 3413, 3433, 3449, 3457, 3461, 3463, 3467, 3469, 3491, 3499, 3511, 3517, 3527, 3529, 3533,
    3539, 3541, 3547, 3557, 3559, 3571, 3581, 3583, 3593, 3607, 3613, 3617, 3623, 3631, 3637, 3643,
    3659, 3671, 3673, 3677, 3691, 3697, 3701, 3709, 3719, 3727, 3733, 3739, 3761, 3767, 3769, 3779,
    3793, 3797, 3803, 3821, 3823, 3833, 3847, 3851, 3853, 3863, 3877, 3881, 3889, 3907, 3911, 3917,
    3919, 3923, 3929, 3931, 3943, 3947, 3967, 3989, 4001, 4003, 4007, 4013, 4019, 4021, 4027, 4049,
    4051, 4057, 4073, 4079, 4091, 4093, 4099, 4111, 4127, 4129, 4133, 4139, 4153, 4157, 4159, 4177,
    4201, 4211, 4217, 4219, 4229, 4231, 4241, 4243, 4253, 4259, 4261, 4271, 4273, 4283, 4289, 4297,
    4327, 4337, 4339, 4349, 4357, 4363, 4373, 4391, 4397, 4409, 4421, 4423, 4441, 4447, 4451, 4457,
    4463, 4481, 4483, 4493, 4507, 4513, 4517, 4519, 4523, 4547, 4549, 4561, 4567, 4583, 4591, 4597,
    4603, 4621, 4637, 4639, 4643, 4649, 4651, 4657, 4663, 4673, 4679, 4691, 4703, 4721, 4723, 4729,
    4733, 4751, 4759, 4783, 4787, 4789, 4793, 4799, 4801, 4813, 4817, 4831, 4861, 4871, 4877, 4889,
    4903, 4909, 4919, 4931, 4933, 4937, 4943, 4951, 4957, 4967, 4969, 4973, 4987, 4993, 4999, 5003,
    5009, 5011, 5021, 5023, 5039, 5051, 5059, 5077, 5081, 5087, 5099, 5101, 5107, 5113, 5119, 5147,
    5153, 5167, 5171, 5179, 5189, 5197, 5209, 5227, 5231, 5233, 5237, 5261, 5273, 5279, 5281, 5297,
    5303, 5309, 5323, 5333, 5347, 5351, 5381, 5387, 5393, 5399, 5407, 5413, 5417, 5419, 5431, 5437,
    5441, 5443, 5449, 5471, 5477, 5479, 5483, 5501, 5503, 5507, 5519, 5521, 5527, 5531, 5557, 5563,
    5569, 5573, 5581, 5591, 5623, 5639, 5641, 5647, 5651, 5653, 5657, 5659, 5669, 5683, 5689, 5693,
    5701, 5711, 5717, 5737, 5741, 5743, 5749, 5779, 5783, 5791, 5801, 5807, 5813, 5821, 5827, 5839,
    5843, 5849, 5851, 5857, 5861, 5867, 5869, 5879, 5881, 5897, 5903, 5923, 5927, 5939, 5953, 5981,
    5987, 6007, 6011, 6029, 6037, 6043, 6047, 6053, 6067, 6073, 6079, 6089, 6091, 6101, 6113, 6121,
    6131, 6133, 6143, 6151, 6163, 6173, 6197, 6199, 6203, 6211, 6217, 6221, 6229, 6247, 6257, 6263,
    6269, 6271, 6277, 6287, 6299, 6301, 6311, 6317, 6323, 6329, 6337, 6343, 6353, 6359, 6361, 6367,
    6373, 6379, 6389, 6397, 6421, 6427, 6449, 6451, 6469, 6473, 6481, 6491, 6521, 6529, 6547, 6551,
    6553, 6563, 6569, 6571, 6577, 6581, 6599, 6607, 6619, 6637, 6653, 6659, 6661, 6673, 6679, 6689,
    6691, 6701, 6703, 6709, 6719, 6733, 6737, 6761, 6763, 6779, 6781, 6791, 6793, 6803, 6823, 6827,
    6829, 6833, 6841, 6857, 6863, 6869, 6871, 6883, 6899, 6907, 6911, 6917, 6947, 6949, 6959, 6961,
    6967, 6971, 6977, 6983, 6991, 6997, 7001, 7013, 7019, 7027, 7039, 7043, 7057, 7069, 7079, 7103,
    7109, 7121, 7127, 7129, 7151, 7159, 7177, 7187, 7193, 7207, 7211, 7213, 7219, 7229, 7237, 7243,
    7247, 7253, 7283, 7297, 7307, 7309, 7321, 7331, 7333, 7349, 7351, 7369, 7393, 7411, 7417, 7433,
    7451, 7457, 7459, 7477, 7481, 7487, 7489, 7499, 7507, 7517, 7523, 7529, 7537, 7541, 7547, 7549,
    7559, 7561, 7573, 7577, 7583, 7589, 7591, 7603, 7607, 7621, 7639, 7643, 7649, 7669, 7673, 7681,
    7687, 7691, 7699, 7703, 7717, 7723, 7727, 7741, 7753, 7757, 7759, 7789, 7793, 7817, 7823, 7829,
    7841, 7853, 7867, 7873, 7877, 7879, 7883, 7901, 7907, 7919, 7927, 7933, 7937, 7949, 7951, 7963,
    7993, 8009, 8011, 8017, 8039, 8053, 8059, 8069, 8081, 8087, 8089, 8093, 8101, 8111, 8117, 8123,
    8147, 8161, 8167, 8171, 8179, 8191, 8209, 8219, 8221, 8231, 8233, 8237, 8243, 8263, 8269, 8273,
    8287, 8291, 8293, 8297, 8311, 8317, 8329, 8353, 8363, 8369, 8377, 8387, 8389, 8419, 8423, 8429,
    8431, 8443, 8447, 8461, 8467, 8501, 8513, 8521, 8527, 8537, 8539, 8543, 8563, 8573, 8581, 8597,
    8599, 8609, 8623, 8627, 8629, 8641, 8647, 8663, 8669, 8677, 8681, 8689, 8693, 8699, 8707, 8713,
    8719, 8731, 8737, 8741, 8747, 8753, 8761, 8779, 8783, 8803, 8807, 8819, 8821, 8831, 8837, 8839,
    8849, 8861, 8863, 8867, 8887, 8893, 8923, 8929, 8933, 8941, 8951, 8963, 8969, 8971, 8999, 9001,
    9007, 9011, 9013, 9029, 9041, 9043, 9049, 9059, 9067, 9091, 9103, 9109, 9127, 9133, 9137, 9151,
    9157, 9161, 9173, 9181, 9187, 9199, 9203, 9209, 9221, 9227, 9239, 9241, 9257, 9277, 9281, 9283,
    9293, 9311, 9319, 9323, 9337, 9341, 9343, 9349, 9371, 9377, 9391, 9397, 9403, 9413, 9419, 9421,
    9431, 9433, 9437, 9439, 9461, 9463, 9467, 9473, 9479, 9491, 9497, 9511, 9521, 9533, 9539, 9547,
    9551, 9587, 9601, 9613, 9619, 9623, 9629, 9631, 9643, 9649, 9661, 9677, 9679, 9689, 9697, 9719,
    9721, 9733, 9739, 9743, 9749, 9767, 9769, 9781, 9787, 9791, 9803, 9811, 9817, 9829, 9833, 9839,
    9851, 9857, 9859, 9871, 9883, 9887, 9901, 9907, 9923, 9929, 9931, 9941, 9949, 9967, 9973,
    10007, 10009, 10037, 10039, 10061, 10067, 10069, 10079, 10091, 10093, 10099, 10103, 10111,
    10133, 10139, 10141, 10151, 10159, 10163, 10169, 10177, 10181, 10193, 10211, 10223, 10243,
    10247, 10253, 10259, 10267, 10271, 10273, 10289, 10301, 10303, 10313, 10321, 10331, 10333,
    10337, 10343, 10357, 10369, 10391, 10399, 10427, 10429, 10433, 10453, 10457, 10459, 10463,
    10477, 10487, 10499, 10501, 10513, 10529, 10531, 10559, 10567, 10589, 10597, 10601, 10607,
    10613, 10627, 10631, 10639, 10651, 10657, 10663, 10667, 10687, 10691, 10709, 10711, 10723,
    10729, 10733, 10739, 10753, 10771, 10781, 10789, 10799, 10831, 10837, 10847, 10853, 10859,
    10861, 10867, 10883, 10889, 10891, 10903, 10909, 10937, 10939, 10949, 10957, 10973, 10979,
    10987, 10993, 11003, 11027, 11047, 11057, 11059, 11069, 11071, 11083, 11087, 11093, 11113,
    11117, 11119, 11131, 11149, 11159, 11161, 11171, 11173, 11177, 11197, 11213, 11239, 11243,
    11251, 11257, 11261, 11273, 11279, 11287, 11299, 11311, 11317, 11321, 11329, 11351, 11353,
    11369, 11383, 11393, 11399, 11411, 11423, 11437, 11443, 11447, 11467, 11471, 11483, 11489,
    11491, 11497, 11503, 11519, 11527, 11549, 11551, 11579, 11587, 11593, 11597, 11617, 11621,
    11633, 11657, 11677, 11681, 11689, 11699, 11701, 11717, 11719, 11731, 11743, 11777, 11779,
    11783, 11789, 11801, 11807, 11813, 11821, 11827, 11831, 11833, 11839, 11863, 11867, 11887,
    11897, 11903, 11909, 11923, 11927, 11933, 11939, 11941, 11953, 11959, 11969, 11971, 11981,
    11987, 12007, 12011, 12037, 12041, 12043, 12049, 12071, 12073, 12097, 12101, 12107, 12109,
    12113, 12119, 12143, 12149, 12157, 12161, 12163, 12197, 12203, 12211, 12227, 12239, 12241,
    12251, 12253, 12263, 12269, 12277, 12281, 12289, 12301, 12323, 12329, 12343, 12347, 12373,
    12377, 12379, 12391, 12401, 12409, 12413, 12421, 12433, 12437, 12451, 12457, 12473, 12479,
    12487, 12491, 12497, 12503, 12511, 12517, 12527, 12539, 12541, 12547, 12553, 12569, 12577,
    12583, 12589, 12601, 12611, 12613, 12619, 12637, 12641, 12647, 12653, 12659, 12671, 12689,
    12697, 12703, 12713, 12721, 12739, 12743, 12757, 12763, 12781, 12791, 12799, 12809, 12821,
    12823, 12829, 12841, 12853, 12889, 12893, 12899, 12907, 12911, 12917, 12919, 12923, 12941,
    12953, 12959, 12967, 12973, 12979, 12983, 13001, 13003, 13007, 13009, 13033, 13037, 13043,
    13049, 13063, 13093, 13099, 13103, 13109, 13121, 13127, 13147, 13151, 13159, 13163, 13171,
    13177, 13183, 13187, 13217, 13219, 13229, 13241, 13249, 13259, 13267, 13291, 13297, 13309,
    13313, 13327, 13331, 13337, 13339, 13367, 13381, 13397, 13399, 13411, 13417, 13421, 13441,
    13451, 13457, 13463, 13469, 13477, 13487, 13499, 13513, 13523, 13537, 13553, 13567, 13577,
    13591, 13597, 13613, 13619, 13627, 13633, 13649, 13669, 13679, 13681, 13687, 13691, 13693,
    13697, 13709, 13711, 13721, 13723, 13729, 13751, 13757, 13759, 13763, 13781, 13789, 13799,
    13807, 13829, 13831, 13841, 13859, 13873, 13877, 13879, 13883, 13901, 13903, 13907, 13913,
    13921, 13931, 13933, 13963, 13967, 13997, 13999, 14009, 14011, 14029, 14033, 14051, 14057,
    14071, 14081, 14083, 14087, 14107, 14143, 14149, 14153, 14159, 14173, 14177, 14197, 14207,
    14221, 14243, 14249, 14251, 14281, 14293, 14303, 14321, 14323, 14327, 14341, 14347, 14369,
    14387, 14389, 14401, 14407, 14411, 14419, 14423, 14431, 14437, 14447, 14449, 14461, 14479,
    14489, 14503, 14519, 14533, 14537, 14543, 14549, 14551, 14557, 14561, 14563, 14591, 14593,
    14621, 14627, 14629, 14633, 14639, 14653, 14657, 14669, 14683, 14699, 14713, 14717, 14723,
    14731, 14737, 14741, 14747, 14753, 14759, 14767, 14771, 14779, 14783, 14797, 14813, 14821,
    14827, 14831, 14843, 14851, 14867, 14869, 14879, 14887, 14891, 14897, 14923, 14929, 14939,
    14947, 14951, 14957, 14969, 14983, 15013, 15017, 15031, 15053, 15061, 15073, 15077, 15083,
    15091, 15101, 15107, 15121, 15131, 15137, 15139, 15149, 15161, 15173, 15187, 15193, 15199,
    15217, 15227, 15233, 15241, 15259, 15263, 15269, 15271, 15277, 15287, 15289, 15299, 15307,
    15313, 15319, 15329, 15331, 15349, 15359, 15361, 15373, 15377, 15383, 15391, 15401, 15413,
    15427, 15439, 15443, 15451, 15461, 15467, 15473, 15493, 15497, 15511, 15527, 15541, 15551,
    15559, 15569, 15581, 15583, 15601, 15607, 15619, 15629, 15641, 15643, 15647, 15649, 15661,
    15667, 15671, 15679, 15683, 15727, 15731, 15733, 15737, 15739, 15749, 15761, 15767, 15773,
    15787, 15791, 15797, 15803, 15809, 15817, 15823, 15859, 15877, 15881, 15887, 15889, 15901,
    15907, 15913, 15919, 15923, 15937, 15959, 15971, 15973, 15991, 16001, 16007, 16033, 16057,
    16061, 16063, 16067, 16069, 16073, 16087, 16091, 16097, 16103, 16111, 16127, 16139, 16141,
    16183, 16187, 16189, 16193, 16217, 16223, 16229, 16231, 16249, 16253, 16267, 16273, 16301,
    16319, 16333, 16339, 16349, 16361, 16363, 16369, 16381, 16411, 16417, 16421, 16427, 16433,
    16447, 16451, 16453, 16477, 16481, 16487, 16493, 16519, 16529, 16547, 16553, 16561, 16567,
    16573, 16603, 16607, 16619, 16631, 16633, 16649, 16651, 16657, 16661, 16673, 16691, 16693,
    16699, 16703, 16729, 16741, 16747, 16759, 16763, 16787, 16811, 16823, 16829, 16831, 16843,
    16871, 16879, 16883, 16889, 16901, 16903, 16921, 16927, 16931, 16937, 16943, 16963, 16979,
    16981, 16987, 16993, 17011, 17021, 17027, 17029, 17033, 17041, 17047, 17053, 17077, 17093,
    17099, 17107, 17117, 17123, 17137, 17159, 17167, 17183, 17189, 17191, 17203, 17207, 17209,
    17231, 17239, 17257, 17291, 17293, 17299, 17317, 17321, 17327, 17333, 17341, 17351, 17359,
    17377, 17383, 17387, 17389, 17393, 17401, 17417, 17419, 17431, 17443, 17449, 17467, 17471,
    17477, 17483, 17489, 17491, 17497, 17509, 17519, 17539, 17551, 17569, 17573, 17579, 17581,
    17597, 17599, 17609, 17623, 17627, 17657, 17659, 17669, 17681, 17683, 17707, 17713, 17729,
    17737, 17747, 17749, 17761, 17783, 17789, 17791, 17807, 17827, 17837, 17839, 17851, 17863,
];
static TWO: Lazy<BigUint> = Lazy::new(|| BigUint::from(2_u8));

#[cfg(test)]
mod tests {
    use super::{
        PRIMES, gen_prime, gen_safe_prime, is_prime, is_prime_baillie_psw, is_safe_prime,
        is_safe_prime_baillie_psw,
    };
    use crate::error::Error;
    use num_bigint::BigUint;
    use num_traits::Num;
    use rand::rng;

    #[test]
    fn gen_safe_prime_tests() {
        let mut rng = rng();
        match gen_prime(16, &mut rng) {
            Ok(_) => panic!("No primes allowed under 16 bits"),
            Err(Error::BitLength(l)) => assert_eq!(l, 16),
        };

        for bits in &[128, 256, 384, 512] {
            let n = gen_safe_prime(*bits, &mut rng).unwrap();
            assert!(is_safe_prime_baillie_psw(&n, &mut rng));
            assert_eq!(n.bits() as usize, *bits);
        }
    }

    #[test]
    fn gen_prime_tests() {
        let mut rng = rng();
        match gen_prime(16, &mut rng) {
            Ok(_) => panic!("No primes allowed under 16 bits"),
            Err(Error::BitLength(l)) => assert_eq!(l, 16),
        };

        for bits in &[256, 512, 1024, 2048] {
            let n = gen_prime(*bits, &mut rng).unwrap();
            assert!(is_prime(&n, &mut rng));
            assert_eq!(n.bits() as usize, *bits);
        }
    }

    #[test]
    fn is_prime_tests() {
        let mut rng = rng();
        for prime in PRIMES.iter().copied() {
            assert!(is_prime(&BigUint::from(prime), &mut rng));
        }

        let mut n = BigUint::from(18_088_387_217_903_330_459_u64);
        assert!(!is_prime(&(n.clone() >> 1), &mut rng));
        assert!(is_prime_baillie_psw(&n, &mut rng));
        for _ in 0..5 {
            n <<= 1;
            n += 1_u8;
            assert!(is_safe_prime(&n, &mut rng));
            assert!(is_prime_baillie_psw(&n, &mut rng));
        }

        n = BigUint::from_str_radix("33376463607021642560387296949", 10).unwrap();
        assert!(!is_prime(&(n.clone() >> 1), &mut rng));
        assert!(is_prime_baillie_psw(&n, &mut rng));
        for _ in 0..5 {
            n <<= 1;
            n += 1_u8;
            assert!(is_safe_prime(&n, &mut rng));
        }

        n = BigUint::from_str_radix("170141183460469231731687303717167733089", 10).unwrap();
        assert!(!is_prime(&(n.clone() >> 1), &mut rng));
        assert!(is_prime_baillie_psw(&n, &mut rng));
        for _ in 0..5 {
            n <<= 1;
            n += 1_u8;
            assert!(is_safe_prime(&n, &mut rng));
        }

        n = BigUint::from_str_radix(
            "113910913923300788319699387848674650656041243163866388656000063249848353322899",
            10,
        )
        .unwrap();
        assert!(!is_prime(&(n.clone() >> 1), &mut rng));
        assert!(is_prime_baillie_psw(&n, &mut rng));
        for _ in 0..4 {
            n <<= 1;
            n += 1_u8;
            assert!(is_safe_prime(&n, &mut rng));
        }

        n = BigUint::from_str_radix("1675975991242824637446753124775730765934920727574049172215445180465220503759193372100234287270862928461253982273310756356719235351493321243304213304923049", 10).unwrap();
        assert!(!is_prime(&(n.clone() >> 1), &mut rng));
        assert!(is_prime(&n, &mut rng));
        for _ in 0..4 {
            n <<= 1;
            n += 1_u8;
            assert!(is_safe_prime(&n, &mut rng));
        }
        n = BigUint::from_str_radix("153739637779647327330155094463476939112913405723627932550795546376536722298275674187199768137486929460478138431076223176750734095693166283451594721829574797878338183845296809008576378039501400850628591798770214582527154641716248943964626446190042367043984306973709604255015629102866732543697075866901827761489", 10).unwrap();

        assert!(!is_prime(&(n.clone() >> 1), &mut rng));
        assert!(is_prime_baillie_psw(&n, &mut rng));
        for _ in 0..3 {
            n <<= 1;
            n += 1_u8;
            assert!(is_safe_prime(&n, &mut rng));
        }
    }

    // Regression test for https://github.com/LF-Decentralized-Trust-labs/agora-glass_pumpkin/issues/16
    #[test]
    fn issue_16_lucas_test_prime_not_flagged_as_composite() {
        let mut rng = rng();
        let n = BigUint::from(18_446_744_073_710_004_191_u128);
        assert!(is_prime(&n, &mut rng));
        assert!(is_prime_baillie_psw(&n, &mut rng));
    }
}
