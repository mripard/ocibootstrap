#![allow(missing_docs)]

use log as _;
use num_traits as _;
use test_log::test;

#[test]
fn build_layout_no_partition() {
    assert_eq!(ocibootstrap_part::build_layout(0, 1000, &[]).unwrap(), &[]);
}

#[test]
fn build_layout_one_partition_no_size() {
    assert_eq!(
        ocibootstrap_part::build_layout(
            0,
            1000,
            &[ocibootstrap_part::PartitionLayoutHint {
                offset_lba: None,
                size_lba: None
            }]
        )
        .unwrap(),
        &[ocibootstrap_part::PartitionLayout {
            start_lba: 0,
            end_lba: 1000,
        }]
    );
}

#[test]
fn build_layout_one_partition_no_size_offset() {
    assert_eq!(
        ocibootstrap_part::build_layout(
            0,
            1000,
            &[ocibootstrap_part::PartitionLayoutHint {
                offset_lba: Some(10),
                size_lba: None
            }]
        )
        .unwrap(),
        &[ocibootstrap_part::PartitionLayout {
            start_lba: 10,
            end_lba: 1000,
        }]
    );
}

#[test]
fn build_layout_one_partition_exact_size() {
    assert_eq!(
        ocibootstrap_part::build_layout(
            0,
            1000,
            &[ocibootstrap_part::PartitionLayoutHint {
                offset_lba: None,
                size_lba: Some(1001)
            }]
        )
        .unwrap(),
        &[ocibootstrap_part::PartitionLayout {
            start_lba: 0,
            end_lba: 1000,
        }]
    );
}

#[test]
fn build_layout_one_partition_exact_size_offset() {
    assert_eq!(
        ocibootstrap_part::build_layout(
            0,
            1000,
            &[ocibootstrap_part::PartitionLayoutHint {
                offset_lba: Some(10),
                size_lba: Some(991)
            }]
        )
        .unwrap(),
        &[ocibootstrap_part::PartitionLayout {
            start_lba: 10,
            end_lba: 1000,
        }]
    );
}

#[test]
fn build_layout_two_partitions_one_size_missing() {
    assert_eq!(
        ocibootstrap_part::build_layout(
            0,
            1000,
            &[
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: None,
                    size_lba: Some(500),
                },
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: None,
                    size_lba: None
                }
            ]
        )
        .unwrap(),
        &[
            ocibootstrap_part::PartitionLayout {
                start_lba: 0,
                end_lba: 499,
            },
            ocibootstrap_part::PartitionLayout {
                start_lba: 500,
                end_lba: 1000,
            }
        ]
    );
}

#[test]
fn build_layout_two_partitions_exact_size() {
    assert_eq!(
        ocibootstrap_part::build_layout(
            0,
            1000,
            &[
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: None,
                    size_lba: Some(500),
                },
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: None,
                    size_lba: Some(501)
                }
            ]
        )
        .unwrap(),
        &[
            ocibootstrap_part::PartitionLayout {
                start_lba: 0,
                end_lba: 499,
            },
            ocibootstrap_part::PartitionLayout {
                start_lba: 500,
                end_lba: 1000,
            }
        ]
    );
}

#[test]
fn build_layout_two_partitions_exact_size_offset() {
    assert_eq!(
        ocibootstrap_part::build_layout(
            0,
            1000,
            &[
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: Some(0),
                    size_lba: Some(500),
                },
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: Some(500),
                    size_lba: Some(501)
                }
            ]
        )
        .unwrap(),
        &[
            ocibootstrap_part::PartitionLayout {
                start_lba: 0,
                end_lba: 499,
            },
            ocibootstrap_part::PartitionLayout {
                start_lba: 500,
                end_lba: 1000,
            }
        ]
    );
}

#[test]
fn build_layout_two_partitions_exact_size_offset_gap() {
    assert_eq!(
        ocibootstrap_part::build_layout(
            0,
            1000,
            &[
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: Some(0),
                    size_lba: Some(500),
                },
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: Some(510),
                    size_lba: Some(491)
                }
            ]
        )
        .unwrap(),
        &[
            ocibootstrap_part::PartitionLayout {
                start_lba: 0,
                end_lba: 499,
            },
            ocibootstrap_part::PartitionLayout {
                start_lba: 510,
                end_lba: 1000,
            }
        ]
    );
}

#[test]
fn build_layout_two_partitions_one_missing_size_offset() {
    assert_eq!(
        ocibootstrap_part::build_layout(
            0,
            1000,
            &[
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: None,
                    size_lba: None,
                },
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: Some(500),
                    size_lba: Some(400)
                }
            ]
        )
        .unwrap(),
        &[
            ocibootstrap_part::PartitionLayout {
                start_lba: 0,
                end_lba: 499,
            },
            ocibootstrap_part::PartitionLayout {
                start_lba: 500,
                end_lba: 899,
            }
        ]
    );
}

#[test]
fn build_layout_two_partitions_no_size() {
    ocibootstrap_part::build_layout(
        0,
        1000,
        &[
            ocibootstrap_part::PartitionLayoutHint {
                offset_lba: None,
                size_lba: None,
            },
            ocibootstrap_part::PartitionLayoutHint {
                offset_lba: None,
                size_lba: None,
            },
        ],
    )
    .unwrap_err();
}

#[test]
fn build_layout_two_partitions_offset_too_small() {
    ocibootstrap_part::build_layout(
        0,
        1000,
        &[
            ocibootstrap_part::PartitionLayoutHint {
                offset_lba: None,
                size_lba: Some(500),
            },
            ocibootstrap_part::PartitionLayoutHint {
                offset_lba: Some(499),
                size_lba: None,
            },
        ],
    )
    .unwrap_err();
}

#[test]
fn build_layout_three_partitions_one_missing_size_middle() {
    assert_eq!(
        ocibootstrap_part::build_layout(
            0,
            1000,
            &[
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: None,
                    size_lba: Some(300),
                },
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: None,
                    size_lba: None,
                },
                ocibootstrap_part::PartitionLayoutHint {
                    offset_lba: None,
                    size_lba: Some(301)
                }
            ]
        )
        .unwrap(),
        &[
            ocibootstrap_part::PartitionLayout {
                start_lba: 0,
                end_lba: 299,
            },
            ocibootstrap_part::PartitionLayout {
                start_lba: 300,
                end_lba: 699,
            },
            ocibootstrap_part::PartitionLayout {
                start_lba: 700,
                end_lba: 1000,
            }
        ]
    );
}
