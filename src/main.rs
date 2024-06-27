#![doc = include_str!("../README.md")]

use core::str::FromStr;
use std::{
    fs::File,
    io::{BufReader, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use clap::Parser;
use container::ContainerSpec;
use flate2::bufread::GzDecoder;
use log::{debug, info};
use reqwest::{blocking::Client, header::WWW_AUTHENTICATE, StatusCode};
use spec::v2::oci::ImageManifest;
use tar::Archive;
use types::CompressionAlgorithm;
use url::Url;
use utils::Architecture;

mod config;
mod container;
mod spec;
mod types;
mod utils;

use crate::{
    spec::{auth::AuthenticateHeader, v2::oci::TagsListResponse, Digest, Rfc6750AuthResponse},
    utils::get_current_oci_os,
};

const DOCKER_HUB_REGISTRY_URL_STR: &str = "https://index.docker.io";

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("Connection Failure")]
    Connection(#[from] reqwest::Error),

    #[error("I/O Error")]
    Io(#[from] std::io::Error),

    #[error("JSON Parsing Failure")]
    Json(#[from] serde_json::Error),

    #[error("Configuration File Format Error")]
    Toml(#[from] toml::de::Error),

    #[error("Invalid URL")]
    Url(#[from] url::ParseError),

    #[error("Error: {0}")]
    Custom(String),
}

#[derive(Debug)]
pub(crate) struct Registry {
    pub(crate) index_url: Url,
    pub(crate) auth: Option<AuthenticateHeader>,
}

impl Registry {
    pub(crate) fn connect(registry: &str) -> Result<Self, Error> {
        let repo_base_url = Url::parse(registry)?;

        let test_url = repo_base_url.join("/v2/")?;

        debug!("Trying to connect to {}", test_url.as_str());

        let resp = Client::new().get(test_url).send()?;

        match resp.status() {
            StatusCode::OK => {
                debug!("Unauthenticated Registry Found.");

                Ok(Self {
                    index_url: repo_base_url,
                    auth: None,
                })
            }
            StatusCode::UNAUTHORIZED => {
                debug!("Registry is authenticated.");

                let www_authenticate = resp.headers()[WWW_AUTHENTICATE].to_str().map_err(|_e| {
                    Error::Custom(String::from("Couldn't decode header as a String"))
                })?;

                debug!("www-authenticate header is {www_authenticate}");

                let auth = AuthenticateHeader::from_str(www_authenticate)
                    .map_err(|_e| Error::Custom(String::from("Couldn't parse header.")))?;

                Ok(Self {
                    index_url: repo_base_url,
                    auth: Some(auth),
                })
            }
            _ => unimplemented!(),
        }
    }

    pub(crate) fn image<'a>(&'a self, name: &str) -> Result<Image<'a>, Error> {
        let token = if let Some(auth) = &self.auth {
            debug!("Registry is authenticated, getting a token.");

            let mut token_url = Url::parse(&auth.realm)?;
            token_url
                .query_pairs_mut()
                .clear()
                .append_pair("scope", &format!("repository:{name}:pull"))
                .append_pair("service", &auth.service);

            debug!("Token URL: {token_url}");

            let client = Client::new().get(token_url).send()?.error_for_status()?;

            let val: Rfc6750AuthResponse = client.json()?;

            debug!("Got token: {}", val.token);

            Some(val.token)
        } else {
            None
        };

        Ok(Image {
            registry: self,
            name: name.to_owned(),
            token,
        })
    }
}

#[derive(Debug)]
pub(crate) struct Image<'a> {
    pub(crate) registry: &'a Registry,
    pub(crate) name: String,
    pub(crate) token: Option<String>,
}

impl Image<'_> {
    pub(crate) fn latest(&self) -> Result<Tag<'_>, Error> {
        self.tags()?
            .into_iter()
            .find(|t| t == "latest")
            .ok_or(Error::Custom(String::from("Latest tag can't be found")))
    }

    pub(crate) fn tags(&self) -> Result<Vec<Tag<'_>>, Error> {
        let url = self
            .registry
            .index_url
            .join(&format!("/v2/{}/tags/list", self.name))?;

        debug!("Tags List URL: {}", url.as_str());

        let text = if let Some(token) = &self.token {
            Client::new()
                .get(url)
                .header("Authorization", format!("Bearer {token}"))
                .send()
        } else {
            Client::new().get(url).send()
        }?
        .error_for_status()?
        .text()?;

        debug!("Tags List Response: {text}");

        let resp: TagsListResponse = serde_json::from_str(&text)?;
        // assert_eq!(resp.name, self.name);

        Ok(resp
            .tags
            .iter()
            .map(|t| Tag {
                image: self,
                tag_name: t.clone(),
            })
            .collect())
    }
}

