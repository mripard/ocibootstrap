use std::{collections::HashMap, fs::File, io::Read, path::PathBuf};

use jiff::Timestamp;
use log::debug;
use nix::unistd::Uid;
use registry::json;
use serde::Deserialize;
use serde_json::value::RawValue;
use tar_split::TarSplitReader;
use types::{Architecture, Digest, OciBootstrapError};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalContainerImage {
    id: String,
    digest: Digest,
    #[serde(default)]
    names: Vec<String>,
    created: Timestamp,
    #[serde(rename = "names-history")]
    names_history: Vec<String>,
    layer: String,
    #[serde(default, rename = "mapped-layers")]
    mapped_layers: Vec<String>,
    metadata: Box<RawValue>,
    #[serde(rename = "big-data-names")]
    big_data_names: Vec<String>,
    #[serde(rename = "big-data-sizes")]
    big_data_sizes: HashMap<String, usize>,
    #[serde(rename = "big-data-digests")]
    big_data_digests: HashMap<String, Digest>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdMap {
    container_id: u32,
    host_id: u32,
    size: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalContainerLayer {
    id: String,
    #[serde(default)]
    names: Vec<String>,
    parent: Option<String>,
    mountlabel: Option<String>,
    created: Timestamp,

    #[serde(rename = "compressed-diff-digest")]
    compressed_diff_digest: Option<Digest>,

    #[serde(rename = "compressed-size")]
    compressed_size: Option<usize>,

    #[serde(rename = "diff-digest")]
    diff_digest: Option<Digest>,

    #[serde(rename = "diff-size")]
    diff_size: Option<usize>,

    compression: Option<u8>,

    #[serde(default)]
    uidset: Vec<u32>,

    #[serde(default)]
    gidset: Vec<u32>,

    #[serde(default)]
    uidmap: Vec<IdMap>,

    #[serde(default)]
    gidmap: Vec<IdMap>,
}

fn get_containers_dir() -> PathBuf {
    if Uid::current().is_root() {
        PathBuf::from("/var/lib/containers")
    } else {
        xdg::BaseDirectories::with_prefix("containers")
            .unwrap()
            .get_data_home()
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
        let base_dir = if Uid::current().is_root() {
            PathBuf::from("/var/lib/containers")
        } else {
            xdg::BaseDirectories::with_prefix("containers")
                .unwrap()
                .get_data_home()
        };

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

    pub(crate) fn image_by_name(&self, container: &str) -> Option<LocalImage<'_>> {
        debug!("Looking for image {container}");

        self.images
            .iter()
            .find(|i| {
                let name = i
                    .names
                    .first()
                    .map_or("(none)", |n| n.split(":").next().unwrap());

                debug!("Found image with name {name}");

                name == container
            })
            .map(|i| LocalImage {
                registry: self,
                name: container.to_owned(),
            })
    }
}

#[derive(Debug)]
pub(crate) struct LocalImage<'a> {
    registry: &'a LocalRegistry,
    name: String,
}

impl LocalImage<'_> {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn tags(&self) -> Vec<LocalTag<'_>> {
        let mut map: HashMap<String, Vec<LocalTag<'_>>> = HashMap::new();
        for image in &self.registry.images {
            if let Some(name) = image.names.get(0) {
                let mut iter = name.split(":");
                let name = iter.next().unwrap().to_string();
                let tag = iter.next().unwrap().to_string();

                if let Some(tags) = map.get_mut(&name) {
                    tags.push(LocalTag {
                        registry: self.registry,
                        image: self,
                        name: tag.clone(),
                    });
                } else {
                    map.insert(
                        name,
                        vec![LocalTag {
                            registry: self.registry,
                            image: self,
                            name: tag.clone(),
                        }],
                    );
                }
            };
        }

        assert_ne!(self.name, "(none)");

        if let Some(tags) = map.get(&self.name) {
            tags.to_vec()
        } else {
            Vec::new()
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LocalTag<'a> {
    registry: &'a LocalRegistry,
    image: &'a LocalImage<'a>,
    name: String,
}

impl LocalTag<'_> {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn manifest_for_platform(
        &self,
        arch: Architecture,
        os: &str,
    ) -> Result<Option<LocalManifest<'_>>, OciBootstrapError> {
        let expected_name = format!("{}:{}", self.image.name, self.name);

        let images: Vec<&LocalContainerImage> = self
            .registry
            .images
            .iter()
            .filter(|i| {
                if let Some(name) = i.names.first() {
                    if name == &expected_name {
                        return true;
                    } else {
                        return false;
                    }
                } else {
                    return false;
                }
            })
            .collect();

        assert_eq!(images.len(), 1);
        let image = images.first().unwrap();
        println!("Found image {}", &image.id);

        let path = self.registry.overlay_images_dir().join(&image.id);
        println!("Path to image dir {}", path.display());

        let manifest_path = path.join("manifest");
        let manifest_file = File::open(manifest_path)?;
        let manifest: json::Manifest = serde_json::from_reader(&manifest_file)?;

        let cfg_digest = manifest.config_digest();
        let cfg_path = path.join(cfg_digest.to_oci_base64());
        println!("Config Path {}", cfg_path.display());
        let cfg_file = File::open(&cfg_path)?;
        let cfg: json::Config = serde_json::from_reader(&cfg_file)?;

        if cfg.architecture() != arch || cfg.os() != os {
            return Ok(None);
        }

        Ok(Some(LocalManifest {
            registry: self.registry,
            tag: self,
            img: image,
            json: manifest,
            config: cfg,
        }))
    }
}

