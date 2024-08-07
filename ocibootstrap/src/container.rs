use core::fmt;
use std::path::Display;

use crate::{
    config::{CONTAINERS_CFG, CONTAINERS_CFG_ALIASES_KEY},
    OciBootstrapError, DOCKER_HUB_REGISTRY_URL_STR,
};

#[derive(Debug)]
pub(crate) struct ContainerSpec {
    pub(crate) domain: Option<String>,
    pub(crate) name: String,
}

impl ContainerSpec {
    pub(crate) fn from_container_name(name: &str) -> Result<Self, OciBootstrapError> {
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

        let mut split_name = expanded_name.split('/');
        let domain = split_name
            .nth(0)
            .ok_or(OciBootstrapError::Custom(String::from(
                "Domain doesn't have the right format",
            )))?;
        if psl::domain(domain.as_bytes()).is_none() {
            return Ok(ContainerSpec {
                domain: None,
                name: expanded_name,
            });
        }

        let container_name = split_name.collect::<Vec<_>>().join("/");
        Ok(ContainerSpec {
            domain: Some(domain.to_owned()),
            name: container_name,
        })
    }
}

impl fmt::Display for ContainerSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(domain) = &self.domain {
            f.write_fmt(format_args!("{}/{}", domain, self.name))
        } else {
            f.write_fmt(format_args!("{}", self.name))
        }
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
    fn test_alias() {
        let container_name = "debian";

        assert!(CONTAINERS_CFG
            .as_ref()
            .unwrap()
            .get(CONTAINERS_CFG_ALIASES_KEY)
            .unwrap()
            .get(container_name)
            .is_some());

        let container = ContainerSpec::from_container_name(container_name).unwrap();
        assert_eq!(container.name, "library/debian");
        assert_eq!(container.domain, Some(String::from("docker.io")));
    }

    #[test]
    fn test_short_name_with_alias() {
        let container_name = "nginx";

        assert!(CONTAINERS_CFG
            .as_ref()
            .unwrap()
            .get(CONTAINERS_CFG_ALIASES_KEY)
            .unwrap()
            .get(container_name)
            .is_none());

        let container = ContainerSpec::from_container_name(container_name).unwrap();
        assert_eq!(container.name, "nginx");
        assert_eq!(container.domain, None);
    }

    #[test]
    fn test_name_without_domain() {
        let container = ContainerSpec::from_container_name("pytorch/pytorch").unwrap();

        assert_eq!(container.name, "pytorch/pytorch");
        assert_eq!(container.domain, None,);
    }

    #[test]
    fn test_other_registry() {
        let container =
            ContainerSpec::from_container_name("registry.access.redhat.com/ubi9").unwrap();

        assert_eq!(container.name, "ubi9");
        assert_eq!(
            container.domain,
            Some(String::from("registry.access.redhat.com"))
        );
    }
}
