#![doc = include_str!("../README.md")]
#![allow(clippy::multiple_crate_versions)]

use core::str::FromStr;
use std::{
    fs::File,
    io::{self, BufReader, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use flate2::bufread::GzDecoder;
use log::debug;
use reqwest::{blocking::Client, header::WWW_AUTHENTICATE, StatusCode};
use serde::{de, Deserialize};
use serde_json::Value;
use tar::Archive;
use types::{Architecture, OciBootstrapError};
use url::Url;

mod spec;
use spec::{
    auth::AuthenticateHeader,
    v2::{
        self,
        oci::{ImageManifest, TagsListResponse},
    },
};

const DIGEST_KEY: &str = "digest";
const SCHEMA_VERSION_KEY: &str = "schemaVersion";
const SIZE_KEY: &str = "size";

#[derive(Debug, Deserialize)]
pub(crate) struct Rfc6750AuthResponse {
    pub(crate) token: String,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum CompressionAlgorithm {
    None,
    Gzip,
    Zstd,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DigestAlgorithm {
    Sha256,
    Sha512,
}

/// A Digest Representation
#[derive(Clone, Debug)]
pub struct Digest {
    digest: DigestAlgorithm,
    bytes: Vec<u8>,
}

impl Digest {
    /// Returns the digest as a String
    #[must_use]
    pub fn as_string(&self) -> String {
        hex::encode(&self.bytes)
    }

    /// Returns the digest as a String, with the OCI representation
    #[must_use]
    pub fn as_oci_string(&self) -> String {
        match self.digest {
            DigestAlgorithm::Sha256 => format!("sha256:{}", self.as_string()),
            DigestAlgorithm::Sha512 => format!("sha512:{}", self.as_string()),
        }
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        let (alg, dig) = s.split_once(':').ok_or(de::Error::invalid_value(
            de::Unexpected::Str(&s),
            &"a digest with the form $ALGO:$DIGEST",
        ))?;

        let bytes = hex::decode(dig).map_err(de::Error::custom)?;

        Ok(match alg {
            "sha256" => Self {
                digest: DigestAlgorithm::Sha256,
                bytes,
            },
            "sha512" => Self {
                digest: DigestAlgorithm::Sha512,
                bytes,
            },
            _ => unimplemented!(),
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ManifestInner {
    SchemaV2(v2::Manifest),
}

impl<'de> Deserialize<'de> for ManifestInner {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let map = value
            .as_object()
            .ok_or(de::Error::invalid_type(de::Unexpected::Seq, &"a map"))?;

        let schema = map
            .get(SCHEMA_VERSION_KEY)
            .ok_or(de::Error::missing_field(SCHEMA_VERSION_KEY))?
            .as_u64()
            .ok_or(de::Error::invalid_type(
                de::Unexpected::Other("something other than an integer"),
                &"an integer",
            ))?;

        Ok(match schema {
            2 => Self::SchemaV2(v2::Manifest::deserialize(value).map_err(de::Error::custom)?),
            _ => unimplemented!(),
        })
    }
}

/// A Container Registry Representation
#[derive(Debug)]
pub struct Registry {
    index_url: Url,
    auth: Option<AuthenticateHeader>,
}

impl Registry {
    /// Connects to a remote container registry
    ///
    /// # Errors
    ///
    /// Returns an error if the given registry URL is malformed, or if the connection fails.
    pub fn connect(registry: &str) -> Result<Self, OciBootstrapError> {
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
                    OciBootstrapError::Custom(String::from("Couldn't decode header as a String"))
                })?;

                debug!("www-authenticate header is {www_authenticate}");

                let auth = AuthenticateHeader::from_str(www_authenticate).map_err(|_e| {
                    OciBootstrapError::Custom(String::from("Couldn't parse header."))
                })?;

                Ok(Self {
                    index_url: repo_base_url,
                    auth: Some(auth),
                })
            }
            _ => unimplemented!(),
        }
    }

    /// Looks up the image name on the registry
    ///
    /// # Errors
    ///
    /// Returns an error if the registry connection fails
    pub fn image<'a>(&'a self, name: &str) -> Result<Image<'a>, OciBootstrapError> {
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

/// A Container Image Representation
#[derive(Debug)]
pub struct Image<'a> {
    pub(crate) registry: &'a Registry,
    pub(crate) name: String,
    pub(crate) token: Option<String>,
}

impl Image<'_> {
    /// Returns the latest tag available for our image
    ///
    /// # Errors
    ///
    /// Returns an error when the connection to the Registry fails, or if it cannot be found
    pub fn latest(&self) -> Result<Tag<'_>, OciBootstrapError> {
        self.tags()?
            .into_iter()
            .find(|t| t == "latest")
            .ok_or(OciBootstrapError::Custom(String::from(
                "Latest tag can't be found",
            )))
    }

    /// Returns all available tags for our image
    ///
    /// # Errors
    ///
    /// Returns an error if the connection to the Registry fails
    pub fn tags(&self) -> Result<Vec<Tag<'_>>, OciBootstrapError> {
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

/// A Container Tag Representation
#[derive(Debug)]
pub struct Tag<'a> {
    image: &'a Image<'a>,
    tag_name: String,
}

