//! rubash#879: 80-bit extended (long double) float semantics for printf.
//!
//! GNU bash converts printf float operands with strtold and formats the
//! long double with glibc: `%f` of `1e999` is the full ~1000-digit decimal
//! expansion (not `inf`), and `%a` prints the 64-bit mantissa nibble-style
//! (`5` -> `0xap-1`). Rust has no 80-bit float and the host libc differs per
//! platform (MSVC `long double` is just `double`), so this module emulates
//! the x86-64 80-bit extended format in pure Rust.
//!
//! A finite value is `mant * 2^exp` with a 64-bit `mant` (explicit leading 1
//! for normals; denormals pin `exp` to [`LD_DENORM_EXP`] with `mant < 2^63`).
//! Decimal/hex literals convert exactly (small [`BigUint`]) and round once
//! to 64 mantissa bits (round-half-even), mirroring strtold; every formatter
//! then works from exact digits (binary fractions expand finitely), so all
//! rounding is exact. Absurd inputs are decided by digit-count magnitude
//! guards or capped to a bounded digit window with a sticky bit, so no path
//! builds a giant bignum.

use std::cmp::Ordering;

/// A printf float operand in long-double terms.
pub(super) enum FloatVal {
    Finite { neg: bool, mant: u64, exp: i32 },
    Infinite { neg: bool },
    Nan { neg: bool },
}

impl FloatVal {
    pub(super) fn is_negative(&self) -> bool {
        match self {
            FloatVal::Finite { neg, .. } => *neg,
            FloatVal::Infinite { neg } => *neg,
            FloatVal::Nan { neg } => *neg,
        }
    }

    /// Positive zero (empty and unconvertible operands; printf.def
    /// getfloatmax returns 0 for a missing argument).
    pub(super) fn zero() -> Self {
        FloatVal::Finite {
            neg: false,
            mant: 0,
            exp: 0,
        }
    }

    /// An exact small integer (quote-constant operands via asciicode).
    pub(super) fn from_small_int(value: u64, neg: bool) -> Self {
        let (mant, exp) = normalize_small_int(value);
        FloatVal::Finite {
            neg: neg && mant != 0,
            mant,
            exp,
        }
    }
}

/// Largest unbiased binary exponent of a finite long double (2^16384 is inf).
const LD_MAX_UNBIASED_EXP: i64 = 16383;
/// Smallest unbiased binary exponent of a normal (`2^-16382`).
const LD_MIN_NORMAL_UNBIASED_EXP: i64 = -16382;
/// Denormal value = `mant * 2^LD_DENORM_EXP` with `mant < 2^63`.
const LD_DENORM_EXP: i32 = -16445;
/// Significant digits kept for decimal conversion. Unlike hex nibbles a
/// dropped decimal digit is not bit-aligned, so truncating the tail can
/// shift the value across a binary rounding boundary that `sticky` alone
/// cannot rescue (only `low3 == 0b100` consults it): the window is wide
/// enough that a boundary counterexample needs a >2048-significant-digit
/// literal, while still bounding the bignum. The dropped tail folds into
/// `sticky`.
const DECIMAL_KEEP_DIGITS: usize = 2048;
/// Significant hex digits kept for hex conversion (same rationale).
const HEX_KEEP_DIGITS: usize = 100;

// ================= minimal unsigned big integer (base 2^64) =================

#[derive(Clone, Default)]
struct BigUint {
    limbs: Vec<u64>,
}

impl BigUint {
    fn zero() -> Self {
        Self { limbs: Vec::new() }
    }

    fn from_u64(v: u64) -> Self {
        if v == 0 {
            Self::zero()
        } else {
            Self { limbs: vec![v] }
        }
    }

    fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    fn is_one(&self) -> bool {
        self.limbs.len() == 1 && self.limbs[0] == 1
    }

    fn normalize(&mut self) {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
    }

    fn bit_len(&self) -> usize {
        match self.limbs.last() {
            None => 0,
            Some(&top) => (self.limbs.len() - 1) * 64 + (64 - top.leading_zeros() as usize),
        }
    }

    fn cmp(&self, other: &Self) -> Ordering {
        if self.limbs.len() != other.limbs.len() {
            return self.limbs.len().cmp(&other.limbs.len());
        }
        for (a, b) in self.limbs.iter().rev().zip(other.limbs.iter().rev()) {
            if a != b {
                return a.cmp(b);
            }
        }
        Ordering::Equal
    }