#[derive(Debug)]
pub(crate) struct Tag<'a> {
    image: &'a Image<'a>,
    tag_name: String,
}

impl<'a> Tag<'a> {
    pub(crate) fn manifest_for_config(
        &'a self,
        arch: Architecture,
        os: &str,
    ) -> Result<TestManifest<'a>, Error> {
        debug!(
            "Trying to find a manifest for {}, running on {}",
            arch.as_oci_str(),
            os
        );

        let url = self.image.registry.index_url.join(&format!(
            "/v2/{}/manifests/{}",
            self.image.name, self.tag_name
        ))?;

        debug!("Manifest URL: {}", url.as_str());

        let mut client = Client::new()
            .get(url)
            .header("Accept", spec::v2::docker::DISTRIBUTION_MANIFEST_MIME_TYPE)
            .header("Accept", spec::v2::oci::IMAGE_INDEX_MIME_TYPE)
            .header("Accept", spec::v2::oci::IMAGE_MANIFEST_MIME_TYPE);

        if let Some(token) = &self.image.token {
            client = client.header("Authorization", format!("Bearer {token}"));
        }

        let text = client.send()?.error_for_status()?.text()?;

        debug!("Manifest Response {text}");

        let resp: spec::Manifest = serde_json::from_str(&text)?;

        let manifest = match &resp {
            spec::Manifest::SchemaV2(s) => match s {
                spec::v2::Manifest::Docker(_) => TestManifest {
                    image: self.image,
                    inner: resp,
                },
                spec::v2::Manifest::OciManifest(_) => unimplemented!(),
                spec::v2::Manifest::OciIndex(m) => {
                    let manifest = m
                        .manifests
                        .iter()
                        .find_map(|v| {
                            if let Some(platform) = &v.platform {
                                debug!(
                                    "Found manifest for {}, os {}",
                                    platform.architecture, platform.os
                                );

                                if platform.architecture != arch.as_oci_str() || platform.os != os {
                                    return None;
                                }
                            }

                            let digest = &v.digest;

                            let url = match self.image.registry.index_url.join(&format!(
                                "/v2/{}/manifests/{}",
                                self.image.name,
                                digest.as_oci_string()
                            )) {
                                Ok(v) => v,
                                Err(e) => return Some(Err::<ImageManifest, Error>(e.into())),
                            };

                            debug!("Manifest URL {}", url);

                            let mut client = Client::new()
                                .get(url)
                                .header("Accept", spec::v2::oci::IMAGE_MANIFEST_MIME_TYPE);

                            if let Some(token) = &self.image.token {
                                client = client.header("Authorization", format!("Bearer {token}"));
                            }

                            let resp = match client.send() {
                                Ok(v) => v,
                                Err(e) => return Some(Err(e.into())),
                            };

                            let resp = match resp.error_for_status() {
                                Ok(v) => v,
                                Err(e) => return Some(Err(e.into())),
                            };

                            let text = match resp.text() {
                                Ok(v) => v,
                                Err(e) => return Some(Err(e.into())),
                            };

                            debug!("Manifest Response {}", text);

                            Some(match serde_json::from_str(&text) {
                                Ok(v) => Ok(v),
                                Err(e) => Err(e.into()),
                            })
                        })
                        .ok_or(Error::Custom(String::from(
                            "No manifest found for the requested platform.",
                        )))??;

                    TestManifest {
                        image: self.image,
                        inner: spec::Manifest::SchemaV2(spec::v2::Manifest::OciManifest(manifest)),
                    }
                }
            },
        };

        Ok(manifest)
    }
}

impl PartialEq<String> for Tag<'_> {
    fn eq(&self, other: &String) -> bool {
        self.tag_name.eq(other)
    }
}

impl PartialEq<str> for Tag<'_> {
    fn eq(&self, other: &str) -> bool {
        self.tag_name.eq(other)
    }
}

struct TestManifestLayer<'a> {
    image: &'a Image<'a>,
    inner: spec::v2::ImageLayer,
}

