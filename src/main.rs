#![allow(dead_code)]
#![allow(missing_docs)]

use std::path::PathBuf;

use clap::Parser;
use log::info;
use oci_spec::Registry;
use simplelog::{ColorChoice, Config, LevelFilter, TermLogger, TerminalMode};

pub mod oci_spec {
    use std::{
        fs::File,
        io::{BufReader, Write},
        os::unix::fs::MetadataExt,
        path::{Path, PathBuf},
    };

    use flate2::bufread::GzDecoder;
    use json_structs::{ImageLayer, ImageManifest};
    use log::debug;
    use reqwest::{blocking::Client, StatusCode};
    use serde::Deserialize;
    use tar::Archive;
    use url::Url;

    pub mod json_structs {
        use std::result::Result;

        use serde::Deserialize;

        #[derive(Clone, Copy, Debug)]
        pub enum DigestAlgorithm {
            Sha256,
            Sha512,
        }

        #[derive(Clone, Debug, Deserialize)]
        #[serde(try_from = "String")]
        pub enum Digest {
            SHA256(String),
        }

        impl Digest {
            pub(crate) fn algorithm(&self) -> DigestAlgorithm {
                match self {
                    Self::SHA256(_) => DigestAlgorithm::Sha256,
                }
            }

            pub(crate) fn as_bytes(&self) -> Vec<u8> {
                match self {
                    Self::SHA256(s) => hex::decode(s).unwrap(),
                }
            }

            pub(crate) fn as_string(&self) -> String {
                match self {
                    Self::SHA256(s) => format!("{}", s),
                }
            }

            pub(crate) fn as_string_with_algorithm(&self) -> String {
                match self {
                    Self::SHA256(s) => format!("sha256:{}", s),
                }
            }
        }

        impl TryFrom<String> for Digest {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                let (alg, dig) = value.split_once(':').unwrap();

                Ok(match alg {
                    "sha256" => Self::SHA256(dig.to_string()),
                    _ => todo!(),
                })
            }
        }

        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct ImageConfig {
            #[serde(rename = "mediaType")]
            media_kind: String,

            size: usize,
            digest: Digest,
        }

