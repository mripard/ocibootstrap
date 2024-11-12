#![doc = include_str!("../README.md")]

use core::iter::zip;
use std::{
    fs::File,
    io::{self, Seek, Write},
};

use bit_field::BitField;
use log::debug;
use num_traits::ToPrimitive;
use part::{div_round_up, num_cast, start_end_to_size};

const LBA_SIZE: usize = 512;

const MBR_LBA_OFFSET: usize = 0;
const MBR_LBA_SIZE: usize = 1;
const MBR_PART_ENTRY_OFFSET: usize = 446;
const MBR_PART_ENTRY_SIZE: usize = 16;

/// An MBR Partition Entry
#[derive(Debug)]
pub struct MasterBootRecordPartition {
    builder: MasterBootRecordPartitionBuilder,
}

/// An MBR Partition Entry Builder Structure
#[derive(Debug)]
pub struct MasterBootRecordPartitionBuilder {
    type_: u8,
    size: Option<u64>,
    bits: u8,
}

impl MasterBootRecordPartitionBuilder {
    /// Creates a new MBR Partition Builder of a specified type
    #[must_use]
    pub fn new(part_type: u8) -> Self {
        Self {
            type_: part_type,
            size: None,
            bits: 0,
        }
    }

    /// Sets the partition size in bytes. Whenever building the MBR, this size might be increased to
    /// be aligned to provide optimal device settings, but will never be decreased.
    ///
    /// If the size isn't provided, the partition will be made to fill any available space. Only one
    /// size-less partition is allowed to be part of a [`MasterBootRecordPartitionTable`].
    #[must_use]
    pub fn size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    /// Marks the partition as bootable.
    #[must_use]
    pub fn bootable(mut self, val: bool) -> Self {
        self.bits.set_bit(7, val);
        self
    }

    /// Creates a [`MasterBootRecordPartition`] from our builder
    #[must_use]
    pub fn build(self) -> MasterBootRecordPartition {
        MasterBootRecordPartition { builder: self }
    }
}

struct MBRTableLayout {
    block_size: u64,

    mbr_header_lba: u64,
    partitions_offset: Vec<(u64, u64, u64)>,
}

/// an MBR Partition Table Representation
#[derive(Debug)]
pub struct MasterBootRecordPartitionTable {
    builder: MasterBootRecordPartitionTableBuilder,
}

impl MasterBootRecordPartitionTable {
    fn lba_to_chs(&self, lba: u64) -> (u16, u8, u8) {
        let hpc: u64 = self.builder.heads_per_cylinder.into();
        let spt: u64 = self.builder.sectors_per_track.into();

        let c = num_cast!(u16, lba / (hpc * spt));
        let h = num_cast!(u8, (lba / spt) % hpc);
        let s = num_cast!(u8, (lba % spt) + 1);

        (c, h, s)
    }

    fn lba_to_chs_bytes(&self, lba: u64) -> [u8; 3] {
        let (c, h, s) = self.lba_to_chs(lba);
        if c > ((1 << 10) - 1) {
            let c_lo = num_cast!(u8, c & 0xff);
            let c_hi = num_cast!(u8, (c >> 8) & 0x3);

            [h, c_hi << 6 | s & 0x3f, c_lo]
        } else {
            [0xff, 0xff, 0xff]
        }
    }

