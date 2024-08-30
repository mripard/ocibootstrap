use core::fmt;

use log::debug;

use crate::{
    config::{CONTAINERS_CFG, CONTAINERS_CFG_ALIASES_KEY},
    OciBootstrapError,
};

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ContainerSpec {
    pub(crate) domain: String,
    pub(crate) name: String,
}

impl ContainerSpec {
    pub(crate) fn from_container_name(name: &str) -> Result<Self, OciBootstrapError> {
        debug!("Parsing container {name}");

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

        debug!("Full container name is {expanded_name}");

        let mut split_name = expanded_name.split('/');
        let domain = split_name
            .nth(0)
            .ok_or(OciBootstrapError::Custom(String::from(
                "Domain doesn't have the right format",
            )))?;

        debug!("Container domain name is {domain}");

        if psl::domain(domain.as_bytes()).is_none() {
            debug!("The domain isn't valid, bailing out.");

            // TODO: We should probably try handle it by looking at the registries.conf
            // unqualified-search-registries key. This would however require to either ask the user
            // for which registry to use, or try all of them.
            return Err(OciBootstrapError::Custom(String::from(
                "Missing domain name",
            )));
        }

        let container_name = split_name.collect::<Vec<_>>().join("/");
        Ok(ContainerSpec {
            domain: domain.to_owned(),
            name: container_name,
        })
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

    use crate::{
        config::{CONTAINERS_CFG, CONTAINERS_CFG_ALIASES_KEY},
        container::ContainerSpec,
    };

    #[test]
    fn test_short_name_with_alias() {
        let container_name = "debian";

        assert!(CONTAINERS_CFG
            .as_ref()
            .unwrap()
            .get(CONTAINERS_CFG_ALIASES_KEY)
            .unwrap()
            .get(container_name)
            .is_some());

        assert_eq!(
            ContainerSpec::from_container_name(container_name).unwrap(),
            ContainerSpec {
                domain: String::from("docker.io"),
                name: String::from("library/debian"),
            }
        );
    }

    #[test]
    fn test_short_name_without_alias() {
        let container_name = "nginx";

        assert!(CONTAINERS_CFG
            .as_ref()
            .unwrap()
            .get(CONTAINERS_CFG_ALIASES_KEY)
            .unwrap()
            .get(container_name)
            .is_none());

        assert!(ContainerSpec::from_container_name(container_name).is_err());
    }

    #[test]
    fn test_long_name_without_alias() {
        let container_name = "pytorch/pytorch";

        assert!(CONTAINERS_CFG
            .as_ref()
            .unwrap()
            .get(CONTAINERS_CFG_ALIASES_KEY)
            .unwrap()
            .get(container_name)
            .is_none());

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
            }
        );
    }
}
