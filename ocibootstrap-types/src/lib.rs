#![doc = include_str!("../README.md")]
#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

use alloc::fmt;
use core::str::FromStr;
use std::{env::consts, io};

use serde::{de, Deserialize};

/// Representation of an hardware architecture
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
#[clap(rename_all = "lower")]
pub enum Architecture {
    /// ARM's AARCH32 Architecture
    Arm,

    /// ARM's AARCH64 Architecture
    Arm64,

    /// Intel's x86 Architecture
    X86,

    /// Intel's X86-64 Architecture
    #[clap(name = "amd64")]
    X86_64,
}

impl Architecture {
    /// Creates our architecture enum from the Rust architecture name
    ///
    /// # Errors
    ///
    /// If the given architecture is unknown
    pub fn from_rust_str(s: &str) -> Result<Self, OciBootstrapError> {
        Ok(match s {
            "aarch64" => Self::Arm64,
            "arm" => Self::Arm,
            "x86_64" => Self::X86_64,
            "x86" => Self::X86,
            _ => {
                return Err(OciBootstrapError::Custom(format!(
                    "Unknown architecture: {s}"
                )))
            }
        })
    }

    /// Returns the OCI architecture name
    #[must_use]
    pub fn as_oci_str(self) -> &'static str {
        match self {
            Self::Arm => "arm",
            Self::Arm64 => "arm64",
            Self::X86 => "x86",
            Self::X86_64 => "amd64",
        }
    }
}

impl Default for Architecture {
    fn default() -> Self {
        Architecture::from_rust_str(consts::ARCH)
            .unwrap_or_else(|_| panic!("Running on unknown architecture: {}", consts::ARCH))
    }
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_oci_str())
    }
}

/// Our Error Type
#[derive(thiserror::Error, Debug)]
pub enum OciBootstrapError {
    /// An error has occurred when connecting to a remote server
    #[error("Connection Failure")]
    Connection(#[from] reqwest::Error),

    /// An error has occurred when accessing the local filesystem or files
    #[error("I/O Error")]
    Io(#[from] io::Error),

    /// An error has occurred when parsing JSON data
    #[error("JSON Parsing Failure")]
    Json(#[from] serde_json::Error),

    /// An error has occurred when parsing TOML configuration files
    #[error("Configuration File Format Error")]
    Toml(#[from] toml::de::Error),

    /// An error has occurred when parsing a URL
    #[error("Invalid URL")]
    Url(#[from] url::ParseError),

    /// An unknown error occurred
    #[error("Error: {0}")]
    Custom(String),
}

/// Digest Algorithm Representation
#[derive(Clone, Copy, Debug)]
pub enum DigestAlgorithm {
    /// NSA SHA-2 SHA-256 Algorithm
    Sha256,

    /// NSA SHA-2 SHA-512 Algorithm
    Sha512,
}

impl FromStr for DigestAlgorithm {
    type Err = OciBootstrapError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let alg = match s {
            "sha256" => DigestAlgorithm::Sha256,
            "sha512" => DigestAlgorithm::Sha512,
            _ => unimplemented!(),
        };

        Ok(alg)
    }
}

impl fmt::Display for DigestAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        })
    }
}

/// A Digest Representation
#[derive(Clone, Debug)]
pub struct Digest {
    digest: DigestAlgorithm,
    bytes: Vec<u8>,
}

impl Digest {
    /// Returns the digest as a String
    #[must_use]
    pub fn as_string(&self) -> String {
        hex::encode(&self.bytes)
    }

    /// Returns the digest as a String, with the OCI representation
    #[must_use]
    pub fn as_oci_string(&self) -> String {
        format!("{}:{}", self.digest, self.as_string())
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