impl<'a> Tag<'a> {
    /// Returns the image manifest for our tag for the given architecture and OS
    ///
    /// # Errors
    ///
    /// Returns an error if the connection to the Registry fails, or if no manifest for the given
    /// platform can be found.
    pub fn manifest_for_config(
        &'a self,
        arch: Architecture,
        os: &str,
    ) -> Result<Manifest<'a>, OciBootstrapError> {
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
            .header("Accept", v2::docker::DISTRIBUTION_MANIFEST_MIME_TYPE)
            .header("Accept", v2::oci::IMAGE_INDEX_MIME_TYPE)
            .header("Accept", v2::oci::IMAGE_MANIFEST_MIME_TYPE);

        if let Some(token) = &self.image.token {
            client = client.header("Authorization", format!("Bearer {token}"));
        }

        let text = client.send()?.error_for_status()?.text()?;

        debug!("Manifest Response {text}");

        let resp: ManifestInner = serde_json::from_str(&text)?;

        let manifest = match &resp {
            ManifestInner::SchemaV2(s) => match s {
                v2::Manifest::Docker(_) => Manifest {
                    image: self.image,
                    inner: resp,
                },
                v2::Manifest::OciManifest(_) => unimplemented!(),
                v2::Manifest::OciIndex(m) => {
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
                                Err(e) => {
                                    return Some(Err::<ImageManifest, OciBootstrapError>(e.into()))
                                }
                            };

                            debug!("Manifest URL {}", url);

                            let mut client = Client::new()
                                .get(url)
                                .header("Accept", v2::oci::IMAGE_MANIFEST_MIME_TYPE);

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
                        .ok_or(OciBootstrapError::Custom(String::from(
                            "No manifest found for the requested platform.",
                        )))??;

                    Manifest {
                        image: self.image,
                        inner: ManifestInner::SchemaV2(v2::Manifest::OciManifest(manifest)),
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

/// A Container Layer Representation
#[derive(Debug)]
pub struct ManifestLayer<'a> {
    image: &'a Image<'a>,
    inner: v2::ImageLayer,
}

impl ManifestLayer<'_> {
    /// Returns the Layer digest
    #[must_use]
    pub fn digest(&self) -> Digest {
        match &self.inner {
            v2::ImageLayer::DockerImage(v) => &v.digest,
            v2::ImageLayer::OciImage(v) => &v.digest,
        }
        .clone()
    }

    /// Returns the Layer size
    #[must_use]
    pub fn size(&self) -> usize {
        match &self.inner {
            v2::ImageLayer::DockerImage(v) => v.size,
            v2::ImageLayer::OciImage(v) => v.size,
        }
    }

    fn try_from_cache(&self, path: &Path) -> Result<Option<LocalBlob>, OciBootstrapError> {
        if !path.exists() {
            return Ok(None);
        }

        debug!("File already exists, checking its size");

        let metadata = path.metadata()?;

        if metadata.size() != self.size() as u64 {
            return Err(OciBootstrapError::Custom(String::from(
                "File exists but doesn't match",
            )));
        }

        Ok(Some(LocalBlob {
            path: path.to_owned(),
            compression: self.inner.compression(),
        }))
    }

    /// Fetches the Layer
    ///
    /// # Errors
    ///
    /// Returns an error if the connection to the Registry fails, or if there's any error accessing
    /// the local file.
    pub fn fetch(&self) -> Result<LocalBlob, OciBootstrapError> {
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
            .map_err(<xdg::BaseDirectoriesError as Into<io::Error>>::into)?
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

/// Representation of an Image Manifest
#[derive(Clone, Debug)]
pub struct Manifest<'a> {
    image: &'a Image<'a>,
    inner: ManifestInner,
}

impl Manifest<'_> {
    /// Returns the laryers  part of that Manifest
    #[must_use]
    pub fn layers(&self) -> Vec<ManifestLayer<'_>> {
        match &self.inner {
            ManifestInner::SchemaV2(s) => match s {
                v2::Manifest::Docker(m) => m
                    .layers
                    .clone()
                    .into_iter()
                    .map(|v| ManifestLayer {
                        image: self.image,
                        inner: v2::ImageLayer::DockerImage(v),
                    })
                    .collect(),
                v2::Manifest::OciManifest(m) => m
                    .layers
                    .clone()
                    .into_iter()
                    .map(|l| ManifestLayer {
                        image: self.image,
                        inner: v2::ImageLayer::OciImage(l),
                    })
                    .collect(),
                v2::Manifest::OciIndex(_) => unreachable!(),
            },
        }
    }
}

/// Representation of an OCI Blob stored locally
#[derive(Debug)]
pub struct LocalBlob {
    path: PathBuf,
    compression: CompressionAlgorithm,
}

impl LocalBlob {
    /// Extracts the content of a compressed blob into the given target directory
    ///
    /// # Errors
    ///
    /// If the backing file cannot be opened, or if it cannot be extracted
    pub fn extract(self, target_dir: &Path) -> Result<(), OciBootstrapError> {
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