impl TestManifestLayer<'_> {
    fn digest(&self) -> Digest {
        match &self.inner {
            spec::v2::ImageLayer::DockerImage(v) => &v.digest,
            spec::v2::ImageLayer::OciImage(v) => &v.digest,
        }
        .clone()
    }

    fn size(&self) -> usize {
        match &self.inner {
            spec::v2::ImageLayer::DockerImage(v) => v.size,
            spec::v2::ImageLayer::OciImage(v) => v.size,
        }
    }

    fn try_from_cache(&self, path: &Path) -> Result<Option<LocalBlob>, Error> {
        if !path.exists() {
            return Ok(None);
        }

        debug!("File already exists, checking its size");

        let metadata = path.metadata()?;

        if metadata.size() != self.size() as u64 {
            return Err(Error::Custom(String::from("File exists but doesn't match")));
        }

        Ok(Some(LocalBlob {
            path: path.to_owned(),
            compression: self.inner.compression(),
        }))
    }

    fn fetch(&self) -> Result<LocalBlob, Error> {
        let url = self.image.registry.index_url.join(&format!(
            "/v2/{}/blobs/{}",
            self.image.name,
            self.digest().as_oci_string()
        ))?;

        debug!("Blob URL {}", url.as_str());

        let resp = if let Some(token) = &self.image.token {
            Client::new()
                .get(url)
                .header("Authorization", format!("Bearer {token}"))
                .send()
        } else {
            Client::new().get(url).send()
        }?
        .error_for_status()?;

        let dir_path = xdg::BaseDirectories::new()
            .map_err(<xdg::BaseDirectoriesError as Into<std::io::Error>>::into)?
            .create_cache_directory(env!("CARGO_CRATE_NAME"))?;

        let file_path = dir_path.join(self.digest().as_string());
        debug!("Blob File Path {}", file_path.display());

        if let Some(v) = self.try_from_cache(&file_path)? {
            Ok(v)
        } else {
            let mut file = File::create_new(&file_path)?;
            file.write_all(&resp.bytes()?)?;

            Ok(LocalBlob {
                path: file_path,
                compression: self.inner.compression(),
            })
        }
    }
}

#[derive(Debug)]
struct LocalBlob {
    path: PathBuf,
    compression: CompressionAlgorithm,
}

impl LocalBlob {
    pub(crate) fn extract(self, target_dir: &Path) -> Result<(), Error> {
        let blob = File::open(self.path)?;
        let blob_reader = BufReader::new(blob);

        let tar = match self.compression {
            CompressionAlgorithm::None => unimplemented!(),
            CompressionAlgorithm::Gzip => GzDecoder::new(blob_reader),
            CompressionAlgorithm::Zstd => unimplemented!(),
        };

        let mut archive = Archive::new(tar);
        archive.set_overwrite(true);
        archive.set_preserve_mtime(true);
        archive.set_preserve_ownerships(true);
        archive.set_preserve_permissions(true);
        archive.set_unpack_xattrs(true);

        archive.unpack(target_dir)?;

        Ok(())
    }
}

#[derive(Clone, Debug)]
struct TestManifest<'a> {
    image: &'a Image<'a>,
    inner: spec::Manifest,
}

impl TestManifest<'_> {
    fn layers(&self) -> Vec<TestManifestLayer<'_>> {
        match &self.inner {
            spec::Manifest::SchemaV2(s) => match s {
                spec::v2::Manifest::Docker(m) => m
                    .layers
                    .clone()
                    .into_iter()
                    .map(|v| TestManifestLayer {
                        image: self.image,
                        inner: spec::v2::ImageLayer::DockerImage(v),
                    })
                    .collect(),
                spec::v2::Manifest::OciManifest(m) => m
                    .layers
                    .clone()
                    .into_iter()
                    .map(|l| TestManifestLayer {
                        image: self.image,
                        inner: spec::v2::ImageLayer::OciImage(l),
                    })
                    .collect(),
                spec::v2::Manifest::OciIndex(_) => unreachable!(),
            },
        }
    }
}

#[derive(Parser)]
#[command(version, about = "OCI Image to Device Utility")]
struct Cli {
    #[arg(help = "Container Name")]
    container: String,

    #[arg(help = "Output Directory")]
    output_dir: PathBuf,
}

fn main() -> Result<(), anyhow::Error> {
    env_logger::init();

    let cli = Cli::parse();

    info!(
        "Running {} {}",
        env!("CARGO_CRATE_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    let container = ContainerSpec::from_container_name(&cli.container)?;
    let registry = Registry::connect(container.registry_url.as_str())?;
    let image = registry.image(&container.name)?;
    let tag = image.latest()?;
    let manifest = tag.manifest_for_config(Architecture::default(), get_current_oci_os())?;

    std::fs::create_dir_all(&cli.output_dir)?;

    for layer in manifest.layers() {
        info!("Found layer {}", layer.digest().as_string());
        let blob = layer.fetch()?;

        info!("Blob retrieved, extracting ...");
        blob.extract(&cli.output_dir)?;

        info!("Done");
    }

    Ok(())
}
