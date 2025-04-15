use core::str::FromStr as _;
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read},
    path::PathBuf,
};

use base64::Engine as _;
use jiff::Timestamp;
use log::{debug, trace};
use nix::unistd::Uid;
use oci_spec::image::{Digest, ImageConfiguration, ImageManifest, Sha256Digest};
use serde::{Deserialize, de};
use serde_json::Value;
use tar_split::TarSplitReader;
use types::{Architecture, OciBootstrapError, OperatingSystem};

use crate::container::{ContainerSpec, digest_to_oci_string};

fn digest_to_oci_base64(digest: &Digest) -> String {
    // For some reason, it appears the blobs when stored on the FS are regular base64 encoding
    // with an extra padding at the beginning
    format!(
        "={}",
        base64::engine::general_purpose::STANDARD.encode(digest_to_oci_string(digest).as_bytes())
    )
}

fn deserialize_sha256_digest<'de, D>(deserializer: D) -> Result<Sha256Digest, D::Error>
where
    D: de::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;

    Sha256Digest::from_str(&s).map_err(de::Error::custom)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalContainerImage {
    #[serde(deserialize_with = "deserialize_sha256_digest")]
    id: Sha256Digest,

    #[serde(rename = "digest")]
    _digest: Digest,

    #[serde(default)]
    names: Vec<String>,

    #[serde(rename = "created")]
    _created: Timestamp,

    #[serde(rename = "names-history")]
    _names_history: Vec<String>,

    layer: String,

    #[serde(default, rename = "mapped-layers")]
    _mapped_layers: Vec<String>,

    #[serde(rename = "metadata")]
    _metadata: Value,

    #[serde(rename = "big-data-names")]
    _big_data_names: Vec<String>,

    #[serde(rename = "big-data-sizes")]
    _big_data_sizes: HashMap<String, usize>,

    #[serde(rename = "big-data-digests")]
    _big_data_digests: HashMap<String, Digest>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdMap {
    #[serde(rename = "container_id")]
    _container_id: u32,

    #[serde(rename = "host_id")]
    _host_id: u32,

    #[serde(rename = "size")]
    _size: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalContainerLayer {
    #[serde(deserialize_with = "deserialize_sha256_digest")]
    id: Sha256Digest,

    #[serde(default, rename = "names")]
    _names: Vec<String>,

    parent: Option<String>,

    #[serde(rename = "mountlabel")]
    _mountlabel: Option<String>,

    #[serde(rename = "created")]
    _created: Timestamp,

    #[serde(rename = "compressed-diff-digest")]
    _compressed_diff_digest: Option<Digest>,

    #[serde(rename = "compressed-size")]
    _compressed_size: Option<usize>,

    #[serde(rename = "diff-digest")]
    _diff_digest: Option<Digest>,

    #[serde(rename = "diff-size")]
    _diff_size: Option<usize>,

    #[serde(rename = "compression")]
    _compression: Option<u8>,

    #[serde(default, rename = "uidset")]
    _uidset: Vec<u32>,

    #[serde(default, rename = "gidset")]
    _gidset: Vec<u32>,

    #[serde(default, rename = "uidmap")]
    _uidmap: Vec<IdMap>,

    #[serde(default, rename = "gidmap")]
    _gidmap: Vec<IdMap>,
}

fn get_containers_dir() -> Result<PathBuf, io::Error> {
    if Uid::current().is_root() {
        Ok(PathBuf::from("/var/lib/containers"))
    } else {
        Ok(xdg::BaseDirectories::with_prefix("containers")
            .map_err(io::Error::from)?
            .get_data_home())
    }
}

#[derive(Debug)]
pub(crate) struct LocalRegistry {
    base_dir: PathBuf,
    images: Vec<LocalContainerImage>,
    layers: Vec<LocalContainerLayer>,
}

impl LocalRegistry {
    pub(crate) fn new() -> Result<Self, OciBootstrapError> {
        let base_dir = get_containers_dir()?;
        let storage_dir = base_dir.join("storage");
        let images_file = File::open(storage_dir.join("overlay-images").join("images.json"))?;
        let images: Vec<LocalContainerImage> = serde_json::from_reader(&images_file)?;

        let layers_dir = storage_dir.join("overlay-layers");
        let layer_file = File::open(layers_dir.join("layers.json"))?;
        let layers: Vec<LocalContainerLayer> = serde_json::from_reader(&layer_file)?;

        Ok(Self {
            base_dir,
            images,
            layers,
        })
    }

    fn storage_dir(&self) -> PathBuf {
        self.base_dir.join("storage")
    }

    fn overlay_images_dir(&self) -> PathBuf {
        self.storage_dir().join("overlay-images")
    }

    fn overlay_layers_dir(&self) -> PathBuf {
        self.storage_dir().join("overlay-layers")
    }

    fn find_layer_by_id(&self, id: &str) -> Option<&LocalContainerLayer> {
        debug!("Looking for layer {id}");

        self.layers.iter().find(|l| {
            if l.id.digest() == id {
                debug!("Found matching layer");
                true
            } else {
                trace!("Layer {} is not a match", l.id);
                false
            }
        })
    }

    pub(crate) fn image_by_spec(&self, spec: &ContainerSpec) -> Option<LocalImage<'_>> {
        let container_name = spec.to_oci_string();

        debug!("Looking for image {container_name}");

        self.images
            .iter()
            .find(|i| i.names.contains(&container_name))
            .map(|image| LocalImage {
                registry: self,
                name: container_name.clone(),
                image,
            })
    }
}

#[derive(Debug)]
pub(crate) struct LocalImage<'a> {
    registry: &'a LocalRegistry,
    name: String,
    image: &'a LocalContainerImage,
}

