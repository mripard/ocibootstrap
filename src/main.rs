#![allow(clippy::multiple_crate_versions)]
#![doc = include_str!("../README.md")]

extern crate alloc;

use core::str::FromStr;
use std::{
    fs::{self, File},
    io::{self, BufReader, Write},
    os::{fd::AsFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
    process::Command,
};

use clap::Parser;
use flate2::bufread::GzDecoder;
use gpt::{
    GuidPartitionBuilder, GuidPartitionTableBuilder, EFI_SYSTEM_PART_GUID,
    EXTENDED_BOOTLOADER_PART_GUID, ROOT_PART_GUID_ARM64,
};
use log::{debug, error, info, log_enabled, trace, Level};
use loopdev::LoopControl;
use our_types::Error;
use reqwest::{blocking::Client, header::WWW_AUTHENTICATE, StatusCode};
use serde::Deserialize;
use spec::v2::oci::ImageManifest;
use sys_mount::{FilesystemType, Mount, Unmount, UnmountFlags};
use tar::Archive;
use temp_dir::TempDir;
use test_log as _;
use url::Url;

mod config;
mod container;
mod spec;
mod types;
mod utils;

use crate::{
    container::ContainerSpec,
    spec::{auth::AuthenticateHeader, v2::oci::TagsListResponse, Digest, Rfc6750AuthResponse},
    types::CompressionAlgorithm,
    utils::{get_current_oci_os, Architecture},
};

const DOCKER_HUB_REGISTRY_URL_STR: &str = "https://index.docker.io";

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
    #[arg(short, long, default_value_t, help = "Architecture")]
    arch: Architecture,

    #[arg(help = "Container Name")]
    container: String,

    #[arg(help = "Output Directory")]
    output: PathBuf,
}

const EFI_SYSTEM_PART_NAME: &str = "esp";
const BOOT_PART_NAME: &str = "boot";
const ROOT_PART_NAME: &str = "root";

#[derive(Debug)]
struct LoopDevice {
    loopdev: loopdev::LoopDevice,
    _file: File,
}

impl LoopDevice {
    pub(crate) fn create(ctrl: &LoopControl, file: File) -> Result<Self, io::Error> {
        let loop_device = ctrl.next_free()?;

        if log_enabled!(Level::Debug) {
            debug!(
                "Using loop device {}",
                loop_device
                    .path()
                    .ok_or(io::Error::new(
                        io::ErrorKind::NotFound,
                        "Loop Device File Not Found"
                    ))?
                    .display()
            );
        }

        loop_device.with().part_scan(true).attach_fd(file.as_fd())?;

        debug!("Attached the loop device to our file");

        Ok(Self {
            loopdev: loop_device,
            _file: file,
        })
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.loopdev
            .path()
            .expect("Couldn't retrieve the loop device path")
    }
}

impl Drop for LoopDevice {
    fn drop(&mut self) {
        debug!("Destroying our loop device");

        let res = self.loopdev.detach();
        if let Err(e) = res {
            error!("Couldn't detach the Loop Device: {}", e);
        }

        debug!("Loop device detached");
    }
}

#[derive(Debug)]
struct MountPoint {
    dev: PathBuf,
    mnt: Mount,
}

impl MountPoint {
    fn new(dev: &Path, mnt: &Path) -> Result<Self, io::Error> {
        debug!("Mounting {} on {}", dev.display(), mnt.display());

        fs::create_dir_all(mnt)?;

        let mount = Mount::builder()
            .fstype(FilesystemType::Set(&["ext4", "vfat"]))
            .mount(dev, mnt)?;

        trace!("Mount Successful");

        Ok(Self {
            dev: dev.to_path_buf(),
            mnt: mount,
        })
    }
}

impl Drop for MountPoint {
    fn drop(&mut self) {
        debug!(
            "Unmounting {} from {}",
            self.dev.display(),
            self.mnt.target_path().display()
        );

        let res = self.mnt.unmount(UnmountFlags::DETACH);
        if let Err(e) = res {
            error!("Couldn't unmount {}: {e}", self.dev.display());
        }
    }
}

#[derive(Debug)]
struct MountPoints {
    mnts: Vec<MountPoint>,
    dir: TempDir,
    _loopdev: LoopDevice,
}

impl Drop for MountPoints {
    fn drop(&mut self) {
        while let Some(item) = self.mnts.pop() {
            drop(item);
        }
    }
}

