use log::debug;
use serde::{de, Deserialize};
use serde_json::Value;

use crate::{CompressionAlgorithm, SCHEMA_VERSION_KEY};

pub(crate) mod docker;
pub(crate) mod oci;

pub(crate) const MIME_TYPE_KEY: &str = "mediaType";

pub(crate) const SCHEMA_VERSION: u64 = 2;

#[derive(Clone, Debug)]
pub(crate) enum ImageLayer {
    DockerImage(docker::ImageLayer),
    OciImage(oci::ImageLayer),
}

impl ImageLayer {
    pub(crate) fn compression(&self) -> CompressionAlgorithm {
        match self {
            ImageLayer::DockerImage(i) => i.compression,
            ImageLayer::OciImage(i) => i.compression,
        }
    }
}

impl<'de> Deserialize<'de> for ImageLayer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let map = value
            .as_object()
            .ok_or(de::Error::invalid_type(de::Unexpected::Seq, &"a map"))?;

        let media_kind = map
            .get(MIME_TYPE_KEY)
            .ok_or(de::Error::missing_field(MIME_TYPE_KEY))?
            .as_str()
            .ok_or(de::Error::invalid_type(
                de::Unexpected::Other("something other than a string"),
                &"a string",
            ))?
            .to_owned();

        debug!("Found Layer with the Media Type {}", media_kind);

        Ok(match media_kind.as_str() {
            "application/vnd.docker.image.rootfs.diff.tar.gzip" => Self::DockerImage(
                docker::ImageLayer::deserialize(value).map_err(de::Error::custom)?,
            ),
            "application/vnd.oci.image.layer.v1.tar+gzip" => {
                Self::OciImage(oci::ImageLayer::deserialize(value).map_err(de::Error::custom)?)
            }
            _ => unimplemented!(),
        })
    }
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug)]
pub(crate) enum Manifest {
    Docker(docker::DistributionManifest),
    OciIndex(oci::ImageIndex),
    OciManifest(oci::ImageManifest),
}

impl<'de> Deserialize<'de> for Manifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let map = value
            .as_object()
            .ok_or(de::Error::invalid_type(de::Unexpected::Seq, &"a map"))?;

        let schema = map
            .get(SCHEMA_VERSION_KEY)
            .ok_or(de::Error::missing_field(SCHEMA_VERSION_KEY))?
            .as_u64()
            .ok_or(de::Error::invalid_type(
                de::Unexpected::Other("something other than an integer"),
                &"an integer",
            ))?;

        if schema != SCHEMA_VERSION {
            return Err(de::Error::invalid_value(
                de::Unexpected::Unsigned(schema),
                &SCHEMA_VERSION.to_string().as_str(),
            ));
        }

        let media_kind = map
            .get(MIME_TYPE_KEY)
            .ok_or(de::Error::missing_field(MIME_TYPE_KEY))?
            .as_str()
            .ok_or(de::Error::invalid_type(
                de::Unexpected::Other("something other than a string"),
                &"a string",
            ))?
            .to_owned();

        debug!("{:#?}", media_kind);

        Ok(match media_kind.as_str() {
            docker::DISTRIBUTION_MANIFEST_MIME_TYPE => Self::Docker(
                docker::DistributionManifest::deserialize(value).map_err(de::Error::custom)?,
            ),
            oci::IMAGE_INDEX_MIME_TYPE => {
                Self::OciIndex(oci::ImageIndex::deserialize(value).map_err(de::Error::custom)?)
            }
            _ => unimplemented!(),
        })
    }
}