impl LocalImage<'_> {
    pub(crate) fn manifest_for_platform(
        &self,
        arch: Architecture,
        os: OperatingSystem,
    ) -> Result<Option<LocalManifest<'_>>, OciBootstrapError> {
        debug!("Looking for image {} manifest", self.name);

        let path = self
            .registry
            .overlay_images_dir()
            .join(self.image.id.digest());
        debug!("Path to image dir {}", path.display());

        let manifest_path = path.join("manifest");
        let manifest_file = File::open(manifest_path)?;
        let manifest: ImageManifest = serde_json::from_reader(&manifest_file)?;

        let cfg_desc = manifest.config();
        let cfg_path = path.join(digest_to_oci_base64(cfg_desc.digest()));
        debug!("Config Path {}", cfg_path.display());

        let cfg_file = File::open(&cfg_path)?;
        let cfg: ImageConfiguration = serde_json::from_reader(&cfg_file)?;

        let cfg_arch: Architecture = cfg.architecture().clone().into();
        let cfg_os: OperatingSystem = cfg.os().clone().into();
        if cfg_arch != arch || cfg_os != os {
            return Ok(None);
        }

        Ok(Some(LocalManifest {
            registry: self.registry,
            img: self,
            _json: manifest,
            config: cfg,
        }))
    }
}

#[derive(Debug)]
pub(crate) struct LocalManifest<'a> {
    registry: &'a LocalRegistry,
    img: &'a LocalImage<'a>,
    _json: ImageManifest,
    config: ImageConfiguration,
}

impl LocalManifest<'_> {
    pub(crate) fn layers(&self) -> Result<Vec<LocalLayer<'_>>, io::Error> {
        let mut layer = self
            .registry
            .find_layer_by_id(&self.img.image.layer)
            .ok_or(io::Error::from(io::ErrorKind::NotFound))?;

        let mut image_layers = vec![LocalLayer(self.registry, layer)];
        while let Some(parent) = &layer.parent {
            debug!("Layer has a parent: {}", parent);

            layer = self
                .registry
                .find_layer_by_id(parent)
                .ok_or(io::Error::from(io::ErrorKind::NotFound))?;

            image_layers.push(LocalLayer(self.registry, layer));
        }

        image_layers.reverse();

        Ok(image_layers)
    }

    pub(crate) fn configuration(&self) -> &ImageConfiguration {
        &self.config
    }
}

#[derive(Debug)]
pub(crate) struct LocalLayer<'a>(&'a LocalRegistry, &'a LocalContainerLayer);

impl LocalLayer<'_> {
    pub(crate) fn digest(&self) -> Digest {
        self.1.id.clone().into()
    }

    pub(crate) fn archive(&self) -> io::Result<TarSplitReader<'_, Box<dyn Read>>> {
        let split_path = self
            .0
            .overlay_layers_dir()
            .join(format!("{}.tar-split.gz", self.1.id.digest()));

        debug!("Opening Tar Split Archive {}", split_path.display());

        tar_split::from_path(
            &self
                .0
                .storage_dir()
                .join("overlay")
                .join(self.1.id.digest())
                .join("diff"),
            &split_path,
        )
    }
}