fn find_device_parts(file: &Path) -> Result<Vec<PathBuf>, Error> {
    #[derive(Debug, Deserialize)]
    struct LsblkPartition {
        path: PathBuf,
    }

    #[derive(Debug, Deserialize)]
    struct LsblkDevice {
        #[serde(rename = "children")]
        parts: Vec<LsblkPartition>,
    }

    #[derive(Debug, Deserialize)]
    struct LsblkOutput {
        #[serde(rename = "blockdevices")]
        devices: Vec<LsblkDevice>,
    }

    let output = Command::new("lsblk")
        .args(["--bytes", "--json", "--paths", "--output-all"])
        .arg(file.as_os_str())
        .output()?;

    let res: LsblkOutput = serde_json::from_slice(&output.stdout)?;

    Ok(res.devices[0]
        .parts
        .iter()
        .map(|p| p.path.clone())
        .collect())
}

fn is_dir_in_root(root: &Path, path: &Path) -> bool {
    debug!("Checking if {} is in {}", path.display(), root.display());

    if let Ok(p) = path.canonicalize() {
        debug!("File can be canonicalized: {}", p.display());

        return p.starts_with(root);
    }

    if let Some(p) = path.parent() {
        is_dir_in_root(root, p)
    } else {
        false
    }
}

fn join_path(root: &Path, path: &Path) -> Result<PathBuf, io::Error> {
    let joined = if path.is_absolute() {
        let mut joined = root.to_path_buf();

        for part in path.components() {
            match part {
                std::path::Component::Prefix(_) => unreachable!(),
                std::path::Component::RootDir | std::path::Component::CurDir => {}
                std::path::Component::ParentDir => joined.push(".."),
                std::path::Component::Normal(c) => joined.push(c),
            }
        }

        joined
    } else {
        root.join(path)
    };

    debug!("Joined Path {}", joined.display());

    let canonical = match joined.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            if e.kind() == io::ErrorKind::NotFound {
                debug!(
                    "File {} doesn't exist... Checking if its parent exists in the root dir",
                    joined.display()
                );

                if is_dir_in_root(root, &joined) {
                    debug!("File ancestors in chroot.. Returning");
                    return Ok(joined);
                }
            }

            return Err(e);
        }
    };

    debug!("Canonicalized Path {}", canonical.display());

    if !canonical.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path isn't contained in root",
        ));
    }

    Ok(canonical)
}

#[cfg(test)]
mod chroot_test {
    use std::{
        fs::{self, File},
        os,
        path::PathBuf,
    };

    use temp_dir::TempDir;
    use test_log::test;

    use crate::join_path;

    const ROOT_CANARY_DIR: &str = "canary";

    const ROOT_TEST_DIR: &str = "test";
    const TEST_CANARY_SUBDIR: &str = "canary";
    const TEST_TEST_SUBDIR: &str = "testdir";

    fn create_directories() -> TempDir {
        let root_dir = TempDir::new().unwrap();
        let root = root_dir.path();

        fs::create_dir(root.join(ROOT_CANARY_DIR)).unwrap();
        let canary = root.join(ROOT_CANARY_DIR);
        File::create(canary.join("canary-test-file.txt")).unwrap();

        fs::create_dir(root.join(ROOT_TEST_DIR)).unwrap();
        let test = root_dir.path().join(ROOT_TEST_DIR);

        File::create(root.join("root-test-file.txt")).unwrap();

        os::unix::fs::symlink(root.join(ROOT_CANARY_DIR), test.join(TEST_CANARY_SUBDIR)).unwrap();
        File::create(test.join("test.txt")).unwrap();
        fs::create_dir(test.join(TEST_TEST_SUBDIR)).unwrap();

        let test_dir = test.join(TEST_TEST_SUBDIR);
        File::create(test_dir.join("test.txt")).unwrap();

        root_dir
    }

    #[test]
    fn test_absolute_file() {
        let root_dir = create_directories();
        let root = root_dir.path().join(ROOT_TEST_DIR);

        assert_eq!(
            join_path(&root, &PathBuf::from("/test.txt")).unwrap(),
            root.join("test.txt")
        );
    }

    #[test]
    fn test_absolute_file_missing() {
        let root_dir = create_directories();
        let root = root_dir.path().join("test");

        assert_eq!(
            join_path(&root, &PathBuf::from("/not-there.txt")).unwrap(),
            root.join("not-there.txt")
        );
    }

    #[test]
    fn test_absolute_file_dir_missing() {
        let root_dir = create_directories();
        let root = root_dir.path().join("test");

        assert_eq!(
            join_path(&root, &PathBuf::from("/invalid/not-there.txt")).unwrap(),
            root.join("invalid/not-there.txt")
        );
    }

    #[test]
    fn test_absolute_file_outside_missing() {
        let root_dir = create_directories();
        let root = root_dir.path().join("test");

        join_path(&root, &PathBuf::from("/testdir/../../../not-there.txt")).unwrap_err();
    }

    #[test]
    fn test_absolute_file_outside_symlink() {
        let root_dir = create_directories();
        let root = root_dir.path().join("test");

        join_path(&root, &PathBuf::from("/canary/not-there.txt")).unwrap_err();
    }