#[derive(Debug)]
pub(crate) struct LocalManifest<'a> {
    registry: &'a LocalRegistry,
    tag: &'a LocalTag<'a>,
    img: &'a LocalContainerImage,
    json: json::Manifest,
    config: json::Config,
}

impl LocalManifest<'_> {
    // fn overlay_image_dir(&self) -> PathBuf {
    //     self.registry.overlay_images_dir().join(&self.img.id)
    // }

    // pub(crate) fn config(&self) -> json::Config {}

    // pub(crate) fn layers(&self) -> Vec<LocalLayer<'_>> {
    //     let mut layer = self
    //         .registry
    //         .layers
    //         .iter()
    //         .find(|l| l.id == self.image.1.layer)
    //         .unwrap();

    //     let mut image_layers = vec![LocalLayer(&self.registry, layer)];
    //     while let Some(parent) = &layer.parent {
    //         layer = self
    //             .registry
    //             .layers
    //             .iter()
    //             .find(|l| &l.id == parent)
    //             .unwrap();
    //         image_layers.insert(0, LocalLayer(&self.registry, layer));
    //     }

    //     image_layers
    // }
}

pub(crate) struct LocalLayer<'a>(&'a LocalRegistry, &'a LocalContainerLayer);

impl LocalLayer<'_> {
    pub(crate) fn id(&self) -> &str {
        &self.1.id
    }

    pub(crate) fn archive(&self) -> TarSplitReader<'_, Box<dyn Read>> {
        let split_path = self
            .0
            .overlay_layers_dir()
            .join(&format!("{}.tar-split.gz", self.1.id));

        tar_split::from_path(
            &self
                .0
                .storage_dir()
                .join("overlay")
                .join(&self.1.id)
                .join("diff"),
            &split_path,
        )
    }
}

// pub(crate) fn test_local_storage(container: &str) {
//     // let registry = LocalRegistry::new().unwrap();
//     // let image = registry.image_by_name(container).unwrap();

//     // info!("Found Image {} id {}", &image.1.names[0], image.1.id);
//     // let layers = image.layers();
//     // info!("Image has {} layers", layers.len());
//     // let layer = layers.get(0).unwrap();
//     // info!("Top Layer is {}", layer.1.id);

//     // let mut archive = layer.archive();
//     // let mut file = File::create("test-output-archive.tar").unwrap();

//     // io::copy(&mut archive, &mut file).unwrap();
//     // debug!("Trying to find container {container}");

//     // let containers_dir = get_containers_dir();
//     // let storage_dir = containers_dir.join("storage");
//     // let images_file =
//     // File::open(storage_dir.join("overlay-images").join("images.json")).unwrap();

