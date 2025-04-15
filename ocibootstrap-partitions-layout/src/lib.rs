#![allow(missing_docs)]

use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::JsonSchema;

use serde::Deserialize;
use serde_json as _;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct FatParameters {
    #[serde(rename = "volume-id")]
    pub volume_id: Option<u32>,
    pub heads: Option<u32>,

    #[serde(rename = "sectors-per-track")]
    pub sectors_per_track: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ExtParameters {
    pub uuid: Option<Uuid>,
}

#[derive(Clone, Debug, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct LvmVolume {
    pub name: Option<String>,
    pub fs: Filesystem,
}

#[derive(Clone, Debug, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct LvmParameters {
    pub name: Option<String>,
    pub volumes: Vec<LvmVolume>,
}

#[derive(Clone, Debug, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RawParameters {
    pub content: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(tag = "type")]
#[serde(deny_unknown_fields)]
pub enum Filesystem {
    #[serde(rename = "fat")]
    Fat32(FatParameters),

    #[serde(rename = "ext4")]
    Ext4(ExtParameters),

    #[serde(rename = "lvm")]
    Lvm(LvmParameters),

    #[serde(rename = "raw")]
    Raw(RawParameters),

    #[serde(rename = "swap")]
    Swap,

    #[serde(rename = "xfs")]
    Xfs,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GptPartition {
    pub uuid: Uuid,
    pub name: Option<String>,
    pub mnt: Option<PathBuf>,
    pub offset_lba: Option<usize>,
    pub size_bytes: Option<usize>,
    pub fs: Filesystem,

    #[serde(default)]
    pub attributes: Vec<usize>,

    #[serde(default)]
    pub bootable: bool,

    #[serde(rename = "platform-required", default)]
    pub platform_required: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GptPartitionTable {
    pub partitions: Vec<GptPartition>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MbrPartition {
    #[serde(rename = "type")]
    pub kind: u8,
    pub mnt: Option<PathBuf>,
    pub offset_lba: Option<usize>,
    pub size_bytes: Option<usize>,
    pub fs: Filesystem,

    #[serde(default)]
    pub bootable: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MbrPartitionTable {
    pub partitions: Vec<MbrPartition>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(tag = "type")]
#[serde(deny_unknown_fields)]
pub enum PartitionTable {
    #[serde(rename = "gpt")]
    Gpt(GptPartitionTable),

    #[serde(rename = "mbr")]
    Mbr(MbrPartitionTable),
}