    fn add_small(&mut self, mut a: u64) {
        let mut i = 0;
        while a != 0 {
            if i == self.limbs.len() {
                self.limbs.push(0);
            }
            let (sum, carry) = self.limbs[i].overflowing_add(a);
            self.limbs[i] = sum;
            a = carry as u64;
            i += 1;
        }
    }

    fn mul_small(&mut self, m: u32) {
        if m == 1 || self.is_zero() {
            return;
        }
        if m == 0 {
            self.limbs.clear();
            return;
        }
        let mut carry: u64 = 0;
        for limb in self.limbs.iter_mut() {
            let cur = (*limb as u128) * (m as u128) + carry as u128;
            *limb = cur as u64;
            carry = (cur >> 64) as u64;
        }
        if carry != 0 {
            self.limbs.push(carry);
        }
    }

    fn bit(&self, i: usize) -> bool {
        (self.limbs.get(i / 64).copied().unwrap_or(0) >> (i % 64)) & 1 == 1
    }

    fn set_bit(&mut self, i: usize) {
        let word = i / 64;
        if self.limbs.len() <= word {
            self.limbs.resize(word + 1, 0);
        }
        self.limbs[word] |= 1 << (i % 64);
    }

    fn shl(&mut self, bits: usize) {
        if self.is_zero() || bits == 0 {
            return;
        }
        let words = bits / 64;
        let rem = bits % 64;
        if words > 0 {
            let mut shifted = vec![0u64; words];
            shifted.append(&mut self.limbs);
            self.limbs = shifted;
        }
        if rem > 0 {
            let mut carry = 0u64;
            for limb in self.limbs.iter_mut() {
                let next = *limb >> (64 - rem);
                *limb = (*limb << rem) | carry;
                carry = next;
            }
            if carry != 0 {
                self.limbs.push(carry);
            }
        }
    }

    fn sub_assign(&mut self, other: &Self) {
        // Requires `self >= other`.
        let mut borrow: i128 = 0;
        for i in 0..self.limbs.len() {
            let subtrahend = other.limbs.get(i).copied().unwrap_or(0) as i128 + borrow;
            let minuend = self.limbs[i] as i128;
            self.limbs[i] = minuend.wrapping_sub(subtrahend) as u64;
            borrow = if minuend < subtrahend { 1 } else { 0 };
        }
        debug_assert!(borrow == 0);
        self.normalize();
    }

    /// Restoring binary long division; `divisor` must be non-zero.
    fn divmod(&self, divisor: &Self) -> (Self, Self) {
        if divisor.is_one() {
            return (self.clone(), Self::zero());
        }
        if self.cmp(divisor) == Ordering::Less {
            return (Self::zero(), self.clone());
        }
        let width = self.bit_len();
        let mut quotient = Self::zero();
        let mut remainder = Self::zero();
        for i in (0..width).rev() {
            remainder.shl(1);
            if self.bit(i) {
                remainder.add_small(1);
            }
            if remainder.cmp(divisor) != Ordering::Less {
                remainder.sub_assign(divisor);
                quotient.set_bit(i);
            }
        }
        (quotient, remainder)
    }

    /// Divide in place by a small `divisor` (< 2^32), returning the remainder.
    fn divmod_small(&mut self, divisor: u32) -> u32 {
        let mut rem: u64 = 0;
        for limb in self.limbs.iter_mut().rev() {
            let cur = ((rem as u128) << 64) | (*limb as u128);
            *limb = (cur / divisor as u128) as u64;
            rem = (cur % divisor as u128) as u64;
        }
        self.normalize();
        rem as u32
    }

    fn low_u64(&self) -> u64 {
        self.limbs.first().copied().unwrap_or(0)
    }

    /// `(self >> bits) mod 2^64`.
    fn shr_to_u64(&self, bits: usize) -> u64 {
        let word = bits / 64;
        let rem = bits % 64;
        let low = self.limbs.get(word).copied().unwrap_or(0) >> rem;
        if rem == 0 {
            return low;
        }
        low | (self.limbs.get(word + 1).copied().unwrap_or(0) << (64 - rem))
    }

