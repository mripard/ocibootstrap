use std::{fs, io};

use once_cell::sync::Lazy;
use toml::{map::Map, Table, Value};

use crate::Error;

pub(crate) const CONTAINERS_CFG_ALIASES_KEY: &str = "aliases";

pub(crate) static CONTAINERS_CFG: Lazy<Result<Map<String, Value>, Error>> = Lazy::new(|| {
    let main = fs::read_to_string("/etc/containers/registries.conf")?;
    let mut config: Table = toml::from_str(&main)?;

    let mut entries = fs::read_dir("/etc/containers/registries.conf.d")?
        .map(|res| res.map(|e| e.path()))
        .collect::<Result<Vec<_>, io::Error>>()?;
    entries.sort();

    for entry in entries {
        let content = fs::read_to_string(&entry)?;
        let cfg: Table = toml::from_str(&content)?;

        for (key, val) in cfg {
            config.insert(key, val);
        }
    }

    Ok(config)
});
