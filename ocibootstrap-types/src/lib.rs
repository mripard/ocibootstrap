#![doc = include_str!("../README.md")]
#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

use alloc::fmt;
use std::{env::consts, io};

/// Representation of an hardware architecture
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
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
    /// Returns an `Architecture` enum from the OCI string representation
    ///
    /// # Errors
    ///
    /// If the given architecture is unknown
    pub fn from_oci_str(s: &str) -> Result<Self, OciBootstrapError> {
        // See GOARCH <https://go.dev/doc/install/source#environment>
        Ok(match s {
            "arm" => Self::Arm,
            "arm64" => Self::Arm64,
            "x86" => Self::X86,
            "amd64" => Self::X86_64,
            _ => {
                return Err(OciBootstrapError::Custom(format!(
                    "Unknown Architecture {s}"
                )));
            }
        })
    }

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
                )));
            }
        })
    }

    /// Returns the OCI architecture name
    #[must_use]
    pub fn as_oci_str(self) -> &'static str {
        // See GOARCH <https://go.dev/doc/install/source#environment>
        match self {
            Self::Arm => "arm",
            Self::Arm64 => "arm64",
            Self::X86 => "x86",
            Self::X86_64 => "amd64",
        }
    }
}

impl From<oci_spec::image::Arch> for Architecture {
    fn from(value: oci_spec::image::Arch) -> Self {
        #[allow(clippy::wildcard_enum_match_arm)]
        match value {
            oci_spec::image::Arch::ARM => Self::Arm,
            oci_spec::image::Arch::ARM64 => Self::Arm64,
            oci_spec::image::Arch::i386 => Self::X86,
            oci_spec::image::Arch::Amd64 => Self::X86_64,
            _ => unimplemented!(),
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

/// Representation of an OS
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatingSystem {
    /// Linux
    Linux,
}

impl OperatingSystem {
    /// Creates our OS enum from the Rust OS name
    ///
    /// # Errors
    ///
    /// If the given OS is unknown
    pub fn from_rust_str(s: &str) -> Result<Self, OciBootstrapError> {
        // See <https://github.com/rust-lang/rust/blob/master/library/std/build.rs#L21>
        Ok(match s {
            "linux" => Self::Linux,
            _ => return Err(OciBootstrapError::Custom(format!("Unknown OS: {s}"))),
        })
    }

    /// Returns the OCI Operating System name
    #[must_use]
    pub fn as_oci_str(self) -> &'static str {
        // See GOOS <https://go.dev/doc/install/source#environment>
        match self {
            Self::Linux => "linux",
        }
    }
}

impl From<oci_spec::image::Os> for OperatingSystem {
    fn from(value: oci_spec::image::Os) -> Self {
        #[allow(clippy::wildcard_enum_match_arm)]
        match value {
            oci_spec::image::Os::Linux => Self::Linux,
            _ => unimplemented!(),
        }
    }
}

impl Default for OperatingSystem {
    fn default() -> Self {
        OperatingSystem::from_rust_str(consts::OS)
            .unwrap_or_else(|_| panic!("Running on unknown OS: {}", consts::OS))
    }
}

impl fmt::Display for OperatingSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_oci_str())
    }
}

/// Our Error Type
#[derive(thiserror::Error, Debug)]
pub enum OciBootstrapError {
    /// An error has occurred when accessing the local filesystem or files
    #[error("I/O Error")]
    Io(#[from] io::Error),

    /// An error has occurred when parsing JSON data
    #[error("JSON Parsing Failure")]
    Json(#[from] serde_json::Error),

    /// An error has occurred when interacting with OCI images or registries
    #[error("OCI Specicication Error")]
    OciSpec(#[from] oci_spec::OciSpecError),

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