    /// Keep only the low `bits` bits.
    fn mask_low(&mut self, bits: usize) {
        let keep = bits / 64;
        let rem = bits % 64;
        if rem > 0 {
            if self.limbs.len() > keep {
                self.limbs[keep] &= (1u64 << rem) - 1;
                self.limbs.truncate(keep + 1);
            }
        } else {
            self.limbs.truncate(keep);
        }
        self.normalize();
    }

    fn to_decimal_string(&self) -> String {
        if self.is_zero() {
            return "0".to_string();
        }
        let mut work = self.clone();
        let mut chunks: Vec<u32> = Vec::new();
        while !work.is_zero() {
            chunks.push(work.divmod_small(1_000_000_000));
        }
        let mut out = chunks.pop().unwrap().to_string();
        while let Some(chunk) = chunks.pop() {
            out.push_str(&format!("{chunk:09}"));
        }
        out
    }
}

/// `floor(log2(n / d))` for `n, d > 0`.
fn floor_log2_ratio(n: &BigUint, d: &BigUint) -> i64 {
    let bn = n.bit_len() as i64;
    let bd = d.bit_len() as i64;
    if bn >= bd {
        let diff = (bn - bd) as usize;
        let mut shifted = d.clone();
        shifted.shl(diff);
        if n.cmp(&shifted) != Ordering::Less {
            diff as i64
        } else {
            diff as i64 - 1
        }
    } else {
        let diff = (bd - bn) as usize;
        let mut shifted = n.clone();
        shifted.shl(diff);
        if shifted.cmp(d) != Ordering::Less {
            -(diff as i64)
        } else {
            -(diff as i64) - 1
        }
    }
}

/// `floor(n * 2^shift / d)` with `shift` possibly negative (then the
/// denominator shifts instead, keeping the floor exact).
fn scaled_divmod(n: &BigUint, d: &BigUint, shift: i64) -> (BigUint, BigUint) {
    if shift >= 0 {
        let mut numerator = n.clone();
        numerator.shl(shift as usize);
        numerator.divmod(d)
    } else {
        let mut denominator = d.clone();
        denominator.shl((-shift) as usize);
        n.divmod(&denominator)
    }
}

/// Round the exact positive value `n * 2^shift / d` (`n > 0`, `d > 0`) to a
/// 64-bit long-double mantissa (round-half-even). `sticky_in` covers value
/// dropped before this call (capped digit windows). Returns the value and
/// the ERANGE flag (overflow, or inexact underflow).
fn round_scaled_to_ld(
    n: &BigUint,
    d: &BigUint,
    shift: i64,
    sticky_in: bool,
    neg: bool,
) -> (FloatVal, bool) {
    debug_assert!(!n.is_zero() && !d.is_zero());
    let binary_exp = floor_log2_ratio(n, d) + shift;
    if binary_exp >= LD_MAX_UNBIASED_EXP + 1 {
        // At or above 2^16384: rounds to infinity (ERANGE always: the
        // rounded result differs from any finite value here).
        return (FloatVal::Infinite { neg }, true);
    }
    if binary_exp <= LD_MIN_NORMAL_UNBIASED_EXP - 1 {
        // Denormal range: round value * 2^16445 to an integer.
        let (doubled, rem) = scaled_divmod(n, d, shift + 16446);
        debug_assert!(doubled.bit_len() <= 64);
        let q = doubled.low_u64();
        let guard = q & 1;
        // Round-half-even at the integer: the dropped fraction is
        // guard/2 + rem/(2*d) plus anything truncated earlier.
        let round_up = guard == 1 && (!rem.is_zero() || sticky_in || ((q >> 1) & 1) == 1);
        let rounded = (q >> 1).wrapping_add(round_up as u64);
        let inexact = guard == 1 || !rem.is_zero() || sticky_in;
        if rounded == 0 {
            // Positive value rounding to zero is always inexact (ERANGE).
            return (
                FloatVal::Finite {
                    neg,
                    mant: 0,
                    exp: 0,
                },
                true,
            );
        }
        // `rounded == 2^63` is the smallest normal, carried in the same
        // representation (mant MSB set, exp pinned).
        return (
            FloatVal::Finite {
                neg,
                mant: rounded,
                exp: LD_DENORM_EXP,
            },
            inexact,
        );
    }
    // Normal range: extract the top 64 bits plus guard and sticky.
    let (wide, rem) = scaled_divmod(n, d, shift + 66 - binary_exp);
    debug_assert!(wide.bit_len() <= 67);
    let kept = wide.shr_to_u64(3);
    let low3 = wide.low_u64() & 0b111;
    let guard = (low3 >> 2) & 1;
    // Dropped fraction: low3/8 + rem/(8*d) plus truncated-earlier value.
    // A bare `100` rest is the halfway tie, broken towards even.
    let tie = low3 == 0b100 && rem.is_zero() && !sticky_in;
    let round_up = guard == 1 && (!tie || (kept & 1) == 1);
    let (mut mant, mut exp) = (kept, binary_exp - 63);
    if round_up {
        mant = mant.wrapping_add(1);
        if mant == 0 {
            // Carried past 2^64: renormalize; reaching 2^16384 is infinity.
            // The renormalized value is `2^63 * 2^exp`, unbiased exponent
            // `exp + 63`; only binary_exp == 16383 can reach 16384 here
            // (larger values returned early above).
            mant = 1u64 << 63;
            exp += 1;
            if exp + 63 >= LD_MAX_UNBIASED_EXP + 1 {
                return (FloatVal::Infinite { neg }, true);
            }
        }
    }
    debug_assert!(mant >> 63 == 1);
    (
        FloatVal::Finite {
            neg,
            mant,
            exp: exp as i32,
        },
        false,
    )
}

