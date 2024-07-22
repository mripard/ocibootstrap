use std::{env::consts, fmt, io};

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
#[clap(rename_all = "lower")]
pub enum Architecture {
    Arm,
    Arm64,
    X86,
    #[clap(name = "amd64")]
    X86_64,
}

impl Architecture {
    pub fn from_rust_str(s: &str) -> Result<Self, Error> {
        Ok(match s {
            "aarch64" => Self::Arm64,
            "arm" => Self::Arm,
            "x86_64" => Self::X86_64,
            "x86" => Self::X86,
            _ => return Err(Error::Custom(format!("Unknown architecture: {s}"))),
        })
    }

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

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Connection Failure")]
    Connection(#[from] reqwest::Error),

    #[error("I/O Error")]
    Io(#[from] io::Error),

    #[error("JSON Parsing Failure")]
    Json(#[from] serde_json::Error),

    #[error("Configuration File Format Error")]
    Toml(#[from] toml::de::Error),

    #[error("Invalid URL")]
    Url(#[from] url::ParseError),

    #[error("Error: {0}")]
    Custom(String),
}
