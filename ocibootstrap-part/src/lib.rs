#![doc = include_str!("../README.md")]

use core::ops::{Add, Div, Mul, Rem, Sub};
use std::io;

use log::debug;
use num_traits::{ConstOne, ConstZero};
use test_log as _;

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

/// Size and Offset Partition Requirements for our layout
#[derive(Debug)]
pub struct PartitionLayoutHint {
    /// Offset Requirement, in LBAs
    pub offset_lba: Option<usize>,

    /// Size Requirement, in LBAs
    pub size_lba: Option<usize>,
}

/// Partition Layout
#[derive(Eq, Debug, PartialEq)]
pub struct PartitionLayout {
    /// Partition Start LBA
    pub start_lba: usize,

    /// Partition End LBA
    pub end_lba: usize,
}

/// Builds the partition layout for partition table out of a set of constraints
///
/// # Errors
///
/// Returns an [`std::io::Error`] if the constraints can't be met
///
/// # Panics
///
/// If the code confused itself
#[expect(clippy::too_many_lines)]
#[expect(clippy::panic_in_result_fn)]
pub fn build_layout(
    first_usable_lba: usize,
    last_usable_lba: usize,
    parts: &[PartitionLayoutHint],
) -> Result<Vec<PartitionLayout>, io::Error> {
    let missing_size_count = parts.iter().filter(|p| p.size_lba.is_none()).count();
    if missing_size_count > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Multiple Partitions with no size",
        ));
    }

    let mut array = Vec::with_capacity(parts.len());
    array.resize_with(parts.len(), Default::default);

    let mut missing_size_part_idx = None;
    let mut missing_size_part_start = None;
    let mut first_available_lba = first_usable_lba;

    for idx in 0..parts.len() {
        let part = &parts[idx];

        let part_offset_lba = if let Some(offset_lba) = part.offset_lba {
            offset_lba
        } else {
            first_available_lba
        };

        debug!("Partition {idx}: Offset is {:#?}", part_offset_lba);

        let Some(part_size_lba) = part.size_lba else {
            debug!("Partition {idx}: No size provided. Start offset is {part_offset_lba}");

            missing_size_part_start = Some(part_offset_lba);
            missing_size_part_idx = Some(idx);
            break;
        };

        debug!("Partition {idx}: Size is {:#?}", part_size_lba);

        first_available_lba = part_offset_lba + part_size_lba;
        array[idx] = Some((part_offset_lba, part_size_lba));
    }

    if let Some(missing_idx) = missing_size_part_idx {
        debug!("Partition {missing_idx}: Missing size, we'll have to figure it out.");

        let missing_part_offset_lba = missing_size_part_start.unwrap_or_else(|| {
            unreachable!("If we have a index for the missing size partition, we have a start LBA.")
        });

        let mut last_allocated_lba = last_usable_lba + 1;
        for idx in (missing_idx..parts.len()).rev() {
            let part = &parts[idx];

            let last_available_lba = last_allocated_lba - 1;
            let (part_offset_lba, part_size_lba) = if let (Some(size_lba), Some(offset_lba)) =
                (part.size_lba, part.offset_lba)
            {
                debug!(
                    "Partition {idx}: Fixed offset (LBA {offset_lba}) and size ({size_lba} LBAs)"
                );

                (offset_lba, size_lba)
            } else if let (Some(size_lba), None) = (part.size_lba, part.offset_lba) {
                debug!(
                    "Partition {idx}: Fixed size ({size_lba} LBAs). Last Available LBA {last_available_lba}"
                );

                let offset_lba = last_available_lba - (size_lba - 1);

                debug!(
                    "Partition {idx}: Fixed size ({size_lba} LBAs). Offset derived at LBA {offset_lba}"
                );

                (offset_lba, size_lba)
            } else if let (None, Some(offset_lba)) = (part.size_lba, part.offset_lba) {
                let size_lba = (last_available_lba - offset_lba) + 1;

                debug!(
                    "Partition {idx}: Fixed offset (LBA {offset_lba}). Size derived at {size_lba} LBAs"
                );

                (offset_lba, size_lba)
            } else {
                let offset_lba = missing_part_offset_lba;
                let size_lba = (last_available_lba - missing_part_offset_lba) + 1;

                debug!(
                    "Partition {idx}: Offset derived at LBA {offset_lba}. Size derived at {size_lba} LBAs"
                );

                (offset_lba, size_lba)
            };

            last_allocated_lba = part_offset_lba;
            array[idx] = Some((part_offset_lba, part_size_lba));
        }
    }

    assert_eq!(
        array.len(),
        parts.len(),
        "Our array must and should be the same size than the partitions slice."
    );

    assert!(
        !array.iter().any(Option::is_none),
        "Our array must and should not have any None by now."
    );

    let mut next_available_lba = first_usable_lba;
    for (offset, size) in array.iter().flatten() {
        if *offset < first_usable_lba {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Partition starts before first usable LBA.",
            ));
        }

        if *offset < next_available_lba {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Partition overlaps with previous partition.",
            ));
        }

        let end = offset + (size - 1);
        if end > last_usable_lba {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Partition overflows the device",
            ));
        }

        next_available_lba = offset + size;
    }

    Ok(array
        .iter()
        .map(|o| {
            let (offset, size) = o
                .unwrap_or_else(|| unreachable!("We already checked above that we only had Somes"));

            PartitionLayout {
                start_lba: offset,
                end_lba: start_size_to_end(offset, size),
            }
        })
        .collect())
}