/// Parse an i128-saturating decimal exponent (`[+-]?digits`).
fn parse_saturating_exp(digits: &str) -> i128 {
    let (sign, body) = match digits.strip_prefix('-') {
        Some(rest) => (-1i128, rest),
        None => (1i128, digits.strip_prefix('+').unwrap_or(digits)),
    };
    if body.is_empty() {
        return 0;
    }
    if body.len() > 19 {
        return sign * (1i128 << 62);
    }
    sign * body.parse::<i128>().unwrap_or(1 << 62)
}

/// Convert a scanned decimal subject (sign included) exactly, then round
/// once to the long-double mantissa.
fn parse_decimal_subject(unsigned: &str, neg: bool) -> (FloatVal, bool) {
    let bytes = unsigned.as_bytes();
    let mut exp_pos = bytes.len();
    for (i, &byte) in bytes.iter().enumerate() {
        if byte == b'e' || byte == b'E' {
            exp_pos = i;
            break;
        }
    }
    let (mant_part, exp_part) = (&unsigned[..exp_pos], &unsigned[exp_pos..]);
    let mut exp10 = if exp_part.is_empty() {
        0i128
    } else {
        parse_saturating_exp(&exp_part[1..])
    };
    let dot = mant_part.find('.').unwrap_or(mant_part.len());
    let (int_digits, frac_digits) = (
        &mant_part[..dot],
        &mant_part[dot + (dot < mant_part.len()) as usize..],
    );
    exp10 -= frac_digits.len() as i128;
    let int_sig = int_digits.trim_start_matches('0');
    let mut sig = String::with_capacity(int_sig.len() + frac_digits.len());
    sig.push_str(int_sig);
    sig.push_str(frac_digits);
    let leading = sig.bytes().take_while(|&b| b == b'0').count();
    let sig = &sig[leading..];
    if sig.is_empty() {
        // Exact zero (also `0e9999`): no ERANGE.
        return (
            FloatVal::Finite {
                neg,
                mant: 0,
                exp: 0,
            },
            false,
        );
    }
    // Trailing zeros fold into the exponent (value-preserving).
    let zeros = sig.bytes().rev().take_while(|&b| b == b'0').count();
    let full_digits = sig.len();
    let sig = &sig[..full_digits - zeros];
    exp10 += zeros as i128;
    // Digit-count magnitude guards (plain i128 arithmetic, no bignum).
    let digits = full_digits - zeros;
    let k_lo = digits as i128 - 1 + exp10;
    if k_lo >= 0 && 3 * k_lo >= 16384 {
        return (FloatVal::Infinite { neg }, true);
    }
    let k_hi = digits as i128 + exp10;
    if k_hi <= 0 && 3 * k_hi <= -16446 {
        return (
            FloatVal::Finite {
                neg,
                mant: 0,
                exp: 0,
            },
            true,
        );
    }
    // Bounded digit window plus sticky bit (see DECIMAL_KEEP_DIGITS).
    let (sig, exp10, sticky) = if sig.len() > DECIMAL_KEEP_DIGITS {
        let drop = sig.len() - DECIMAL_KEEP_DIGITS;
        let sticky = sig.as_bytes()[DECIMAL_KEEP_DIGITS..]
            .iter()
            .any(|&b| b != b'0');
        (&sig[..DECIMAL_KEEP_DIGITS], exp10 + drop as i128, sticky)
    } else {
        (sig, exp10, false)
    };
    let mut num = BigUint::zero();
    for byte in sig.bytes() {
        num.mul_small(10);
        num.add_small((byte - b'0') as u64);
    }
    // |exp10| is guard-bounded to a few thousand here.
    let exp10 = exp10 as i64;
    if exp10 >= 0 {
        for _ in 0..exp10 {
            num.mul_small(5);
        }
        round_scaled_to_ld(&num, &BigUint::from_u64(1), exp10, sticky, neg)
    } else {
        let mut den = BigUint::from_u64(1);
        for _ in 0..-exp10 {
            den.mul_small(5);
        }
        round_scaled_to_ld(&num, &den, exp10, sticky, neg)
    }
}

