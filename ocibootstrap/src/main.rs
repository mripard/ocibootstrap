#![allow(clippy::multiple_crate_versions)]
#![doc = include_str!("../../README.md")]

use std::{
    fs::{self, File},
    io::{self, Write},
    os::fd::AsFd,
    path::{Path, PathBuf},
    process::Command,
};

use clap::Parser;
use gpt::{
    GuidPartitionBuilder, GuidPartitionTableBuilder, EFI_SYSTEM_PART_GUID,
    EXTENDED_BOOTLOADER_PART_GUID, ROOT_PART_GUID_ARM64,
};
use log::{debug, error, info, log_enabled, trace, Level};
use loopdev::LoopControl;
use registry::{Manifest, Registry};
use serde::Deserialize;
use sys_mount::{FilesystemType, Mount, Unmount, UnmountFlags};
use temp_dir::TempDir;
use types::{Architecture, OciBootstrapError};

mod config;
mod container;
mod utils;

use crate::{container::ContainerSpec, utils::get_current_oci_os};

const DOCKER_HUB_REGISTRY_URL_STR: &str = "https://index.docker.io";

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

fn find_device_parts(file: &Path) -> Result<Vec<PathBuf>, OciBootstrapError> {
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

fn create_and_mount_loop_device(file: File) -> Result<MountPoints, OciBootstrapError> {
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

fn write_manifest_to_dir(manifest: &Manifest<'_>, dir: &Path) -> Result<(), OciBootstrapError> {
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
