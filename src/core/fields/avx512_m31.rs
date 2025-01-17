use core::arch::x86_64::{
    __m512i, _mm512_add_epi32, _mm512_min_epu32, _mm512_mul_epu32, _mm512_srli_epi64,
    _mm512_sub_epi32,
};
use std::arch::x86_64::_mm512_permutex2var_epi32;
use std::fmt::Display;
use std::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use super::m31::{M31, P};
pub const K_BLOCK_SIZE: usize = 16;
pub const M512P: __m512i = unsafe { core::mem::transmute([P; K_BLOCK_SIZE]) };

/// AVX512 implementation of M31.
/// Stores 16 M31 elements in a single 512-bit register.
/// Each M31 element is unreduced in the range [0, P].
#[derive(Copy, Clone, Debug)]
pub struct M31AVX512(__m512i);

impl M31AVX512 {
    pub fn from_array(v: [M31; K_BLOCK_SIZE]) -> M31AVX512 {
        unsafe { Self(std::mem::transmute(v)) }
    }

    pub fn from_m512_unchecked(x: __m512i) -> Self {
        Self(x)
    }

    pub fn to_array(self) -> [M31; K_BLOCK_SIZE] {
        unsafe { std::mem::transmute(self.reduce()) }
    }

    /// Reduces each word in the 512-bit register to the range `[0, P)`, excluding P.
    pub fn reduce(self) -> M31AVX512 {
        Self(unsafe { _mm512_min_epu32(self.0, _mm512_sub_epi32(self.0, M512P)) })
    }
}

impl Display for M31AVX512 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let v = self.to_array();
        for elem in v.iter() {
            write!(f, "{} ", elem)?;
        }
        Ok(())
    }
}

impl Add for M31AVX512 {
    type Output = Self;

    /// Adds two packed M31 elements, and reduces the result to the range [0,P].
    /// Each value is assumed to be in unreduced form, [0, P] including P.
    #[inline(always)]
    fn add(self, rhs: Self) -> Self::Output {
        Self(unsafe {
            // Add word by word. Each word is in the range [0, 2P].
            let c = _mm512_add_epi32(self.0, rhs.0);
            // Apply min(c, c-P) to each word.
            // When c in [P,2P], then c-P in [0,P] which is always less than [P,2P].
            // When c in [0,P-1], then c-P in [2^32-P,2^32-1] which is always greater than [0,P-1].
            _mm512_min_epu32(c, _mm512_sub_epi32(c, M512P))
        })
    }
}

