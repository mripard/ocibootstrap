use alloc::fmt;
use std::env::consts;

use log::debug;
use num_traits::{CheckedAdd, CheckedDiv, CheckedMul, Euclid, FromPrimitive, Num};

use crate::Error;

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
#[clap(rename_all = "lower")]
pub(crate) enum Architecture {
    Arm,
    Arm64,
    X86,
    #[clap(name = "amd64")]
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

    pub(crate) fn as_oci_str(self) -> &'static str {
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

pub(crate) fn round_down<T>(number: T, multiple: T) -> T
where
    T: Copy + Num + CheckedDiv + CheckedMul,
{
    let div = T::checked_div(&number, &multiple).expect("Division by zero or would overflow");

    T::checked_mul(&div, &multiple).expect("Multiplication would overflow")
}

pub(crate) fn round_up<T>(number: T, multiple: T) -> T
where
    T: Copy + Num + CheckedAdd + CheckedMul + Euclid + FromPrimitive,
{
    let rem = T::rem_euclid(&number, &multiple);

    if rem.is_zero() {
        return number;
    }

    let div = T::checked_add(&T::div_euclid(&number, &multiple), &T::one())
        .expect("Addition would overflow");

    T::checked_mul(&div, &multiple).expect("Multiplication would overflow")
}