/// Convert a scanned hex-float subject (`0x...p...`, sign included) exactly,
/// then round once to the long-double mantissa.
fn parse_hex_subject(unsigned: &str, neg: bool) -> (FloatVal, bool) {
    let hex = &unsigned[2..];
    let bytes = hex.as_bytes();
    let mut p_pos = bytes.len();
    for (i, &byte) in bytes.iter().enumerate() {
        if byte == b'p' || byte == b'P' {
            p_pos = i;
            break;
        }
    }
    let (mant_hex, exp_hex) = (&hex[..p_pos], &hex[p_pos..]);
    let mut pexp: i128 = if exp_hex.is_empty() {
        0
    } else {
        parse_saturating_exp(&exp_hex[1..])
    };
    let dot = mant_hex.find('.').unwrap_or(mant_hex.len());
    let (int_hex, frac_hex) = (
        &mant_hex[..dot],
        &mant_hex[dot + (dot < mant_hex.len()) as usize..],
    );
    let frac_len = frac_hex.len();
    let int_sig = int_hex.trim_start_matches('0');
    let mut sig = String::with_capacity(int_sig.len() + frac_hex.len());
    sig.push_str(int_sig);
    sig.push_str(frac_hex);
    let leading = sig.bytes().take_while(|&b| b == b'0').count();
    let sig = &sig[leading..];
    if sig.is_empty() {
        return (
            FloatVal::Finite {
                neg,
                mant: 0,
                exp: 0,
            },
            false,
        );
    }
    // Digit-count magnitude guards.
    let digits = sig.len() as i128;
    pexp -= 4 * frac_len as i128;
    if 4 * (digits - 1) + pexp >= 16384 {
        return (FloatVal::Infinite { neg }, true);
    }
    // Below half the minimum denormal (2^-16446) always rounds to zero;
    // the exact half must reach the rounder, where tie-to-even keeps it
    // zero on its own.
    if 4 * digits + pexp <= -16446 {
        return (
            FloatVal::Finite {
                neg,
                mant: 0,
                exp: 0,
            },
            true,
        );
    }
    // Bounded digit window plus sticky bit. Dropped hex nibbles are exact
    // 4-bit groups, so the exponent compensates (`pexp += 4 * drop`) and
    // the tail can only contribute to `sticky`.
    let (sig, pexp, sticky) = if sig.len() > HEX_KEEP_DIGITS {
        let drop = sig.len() - HEX_KEEP_DIGITS;
        let sticky = sig.as_bytes()[HEX_KEEP_DIGITS..].iter().any(|&b| b != b'0');
        (&sig[..HEX_KEEP_DIGITS], pexp + 4 * drop as i128, sticky)
    } else {
        (sig, pexp, false)
    };
    let mut num = BigUint::zero();
    for byte in sig.bytes() {
        num.mul_small(16);
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => 0,
        };
        num.add_small(digit as u64);
    }
    // `pexp` is guard-bounded; the shift stays small.
    round_scaled_to_ld(&num, &BigUint::from_u64(1), pexp as i64, sticky, neg)
}

