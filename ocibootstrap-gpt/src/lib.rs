#![doc = include_str!("../README.md")]

use std::{
    fs::File,
    io::{self, Seek, Write},
};

use bit_field::BitField;
use log::debug;
use mbr::{MasterBootRecordPartitionBuilder, MasterBootRecordPartitionTableBuilder};
use part::{num_cast, round_down, round_up, start_end_to_size};
use uuid::{uuid, Uuid};

const BLOCK_SIZE: usize = 512;

const MBR_HEADER_OFFSET_LBA: usize = 0;
const MBR_SIZE_LBA: usize = 1;

const GPT_SIGNATURE_HEADER: u64 = 0x5452_4150_2049_4645;
const GPT_VERSION_HEADER: u32 = 0x0001_0000;
const GPT_HEADER_SIZE_LBA: usize = 1;
const GPT_PARTITION_NUM: usize = 128;
const GPT_PARTITION_ENTRY_SIZE: usize = 128;
const GPT_PARTITION_HEADER_SIZE_LBA: usize =
    (GPT_PARTITION_NUM * GPT_PARTITION_ENTRY_SIZE) / BLOCK_SIZE;

const GPT_PARTITION_ALIGNMENT: usize = 4 << 20;

/// Standard EFI System Partition GUID. See the
/// [UAPI discoverable partition specification](https://uapi-group.org/specifications/specs/discoverable_partitions_specification/)
/// for further details.
pub const EFI_SYSTEM_PART_GUID: Uuid = uuid!("c12a7328-f81f-11d2-ba4b-00a0c93ec93b");

/// Standard Extended Bootloader GUID. See the
/// [UAPI discoverable partition specification](https://uapi-group.org/specifications/specs/discoverable_partitions_specification/)
/// for further details.
pub const EXTENDED_BOOTLOADER_PART_GUID: Uuid = uuid!("bc13c2ff-59e6-4262-a352-b275fd6f7172");

/// Standard Root Partition GUID for the ARM64/AARCH64 architecture. See the
/// [UAPI discoverable partition specification](https://uapi-group.org/specifications/specs/discoverable_partitions_specification/)
/// for further details.
pub const ROOT_PART_GUID_ARM64: Uuid = uuid!("b921b045-1df0-41c3-af44-4c6f280d3fae");

fn guid_bytes(uuid: &Uuid) -> [u8; 16] {
    let uuid_fields = uuid.as_fields();

    let mut uuid = [0; 16];
    uuid[0..4].copy_from_slice(&uuid_fields.0.to_le_bytes());
    uuid[4..6].copy_from_slice(&uuid_fields.1.to_le_bytes());
    uuid[6..8].copy_from_slice(&uuid_fields.2.to_le_bytes());
    uuid[8..].copy_from_slice(uuid_fields.3);

    uuid
}

struct GuidPartitionTableLayout {
    block_size: usize,

    mbr_header_lba: usize,
    primary_gpt_header_lba: usize,
    primary_gpt_table_lba: usize,
    first_usable: usize,
    partitions_offset: Vec<(usize, usize)>,
    last_usable: usize,
    backup_gpt_table_lba: usize,
    backup_gpt_header_lba: usize,
}

/// GUID Partition Table Representation
#[derive(Debug)]
pub struct GuidPartitionTable {
    builder: GuidPartitionTableBuilder,
}

