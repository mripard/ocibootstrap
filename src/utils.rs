use std::env::consts;

use log::debug;

pub(crate) fn convert_rust_arch_to_oci(arch: &str) -> &str {
    match arch {
        "arm" => "arm",
        "aarch64" => "arm64",
        "powerpc64" => "ppc64le",
        "riscv64" => "riscv64",
        "x86_64" => "amd64",
        val => {
            debug!("Unknown architecture {}, using as-is.", val);
            val
        }
    }
}

pub(crate) fn get_current_oci_arch() -> &'static str {
    convert_rust_arch_to_oci(consts::ARCH)
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