use core::str::FromStr;

use mime::Mime;
use serde::{de, Deserialize};
use serde_json::Value;

use crate::{
    spec::v2::MIME_TYPE_KEY, CompressionAlgorithm, Digest, DIGEST_KEY, SCHEMA_VERSION_KEY, SIZE_KEY,
};

#[derive(Clone, Debug)]
pub(crate) struct ContainerConfig {
    _digest: Digest,
    _size: usize,
}

impl<'de> Deserialize<'de> for ContainerConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let mut value = Value::deserialize(deserializer)?;
        let map = value.as_object_mut().ok_or(de::Error::invalid_type(
            de::Unexpected::Other("something other than a map"),
            &"a map",
        ))?;

        let media_kind = map
            .remove(MIME_TYPE_KEY)
            .ok_or(de::Error::missing_field(MIME_TYPE_KEY))?
            .as_str()
            .ok_or(de::Error::invalid_type(
                de::Unexpected::Other("something other than a string"),
                &"a string",
            ))?
            .to_owned();

        if media_kind != "application/vnd.docker.container.image.v1+json" {
            return Err(de::Error::invalid_value(
                de::Unexpected::Str(&media_kind),
                &"application/vnd.docker.container.image.v1+json",
            ));
        }

        let digest = Digest::deserialize(
            map.remove(DIGEST_KEY)
                .ok_or(de::Error::missing_field(DIGEST_KEY))?,
        )
        .map_err(de::Error::custom)?;

        let size = usize::deserialize(
            map.remove(SIZE_KEY)
                .ok_or(de::Error::missing_field(SIZE_KEY))?,
        )
        .map_err(de::Error::custom)?;

        if let Some(key) = map.keys().next() {
            return Err(de::Error::unknown_field(
                key,
                &[MIME_TYPE_KEY, SIZE_KEY, DIGEST_KEY],
            ));
        }

        Ok(Self {
            _digest: digest,
            _size: size,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ImageLayer {
    pub(crate) size: usize,
    pub(crate) digest: Digest,
    pub(crate) compression: CompressionAlgorithm,
}

impl<'de> Deserialize<'de> for ImageLayer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let mut value = Value::deserialize(deserializer)?;
        let map = value
            .as_object_mut()
            .ok_or(de::Error::invalid_type(de::Unexpected::Seq, &"a map"))?;

        let media_kind = map
            .remove(MIME_TYPE_KEY)
            .ok_or(de::Error::missing_field(MIME_TYPE_KEY))?
            .as_str()
            .ok_or(de::Error::invalid_type(
                de::Unexpected::Other("something other than a string"),
                &"a string",
            ))?
            .to_owned();

        let compression = match media_kind.as_str() {
            "application/vnd.docker.image.rootfs.diff.tar.gzip" => CompressionAlgorithm::Gzip,
            _ => unimplemented!(),
        };

        let digest = Digest::deserialize(
            map.remove(DIGEST_KEY)
                .ok_or(de::Error::missing_field(DIGEST_KEY))?,
        )
        .map_err(de::Error::custom)?;

        let size = usize::deserialize(
            map.remove(SIZE_KEY)
                .ok_or(de::Error::missing_field(SIZE_KEY))?,
        )
        .map_err(de::Error::custom)?;

        if let Some(key) = map.keys().next() {
            return Err(de::Error::unknown_field(
                key,
                &[MIME_TYPE_KEY, SIZE_KEY, DIGEST_KEY, "compression"],
            ));
        }

        Ok(Self {
            size,
            digest,
            compression,
        })
    }
}

pub(crate) const DISTRIBUTION_MANIFEST_MIME_TYPE: &str =
    "application/vnd.docker.distribution.manifest.v2+json";

const MANIFEST_CONFIG_KEY: &str = "config";
const MANIFEST_LAYERS_KEY: &str = "layers";

#[derive(Clone, Debug)]
pub(crate) struct DistributionManifest {
    _config: ContainerConfig,
    pub(crate) layers: Vec<ImageLayer>,
}

impl<'de> Deserialize<'de> for DistributionManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let mut value = Value::deserialize(deserializer)?;
        let map = value
            .as_object_mut()
            .ok_or(de::Error::invalid_type(de::Unexpected::Seq, &"a map"))?;

        let schema = map
            .remove(SCHEMA_VERSION_KEY)
            .ok_or(de::Error::missing_field(SCHEMA_VERSION_KEY))?
            .as_u64()
            .ok_or(de::Error::invalid_type(
                de::Unexpected::Other("something other than an integer"),
                &"an integer",
            ))?;

        if schema != super::SCHEMA_VERSION {
            return Err(de::Error::invalid_value(
                de::Unexpected::Unsigned(schema),
                &super::SCHEMA_VERSION.to_string().as_str(),
            ));
        }

        let media_kind = map
            .remove(MIME_TYPE_KEY)
            .ok_or(de::Error::missing_field(MIME_TYPE_KEY))?
            .as_str()
            .ok_or(de::Error::invalid_type(
                de::Unexpected::Other("something other than a string"),
                &"a string",
            ))?
            .to_owned();

        let mime = Mime::from_str(&media_kind).map_err(de::Error::custom)?;
        if mime.essence_str() != DISTRIBUTION_MANIFEST_MIME_TYPE {
            return Err(de::Error::invalid_value(
                de::Unexpected::Str(&media_kind),
                &DISTRIBUTION_MANIFEST_MIME_TYPE,
            ));
        }

        let config = ContainerConfig::deserialize(
            map.remove(MANIFEST_CONFIG_KEY)
                .ok_or(de::Error::missing_field(MANIFEST_CONFIG_KEY))?,
        )
        .map_err(de::Error::custom)?;

        let layers: Vec<ImageLayer> = Deserialize::deserialize(
            map.remove(MANIFEST_LAYERS_KEY)
                .ok_or(de::Error::missing_field(MANIFEST_LAYERS_KEY))?,
        )
        .map_err(de::Error::custom)?;

        if let Some(key) = map.keys().next() {
            return Err(de::Error::unknown_field(
                key,
                &[
                    SCHEMA_VERSION_KEY,
                    MIME_TYPE_KEY,
                    MANIFEST_CONFIG_KEY,
                    MANIFEST_LAYERS_KEY,
                ],
            ));
        }

        Ok(Self {
            _config: config,
            layers,
        })
    }
}