impl GuidPartitionTable {
    #[allow(clippy::too_many_lines, clippy::unwrap_in_result)]
    fn build_gpt_layout(&self, file: &File) -> Result<GuidPartitionTableLayout, io::Error> {
        let metadata = file.metadata()?;

        let blocks = num_cast!(usize, metadata.len()) / BLOCK_SIZE;

        debug!(
            "File has len of {} bytes, {} blocks",
            metadata.len(),
            blocks
        );

        let mbr_lba = MBR_HEADER_OFFSET_LBA;
        debug!("Setting up Protective MBR at LBA {}", mbr_lba);

        let primary_gpt_lba: usize = mbr_lba + MBR_SIZE_LBA;
        debug!("Primary GPT Header is located at LBA {primary_gpt_lba}");

        let primary_gpt_parts_lba = primary_gpt_lba + GPT_HEADER_SIZE_LBA;
        debug!("Primary GPT Partition table is located at LBA {primary_gpt_parts_lba}");

        debug!("GPT Partition Table Size: {GPT_PARTITION_HEADER_SIZE_LBA} LBAs");

        let first_usable_lba_unaligned = primary_gpt_parts_lba + GPT_PARTITION_HEADER_SIZE_LBA;
        debug!("First Usable LBA (Unaligned): {first_usable_lba_unaligned}");

        let gpt_partition_alignment_lba = GPT_PARTITION_ALIGNMENT / BLOCK_SIZE;

        let first_usable_lba = round_up(first_usable_lba_unaligned, gpt_partition_alignment_lba);
        debug!("First Usable LBA: {first_usable_lba}");

        if first_usable_lba >= blocks {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "File is too small",
            ));
        }

        let backup_gpt_lba = blocks - GPT_HEADER_SIZE_LBA;
        debug!("Backup GPT Header is located at LBA {backup_gpt_lba}");

        let backup_gpt_parts_lba = backup_gpt_lba - GPT_PARTITION_HEADER_SIZE_LBA;
        debug!("Backup GPT Partition table is located at LBA {backup_gpt_parts_lba}");

        let last_usable_lba = round_down(backup_gpt_parts_lba - 1, gpt_partition_alignment_lba);
        debug!("Last Usable LBA: {last_usable_lba}");

        if first_usable_lba + gpt_partition_alignment_lba > last_usable_lba {
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
                    let size_lba = num_cast!(usize, size) / BLOCK_SIZE;
                    let aligned_size_lba = round_up(size_lba, gpt_partition_alignment_lba);

                    debug!("Partition {idx}: Aligned Size {aligned_size_lba} LBAs");

                    if aligned_size_lba > available_blocks {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "No space left on the device",
                        ));
                    }

                    available_blocks -= aligned_size_lba;
                    debug!("Available LBAs {available_blocks}");

                    Some(aligned_size_lba)
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
            .collect::<Result<Vec<Option<usize>>, io::Error>>()?;

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

                (offset, next_lba - 1)
            })
            .collect::<Vec<(usize, usize)>>();

        Ok(GuidPartitionTableLayout {
            block_size: BLOCK_SIZE,
            mbr_header_lba: mbr_lba,
            primary_gpt_header_lba: primary_gpt_lba,
            primary_gpt_table_lba: primary_gpt_parts_lba,
            first_usable: first_usable_lba,
            partitions_offset: parts,
            last_usable: last_usable_lba,
            backup_gpt_table_lba: backup_gpt_parts_lba,
            backup_gpt_header_lba: backup_gpt_lba,
        })
    }

    /// Writes a GPT to a file
    ///
    /// # Errors
    ///
    /// This function will return an [`std::io::Error`] if there's an issue with the Partition Table
    /// layout, or when accessing the underlying [`File`].
    ///
    /// # Panics
    ///
    /// Panics if we have an integer overflow in one of the integer type conversions
    #[allow(clippy::too_many_lines, clippy::unwrap_in_result)]
    pub fn write(self, mut file: &File) -> Result<(), io::Error> {
        let cfg = self.build_gpt_layout(file)?;

        let mut primary_gpt = [0u8; 92];
        primary_gpt[0..8].copy_from_slice(&GPT_SIGNATURE_HEADER.to_le_bytes());
        primary_gpt[8..12].copy_from_slice(&GPT_VERSION_HEADER.to_le_bytes());

        let len = num_cast!(u32, primary_gpt.len());
        debug!("Header Len is {len}");

        primary_gpt[12..16].copy_from_slice(&len.to_le_bytes());
        primary_gpt[16..20].copy_from_slice(&[0, 0, 0, 0]);
        primary_gpt[20..24].copy_from_slice(&[0, 0, 0, 0]);
        primary_gpt[24..32].copy_from_slice(&cfg.primary_gpt_header_lba.to_le_bytes());
        primary_gpt[32..40].copy_from_slice(&cfg.backup_gpt_header_lba.to_le_bytes());
        primary_gpt[40..48].copy_from_slice(&cfg.first_usable.to_le_bytes());
        primary_gpt[48..56].copy_from_slice(&cfg.last_usable.to_le_bytes());
        primary_gpt[56..72].copy_from_slice(&guid_bytes(&self.builder.guid));

        let first_part_entry_lba = 2u64;
        primary_gpt[72..80].copy_from_slice(&first_part_entry_lba.to_le_bytes());

        let num_parts = num_cast!(u32, GPT_PARTITION_NUM);
        primary_gpt[80..84].copy_from_slice(&num_parts.to_le_bytes());

        let part_entry_size = num_cast!(u32, GPT_PARTITION_ENTRY_SIZE);
        primary_gpt[84..88].copy_from_slice(&part_entry_size.to_le_bytes());

        let mut parts: Vec<u8> = Vec::new();
        for (part, (first_lba, last_lba)) in
            Iterator::zip(self.builder.partitions.iter(), cfg.partitions_offset.iter())
        {
            let mut entry = [0u8; GPT_PARTITION_ENTRY_SIZE];

            entry[0..16].copy_from_slice(&guid_bytes(&part.builder.type_));
            entry[16..32].copy_from_slice(&guid_bytes(&part.builder.guid));
            entry[32..40].copy_from_slice(&first_lba.to_le_bytes());
            entry[40..48].copy_from_slice(&last_lba.to_le_bytes());

            entry[48..56].copy_from_slice(&part.builder.bits.to_le_bytes());

            if let Some(name) = &part.builder.name {
                let mut start = 56;

                for ch in name.encode_utf16() {
                    entry[start..(start + 2)].copy_from_slice(&ch.to_le_bytes());
                    start += 2;
                }
            }

            parts.extend_from_slice(&entry);
        }

        let gpt_part_entries_size = GPT_PARTITION_NUM * GPT_PARTITION_ENTRY_SIZE;
        parts.resize(gpt_part_entries_size, 0);

        let parts_crc = crc32fast::hash(&parts);
        primary_gpt[88..92].copy_from_slice(&parts_crc.to_le_bytes());

        let mut backup_gpt = primary_gpt;
        backup_gpt[24..32].copy_from_slice(&cfg.backup_gpt_header_lba.to_le_bytes());
        backup_gpt[32..40].copy_from_slice(&cfg.primary_gpt_header_lba.to_le_bytes());
        backup_gpt[72..80].copy_from_slice(&cfg.backup_gpt_table_lba.to_le_bytes());

        let primary_gpt_crc = crc32fast::hash(&primary_gpt);
        primary_gpt[16..20].copy_from_slice(&primary_gpt_crc.to_le_bytes());

        let backup_gpt_crc = crc32fast::hash(&backup_gpt);
        backup_gpt[16..20].copy_from_slice(&backup_gpt_crc.to_le_bytes());

        file.seek(io::SeekFrom::Start(num_cast!(
            u64,
            cfg.mbr_header_lba * cfg.block_size
        )))?;

        MasterBootRecordPartitionTableBuilder::new()
            .add_partition(
                MasterBootRecordPartitionBuilder::new(0xee)
                    .size(num_cast!(
                        usize,
                        start_end_to_size(cfg.primary_gpt_header_lba, cfg.backup_gpt_header_lba)
                            * cfg.block_size
                    ))
                    .build(),
            )
            .build()
            .write(file)?;

        file.seek(io::SeekFrom::Start(num_cast!(
            u64,
            cfg.primary_gpt_header_lba * cfg.block_size
        )))?;
        file.write_all(&primary_gpt)?;

        file.seek(io::SeekFrom::Start(num_cast!(
            u64,
            cfg.primary_gpt_table_lba * cfg.block_size
        )))?;
        file.write_all(&parts)?;

        file.seek(io::SeekFrom::Start(num_cast!(
            u64,
            cfg.backup_gpt_table_lba * cfg.block_size
        )))?;
        file.write_all(&parts)?;

        file.seek(io::SeekFrom::Start(num_cast!(
            u64,
            cfg.backup_gpt_header_lba * cfg.block_size
        )))?;
        file.write_all(&backup_gpt)?;

        file.flush()?;
        file.sync_data()?;

        Ok(())
    }
}

