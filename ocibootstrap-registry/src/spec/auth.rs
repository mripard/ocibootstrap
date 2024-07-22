use core::str::FromStr;

#[derive(Debug)]
pub struct AuthenticateHeader {
    pub(crate) realm: String,
    pub(crate) service: String,
}

impl FromStr for AuthenticateHeader {
    type Err = types::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut realm = None;
        let mut service = None;

        if !s.starts_with("Bearer ") {
            return Err(types::Error::Custom(String::from(
                "Authentication Header doesn't have the right format",
            )));
        }

        let (_, params) = s.split_once(' ').ok_or(types::Error::Custom(String::from(
            "Authentication Header doesn't have the right format",
        )))?;
        for param in params.split(',') {
            let (key, val) = param
                .split_once('=')
                .ok_or(types::Error::Custom(String::from(
                    "Authentication Header doesn't have the right format",
                )))?;

            if !(val.starts_with('"') && val.ends_with('"')) {
                return Err(types::Error::Custom(String::from(
                    "Authentication Header doesn't have the right format",
                )));
            }

            let val = val[1..(val.len() - 1)].to_owned();
            match key {
                "realm" => realm = Some(val),
                "service" => service = Some(val),
                "scope" => {}
                _ => unimplemented!(),
            }
        }

        let realm = realm.ok_or(types::Error::Custom(String::from(
            "Authentication Header doesn't have a realm parameter",
        )))?;

        let service = service.ok_or(types::Error::Custom(String::from(
            "Authentication Header doesn't have a service parameter",
        )))?;

        Ok(AuthenticateHeader { realm, service })
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use test_log::test;

    use crate::spec::auth::AuthenticateHeader;

    #[test]
    fn test_docker_io() {
        let auth = AuthenticateHeader::from_str(
            "Bearer realm=\"https://auth.docker.io/token\",service=\"registry.docker.io\"",
        )
        .unwrap();

        assert_eq!(auth.realm, "https://auth.docker.io/token");
        assert_eq!(auth.service, "registry.docker.io");
    }

    #[test]
    fn test_ghcr_io() {
        let auth = AuthenticateHeader::from_str(
            "Bearer realm=\"https://ghcr.io/token\",service=\"ghcr.io\",scope=\"repository:user/image:pull\"",
        ).unwrap();

        assert_eq!(auth.realm, "https://ghcr.io/token");
        assert_eq!(auth.service, "ghcr.io");
    }
}
