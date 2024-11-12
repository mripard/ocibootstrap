#![doc = include_str!("../README.md")]

use core::ops::{Add, Div, Mul, Rem, Sub};

use num_traits::{ConstOne, ConstZero};

/// Returns a rounded down number to the nearest multiple
///
/// # Panics
///
/// If the multiple is zero.
pub fn round_down<T>(number: T, multiple: T) -> T
where
    T: Copy + Div<Output = T> + Mul<Output = T>,
{
    let div = number / multiple;

    div * multiple
}

/// Returns a rounded up number to the nearest multiple
///
/// # Panics
///
/// If the multiple is zero.
pub fn round_up<T>(number: T, multiple: T) -> T
where
    T: ConstOne + ConstZero + Copy + Div<Output = T> + Eq + Mul<Output = T> + Rem<Output = T>,
{
    let rem = number % multiple;

    if rem == T::ZERO {
        return number;
    }

    let div = (number / multiple) + T::ONE;

    div * multiple
}

/// Returns the result of the rounded up division of numerator by denominator
///
/// # Panics
///
/// If the multiple is zero.
pub fn div_round_up<T>(numerator: T, denominator: T) -> T
where
    T: ConstOne + ConstZero + Copy + Div<Output = T> + Eq + Mul<Output = T> + Rem<Output = T>,
{
    let rounded = round_up(numerator, denominator);

    rounded / denominator
}

#[must_use]
#[doc(hidden)]
pub fn type_name_of_expr<T>(_: T) -> &'static str {
    core::any::type_name::<T>()
}

/// Converts an integer to another integer type
///
/// # Panics
///
/// If the conversion fails.
#[macro_export]
macro_rules! num_cast {
    ($t: ty, $v: expr) => {
        <$t>::try_from($v).expect(&format!(
            "Integer Overflow ({} to {})",
            core::any::type_name::<$t>(),
            $crate::type_name_of_expr($v),
        ))
    };
}

/// Computes the size between a start and end indexes
///
/// # Panics
///
/// If start or end are negative, or if end is lower than start.
pub fn start_end_to_size<T>(start: T, end: T) -> T
where
    T: Add<Output = T> + Ord + ConstOne + ConstZero + Sub<Output = T>,
{
    assert!(start >= T::ZERO, "Negative start offset");
    assert!(end >= T::ZERO, "Negative end offset");
    assert!(end >= start, "End offset is lower than start offset");

    (end - start) + T::ONE
}

/// Computes the end index from a start index and a size
///
/// # Panics
///
/// If start is negative, or if the size is lower than or equal to zero.
pub fn start_size_to_end<T>(start: T, size: T) -> T
where
    T: Add<Output = T> + ConstOne + ConstZero + Ord + Sub<Output = T>,
{
    assert!(start >= T::ZERO, "Negative start offset");
    assert!(size >= T::ONE, "Size too small");

    (start + size) - T::ONE
}

/// Computes the start index from a size and end index
///
/// # Panics
///
/// If size or end are negative, if the size is zero, or if the start index
/// would be negative.
pub fn size_end_to_start<T>(size: T, end: T) -> T
where
    T: Add<Output = T> + Copy + ConstOne + ConstZero + Ord + Sub<Output = T>,
{
    assert!(size >= T::ONE, "Size too small");
    assert!(end >= T::ZERO, "Negative end offset");
    assert!(end >= (size - T::ONE), "Size too large for end offset.");

    end - (size - T::ONE)
}
