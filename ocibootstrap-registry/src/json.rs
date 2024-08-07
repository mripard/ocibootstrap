use std::collections::HashMap;

use jiff::Timestamp;
use serde::{de, Deserialize};
use serde_json::Value;
use types::{Architecture, Digest};

use crate::{spec::v2, SCHEMA_VERSION_KEY};

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    created: Timestamp,
    author: Option<String>,
    architecture: String,
    os: String,
    config: HashMap<String, Value>,
}

impl Config {
    pub fn architecture(&self) -> Architecture {
        Architecture::from_oci_str(&self.architecture).unwrap()
    }

    pub fn os(&self) -> &str {
        &self.os
    }
}

#[derive(Clone, Debug)]
pub enum Manifest {
    SchemaV2(v2::Manifest),
}

impl Manifest {
    pub fn config_digest(&self) -> Digest {
        match self {
            Manifest::SchemaV2(s) => match s {
                v2::Manifest::Docker(d) => d.config.digest.clone(),
                v2::Manifest::OciIndex(_) => todo!(),
                v2::Manifest::OciManifest(m) => m.config.digest.clone(),
            },
        }
    }
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
