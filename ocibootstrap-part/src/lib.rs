#![doc = include_str!("../README.md")]

use core::ops::{Div, Mul, Rem};

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
