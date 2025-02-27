use core::fmt;

use log::debug;
use types::Digest;

use crate::{
    OciBootstrapError,
    config::{CONTAINERS_CFG, CONTAINERS_CFG_ALIASES_KEY},
};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ContainerReference {
    Tag(String),
    Digest(Digest),
}

impl fmt::Display for ContainerReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContainerReference::Tag(t) => f.write_str(t),
            ContainerReference::Digest(d) => f.write_str(&d.to_oci_string()),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ContainerSpec {
    pub(crate) domain: String,
    pub(crate) name: String,
    pub(crate) reference: ContainerReference,
}

impl ContainerSpec {
    pub(crate) fn from_container_name(name: &str) -> Result<Self, OciBootstrapError> {
        debug!("Parsing container {name}");

        let (name, reference) = if let Some((name, digest)) = name.rsplit_once('@') {
            let digest = Digest::from_oci_str(digest)?;
            (name, ContainerReference::Digest(digest))
        } else if let Some((name, tag)) = name.rsplit_once(':') {
            (name, ContainerReference::Tag(tag.to_owned()))
        } else {
            (name, ContainerReference::Tag("latest".to_owned()))
        };

        debug!("Container name is {name}, reference is {reference}");

        let expanded_name = if let Ok(cfg) = CONTAINERS_CFG.as_ref() {
            if let Some(aliases) = cfg.get(CONTAINERS_CFG_ALIASES_KEY) {
                if let Some(v) = aliases.get(name) {
                    toml::Value::try_into(v.clone())?
                } else {
                    String::from(name)
                }
            } else {
                String::from(name)
            }
        } else {
            String::from(name)
        };

        debug!("Expanded container name is {expanded_name}");

        let (domain_name, container_name) =
            expanded_name
                .split_once('/')
                .ok_or(OciBootstrapError::Custom(String::from(
                    "Domain doesn't have the right format",
                )))?;

        if domain_name != "localhost" && psl::domain(domain_name.as_bytes()).is_none() {
            debug!("The domain {domain_name} isn't valid, bailing out.");

            // TODO: We should probably try handle it by looking at the registries.conf
            // unqualified-search-registries key. This would however require to either ask the user
            // for which registry to use, or try all of them.
            return Err(OciBootstrapError::Custom(String::from(
                "Invalid domain name",
            )));
        }

        debug!("Container domain name is {domain_name}");

        let spec = ContainerSpec {
            domain: domain_name.to_owned(),
            name: container_name.to_owned(),
            reference,
        };

        debug!("Full container name is {}", spec.to_oci_string());

        Ok(spec)
    }

    pub(crate) fn to_oci_string(&self) -> String {
        match &self.reference {
            ContainerReference::Tag(t) => format!("{}/{}:{}", self.domain, self.name, t),
            ContainerReference::Digest(d) => {
                format!("{}/{}@{}", self.domain, self.name, d.to_oci_string())
            }
        }
    }
}

impl fmt::Display for ContainerSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("{}/{}", self.domain, self.name))
    }
}

#[cfg(test)]
mod registry_url_tests {
    use test_log::test;
    use types::Digest;

    use crate::{
        config::{CONTAINERS_CFG, CONTAINERS_CFG_ALIASES_KEY},
        container::{ContainerReference, ContainerSpec},
    };

    #[test]
    fn test_short_name_with_alias() {
        let container_name = "debian";

        assert!(
            CONTAINERS_CFG
                .as_ref()
                .unwrap()
                .get(CONTAINERS_CFG_ALIASES_KEY)
                .unwrap()
                .get(container_name)
                .is_some()
        );

        assert_eq!(
            ContainerSpec::from_container_name(container_name).unwrap(),
            ContainerSpec {
                domain: String::from("docker.io"),
                name: String::from("library/debian"),
                reference: ContainerReference::Tag(String::from("latest"))
            }
        );
    }

    #[test]
    fn test_short_name_with_alias_tag() {
        let container_name = "ubuntu:24.04";

        assert!(
            CONTAINERS_CFG
                .as_ref()
                .unwrap()
                .get(CONTAINERS_CFG_ALIASES_KEY)
                .unwrap()
                .get("ubuntu")
                .is_some()
        );

        assert_eq!(
            ContainerSpec::from_container_name(container_name).unwrap(),
            ContainerSpec {
                domain: String::from("docker.io"),
                name: String::from("library/ubuntu"),
                reference: ContainerReference::Tag(String::from("24.04"))
            }
        );
    }

    #[test]
    fn test_short_name_without_alias() {
        let container_name = "nginx";

        assert!(
            CONTAINERS_CFG
                .as_ref()
                .unwrap()
                .get(CONTAINERS_CFG_ALIASES_KEY)
                .unwrap()
                .get(container_name)
                .is_none()
        );

        assert!(ContainerSpec::from_container_name(container_name).is_err());
    }

    #[test]
    fn test_long_name_without_alias() {
        let container_name = "pytorch/pytorch";

        assert!(
            CONTAINERS_CFG
                .as_ref()
                .unwrap()
                .get(CONTAINERS_CFG_ALIASES_KEY)
                .unwrap()
                .get(container_name)
                .is_none()
        );

        assert!(ContainerSpec::from_container_name("pytorch/pytorch").is_err());
    }

    #[test]
    fn test_full_name() {
        let container_name = "registry.access.redhat.com/ubi9";

        assert_eq!(
            ContainerSpec::from_container_name(container_name).unwrap(),
            ContainerSpec {
                domain: String::from("registry.access.redhat.com"),
                name: String::from("ubi9"),
                reference: ContainerReference::Tag(String::from("latest"))
            }
        );
    }

    #[test]
    fn test_full_name_with_tag() {
        let container_name = "quay.io/fedora/fedora-minimal:40";

        assert_eq!(
            ContainerSpec::from_container_name(container_name).unwrap(),
            ContainerSpec {
                domain: String::from("quay.io"),
                name: String::from("fedora/fedora-minimal"),
                reference: ContainerReference::Tag(String::from("40"))
            }
        );
    }

    #[test]
    fn test_full_name_with_digest() {
        let container_name = "quay.io/fedora/fedora-minimal@sha256:ea58cd083e2410fd40f1c41be33ed785028c8f6f99d0ea258c80eedbc5ded1bc";

        assert_eq!(
            ContainerSpec::from_container_name(container_name).unwrap(),
            ContainerSpec {
                domain: String::from("quay.io"),
                name: String::from("fedora/fedora-minimal"),
                reference: ContainerReference::Digest(
                    Digest::from_oci_str(
                        "sha256:ea58cd083e2410fd40f1c41be33ed785028c8f6f99d0ea258c80eedbc5ded1bc"
                    )
                    .unwrap()
                )
            }
        );
    }
}