    #[test]
    fn test_relative_file() {
        let root_dir = create_directories();
        let root = root_dir.path().join("test");

        assert_eq!(
            join_path(&root, &PathBuf::from("test.txt")).unwrap(),
            root.join("test.txt")
        );
    }

    #[test]
    fn test_relative_dir_file() {
        let root_dir = create_directories();
        let root = root_dir.path().join("test");

        assert_eq!(
            join_path(&root, &PathBuf::from("testdir/test.txt")).unwrap(),
            root.join("testdir/test.txt")
        );
    }

    #[test]
    fn test_relative_file_outside() {
        let root_dir = create_directories();
        let root = root_dir.path().join("test");

        join_path(&root, &PathBuf::from("../root-test-file.txt")).unwrap_err();
    }

    #[test]
    fn test_relative_file_symlink() {
        let root_dir = create_directories();
        let root = root_dir.path().join("test");

        join_path(&root, &PathBuf::from("canary/canary-test-file.txt")).unwrap_err();
    }
}

fn create_and_mount_loop_device(file: File) -> Result<MountPoints, Error> {
    let loop_control = LoopControl::open()?;
    let loop_device = LoopDevice::create(&loop_control, file)?;

    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path().to_path_buf();
    debug!("Temp output dir is {}", output_dir.display());

    let mut mount_points = find_device_parts(&loop_device.path())?
        .into_iter()
        .enumerate()
        .map(|(idx, part)| {
            match idx {
                0 => {
                    let output = Command::new("mkfs.vfat").arg(part.as_os_str()).output()?;

                    if !output.status.success() {
                        unimplemented!();
                    }
                }
                1 | 2 => {
                    let output = Command::new("mkfs.ext4").arg(part.as_os_str()).output()?;

                    if !output.status.success() {
                        unimplemented!();
                    }
                }
                _ => unimplemented!(),
            };

            let mount_point = PathBuf::from(match idx {
                0 => "/boot/efi",
                1 => "/boot",
                2 => "/",
                _ => unimplemented!(),
            });

            debug!(
                "Partition {} Mounted on {}",
                part.display(),
                mount_point.display()
            );

            Ok((part, mount_point))
        })
        .collect::<Result<Vec<_>, io::Error>>()?;

    mount_points.sort_by(|a, b| Ord::cmp(&a.1.components().count(), &b.1.components().count()));

    let mounts = mount_points
        .into_iter()
        .map(|(part, mount)| {
            let mount_dir = join_path(&output_dir, &mount)?;
            MountPoint::new(&part, &mount_dir)
        })
        .collect::<Result<Vec<_>, io::Error>>()?;

    Ok(MountPoints {
        _loopdev: loop_device,
        dir: temp_dir,
        mnts: mounts,
    })
}

fn write_manifest_to_dir(manifest: &TestManifest<'_>, dir: &Path) -> Result<(), Error> {
    fs::create_dir_all(dir)?;

    for layer in manifest.layers() {
        info!("Found layer {}", layer.digest().as_string());
        let blob = layer.fetch()?;

        info!("Blob retrieved, extracting ...");
        blob.extract(dir)?;

        info!("Done");
    }

    Ok(())
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
    let manifest = tag.manifest_for_config(cli.arch, get_current_oci_os())?;

    if !cli.output.exists() || cli.output.is_dir() {
        write_manifest_to_dir(&manifest, &cli.output)?;
    } else if cli.output.is_file() {
        let mut file = File::options().read(true).write(true).open(&cli.output)?;

        GuidPartitionTableBuilder::new()
            .add_partition(
                GuidPartitionBuilder::new(EFI_SYSTEM_PART_GUID)
                    .name(EFI_SYSTEM_PART_NAME)
                    .size(256 << 20)
                    .platform_required(true)
                    .bootable(true)
                    .build(),
            )
            .add_partition(
                GuidPartitionBuilder::new(EXTENDED_BOOTLOADER_PART_GUID)
                    .name(BOOT_PART_NAME)
                    .size(512 << 20)
                    .build(),
            )
            .add_partition(
                GuidPartitionBuilder::new(match cli.arch {
                    Architecture::Arm64 => ROOT_PART_GUID_ARM64,
                    Architecture::Arm | Architecture::X86 | Architecture::X86_64 => {
                        unimplemented!()
                    }
                })
                .name(ROOT_PART_NAME)
                .build(),
            )
            .build()
            .write(&file)?;
        file.flush()?;
        file.sync_all()?;

        let mounts = create_and_mount_loop_device(file)?;
        write_manifest_to_dir(&manifest, mounts.dir.path())?;

        drop(mounts);
    } else {
        unimplemented!();
    }

    Ok(())
}
