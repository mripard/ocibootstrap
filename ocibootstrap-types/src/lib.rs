#![doc = include_str!("../README.md")]
#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

use alloc::fmt;
use std::{env::consts, io};

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
