use url::Url;

use crate::{
    config::{CONTAINERS_CFG, CONTAINERS_CFG_ALIASES_KEY},
    Error, DOCKER_HUB_REGISTRY_URL_STR,
};

#[derive(Debug)]
pub(crate) struct ContainerSpec {
    pub(crate) name: String,
    pub(crate) registry_url: Url,
}

impl ContainerSpec {
    pub(crate) fn from_container_name(name: &str) -> Result<Self, Error> {
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

        let expanded_name = if expanded_name.contains('/') {
            expanded_name
        } else {
            format!("library/{expanded_name}")
        };

        let mut split_name = expanded_name.split('/');
        let domain = split_name.nth(0).ok_or(Error::Custom(String::from(
            "Domain doesn't have the right format",
        )))?;
        if psl::domain(domain.as_bytes()).is_none() {
            return Ok(ContainerSpec {
                name: expanded_name,
                registry_url: Url::parse(DOCKER_HUB_REGISTRY_URL_STR)?,
            });
        }

        let domain = if domain == "docker.io" {
            String::from(DOCKER_HUB_REGISTRY_URL_STR)
        } else {
            format!("https://{domain}")
        };

        let container_name = split_name.collect::<Vec<_>>().join("/");
        Ok(ContainerSpec {
            name: container_name,
            registry_url: Url::parse(&domain)?,
        })
    }
}

#[cfg(test)]
mod registry_url_tests {
    use test_log::test;
    use url::Url;

    use crate::{
        config::{CONTAINERS_CFG, CONTAINERS_CFG_ALIASES_KEY},
        container::ContainerSpec,
        DOCKER_HUB_REGISTRY_URL_STR,
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
        assert_eq!(
            container.registry_url,
            Url::parse(DOCKER_HUB_REGISTRY_URL_STR).unwrap()
        );
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
        assert_eq!(container.name, "library/nginx");
        assert_eq!(
            container.registry_url,
            Url::parse(DOCKER_HUB_REGISTRY_URL_STR).unwrap()
        );
    }

    #[test]
    fn test_name_without_domain() {
        let container = ContainerSpec::from_container_name("pytorch/pytorch").unwrap();

        assert_eq!(container.name, "pytorch/pytorch");
        assert_eq!(
            container.registry_url,
            Url::parse(DOCKER_HUB_REGISTRY_URL_STR).unwrap()
        );
    }

    #[test]
    fn test_other_registry() {
        let container =
            ContainerSpec::from_container_name("registry.access.redhat.com/ubi9").unwrap();

        assert_eq!(container.name, "ubi9");
        assert_eq!(
            container.registry_url,
            Url::parse("https://registry.access.redhat.com").unwrap()
        );
    }
}
