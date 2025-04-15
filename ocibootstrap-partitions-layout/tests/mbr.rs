#![allow(missing_docs)]

use std::path::PathBuf;

use ocibootstrap_partitions_layout::{Filesystem, PartitionTable};
use serde as _;

#[test]
fn test_empty_mbr() {
    let json = r#"
        {
            "type": "mbr",
            "partitions": []
        }
        "#;

    let table: PartitionTable = serde_json::from_str(&json).unwrap();
    let mbr = match table {
        PartitionTable::Mbr(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(mbr.partitions.len(), 0);
}

#[test]
fn test_mbr_one_partition_raw() {
    let json = r#"
        {
            "type": "mbr",
            "partitions": [
                {
                    "type": 42,
                    "fs": {
                        "type": "raw",
                        "content": "/test.bin"
                    }
                }
            ]
        }
        "#;

    let table: PartitionTable = serde_json::from_str(&json).unwrap();
    let mbr = match table {
        PartitionTable::Mbr(v) => v,
        _ => unreachable!(),
    };

    assert_eq!(mbr.partitions.len(), 1);

    let part = &mbr.partitions[0];
    assert_eq!(part.kind, 42);

    let fs = match &part.fs {
        Filesystem::Raw(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.content, PathBuf::from("/test.bin"));
}

#[test]
fn test_mbr_one_partition_fat() {
    let json = r#"
        {
            "type": "mbr",
            "partitions": [
                {
                    "type": 42,
                    "fs": {
                        "type": "fat",
                        "volume-id": 84
                    }
                }
            ]
        }
        "#;

    let table: PartitionTable = serde_json::from_str(&json).unwrap();
    let mbr = match table {
        PartitionTable::Mbr(v) => v,
        _ => unreachable!(),
    };

    assert_eq!(mbr.partitions.len(), 1);

    let part = &mbr.partitions[0];
    assert_eq!(part.kind, 42);

    let fs = match &part.fs {
        Filesystem::Fat32(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.volume_id, Some(84));
    assert_eq!(fs.heads, None);
    assert_eq!(fs.sectors_per_track, None);
}

#[test]
fn test_mbr_one_partition_ext4() {
    let json = r#"
        {
            "type": "mbr",
            "partitions": [
                {
                    "type": 42,
                    "fs": {
                        "type": "ext4"
                    }
                }
            ]
        }
        "#;

    let table: PartitionTable = serde_json::from_str(&json).unwrap();
    let mbr = match table {
        PartitionTable::Mbr(v) => v,
        _ => unreachable!(),
    };

    assert_eq!(mbr.partitions.len(), 1);

    let part = &mbr.partitions[0];
    assert_eq!(part.kind, 42);

    let fs = match &part.fs {
        Filesystem::Ext4(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.uuid, None);
}

#[test]
fn test_mbr_full() {
    let json = r#"
        {
            "type": "mbr",
            "partitions": [
                {
                    "type": 42,
                    "offset_lba": 2,
                    "size_bytes": 1024,
                    "bootable": true,
                    "fs": {
                        "type": "raw",
                        "content": "/test.bin"
                    }
                },
                {
                    "type": 84,
                    "size_bytes": 2048,
                    "mnt": "/efi",
                    "fs": {
                        "type": "fat",
                        "volume-id": 168,
                        "heads": 32,
                        "sectors-per-track": 64
                    }
                },
                {
                    "type": 126,
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
    let mbr = match table {
        PartitionTable::Mbr(v) => v,
        _ => unreachable!(),
    };

    assert_eq!(mbr.partitions.len(), 3);

    let part = &mbr.partitions[0];
    assert_eq!(part.kind, 42);
    assert_eq!(part.bootable, true);
    assert_eq!(part.mnt, None);
    assert_eq!(part.offset_lba, Some(2));
    assert_eq!(part.size_bytes, Some(1024));

    let fs = match &part.fs {
        Filesystem::Raw(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.content, PathBuf::from("/test.bin"));

    let part = &mbr.partitions[1];
    assert_eq!(part.kind, 84);
    assert_eq!(part.bootable, false);
    assert_eq!(part.mnt, Some(PathBuf::from("/efi")));
    assert_eq!(part.offset_lba, None);
    assert_eq!(part.size_bytes, Some(2048));

    let fs = match &part.fs {
        Filesystem::Fat32(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.volume_id, Some(168));
    assert_eq!(fs.heads, Some(32));
    assert_eq!(fs.sectors_per_track, Some(64));

    let part = &mbr.partitions[2];
    assert_eq!(part.kind, 126);
    assert_eq!(part.bootable, false);
    assert_eq!(part.mnt, Some(PathBuf::from("/")));
    assert_eq!(part.offset_lba, None);
    assert_eq!(part.size_bytes, None);

    let fs = match &part.fs {
        Filesystem::Ext4(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(
        fs.uuid,
        Some(uuid::uuid!("e77a46a5-2a18-4f0a-b72a-9d143f2020f3"))
    );
}