/// Convert a scanned float subject (leading blanks + sign + body, exactly as
/// `scan_float_subject` delimited it) to the long-double value, with the
/// ERANGE flag (GNU chk_converror: overflow, or inexact underflow).
pub(super) fn parse_long_double_subject(subject: &str) -> (FloatVal, bool) {
    let body = subject.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (neg, unsigned) = match body.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, body.strip_prefix('+').unwrap_or(body)),
    };
    let bytes = unsigned.as_bytes();
    // inf / nan words (the scan guarantees the shape; any continuation is
    // already outside the subject, so a prefix test is exact).
    if bytes.len() >= 3 {
        let word = |w: &[u8; 3]| {
            bytes[0].eq_ignore_ascii_case(&w[0])
                && bytes[1].eq_ignore_ascii_case(&w[1])
                && bytes[2].eq_ignore_ascii_case(&w[2])
        };
        if word(b"inf") {
            return (FloatVal::Infinite { neg }, false);
        }
        if word(b"nan") {
            return (FloatVal::Nan { neg }, false);
        }
    }
    if bytes.len() >= 2 && bytes[0] == b'0' && (bytes[1] == b'x' || bytes[1] == b'X') {
        return parse_hex_subject(unsigned, neg);
    }
    parse_decimal_subject(unsigned, neg)
}

/// Normalize a small non-negative integer to `(mant, exp)` long-double form.
pub(super) fn normalize_small_int(value: u64) -> (u64, i32) {
    if value == 0 {
        return (0, 0);
    }
    let shift = value.leading_zeros();
    (value << shift, -(shift as i32))
}

/// Exact decimal expansion of `mant * 2^exp`: `(integer_digits, fraction_digits)`.
/// Binary fractions expand finitely, so the fraction holds at most `-exp`
/// digits and the loop always terminates.
pub(super) fn exact_parts(mant: u64, exp: i32) -> (String, String) {
    if mant == 0 {
        return ("0".to_string(), String::new());
    }
    if exp >= 0 {
        let mut int = BigUint::from_u64(mant);
        int.shl(exp as usize);
        return (int.to_decimal_string(), String::new());
    }
    let shift = (-(exp as i64)) as usize;
    let int_part = if shift >= 64 {
        "0".to_string()
    } else {
        (mant >> shift).to_string()
    };
    let mut rem = if shift >= 64 {
        BigUint::from_u64(mant)
    } else {
        let mut masked = BigUint::from_u64(mant);
        masked.mask_low(shift);
        masked
    };
    let mut modulus = BigUint::from_u64(1);
    modulus.shl(shift);
    let mut frac = String::new();
    while !rem.is_zero() && frac.len() < shift {
        rem.mul_small(10);
        let digit = rem.shr_to_u64(shift);
        debug_assert!(digit < 10);
        let mut drop = BigUint::from_u64(digit);
        drop.shl(shift);
        rem.sub_assign(&drop);
        frac.push(decimal_digit_char(digit as u8));
    }
    (int_part, frac)
}

/// The 16 nibbles (MSB first) of a normalized 64-bit mantissa, for `%a`.
pub(super) fn mantissa_nibbles(mant: u64) -> [u8; 16] {
    let mut nibbles = [0u8; 16];
    for (i, slot) in nibbles.iter_mut().enumerate() {
        *slot = ((mant >> (60 - 4 * i)) & 0xF) as u8;
    }
    nibbles
}

/// Render a decimal digit value (0-9) as a char. The reviewed byte->char
/// widening for the printf float engines (see the allowlist in
/// scripts/check-utf8-boundary-hygiene.sh): the operand is a digit by
/// construction, never a UTF-8 continuation byte.
pub(super) fn decimal_digit_char(digit: u8) -> char {
    debug_assert!(digit < 10);
    (b'0' + digit) as char
}

/// Render a hex nibble (0-15) as a char (same reviewed-widening story as
/// `decimal_digit_char`).
pub(super) fn hex_digit_char(nibble: u8, uppercase: bool) -> char {
    debug_assert!(nibble < 16);
    let base = if nibble < 10 {
        b'0'
    } else if uppercase {
        b'A' - 10
    } else {
        b'a' - 10
    };
    (base + nibble) as char
}
