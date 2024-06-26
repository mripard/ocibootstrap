use std::collections::HashMap;

use base64::Engine;
use mime::Mime;
use serde::{de, Deserialize};
use serde_json::Value;
use url::Url;

use crate::{
    spec::{v2::MIME_TYPE_KEY, DIGEST_KEY, SCHEMA_VERSION_KEY, SIZE_KEY},
    types::CompressionAlgorithm,
    Digest,
};

const ANNOTATIONS_KEY: &str = "annotations";
const ARTIFACT_TYPE_KEY: &str = "artifactType";
const SUBJECT_KEY: &str = "subject";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TagsListResponse {
    #[serde(rename = "name")]
    _name: String,
    pub(crate) tags: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ImageConfig {
    _digest: Digest,
    _size: usize,
    _urls: Option<Vec<Url>>,
    _annotations: Option<HashMap<String, String>>,
    _data: Option<Vec<u8>>,
}

impl<'de> Deserialize<'de> for ImageConfig {
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

        if media_kind != "application/vnd.oci.image.config.v1+json" {
            return Err(de::Error::invalid_value(
                de::Unexpected::Str(&media_kind),
                &"application/vnd.oci.image.config.v1+json",
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

        let urls = if let Some(urls) = map.remove("urls") {
            let urls: Vec<String> = Deserialize::deserialize(urls).map_err(de::Error::custom)?;

            Some(
                urls.iter()
                    .map(|s| {
                        Url::parse(s).map_err(|_e| {
                            de::Error::invalid_value(de::Unexpected::Str(s), &"a valid URL")
                        })
                    })
                    .collect::<Result<Vec<_>, D::Error>>()?,
            )
        } else {
            None
        };

        let annotations = if let Some(annotations) = map.remove(ANNOTATIONS_KEY) {
            Some(Deserialize::deserialize(annotations).map_err(de::Error::custom)?)
        } else {
            None
        };

        let data = if let Some(data) = map.remove("data") {
            Some(
                data.as_str()
                    .ok_or(de::Error::invalid_type(
                        de::Unexpected::Other("something other than an array"),
                        &"an array",
                    ))
                    .and_then(|s| {
                        base64::engine::general_purpose::STANDARD
                            .decode(s)
                            .map_err(de::Error::custom)
                    })?,
            )
        } else {
            None
        };

        if let Some(key) = map.keys().next() {
            return Err(de::Error::unknown_field(
                key,
                &[
                    MIME_TYPE_KEY,
                    SIZE_KEY,
                    DIGEST_KEY,
                    "urls",
                    ANNOTATIONS_KEY,
                    "data",
                ],
            ));
        }

        Ok(Self {
            _digest: digest,
            _size: size,
            _urls: urls,
            _annotations: annotations,
            _data: data,
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

        let media = media_kind.parse::<Mime>().map_err(de::Error::custom)?;
        if media.type_() != "application" || media.subtype() != "vnd.oci.image.layer.v1.tar" {
            return Err(de::Error::invalid_value(
                de::Unexpected::Str(&media_kind),
                &"application/vnd.oci.image.layer.v1.tar",
            ));
        }

        let compression =
            media
                .suffix()
                .map_or(CompressionAlgorithm::None, |comp| match comp.as_str() {
                    "gzip" => CompressionAlgorithm::Gzip,
                    "zstd" => CompressionAlgorithm::Zstd,
                    _ => unimplemented!(),
                });

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
            size,
            digest,
            compression,
        })
    }
}

pub(crate) const IMAGE_MANIFEST_MIME_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

const IMAGE_MANIFEST_CONFIG_KEY: &str = "config";
const IMAGE_MANIFEST_LAYERS_KEY: &str = "layers";

#[derive(Clone, Debug)]
pub(crate) struct ImageManifest {
    _artifact_kind: Option<String>,
    _config: ImageConfig,
    pub(crate) layers: Vec<ImageLayer>,
    _annotations: Option<HashMap<String, Value>>,
}

impl<'de> Deserialize<'de> for ImageManifest {
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

        let mime = media_kind.parse::<Mime>().map_err(de::Error::custom)?;
        if mime.essence_str() != IMAGE_MANIFEST_MIME_TYPE {
            return Err(de::Error::invalid_value(
                de::Unexpected::Str(&media_kind),
                &IMAGE_MANIFEST_MIME_TYPE,
            ));
        }

        let artifact = if let Some(artifact) = map.remove(ARTIFACT_TYPE_KEY) {
            Some(
                artifact
                    .as_str()
                    .ok_or(de::Error::invalid_type(
                        de::Unexpected::Other("something other than a string"),
                        &"a string",
                    ))?
                    .to_owned(),
            )
        } else {
            None
        };

        let config = ImageConfig::deserialize(
            map.remove(IMAGE_MANIFEST_CONFIG_KEY)
                .ok_or(de::Error::missing_field(IMAGE_MANIFEST_CONFIG_KEY))?,
        )
        .map_err(de::Error::custom)?;

        let layers: Vec<ImageLayer> = Deserialize::deserialize(
            map.remove(IMAGE_MANIFEST_LAYERS_KEY)
                .ok_or(de::Error::missing_field(IMAGE_MANIFEST_LAYERS_KEY))?,
        )
        .map_err(de::Error::custom)?;

        if map.remove(SUBJECT_KEY).is_some() {
            unimplemented!();
        }

        let annotations = if let Some(annotations) = map.remove(ANNOTATIONS_KEY) {
            Some(Deserialize::deserialize(annotations).map_err(de::Error::custom)?)
        } else {
            None
        };

        if let Some(key) = map.keys().next() {
            return Err(de::Error::unknown_field(
                key,
                &[
                    SCHEMA_VERSION_KEY,
                    MIME_TYPE_KEY,
                    ARTIFACT_TYPE_KEY,
                    IMAGE_MANIFEST_CONFIG_KEY,
                    IMAGE_MANIFEST_LAYERS_KEY,
                    SUBJECT_KEY,
                    ANNOTATIONS_KEY,
                ],
            ));
        }

        Ok(Self {
            _artifact_kind: artifact,
            _config: config,
            layers,
            _annotations: annotations,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageIndexPlatform {
    pub(crate) architecture: String,
    pub(crate) os: String,
    #[serde(rename = "os.version")]
    _os_version: Option<String>,
    #[serde(default, rename = "os.features")]
    _os_features: Vec<String>,
    #[serde(rename = "variant")]
    _variant: Option<String>,
}

const IMAGE_INDEX_MANIFEST_MIME_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

const IMAGE_INDEX_MANIFEST_PLATFORM_KEY: &str = "platform";

#[derive(Clone, Debug)]
pub(crate) struct ImageIndexManifest {
    _size: usize,
    pub(crate) digest: Digest,
    pub(crate) platform: Option<ImageIndexPlatform>,
}

impl<'de> Deserialize<'de> for ImageIndexManifest {
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

        let mime = media_kind.parse::<Mime>().map_err(de::Error::custom)?;
        if mime.essence_str() != IMAGE_INDEX_MANIFEST_MIME_TYPE {
            return Err(de::Error::invalid_value(
                de::Unexpected::Str(&media_kind),
                &IMAGE_INDEX_MANIFEST_MIME_TYPE,
            ));
        }

        let size = usize::deserialize(
            map.remove(SIZE_KEY)
                .ok_or(de::Error::missing_field(SIZE_KEY))?,
        )
        .map_err(de::Error::custom)?;

        let digest = Digest::deserialize(
            map.remove(DIGEST_KEY)
                .ok_or(de::Error::missing_field(DIGEST_KEY))?,
        )
        .map_err(de::Error::custom)?;

        let platform = if let Some(platform) = map.remove(IMAGE_INDEX_MANIFEST_PLATFORM_KEY) {
            Some(Deserialize::deserialize(platform).map_err(de::Error::custom)?)
        } else {
            None
        };

        if let Some(key) = map.keys().next() {
            return Err(de::Error::unknown_field(
                key,
                &[
                    MIME_TYPE_KEY,
                    SIZE_KEY,
                    DIGEST_KEY,
                    IMAGE_INDEX_MANIFEST_PLATFORM_KEY,
                ],
            ));
        }

        Ok(Self {
            _size: size,
            digest,
            platform,
        })
    }
}

pub(crate) const IMAGE_INDEX_MIME_TYPE: &str = "application/vnd.oci.image.index.v1+json";

const IMAGE_INDEX_MANIFESTS_KEY: &str = "manifests";

#[derive(Clone, Debug)]
pub(crate) struct ImageIndex {
    _artifact_kind: Option<String>,
    pub(crate) manifests: Vec<ImageIndexManifest>,
    _annotations: Option<HashMap<String, String>>,
}

impl<'de> Deserialize<'de> for ImageIndex {
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

        let mime = media_kind.parse::<Mime>().map_err(de::Error::custom)?;
        if mime.essence_str() != IMAGE_INDEX_MIME_TYPE {
            return Err(de::Error::invalid_value(
                de::Unexpected::Str(&media_kind),
                &IMAGE_INDEX_MIME_TYPE,
            ));
        }

        let artifact = if let Some(artifact) = map.remove(ARTIFACT_TYPE_KEY) {
            Some(
                artifact
                    .as_str()
                    .ok_or(de::Error::invalid_type(
                        de::Unexpected::Other("something other than a string"),
                        &"a string",
                    ))?
                    .to_owned(),
            )
        } else {
            None
        };

        let manifests: Vec<ImageIndexManifest> = Deserialize::deserialize(
            map.remove(IMAGE_INDEX_MANIFESTS_KEY)
                .ok_or(de::Error::missing_field(IMAGE_INDEX_MANIFESTS_KEY))?,
        )
        .map_err(de::Error::custom)?;

        if map.remove(SUBJECT_KEY).is_some() {
            unimplemented!();
        }

        let annotations = if let Some(annotations) = map.remove(ANNOTATIONS_KEY) {
            Some(Deserialize::deserialize(annotations).map_err(de::Error::custom)?)
        } else {
            None
        };

        if let Some(key) = map.keys().next() {
            return Err(de::Error::unknown_field(
                key,
                &[
                    SCHEMA_VERSION_KEY,
                    MIME_TYPE_KEY,
                    ARTIFACT_TYPE_KEY,
                    IMAGE_INDEX_MANIFESTS_KEY,
                    SUBJECT_KEY,
                    ANNOTATIONS_KEY,
                ],
            ));
        }

        Ok(Self {
            _artifact_kind: artifact,
            manifests,
            _annotations: annotations,
        })
    }
}