//     // let mut images: Vec<LocalContainerImage> = serde_json::from_reader(&images_file).unwrap();
//     // images.sort_by(|a, b| Ord::cmp(&a.created, &b.created));
//     // images.reverse();

//     // let layers_dir = storage_dir.join("overlay-layers");
//     // let layer_file = File::open(layers_dir.join("layers.json")).unwrap();
//     // let layers: Vec<LocalContainerLayer> = serde_json::from_reader(&layer_file).unwrap();

//     // images
//     //     .iter()
//     //     .find(|i| {
//     //         let name = i
//     //             .names
//     //             .first()
//     //             .map_or("(none)", |n| n.split(":").next().unwrap());

//     //         debug!("Found image with name {name}");

//     //         name == container
//     //     })
//     //     .map(|i| {
//     //         let name = i.names.first().map_or("(none)", AsRef::as_ref);
//     //         println!("{} - {:#?}", name, i.layer);

//     //         let mut layer = layers.iter().find(|l| l.id == i.layer).unwrap();

//     //         let mut image_layers = vec![layer.id.clone()];
//     //         while let Some(parent) = &layer.parent {
//     //             layer = layers.iter().find(|l| &l.id == parent).unwrap();
//     //             image_layers.insert(0, layer.id.clone())
//     //         }

//     //         println!("{:#?}", image_layers);
//     //         let split_path = layers_dir.join(&format!("{}.tar-split.gz", &image_layers[0]));
//     //         println!("{}", split_path.display());

//     //         let mut reader = tar_split::from_path(
//     //             &storage_dir
//     //                 .join("overlay")
//     //                 .join(&image_layers[0])
//     //                 .join("diff"),
//     //             &split_path,
//     //         );

//     //         let mut file = File::create("test-output-archive.tar").unwrap();

//     //         io::copy(&mut reader, &mut file).unwrap();

//     //         // let mut archive = Archive::new(reader);
//     //         // archive.set_overwrite(true);
//     //         // archive.set_preserve_mtime(true);
//     //         // archive.set_preserve_ownerships(true);
//     //         // archive.set_preserve_permissions(true);
//     //         // archive.set_unpack_xattrs(true);

//     //         // archive.unpack(PathBuf::from("./test-archive")).unwrap();
//     //     })
//     //     .unwrap();

//     // for image in images {
//     //     let name = image.names.first().map_or("(none)", AsRef::as_ref);
//     //     println!("{} - {:#?}", name, image.layer);

//     //     let mut layer = layers.iter().find(|l| l.id == image.layer).unwrap();

//     //     let mut image_layers = vec![layer.id.clone()];
//     //     while let Some(parent) = &layer.parent {
//     //         layer = layers.iter().find(|l| &l.id == parent).unwrap();
//     //         image_layers.insert(0, layer.id.clone())
//     //     }

//     //     println!("{:#?}", image_layers);
//     //     let split_path = layers_dir.join(&format!("{}.tar-split.gz", &image_layers[0]));
//     //     println!("{}", split_path.display());

//     //     let mut reader =
//     // tar_split::from_path(&split_path).extract(&PathBuf::from("test-archive"));     // loop {
//     //     //     let mut buf = [0; 4096];

//     //     //     match reader.read(&mut buf) {
//     //     //         Ok(s) => {
//     //     //             if s == 0 {
//     //     //                 break;
//     //     //             }

//     //     //             let mut archive = Archive::new(&buf[0..s]);
//     //     //             archive.set_overwrite(true);
//     //     //             archive.set_preserve_mtime(true);
//     //     //             archive.set_preserve_ownerships(true);
//     //     //             archive.set_preserve_permissions(true);
//     //     //             archive.set_unpack_xattrs(true);

//     //     //             archive.unpack(PathBuf::from("./test-archive")).unwrap();
//     //     //         }
//     //     //         Err(_) => todo!(),
//     //     //     }

//     //     //     panic!()
//     //     // }

//     //     panic!();
//     // }
// }