    #[allow(clippy::unwrap_in_result)]
    fn build_table_layout(&self, file: &File) -> Result<MBRTableLayout, io::Error> {
        let metadata = file.metadata()?;

        let block_size_u64 = num_cast!(u64, LBA_SIZE);
        let blocks = metadata.len() / block_size_u64;

        debug!(
            "File has len of {} bytes, {} blocks",
            metadata.len(),
            blocks
        );

        let mbr_lba_offset_u32 = num_cast!(u32, MBR_LBA_OFFSET);
        debug!("Setting up MBR at LBA {mbr_lba_offset_u32}");

        let mbr_lba_size_u32 = num_cast!(u32, MBR_LBA_SIZE);
        let first_usable_lba = u64::from(mbr_lba_offset_u32 + mbr_lba_size_u32);
        debug!("First Usable LBA: {first_usable_lba}");

        let last_usable_lba = blocks - 1;
        debug!("Last Usable LBA: {last_usable_lba}");

        if first_usable_lba > last_usable_lba {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "File is too small",
            ));
        }

        let mut available_blocks = start_end_to_size(first_usable_lba, last_usable_lba);
        debug!("Available LBAs: {available_blocks}");

        let mut found_no_size = false;
        let part_sizes_lba = self
            .builder
            .partitions
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                Ok(if let Some(size) = p.builder.size {
                    let size_lba = div_round_up(size, block_size_u64);

                    debug!("Partition {idx}: Size {size_lba} LBAs");

                    if size_lba > available_blocks {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "No space left on the device",
                        ));
                    }

                    available_blocks -= size_lba;
                    debug!("Available LBAs {available_blocks}");

                    Some(size_lba)
                } else {
                    if found_no_size {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "Multiple Partitions with no size",
                        ));
                    }

                    found_no_size = true;
                    None
                })
            })
            .collect::<Result<Vec<Option<u64>>, io::Error>>()?;

        let mut next_lba = first_usable_lba;
        let parts = part_sizes_lba
            .iter()
            .map(|o| {
                let part_size = if let Some(size) = o {
                    *size
                } else {
                    available_blocks
                };

                let offset = next_lba;
                next_lba += part_size;

                (offset, part_size, next_lba - 1)
            })
            .collect::<Vec<(u64, u64, u64)>>();

        Ok(MBRTableLayout {
            block_size: block_size_u64,
            mbr_header_lba: u64::from(mbr_lba_offset_u32),
            partitions_offset: parts,
        })
    }

    /// Writes an MBR to a file
    ///
    /// # Errors
    ///
    /// This function will return an [`std::io::Error`] if there's an issue with the Partition Table
    /// layout, or when accessing the underlying [`File`].
    ///
    /// # Panics
    ///
    /// Panics if we have an integer overflow in one of the integer type conversions
    pub fn write(self, mut file: &File) -> Result<(), io::Error> {
        let cfg = self.build_table_layout(file)?;

        let mut mbr = [0u8; 512];

        let disk_id = rand::random::<u32>();

        debug!("Using Disk Identifier 0x{:x}", disk_id);

        mbr[440..444].copy_from_slice(&disk_id.to_le_bytes());

        for (idx, (part, (first_lba, size, last_lba))) in
            zip(&self.builder.partitions, cfg.partitions_offset).enumerate()
        {
            let mut mbr_part = [0u8; MBR_PART_ENTRY_SIZE];
            mbr_part[0] = part.builder.bits;

            let chs_bytes = self.lba_to_chs_bytes(first_lba);
            mbr_part[1..4].copy_from_slice(&chs_bytes);

            mbr_part[4] = part.builder.type_;

            let chs_bytes = self.lba_to_chs_bytes(last_lba);
            mbr_part[5..8].copy_from_slice(&chs_bytes);

            mbr_part[8..12].copy_from_slice(&first_lba.to_u32().unwrap_or(u32::MAX).to_le_bytes());
            mbr_part[12..16].copy_from_slice(&size.to_u32().unwrap_or(u32::MAX).to_le_bytes());

            let part_idx = MBR_PART_ENTRY_OFFSET + MBR_PART_ENTRY_SIZE * idx;
            mbr[part_idx..(part_idx + MBR_PART_ENTRY_SIZE)].copy_from_slice(&mbr_part);
        }

        mbr[510] = 0x55;
        mbr[511] = 0xaa;

        file.seek(io::SeekFrom::Start(cfg.mbr_header_lba * cfg.block_size))?;
        file.write_all(&mbr)?;
        file.flush()?;
        file.sync_data()?;

        Ok(())
    }
}

/// An MBR Partition Table Builder Structure
#[derive(Debug)]
pub struct MasterBootRecordPartitionTableBuilder {
    heads_per_cylinder: u8,
    sectors_per_track: u8,
    partitions: Vec<MasterBootRecordPartition>,
}

impl MasterBootRecordPartitionTableBuilder {
    /// Creates a new MBR Partition Table Builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            heads_per_cylinder: 16,
            sectors_per_track: 63,
            partitions: Vec::new(),
        }
    }

    /// Adds a [`MasterBootRecordPartition`] to the Partition Table
    #[must_use]
    pub fn add_partition(mut self, part: MasterBootRecordPartition) -> Self {
        self.partitions.push(part);
        self
    }

    /// Creates a [`GuidPartitionTable`] from our builder
    #[must_use]
    pub fn build(self) -> MasterBootRecordPartitionTable {
        MasterBootRecordPartitionTable { builder: self }
    }
}

impl Default for MasterBootRecordPartitionTableBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, process::Command};

    use log::{debug, trace};
    use num_traits::ToPrimitive;
    use serde::{de, Deserialize};
    use tempfile::NamedTempFile;
    use test_log::test;

    use crate::{
        MasterBootRecordPartitionBuilder, MasterBootRecordPartitionTableBuilder, LBA_SIZE,
        MBR_LBA_OFFSET, MBR_LBA_SIZE,
    };

    const TEST_PARTITION_TYPE: u8 = 42;
    const TEST_PARTITION_SECONDARY_TYPE: u8 = 142;

    const TEMP_FILE_SIZE: u64 = 2 << 30;

    fn deserialize_hex_to_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        if let Some(s) = s.strip_prefix("0x") {
            u32::from_str_radix(&s, 16)
        } else if let Some(s) = s.strip_prefix("0o") {
            u32::from_str_radix(&s, 8)
        } else if let Some(s) = s.strip_prefix("0b") {
            u32::from_str_radix(&s, 2)
        } else {
            u32::from_str_radix(&s, 10)
        }
        .map_err(de::Error::custom)
    }

    fn deserialize_hex_to_u8<'de, D>(deserializer: D) -> Result<u8, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        u8::from_str_radix(&s, 16).map_err(de::Error::custom)
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SfDiskMbrPartition {
        #[serde(rename = "node")]
        _node: PathBuf,
        start: u64,
        size: u64,
        #[serde(rename = "type", deserialize_with = "deserialize_hex_to_u8")]
        kind: u8,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SfDiskMbrPartitionTable {
        #[serde(deserialize_with = "deserialize_hex_to_u32")]
        id: u32,
        #[serde(rename = "device")]
        _device: PathBuf,
        #[serde(rename = "unit")]
        _unit: String,
        #[serde(rename = "sectorsize")]
        sector_size: u64,
        #[serde(default)]
        partitions: Vec<SfDiskMbrPartition>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(tag = "label", rename_all = "lowercase")]
    enum SfDiskPartitionTable {
        Dos(SfDiskMbrPartitionTable),
    }

    #[derive(Debug, Deserialize)]
    struct SfdiskOutput {
        #[serde(rename = "partitiontable")]
        table: SfDiskPartitionTable,
    }

    #[test]
    fn test_table_no_partition() {
        let temp_file = NamedTempFile::new().unwrap();

        temp_file.as_file().set_len(TEMP_FILE_SIZE).unwrap();

        MasterBootRecordPartitionTableBuilder::new()
            .build()
            .write(temp_file.as_file())
            .unwrap();

        let output = Command::new("sfdisk")
            .arg("-J")
            .arg(temp_file.path())
            .output()
            .unwrap();

        trace!("{}", String::from_utf8(output.stdout.clone()).unwrap());

        let res: SfdiskOutput = serde_json::from_slice(&output.stdout).unwrap();

        let table = match res.table {
            SfDiskPartitionTable::Dos(v) => v,
        };

        assert_ne!(table.id, 0);

        let block_size_u64 = LBA_SIZE.to_u64().unwrap();
        assert_eq!(table.sector_size, block_size_u64);
        assert_eq!(table.partitions.len(), 0);
    }

    #[test]
    fn test_file_too_small() {
        let temp_file = NamedTempFile::new().unwrap();

        // The MBR overhead is 1 LBA
        temp_file
            .as_file()
            .set_len((MBR_LBA_SIZE * LBA_SIZE).to_u64().unwrap())
            .unwrap();

        MasterBootRecordPartitionTableBuilder::new()
            .build()
            .write(temp_file.as_file())
            .unwrap_err();
    }

    #[test]
    fn test_one_partition_no_size() {
        let file_size = TEMP_FILE_SIZE;

        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(file_size).unwrap();

        MasterBootRecordPartitionTableBuilder::new()
            .add_partition(MasterBootRecordPartitionBuilder::new(TEST_PARTITION_TYPE).build())
            .build()
            .write(temp_file.as_file())
            .unwrap();

        let output = Command::new("sfdisk")
            .arg("-J")
            .arg(temp_file.path())
            .output()
            .unwrap();

        trace!("{}", String::from_utf8(output.stdout.clone()).unwrap());

        let res: SfdiskOutput = serde_json::from_slice(&output.stdout).unwrap();

        let table = match res.table {
            SfDiskPartitionTable::Dos(v) => v,
        };

        assert_ne!(table.id, 0);

        let block_size_u64 = LBA_SIZE.to_u64().unwrap();
        assert_eq!(table.sector_size, block_size_u64);
        assert_eq!(table.partitions.len(), 1);

        let part = &table.partitions[0];
        assert_eq!(part.kind, TEST_PARTITION_TYPE);

        let start = (MBR_LBA_OFFSET + MBR_LBA_SIZE).to_u64().unwrap();
        assert_eq!(part.start, start);

        let size = ((TEMP_FILE_SIZE / block_size_u64) - start)
            .to_u64()
            .unwrap();
        assert_eq!(part.size, size);
    }

    #[test]
    fn test_one_partition_exact_size() {
        let file_size = TEMP_FILE_SIZE;

        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(file_size).unwrap();

        let first_lba = (MBR_LBA_OFFSET + MBR_LBA_SIZE).to_u64().unwrap();
        let last_lba = ((TEMP_FILE_SIZE.to_usize().unwrap() / LBA_SIZE) - MBR_LBA_SIZE)
            .to_u64()
            .unwrap();

        let block_size_u64 = LBA_SIZE.to_u64().unwrap();
        MasterBootRecordPartitionTableBuilder::new()
            .add_partition(
                MasterBootRecordPartitionBuilder::new(TEST_PARTITION_TYPE)
                    .size((last_lba - first_lba) * block_size_u64)
                    .build(),
            )
            .build()
            .write(temp_file.as_file())
            .unwrap();

        let output = Command::new("sfdisk")
            .arg("-J")
            .arg(temp_file.path())
            .output()
            .unwrap();

        trace!("{}", String::from_utf8(output.stdout.clone()).unwrap());

        let res: SfdiskOutput = serde_json::from_slice(&output.stdout).unwrap();

        let table = match res.table {
            SfDiskPartitionTable::Dos(v) => v,
        };

        assert_ne!(table.id, 0);
        assert_eq!(table.sector_size, block_size_u64);
        assert_eq!(table.partitions.len(), 1);

        let part = &table.partitions[0];
        assert_eq!(part.kind, TEST_PARTITION_TYPE);
        assert_eq!(part.start, first_lba);
        assert_eq!(part.size, last_lba - first_lba);
    }

    #[test]
    fn test_one_partition_exact_size_non_lba_aligned() {
        let file_size = TEMP_FILE_SIZE;

        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(file_size).unwrap();

        let first_lba = (MBR_LBA_OFFSET + MBR_LBA_SIZE).to_u64().unwrap();
        let last_lba = ((TEMP_FILE_SIZE.to_usize().unwrap() / LBA_SIZE) - MBR_LBA_SIZE)
            .to_u64()
            .unwrap();

        let block_size_u64 = LBA_SIZE.to_u64().unwrap();
        let part_size_bytes = (last_lba - first_lba) * block_size_u64 - 10;
        MasterBootRecordPartitionTableBuilder::new()
            .add_partition(
                MasterBootRecordPartitionBuilder::new(TEST_PARTITION_TYPE)
                    .size(part_size_bytes)
                    .build(),
            )
            .build()
            .write(temp_file.as_file())
            .unwrap();

        let output = Command::new("sfdisk")
            .arg("-J")
            .arg(temp_file.path())
            .output()
            .unwrap();

        trace!("{}", String::from_utf8(output.stdout.clone()).unwrap());

        let res: SfdiskOutput = serde_json::from_slice(&output.stdout).unwrap();

        let table = match res.table {
            SfDiskPartitionTable::Dos(v) => v,
        };

        assert_ne!(table.id, 0);
        assert_eq!(table.sector_size, block_size_u64);
        assert_eq!(table.partitions.len(), 1);

        let part = &table.partitions[0];
        assert_eq!(part.kind, TEST_PARTITION_TYPE);
        assert_eq!(part.start, first_lba);
        assert_eq!(part.size, last_lba - first_lba);
    }

    #[test]
    fn test_two_partitions_one_size_missing() {
        let file_size = TEMP_FILE_SIZE;

        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(file_size).unwrap();

        let first_lba = (MBR_LBA_OFFSET + MBR_LBA_SIZE).to_u64().unwrap();

        debug!("First LBA is {first_lba}");

        let last_lba = ((TEMP_FILE_SIZE.to_usize().unwrap() / LBA_SIZE) - MBR_LBA_SIZE)
            .to_u64()
            .unwrap();

        debug!("Last LBA is {last_lba}");

        let cutoff_lba = (last_lba - first_lba) / 2;

        debug!("Cutoff LBA is {cutoff_lba}");

        let block_size_u64 = LBA_SIZE.to_u64().unwrap();
        MasterBootRecordPartitionTableBuilder::new()
            .add_partition(
                MasterBootRecordPartitionBuilder::new(TEST_PARTITION_TYPE)
                    .size((cutoff_lba - first_lba) * block_size_u64)
                    .build(),
            )
            .add_partition(
                MasterBootRecordPartitionBuilder::new(TEST_PARTITION_SECONDARY_TYPE).build(),
            )
            .build()
            .write(temp_file.as_file())
            .unwrap();

        let output = Command::new("sfdisk")
            .arg("-J")
            .arg(temp_file.path())
            .output()
            .unwrap();

        trace!("{}", String::from_utf8(output.stdout.clone()).unwrap());

        let res: SfdiskOutput = serde_json::from_slice(&output.stdout).unwrap();

        let table = match res.table {
            SfDiskPartitionTable::Dos(v) => v,
        };

        assert_ne!(table.id, 0);
        assert_eq!(table.sector_size, block_size_u64);
        assert_eq!(table.partitions.len(), 2);

        let part = &table.partitions[0];
        assert_eq!(part.kind, TEST_PARTITION_TYPE);
        assert_eq!(part.start, first_lba);
        assert_eq!(part.size, cutoff_lba - first_lba);

        let part = &table.partitions[1];
        assert_eq!(part.kind, TEST_PARTITION_SECONDARY_TYPE);
        assert_eq!(part.start, cutoff_lba);
        assert_eq!(part.size, (last_lba + 1) - cutoff_lba);
    }

    #[test]
    fn test_two_partitions_exact_size() {
        let file_size = TEMP_FILE_SIZE;

        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(file_size).unwrap();

        let first_lba = (MBR_LBA_OFFSET + MBR_LBA_SIZE).to_u64().unwrap();

        debug!("First LBA is {first_lba}");

        let last_lba = ((TEMP_FILE_SIZE.to_usize().unwrap() / LBA_SIZE) - MBR_LBA_SIZE)
            .to_u64()
            .unwrap();

        debug!("Last LBA is {last_lba}");

        let cutoff_lba = (last_lba - first_lba) / 2;

        debug!("Cutoff LBA is {cutoff_lba}");

        let block_size_u64 = LBA_SIZE.to_u64().unwrap();
        MasterBootRecordPartitionTableBuilder::new()
            .add_partition(
                MasterBootRecordPartitionBuilder::new(TEST_PARTITION_TYPE)
                    .size((cutoff_lba - first_lba) * block_size_u64)
                    .build(),
            )
            .add_partition(
                MasterBootRecordPartitionBuilder::new(TEST_PARTITION_SECONDARY_TYPE)
                    .size((last_lba - cutoff_lba) * block_size_u64)
                    .build(),
            )
            .build()
            .write(temp_file.as_file())
            .unwrap();

        let output = Command::new("sfdisk")
            .arg("-J")
            .arg(temp_file.path())
            .output()
            .unwrap();

        trace!("{}", String::from_utf8(output.stdout.clone()).unwrap());

        let res: SfdiskOutput = serde_json::from_slice(&output.stdout).unwrap();

        let table = match res.table {
            SfDiskPartitionTable::Dos(v) => v,
        };

        assert_ne!(table.id, 0);
        assert_eq!(table.sector_size, block_size_u64);
        assert_eq!(table.partitions.len(), 2);

        let part = &table.partitions[0];
        assert_eq!(part.kind, TEST_PARTITION_TYPE);
        assert_eq!(part.start, first_lba);
        assert_eq!(part.size, cutoff_lba - first_lba);

        let part = &table.partitions[1];
        assert_eq!(part.kind, TEST_PARTITION_SECONDARY_TYPE);
        assert_eq!(part.start, cutoff_lba);
        assert_eq!(part.size, last_lba - cutoff_lba);
    }

    #[test]
    fn test_multiple_partitions_no_size() {
        let file_size = TEMP_FILE_SIZE;

        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(file_size).unwrap();

        MasterBootRecordPartitionTableBuilder::new()
            .add_partition(MasterBootRecordPartitionBuilder::new(TEST_PARTITION_TYPE).build())
            .add_partition(
                MasterBootRecordPartitionBuilder::new(TEST_PARTITION_SECONDARY_TYPE).build(),
            )
            .build()
            .write(temp_file.as_file())
            .unwrap_err();
    }
}