        #[derive(Clone, Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct ImageLayer {
            #[serde(rename = "mediaType")]
            pub(crate) media_kind: String,
            pub(crate) size: usize,
            pub(crate) digest: Digest,
        }

        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct ImageManifest {
            #[serde(rename = "schemaVersion")]
            pub(crate) schema_version: u8,
            #[serde(rename = "mediaType")]
            pub(crate) media_kind: String,

            pub(crate) config: ImageConfig,
            pub(crate) layers: Vec<ImageLayer>,
        }
    }

    type Result<T> = std::result::Result<T, ()>;

    // #[derive(Debug, Deserialize_repr)]
    // #[repr(u8)]
    // enum SchemaVersion {
    //     V2 = 2,
    // }

    #[derive(Debug, Deserialize)]
    struct TokenResponse {
        token: String,
    }

    #[derive(Debug, Deserialize)]
    struct TagResponse(String);

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TagsListResponse {
        name: String,
        tags: Vec<TagResponse>,
    }

    // #[derive(Debug, Deserialize)]
    // #[serde(deny_unknown_fields)]
    // struct ConfigResponse {
    //     #[serde(rename = "mediaType")]
    //     media_kind: String,
    //     size: usize,
    //     digest: Digest,
    // }

    // #[derive(Debug, Deserialize)]
    // #[serde(deny_unknown_fields)]
    // struct LayerResponse {
    //     #[serde(rename = "mediaType")]
    //     media_kind: String,
    //     size: usize,
    //     digest: Digest,
    // }

    // #[derive(Debug, Deserialize)]
    // #[serde(deny_unknown_fields)]
    // struct ManifestResponse {
    //     #[serde(rename = "schemaVersion")]
    //     schema_version: SchemaVersion,

    //     #[serde(rename = "mediaType")]
    //     media_kind: String,

    //     config: ConfigResponse,
    //     layers: Vec<LayerResponse>,
    // }

    #[derive(Debug)]
    pub struct Registry {
        url: Url,
        auth: bool,
    }

    impl Registry {
        pub fn connect(domain: &str) -> Result<Self> {
            let repo_base_url = Url::parse(domain).unwrap();

            let test_url = repo_base_url.join("/v2/").unwrap();

            debug!("Trying to connect to {}", test_url.as_str());

            let resp = Client::new()
                .get(test_url)
                .send()
                .unwrap()
                .error_for_status();

            if let Err(e) = resp {
                let status = e.status().unwrap();
                if status == StatusCode::UNAUTHORIZED {
                    debug!("Registry is authenticated.");
                    Ok(Self {
                        url: repo_base_url,
                        auth: true,
                    })
                } else {
                    Err(())
                }
            } else {
                debug!("Unauthenticated Registry Found.");
                Ok(Self {
                    url: repo_base_url,
                    auth: false,
                })
            }
        }

        pub fn image<'a>(&'a self, name: &str) -> Result<Image<'a>> {
            let token = if self.auth {
                debug!("Registry is authenticated, getting a token.");

                let mut token_url = self.url.join("token").unwrap();

                token_url.set_query(Some(&format!("scope=repository:{}:pull", name)));

                debug!("Token URL: {}", token_url.as_str());

                let client = Client::new()
                    .get(token_url)
                    .send()
                    .unwrap()
                    .error_for_status()
                    .unwrap();

                let val: TokenResponse = client.json().unwrap();

                debug!("Got token: {}", val.token);

                Some(val.token)
            } else {
                None
            };

            Ok(Image {
                registry: self,
                name: name.to_string(),
                token,
            })
        }
    }

    #[derive(Debug)]
    pub struct Image<'a> {
        registry: &'a Registry,
        name: String,
        token: Option<String>,
    }

    impl Image<'_> {
        pub fn latest<'a>(&'a self) -> Result<Tag<'a>> {
            self.tags()?.into_iter().find(|t| t == "latest").ok_or(())
        }

        pub fn tags<'a>(&'a self) -> Result<Vec<Tag<'a>>> {
            let url = self
                .registry
                .url
                .join(&format!("/v2/{}/tags/list", self.name))
                .unwrap();

            debug!("Tags List URL: {}", url.as_str());

            let resp = if let Some(token) = &self.token {
                Client::new()
                    .get(url)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
            } else {
                Client::new().get(url).send()
            }
            .unwrap()
            .error_for_status()
            .unwrap();

            let tags = resp
                .json::<TagsListResponse>()
                .unwrap()
                .tags
                .iter()
                .map(|t| Tag {
                    image: self,
                    tag_name: t.0.clone(),
                })
                .collect();

            Ok(tags)
        }
    }

    #[derive(Debug)]
    pub struct Tag<'a> {
        image: &'a Image<'a>,
        tag_name: String,
    }

    impl<'a> Tag<'a> {
        pub fn manifest(&'a self) -> Result<Manifest<'a>> {
            let url = self
                .image
                .registry
                .url
                .join(&format!(
                    "/v2/{}/manifests/{}",
                    self.image.name, self.tag_name
                ))
                .unwrap();

            debug!("Manifest URL: {}", url.as_str());

            let resp = if let Some(token) = &self.image.token {
                Client::new()
                    .get(url)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
            } else {
                Client::new().get(url).send()
            }
            .unwrap()
            .error_for_status()
            .unwrap();

            let manifest: ImageManifest = resp.json().unwrap();
            assert_eq!(manifest.schema_version, 2);
            assert_eq!(
                manifest.media_kind,
                "application/vnd.docker.distribution.manifest.v2+json"
            );

            Ok(Manifest {
                image: self.image,
                inner: manifest,
            })
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

    #[derive(Debug)]
    pub struct Manifest<'a> {
        image: &'a Image<'a>,
        inner: ImageManifest,
    }

    impl<'a> Manifest<'a> {
        pub fn layers(&self) -> Vec<Layer<'a>> {
            self.inner
                .layers
                .clone()
                .into_iter()
                .map(|v| Layer {
                    image: self.image,
                    inner: v,
                })
                .collect()
        }
    }

    #[derive(Debug)]
    pub struct Layer<'a> {
        pub(crate) image: &'a Image<'a>,
        pub(crate) inner: ImageLayer,
    }

    impl Layer<'_> {
        pub fn fetch(&self) -> LocalBlob {
            let url = self
                .image
                .registry
                .url
                .join(&format!(
                    "/v2/{}/blobs/{}",
                    self.image.name,
                    self.inner.digest.as_string_with_algorithm()
                ))
                .unwrap();

            debug!("Blob URL {}", url.as_str());

            let resp = if let Some(token) = &self.image.token {
                Client::new()
                    .get(url)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
            } else {
                Client::new().get(url).send()
            }
            .unwrap()
            .error_for_status()
            .unwrap();

            let dir_path = xdg::BaseDirectories::new()
                .unwrap()
                .create_cache_directory(env!("CARGO_CRATE_NAME"))
                .unwrap();

            let file_path = dir_path.join(self.inner.digest.as_string());
            debug!("Blob File Path {}", file_path.display());

            if file_path.exists() {
                debug!("File already exists, checking its size");

                let metadata = file_path.metadata().unwrap();
                assert_eq!(metadata.size() as usize, self.inner.size);

                // debug!("Size match, checking hash.");

                // let mut file = File::open(&file_path).unwrap();
                // let hash = match self.inner.digest.algorithm() {
                //     DigestAlgorithm::Sha256 => {
                //         let mut hasher = Sha256::new();
                //         std::io::copy(&mut file, &mut hasher).unwrap();
                //         hasher.finalize()
                //     }
                //     DigestAlgorithm::Sha512 => todo!(),
                // };

                // debug!("Computed Hash is {:X?}", hash.as_slice());
                // assert_eq!(hash.as_slice(), self.inner.digest.as_bytes());

                return LocalBlob(file_path);
            }

            let mut file = File::create_new(file_path.clone()).unwrap();
            file.write_all(&resp.bytes().unwrap()).unwrap();

            LocalBlob(file_path)
        }
    }

    #[derive(Debug)]
    pub struct LocalBlob(PathBuf);

    impl LocalBlob {
        pub fn extract(self, target_dir: &Path) {
            let blob = File::open(&self.0).unwrap();
            let blob_reader = BufReader::new(blob);

            let tar = GzDecoder::new(blob_reader);
            let mut archive = Archive::new(tar);

            archive.set_overwrite(true);
            archive.set_preserve_mtime(true);
            archive.set_preserve_ownerships(true);
            archive.set_preserve_permissions(true);
            archive.set_unpack_xattrs(true);

            archive.unpack(target_dir).unwrap();
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

    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() {
    let cli = Cli::parse();

    TermLogger::init(
        match cli.verbose {
            0 => LevelFilter::Info,
            1 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        },
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Never,
    )
    .unwrap();

    info!(
        "Running {} {}",
        env!("CARGO_CRATE_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    let registry = Registry::connect("https://ghcr.io").unwrap();

    let image = registry.image("mripard/fedora-silverblue-image").unwrap();
    let tag = image.latest().unwrap();
    let manifest = tag.manifest().unwrap();

    std::fs::create_dir_all(&cli.output_dir).unwrap();

    for layer in manifest.layers() {
        info!("Found layer {}", layer.inner.digest.as_string());
        let blob = layer.fetch();

        info!("Blob retrieved, extracting ...");
        blob.extract(&cli.output_dir);

        info!("Done");
    }
}
