#![allow(missing_docs)]

use std::path::PathBuf;

use ocibootstrap_partitions_layout::{Filesystem, PartitionTable};
use serde as _;
use uuid::uuid;

#[test]
fn test_gpt_empty() {
    let json = r#"
        {
            "type": "gpt",
            "partitions": []
        }
        "#;

    let table: PartitionTable = serde_json::from_str(&json).unwrap();
    let gpt = match table {
        PartitionTable::Gpt(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(gpt.partitions.len(), 0);
}

#[test]
fn test_gpt_one_partition_raw() {
    let json = r#"
        {
            "type": "gpt",
            "partitions": [
                {
                    "uuid": "71ba0911-0b09-4390-9031-5c537cedf0fe",
                    "fs": {
                        "type": "raw",
                        "content": "/test.bin"
                    }
                }
            ]
        }
        "#;

    let table: PartitionTable = serde_json::from_str(&json).unwrap();
    let gpt = match table {
        PartitionTable::Gpt(v) => v,
        _ => unreachable!(),
    };

    assert_eq!(gpt.partitions.len(), 1);

    let part = &gpt.partitions[0];
    assert_eq!(part.uuid, uuid!("71ba0911-0b09-4390-9031-5c537cedf0fe"));
    assert_eq!(part.name, None);
    assert_eq!(part.mnt, None);
    assert_eq!(part.offset_lba, None);
    assert_eq!(part.size_bytes, None);
    assert_eq!(part.attributes, Vec::<usize>::new());
    assert_eq!(part.bootable, false);
    assert_eq!(part.platform_required, false);

    let fs = match &part.fs {
        Filesystem::Raw(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.content, PathBuf::from("/test.bin"));
}

#[test]
fn test_gpt_one_partition_fat() {
    let json = r#"
        {
            "type": "gpt",
            "partitions": [
                {
                    "uuid": "71ba0911-0b09-4390-9031-5c537cedf0fe",
                    "fs": {
                        "type": "fat",
                        "volume-id": 84
                    }
                }
            ]
        }
        "#;

    let table: PartitionTable = serde_json::from_str(&json).unwrap();
    let gpt = match table {
        PartitionTable::Gpt(v) => v,
        _ => unreachable!(),
    };

    assert_eq!(gpt.partitions.len(), 1);

    let part = &gpt.partitions[0];
    assert_eq!(part.uuid, uuid!("71ba0911-0b09-4390-9031-5c537cedf0fe"));
    assert_eq!(part.name, None);
    assert_eq!(part.mnt, None);
    assert_eq!(part.offset_lba, None);
    assert_eq!(part.size_bytes, None);
    assert_eq!(part.attributes, Vec::<usize>::new());
    assert_eq!(part.bootable, false);
    assert_eq!(part.platform_required, false);

    let fs = match &part.fs {
        Filesystem::Fat32(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.volume_id, Some(84));
    assert_eq!(fs.heads, None);
    assert_eq!(fs.sectors_per_track, None);
}

#[test]
fn test_gpt_one_partition_ext4() {
    let json = r#"
        {
            "type": "gpt",
            "partitions": [
                {
                    "uuid": "71ba0911-0b09-4390-9031-5c537cedf0fe",
                    "fs": {
                        "type": "ext4"
                    }
                }
            ]
        }
        "#;

    let table: PartitionTable = serde_json::from_str(&json).unwrap();
    let gpt = match table {
        PartitionTable::Gpt(v) => v,
        _ => unreachable!(),
    };

    assert_eq!(gpt.partitions.len(), 1);

    let part = &gpt.partitions[0];
    assert_eq!(part.uuid, uuid!("71ba0911-0b09-4390-9031-5c537cedf0fe"));
    assert_eq!(part.name, None);
    assert_eq!(part.mnt, None);
    assert_eq!(part.offset_lba, None);
    assert_eq!(part.size_bytes, None);
    assert_eq!(part.attributes, Vec::<usize>::new());
    assert_eq!(part.bootable, false);
    assert_eq!(part.platform_required, false);

    let fs = match &part.fs {
        Filesystem::Ext4(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.uuid, None);
}

#[test]
fn test_gpt_full() {
    let json = r#"
        {
            "type": "gpt",
            "partitions": [
                {
                    "uuid": "71ba0911-0b09-4390-9031-5c537cedf0fe",
                    "name": "bootloader",
                    "offset_lba": 2,
                    "size_bytes": 1024,
                    "attributes": [63],
                    "platform-required": true,
                    "fs": {
                            "type": "raw",
                            "content": "/test.bin"
                    }
                },
                {
                    "uuid": "71ba0911-0b09-4390-9031-5c537cedf0fe",
                    "name": "EFI System Partition",
                    "mnt": "/efi",
                    "size_bytes": 2048,
                    "bootable": true,
                    "platform-required": true,
                    "fs": {
                        "type": "fat",
                        "volume-id": 168,
                        "heads": 32,
                        "sectors-per-track": 64
                    }
                },
                {
                    "uuid": "71ba0911-0b09-4390-9031-5c537cedf0fe",
                    "attributes": [59],
                    "name": "root",
                    "mnt": "/",
                    "fs": {
                        "type": "ext4",
                        "uuid": "e77a46a5-2a18-4f0a-b72a-9d143f2020f3"
                    }
                }
            ]
        }
        "#;

    let table: PartitionTable = serde_json::from_str(&json).unwrap();
    let gpt = match table {
        PartitionTable::Gpt(v) => v,
        _ => unreachable!(),
    };

    assert_eq!(gpt.partitions.len(), 3);

    let part = &gpt.partitions[0];
    assert_eq!(part.uuid, uuid!("71ba0911-0b09-4390-9031-5c537cedf0fe"));
    assert_eq!(part.name, Some(String::from("bootloader")));
    assert_eq!(part.mnt, None);
    assert_eq!(part.offset_lba, Some(2));
    assert_eq!(part.size_bytes, Some(1024));
    assert_eq!(part.attributes, vec![63]);
    assert_eq!(part.bootable, false);
    assert_eq!(part.platform_required, true);

    let fs = match &part.fs {
        Filesystem::Raw(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.content, PathBuf::from("/test.bin"));

    let part = &gpt.partitions[1];
    assert_eq!(part.uuid, uuid!("71ba0911-0b09-4390-9031-5c537cedf0fe"));
    assert_eq!(part.name, Some(String::from("EFI System Partition")));
    assert_eq!(part.mnt, Some(PathBuf::from("/efi")));
    assert_eq!(part.offset_lba, None);
    assert_eq!(part.size_bytes, Some(2048));
    assert_eq!(part.attributes, Vec::<usize>::new());
    assert_eq!(part.bootable, true);
    assert_eq!(part.platform_required, true);

    let fs = match &part.fs {
        Filesystem::Fat32(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.volume_id, Some(168));
    assert_eq!(fs.heads, Some(32));
    assert_eq!(fs.sectors_per_track, Some(64));

    let part = &gpt.partitions[2];
    assert_eq!(part.uuid, uuid!("71ba0911-0b09-4390-9031-5c537cedf0fe"));
    assert_eq!(part.name, Some(String::from("root")));
    assert_eq!(part.mnt, Some(PathBuf::from("/")));
    assert_eq!(part.offset_lba, None);
    assert_eq!(part.size_bytes, None);
    assert_eq!(part.attributes, vec![59]);
    assert_eq!(part.bootable, false);
    assert_eq!(part.platform_required, false);

    let fs = match &part.fs {
        Filesystem::Ext4(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.uuid, Some(uuid!("e77a46a5-2a18-4f0a-b72a-9d143f2020f3")));
}