/// A GUID Partition Table Builder Structure
#[derive(Debug)]
pub struct GuidPartitionTableBuilder {
    guid: Uuid,
    partitions: Vec<GuidPartition>,
}

impl GuidPartitionTableBuilder {
    /// Creates a new GUID Partition Table Builder with the specified [`uuid::Uuid`]
    #[must_use]
    pub fn new_with_uuid(guid: Uuid) -> Self {
        Self {
            guid,
            partitions: Vec::new(),
        }
    }

    /// Create a new GUID Partition Table Builder with a random [`uuid::Uuid`] according to the UUID
    /// v4 specification
    #[must_use]
    pub fn new() -> Self {
        Self::new_with_uuid(Uuid::new_v4())
    }

    /// Adds a [`GuidPartition`] to the Partition Table
    #[must_use]
    pub fn add_partition(mut self, part: GuidPartition) -> Self {
        self.partitions.push(part);
        self
    }

    /// Creates a [`GuidPartitionTable`] from our builder
    #[must_use]
    pub fn build(self) -> GuidPartitionTable {
        GuidPartitionTable { builder: self }
    }
}

impl Default for GuidPartitionTableBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A GUID Partition
#[derive(Debug)]
pub struct GuidPartition {
    builder: GuidPartitionBuilder,
}

