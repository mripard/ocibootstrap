use core::{fmt, str::FromStr};
use std::{collections::HashMap, path::PathBuf};

use log::debug;
use oci_spec::image::ImageConfiguration;
use types::OciBootstrapError;
use uuid::Uuid;

#[derive(Clone, Copy, Debug)]
pub(crate) struct FatParameters {
    pub(crate) volume_id: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExtParameters {
    pub(crate) uuid: Option<Uuid>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Filesystem {
    Fat32(FatParameters),
    Ext4(ExtParameters),
}

impl Filesystem {
    fn from_labels(
        labels: &HashMap<String, String>,
        part_name: &str,
    ) -> Result<Self, OciBootstrapError> {
        match labels
            .get(&format!(
                "com.github.mripard.ocibootstrap.partition.{part_name}.fs",
            ))
            .ok_or(OciBootstrapError::Custom(format!(
                "Partition {part_name}: Missing Partition File System",
            )))?
            .as_str()
        {
            "ext4" => {
                let uuid = labels
                    .get(&format!(
                        "com.github.mripard.ocibootstrap.partition.{part_name}.ext4.uuid",
                    ))
                    .map(|s| Uuid::from_str(s))
                    .transpose()
                    .map_err(|_err| {
                        OciBootstrapError::Custom(format!(
                            "Partition {part_name}: Invalid UUID Format",
                        ))
                    })?;

                Ok(Filesystem::Ext4(ExtParameters { uuid }))
            }
            "fat" => {
                let vol_id = labels
                    .get(&format!(
                        "com.github.mripard.ocibootstrap.partition.{part_name}.fat.vol_id",
                    ))
                    .map(|s| u32::from_str_radix(s, 16))
                    .transpose()
                    .map_err(|_err| {
                        OciBootstrapError::Custom(format!(
                            "Partition {part_name}: Invalid Id Format",
                        ))
                    })?;

                Ok(Filesystem::Fat32(FatParameters { volume_id: vol_id }))
            }
            _ => unimplemented!(),
        }
    }
}

impl fmt::Display for Filesystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Filesystem::Fat32(_) => f.write_str("fat"),
            Filesystem::Ext4(_) => f.write_str("ext4"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct GptPartition {
    pub(crate) uuid: Uuid,
    pub(crate) name: Option<String>,
    pub(crate) mnt: PathBuf,
    pub(crate) size: Option<u64>,
    pub(crate) fs: Filesystem,
    pub(crate) bootable: bool,
    pub(crate) platform_required: bool,
}

pub(crate) struct GptPartitionTable {
    partitions: Vec<GptPartition>,
}

impl GptPartitionTable {
    pub(crate) fn partitions(&self) -> &[GptPartition] {
        &self.partitions
    }
}

pub(crate) enum PartitionTable {
    Gpt(GptPartitionTable),
}

impl PartitionTable {
    fn gpt_from_config(
        labels: &HashMap<String, String>,
    ) -> Result<GptPartitionTable, OciBootstrapError> {
        let part_names: Vec<String> = serde_json::from_str(
            labels
                .get("com.github.mripard.ocibootstrap.partitions")
                .ok_or(OciBootstrapError::Custom(
                    "Missing partitions list".to_owned(),
                ))?,
        )?;

        debug!("Found {} partitions.", part_names.len());

        let mut partitions = Vec::with_capacity(part_names.len());
        for (idx, part_name) in part_names.iter().enumerate() {
            debug!("Partition {idx}: Name {part_name}");

            let part_uuid = Uuid::from_str(
                labels
                    .get(&format!(
                        "com.github.mripard.ocibootstrap.partition.{part_name}.partition_uuid",
                    ))
                    .ok_or(OciBootstrapError::Custom(format!(
                        "Partition {idx}: Missing Partition UUID",
                    )))?,
            )
            .map_err(|_err| OciBootstrapError::Custom(format!("Partition {idx}: Invalid UUID")))?;

            debug!("Partition {idx}: Partition UUID {part_uuid}");

            let part_mnt = PathBuf::from(
                labels
                    .get(&format!(
                        "com.github.mripard.ocibootstrap.partition.{part_name}.mount_point",
                    ))
                    .ok_or(OciBootstrapError::Custom(format!(
                        "Partition {idx}: Missing Partition Mount Point",
                    )))?,
            );

            debug!("Partition Mount Point {}", part_mnt.display());

            let part_size = labels
                .get(&format!(
                    "com.github.mripard.ocibootstrap.partition.{part_name}.size",
                ))
                .map(|s| {
                    u64::from_str(s).map(|size| size << 20).map_err(|_err| {
                        OciBootstrapError::Custom(format!("Partition {idx}: Invalid bool value"))
                    })
                })
                .transpose()?;

            debug!("Partition Size {:#?}", part_size);

            let part_fs = Filesystem::from_labels(labels, part_name)?;
            debug!("Partition {idx}: Filesystem {part_fs}");

            let part_bootable = if let Some(bootable) = labels.get(&format!(
                "com.github.mripard.ocibootstrap.partition.{part_name}.flags.bootable",
            )) {
                bool::from_str(bootable).map_err(|_err| {
                    OciBootstrapError::Custom(format!("Partition {idx}: Invalid bool value"))
                })?
            } else {
                false
            };

            let part_required = if let Some(bootable) = labels.get(&format!(
                "com.github.mripard.ocibootstrap.partition.{part_name}.flags.required",
            )) {
                bool::from_str(bootable).map_err(|_err| {
                    OciBootstrapError::Custom(format!("Partition {idx}: Invalid bool value"))
                })?
            } else {
                false
            };

            partitions.push(GptPartition {
                uuid: part_uuid,
                name: Some(part_name.clone()),
                mnt: part_mnt,
                size: part_size,
                fs: part_fs,
                bootable: part_bootable,
                platform_required: part_required,
            });
        }

        Ok(GptPartitionTable { partitions })
    }
}

impl TryFrom<&ImageConfiguration> for PartitionTable {
    type Error = OciBootstrapError;

    fn try_from(config: &ImageConfiguration) -> Result<Self, Self::Error> {
        let labels = config.labels_of_config().ok_or(OciBootstrapError::Custom(
            "Container Configuration has no labels.".to_owned(),
        ))?;

        let layout_type = labels
            .get("com.github.mripard.ocibootstrap.partitions_layout")
            .ok_or(OciBootstrapError::Custom(
                "Missing partition layout".to_owned(),
            ))?;

        debug!("Found {layout_type} partition layout type.");

        Ok(match layout_type.as_str() {
            "gpt" => Self::Gpt(PartitionTable::gpt_from_config(labels)?),
            _ => {
                return Err(OciBootstrapError::Custom(format!(
                    "Invalid Layout Type: {layout_type}"
                )))
            }
        })
    }
}
