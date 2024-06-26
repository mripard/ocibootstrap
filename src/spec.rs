use serde::{de, Deserialize};
use serde_json::Value;

use crate::types::DigestAlgorithm;

pub(crate) mod auth;
pub(crate) mod v2;

const DIGEST_KEY: &str = "digest";
const SCHEMA_VERSION_KEY: &str = "schemaVersion";
const SIZE_KEY: &str = "size";

#[derive(Debug, Deserialize)]
pub(crate) struct Rfc6750AuthResponse {
    pub(crate) token: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Digest {
    digest: DigestAlgorithm,
    bytes: Vec<u8>,
}

impl Digest {
    pub(crate) fn as_string(&self) -> String {
        hex::encode(&self.bytes)
    }

    pub(crate) fn as_oci_string(&self) -> String {
        match self.digest {
            DigestAlgorithm::Sha256 => format!("sha256:{}", self.as_string()),
            DigestAlgorithm::Sha512 => format!("sha512:{}", self.as_string()),
        }
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        let (alg, dig) = s.split_once(':').ok_or(de::Error::invalid_value(
            de::Unexpected::Str(&s),
            &"a digest with the form $ALGO:$DIGEST",
        ))?;

        let bytes = hex::decode(dig).map_err(de::Error::custom)?;

        Ok(match alg {
            "sha256" => Self {
                digest: DigestAlgorithm::Sha256,
                bytes,
            },
            "sha512" => Self {
                digest: DigestAlgorithm::Sha512,
                bytes,
            },
            _ => unimplemented!(),
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Manifest {
    SchemaV2(v2::Manifest),
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

        Ok(match schema {
            2 => Self::SchemaV2(v2::Manifest::deserialize(value).map_err(de::Error::custom)?),
            _ => unimplemented!(),
        })
    }
}
