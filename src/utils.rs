use std::env::consts;

use log::debug;

use crate::Error;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Architecture {
    Arm,
    Arm64,
    X86,
    X86_64,
}

impl Architecture {
    pub(crate) fn from_rust_str(s: &str) -> Result<Self, Error> {
        Ok(match s {
            "aarch64" => Self::Arm64,
            "arm" => Self::Arm,
            "x86_64" => Self::X86_64,
            "x86" => Self::X86,
            _ => return Err(Error::Custom(format!("Unknown architecture: {s}"))),
        })
    }

    pub(crate) fn as_oci_str(&self) -> &'static str {
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
        Architecture::from_rust_str(consts::ARCH).expect(&format!(
            "Running on unknown architecture: {}",
            consts::ARCH
        ))
    }
}

pub(crate) fn convert_rust_os_to_oci(os: &str) -> &str {
    match os {
        "android" => "android",
        "dragonfly" => "dragonfly",
        "freebsd" => "freebsd",
        "ios" => "ios",
        "linux" => "linux",
        "macos" => "darwin",
        "netbsd" => "netbsd",
        "openbsd" => "openbsd",
        "solaris" => "solaris",
        "windows" => "windows",
        val => {
            debug!("Unknown architecture {}, using as-is.", val);
            val
        }
    }
}

pub(crate) fn get_current_oci_os() -> &'static str {
    convert_rust_os_to_oci(consts::OS)
}