/// A GUID Partition Builder Structure
#[derive(Debug)]
pub struct GuidPartitionBuilder {
    type_: Uuid,
    guid: Uuid,
    name: Option<String>,
    size: Option<usize>,
    bits: u64,
}

impl GuidPartitionBuilder {
    /// Creates a new GUID Partition Builder of a specified [`uuid::Uuid`] type and [`uuid::Uuid`]
    /// GUID
    #[must_use]
    pub fn new_with_uuid(part_type: Uuid, part_guid: Uuid) -> Self {
        Self {
            type_: part_type,
            guid: part_guid,
            name: None,
            size: None,
            bits: 0,
        }
    }

    /// Creates a new GUID Partition Builder of a specified [`uuid::Uuid`] type and a random GUID
    /// according to the UUID v4 specification
    #[must_use]
    pub fn new(part_type: Uuid) -> Self {
        Self::new_with_uuid(part_type, Uuid::new_v4())
    }

    /// Sets the partition size in bytes. Whenever building the GPT, this size might be increased to
    /// be aligned to provide optimal device settings, but will never be decreased.
    ///
    /// If the size isn't provided, the partition will be made to fill any available space. Only one
    /// size-less partition is allowed to be part of a [`GuidPartitionTable`].
    #[must_use]
    pub fn size(mut self, size: usize) -> Self {
        self.size = Some(size);
        self
    }