impl AddAssign for M31AVX512 {
    #[inline(always)]
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Mul for M31AVX512 {
    type Output = Self;

    /// Computes the product of two packed M31 elements
    /// Each value is assumed to be in unreduced form, [0, P] including P.
    /// Returned values are in unreduced form, [0, P] including P.
    #[inline(always)]
    fn mul(self, rhs: Self) -> Self::Output {
        /// An input to _mm512_permutex2var_epi32, and is used to interleave the even words of a
        /// with the even words of b.
        const EVENS_INTERLEAVE_EVENS: __m512i = unsafe {
            core::mem::transmute([
                0b00000, 0b10000, 0b00010, 0b10010, 0b00100, 0b10100, 0b00110, 0b10110, 0b01000,
                0b11000, 0b01010, 0b11010, 0b01100, 0b11100, 0b01110, 0b11110,
            ])
        };
        /// An input to _mm512_permutex2var_epi32, and is used to interleave the odd words of a
        /// with the odd words of b.
        const ODDS_INTERLEAVE_ODDS: __m512i = unsafe {
            core::mem::transmute([
                0b00001, 0b10001, 0b00011, 0b10011, 0b00101, 0b10101, 0b00111, 0b10111, 0b01001,
                0b11001, 0b01011, 0b11011, 0b01101, 0b11101, 0b01111, 0b11111,
            ])
        };

        unsafe {
            // Set up a word s.t. the lower half of each 64-bit word has the even 32-bit words of
            // the first operand.
            let val0_e = self.0;
            // Set up a word s.t. the lower half of each 64-bit word has the odd 32-bit words of
            // the first operand.
            let val0_o = _mm512_srli_epi64(self.0, 32);

            // Double the second operand.
            let val1 = _mm512_add_epi32(rhs.0, rhs.0);
            let val1_e = val1;
            let val1_o = _mm512_srli_epi64(val1, 32);

            // To compute prod = val0 * val1 start by multiplying
            // val0_e/o by val1_e/o.
            let prod_e_dbl = _mm512_mul_epu32(val0_e, val1_e);
            let prod_o_dbl = _mm512_mul_epu32(val0_o, val1_o);

            // The result of a multiplication holds val1*twiddle_dbl in as 64-bits.
            // Each 64b-bit word looks like this:
            //               1    31       31    1
            // prod_e_dbl - |0|prod_e_h|prod_e_l|0|
            // prod_o_dbl - |0|prod_o_h|prod_o_l|0|

            // Interleave the even words of prod_e_dbl with the even words of prod_o_dbl:
            let prod_ls = _mm512_permutex2var_epi32(prod_e_dbl, EVENS_INTERLEAVE_EVENS, prod_o_dbl);
            // prod_ls -    |prod_o_l|0|prod_e_l|0|

            // Divide by 2:
            let prod_ls = Self(_mm512_srli_epi64(prod_ls, 1));
            // prod_ls -    |0|prod_o_l|0|prod_e_l|

            // Interleave the odd words of prod_e_dbl with the odd words of prod_o_dbl:
            let prod_hs = Self(_mm512_permutex2var_epi32(
                prod_e_dbl,
                ODDS_INTERLEAVE_ODDS,
                prod_o_dbl,
            ));
            // prod_hs -    |0|prod_o_h|0|prod_e_h|

            Self::add(prod_ls, prod_hs)
        }
    }
}

impl MulAssign for M31AVX512 {
    #[inline(always)]
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}

impl Neg for M31AVX512 {
    type Output = Self;

    #[inline(always)]
    fn neg(self) -> Self::Output {
        Self(unsafe { _mm512_sub_epi32(M512P, self.0) })
    }
}

/// Subtracts two packed M31 elements, and reduces the result to the range [0,P].
/// Each value is assumed to be in unreduced form, [0, P] including P.
impl Sub for M31AVX512 {
    type Output = Self;

    #[inline(always)]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(unsafe {
            // Subtract word by word. Each word is in the range [-P, P].
            let c = _mm512_sub_epi32(self.0, rhs.0);
            // Apply min(c, c+P) to each word.
            // When c in [0,P], then c+P in [P,2P] which is always greater than [0,P].
            // When c in [2^32-P,2^32-1], then c+P in [0,P-1] which is always less than
            // [2^32-P,2^32-1].
            _mm512_min_epu32(_mm512_add_epi32(c, M512P), c)
        })
    }
}

impl SubAssign for M31AVX512 {
    #[inline(always)]
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

#[cfg(test)]
mod tests {

    use itertools::Itertools;

    use super::M31AVX512;
    use crate::core::fields::m31::{M31, P};
    use crate::core::fields::Field;

    /// Tests field operations where field elements are in reduced form.
    #[test]
    fn test_avx512_basic_ops() {
        if !crate::platform::avx512_detected() {
            return;
        }

        let values = [
            0,
            1,
            2,
            10,
            (P - 1) / 2,
            (P + 1) / 2,
            P - 2,
            P - 1,
            0,
            1,
            2,
            10,
            (P - 1) / 2,
            (P + 1) / 2,
            P - 2,
            P - 1,
        ]
        .map(M31::from_u32_unchecked);
        let avx_values = M31AVX512::from_array(values);

        assert_eq!(
            (avx_values + avx_values)
                .to_array()
                .into_iter()
                .collect_vec(),
            values.iter().map(|x| x.double()).collect_vec()
        );
        assert_eq!(
            (avx_values * avx_values)
                .to_array()
                .into_iter()
                .collect_vec(),
            values.iter().map(|x| x.square()).collect_vec()
        );
        assert_eq!(
            (-avx_values).to_array().into_iter().collect_vec(),
            values.iter().map(|x| -*x).collect_vec()
        );
    }
}
