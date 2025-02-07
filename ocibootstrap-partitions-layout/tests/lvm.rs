#![allow(missing_docs)]

use ocibootstrap_partitions_layout::{Filesystem, PartitionTable};
use serde as _;
use uuid::uuid;

#[test]
fn test_lvm_empty() {
    let json = r#"
        {
            "type": "gpt",
            "partitions": [
                {
                    "uuid": "71ba0911-0b09-4390-9031-5c537cedf0fe",
                    "fs": {
                        "type": "lvm",
                        "volumes": []
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
    assert_eq!(part.bootable, false);
    assert_eq!(part.platform_required, false);

    let fs = match &part.fs {
        Filesystem::Lvm(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.volumes.len(), 0);
}

#[test]
fn test_lvm_nested() {
    let json = r#"
        {
            "type": "gpt",
            "partitions": [
                {
                    "uuid": "71ba0911-0b09-4390-9031-5c537cedf0fe",
                    "fs": {
                        "type": "lvm",
                        "volumes": [
                            {
                                "fs": {
                                    "type": "lvm",
                                    "volumes": []
                                }
                            }
                        ]
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
    assert_eq!(part.bootable, false);
    assert_eq!(part.platform_required, false);

    let fs = match &part.fs {
        Filesystem::Lvm(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.volumes.len(), 1);

    let volume = &fs.volumes[0];
    let fs = match &volume.fs {
        Filesystem::Lvm(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.volumes.len(), 0);
}

#[test]
fn test_lvm_one_part_full() {
    let json = r#"
        {
            "type": "gpt",
            "partitions": [
                {
                    "uuid": "71ba0911-0b09-4390-9031-5c537cedf0fe",
                    "fs": {
                        "type": "lvm",
                        "name": "Group",
                        "volumes": [
                            {
                                "name": "Volume",
                                "fs": {
                                    "type": "ext4"
                                }
                            }
                        ]
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
    assert_eq!(part.bootable, false);
    assert_eq!(part.platform_required, false);

    let fs = match &part.fs {
        Filesystem::Lvm(v) => v,
        _ => unreachable!(),
    };
    assert_eq!(fs.name, Some(String::from("Group")));
    assert_eq!(fs.volumes.len(), 1);

    let volume = &fs.volumes[0];
    assert_eq!(volume.name, Some(String::from("Volume")));
    let _fs = match &volume.fs {
        Filesystem::Ext4(v) => v,
        _ => unreachable!(),
    };
}