    /// Sets the partition name
    #[must_use]
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_owned());
        self
    }

    /// Marks the partition as required for the platform to function. See Table 5.8 of the UEFI
    /// Specification for further explanations.
    #[must_use]
    pub fn platform_required(mut self, val: bool) -> Self {
        self.bits.set_bit(0, val);
        self
    }

    /// Marks the partition as ignored by the EFI during partition discovery. See Table 5.8 of the
    /// UEFI Specification for further explanations.
    #[must_use]
    pub fn efi_ignore(mut self, val: bool) -> Self {
        self.bits.set_bit(1, val);
        self
    }

    /// Marks the partition as bootable for Legacy BIOS implementations. See Table 5.8 of the UEFI
    /// Specification for further explanations.
    #[must_use]
    pub fn bootable(mut self, val: bool) -> Self {
        self.bits.set_bit(2, val);
        self
    }

    /// Creates a [`GuidPartition`] from our builder
    #[must_use]
    pub fn build(self) -> GuidPartition {
        GuidPartition { builder: self }
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, process::Command};

    use log::trace;
    use part::{num_cast, start_end_to_size};
    use serde::Deserialize;
    use tempfile::NamedTempFile;
    use test_log::test;
    use uuid::Uuid;

    use crate::{
        round_down, round_up, GuidPartitionBuilder, GuidPartitionTableBuilder, BLOCK_SIZE,
        EFI_SYSTEM_PART_GUID, EXTENDED_BOOTLOADER_PART_GUID, GPT_HEADER_SIZE_LBA,
        GPT_PARTITION_ALIGNMENT, GPT_PARTITION_HEADER_SIZE_LBA, MBR_SIZE_LBA,
    };

    const TEMP_FILE_SIZE: u64 = 2 << 30;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SfDiskGptPartition {
        #[serde(rename = "node")]
        _node: PathBuf,
        start: usize,
        size: usize,
        #[serde(rename = "type")]
        kind: Uuid,
        uuid: Uuid,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SfDiskGptPartitionTable {
        id: Uuid,
        #[serde(rename = "device")]
        _device: PathBuf,
        #[serde(rename = "unit")]
        _unit: String,
        #[serde(rename = "firstlba")]
        first_lba: usize,
        #[serde(rename = "lastlba")]
        last_lba: usize,
        #[serde(rename = "sectorsize")]
        sector_size: usize,
        #[serde(default)]
        partitions: Vec<SfDiskGptPartition>,
    }

    #[derive(Deserialize)]
    #[serde(tag = "label", rename_all = "lowercase")]
    enum SfDiskPartitionTable {
        Dos,
        Gpt(SfDiskGptPartitionTable),
    }

    #[derive(Deserialize)]
    struct SfdiskOutput {
        #[serde(rename = "partitiontable")]
        table: SfDiskPartitionTable,
    }

    fn first_lba() -> usize {
        round_up(
            MBR_SIZE_LBA + GPT_HEADER_SIZE_LBA + GPT_PARTITION_HEADER_SIZE_LBA,
            GPT_PARTITION_ALIGNMENT / BLOCK_SIZE,
        )
    }

    #[test]
    fn test_table_no_partition() {
        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(TEMP_FILE_SIZE).unwrap();

        GuidPartitionTableBuilder::new()
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

        let gpt = match res.table {
            SfDiskPartitionTable::Gpt(v) => v,
            _ => panic!(),
        };

        assert_eq!(gpt.first_lba, first_lba());

        let last_lba = round_down(
            (num_cast!(usize, TEMP_FILE_SIZE) / BLOCK_SIZE)
                - GPT_PARTITION_HEADER_SIZE_LBA
                - GPT_HEADER_SIZE_LBA,
            GPT_PARTITION_ALIGNMENT / BLOCK_SIZE,
        );
        assert_eq!(gpt.last_lba, last_lba);
        assert_eq!(gpt.partitions.len(), 0);
    }

    #[test]
    fn test_table_with_uuid() {
        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(TEMP_FILE_SIZE).unwrap();

        let uuid = Uuid::new_v4();
        GuidPartitionTableBuilder::new_with_uuid(uuid)
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

        let gpt = match res.table {
            SfDiskPartitionTable::Gpt(v) => v,
            _ => panic!(),
        };

        assert_eq!(gpt.id, uuid);
    }

    #[test]
    fn test_file_too_small() {
        let temp_file = NamedTempFile::new().unwrap();

        // The GPT overhead is 1 LBA for the protective MBR, 1 LBA for the Primary Header, 32 LBAs
        // for the Primary Partition Table, 32 LBAs for the Backup Partition Table, and 1 LBA for
        // the Backup Header. The total overhead is thus 67 LBAs.
        temp_file
            .as_file()
            .set_len(num_cast!(
                u64,
                MBR_SIZE_LBA + 2 * GPT_HEADER_SIZE_LBA + 2 * GPT_PARTITION_HEADER_SIZE_LBA - 1
            ))
            .unwrap();

        GuidPartitionTableBuilder::new()
            .build()
            .write(temp_file.as_file())
            .unwrap_err();
    }

    #[test]
    fn test_one_partition_no_size() {
        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(TEMP_FILE_SIZE).unwrap();

        GuidPartitionTableBuilder::new()
            .add_partition(GuidPartitionBuilder::new(EFI_SYSTEM_PART_GUID).build())
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

        let gpt = match res.table {
            SfDiskPartitionTable::Gpt(v) => v,
            _ => panic!(),
        };

        assert_eq!(gpt.sector_size, BLOCK_SIZE);

        let first_lba = first_lba();
        assert_eq!(gpt.first_lba, first_lba);

        let last_lba = round_down(
            (num_cast!(usize, TEMP_FILE_SIZE) / BLOCK_SIZE)
                - GPT_PARTITION_HEADER_SIZE_LBA
                - GPT_HEADER_SIZE_LBA,
            GPT_PARTITION_ALIGNMENT / BLOCK_SIZE,
        );
        assert_eq!(gpt.last_lba, last_lba);
        assert_eq!(gpt.partitions.len(), 1);

        let part = &gpt.partitions[0];
        assert_eq!(part.kind, EFI_SYSTEM_PART_GUID);
        assert_eq!(part.start, gpt.first_lba);
        assert_eq!(part.size, start_end_to_size(first_lba, last_lba));
    }

    #[test]
    fn test_partition_with_uuid() {
        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(TEMP_FILE_SIZE).unwrap();

        let uuid = Uuid::new_v4();
        GuidPartitionTableBuilder::new()
            .add_partition(GuidPartitionBuilder::new_with_uuid(EFI_SYSTEM_PART_GUID, uuid).build())
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

        let gpt = match res.table {
            SfDiskPartitionTable::Gpt(v) => v,
            _ => panic!(),
        };

        assert_eq!(gpt.partitions.len(), 1);
        let part = &gpt.partitions[0];
        assert_eq!(part.uuid, uuid);
    }

    #[test]
    fn test_one_partition_exact_aligned_size() {
        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(TEMP_FILE_SIZE).unwrap();

        let first_lba = first_lba();
        let last_lba = round_down(
            (num_cast!(usize, TEMP_FILE_SIZE) / BLOCK_SIZE)
                - GPT_PARTITION_HEADER_SIZE_LBA
                - GPT_HEADER_SIZE_LBA,
            GPT_PARTITION_ALIGNMENT / BLOCK_SIZE,
        );

        GuidPartitionTableBuilder::new()
            .add_partition(
                GuidPartitionBuilder::new(EFI_SYSTEM_PART_GUID)
                    .size((last_lba - first_lba) * BLOCK_SIZE)
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

        let gpt = match res.table {
            SfDiskPartitionTable::Gpt(v) => v,
            _ => panic!(),
        };

        assert_eq!(gpt.sector_size, BLOCK_SIZE);
        assert_eq!(gpt.first_lba, first_lba);
        assert_eq!(gpt.last_lba, last_lba);
        assert_eq!(gpt.partitions.len(), 1);

        let part = &gpt.partitions[0];
        assert_eq!(part.kind, EFI_SYSTEM_PART_GUID);
        assert_eq!(part.start, gpt.first_lba);
        assert_eq!(part.size, gpt.last_lba - gpt.first_lba);
    }

    #[test]
    fn test_two_partitions_exact_aligned_size() {
        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(TEMP_FILE_SIZE).unwrap();

        let first_lba = first_lba();
        let last_lba = round_down(
            (num_cast!(usize, TEMP_FILE_SIZE) / BLOCK_SIZE)
                - GPT_PARTITION_HEADER_SIZE_LBA
                - GPT_HEADER_SIZE_LBA,
            GPT_PARTITION_ALIGNMENT / BLOCK_SIZE,
        );

        let cutoff_lba = round_down(
            (last_lba - first_lba) / 2,
            GPT_PARTITION_ALIGNMENT / BLOCK_SIZE,
        );

        GuidPartitionTableBuilder::new()
            .add_partition(
                GuidPartitionBuilder::new(EFI_SYSTEM_PART_GUID)
                    .size(((cutoff_lba - 1) - first_lba) * BLOCK_SIZE)
                    .build(),
            )
            .add_partition(
                GuidPartitionBuilder::new(EXTENDED_BOOTLOADER_PART_GUID)
                    .size((last_lba - cutoff_lba) * BLOCK_SIZE)
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

        let gpt = match res.table {
            SfDiskPartitionTable::Gpt(v) => v,
            _ => panic!(),
        };

        assert_eq!(gpt.sector_size, BLOCK_SIZE);
        assert_eq!(gpt.first_lba, first_lba);
        assert_eq!(gpt.last_lba, last_lba);
        assert_eq!(gpt.partitions.len(), 2);

        let part = &gpt.partitions[0];
        assert_eq!(part.kind, EFI_SYSTEM_PART_GUID);
        assert_eq!(part.start, gpt.first_lba);
        assert_eq!(part.size, cutoff_lba - gpt.first_lba);

        let part = &gpt.partitions[1];
        assert_eq!(part.kind, EXTENDED_BOOTLOADER_PART_GUID);
        assert_eq!(part.start, cutoff_lba);
        assert_eq!(part.size, gpt.last_lba - cutoff_lba);
    }

    #[test]
    fn test_multiple_partitions_no_size() {
        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().set_len(TEMP_FILE_SIZE).unwrap();

        GuidPartitionTableBuilder::new()
            .add_partition(GuidPartitionBuilder::new(EFI_SYSTEM_PART_GUID).build())
            .add_partition(GuidPartitionBuilder::new(EXTENDED_BOOTLOADER_PART_GUID).build())
            .build()
            .write(temp_file.as_file())
            .unwrap_err();
    }
}
