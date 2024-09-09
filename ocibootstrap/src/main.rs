#![allow(clippy::multiple_crate_versions)]
#![doc = include_str!("../../README.md")]

use std::{
    fs::{self, File},
    io::{self, Write},
    os::fd::AsFd,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::bail;
use clap::{Parser, Subcommand};
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
use types::{Architecture, OciBootstrapError, OperatingSystem};
use uuid::Uuid;

mod config;
mod container;

use crate::container::ContainerSpec;

#[derive(Debug, Subcommand)]
enum CliSubcommand {
    Device {
        #[arg(help = "Container Name")]
        container: String,

        #[arg(help = "Output Device File")]
        output: PathBuf,
    },
    Directory {
        #[arg(help = "Container Name")]
        container: String,

        #[arg(help = "Output Directory")]
        output: PathBuf,
    },
}

#[derive(Parser)]
#[command(version, about = "OCI Image to Device Utility")]
struct Cli {
    #[arg(short, long, default_value_t, help = "Architecture")]
    arch: Architecture,

    #[clap(subcommand)]
    command: CliSubcommand,
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
    host_mnt: Mount,
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
            host_mnt: mount,
        })
    }
}

impl Drop for MountPoint {
    fn drop(&mut self) {
        debug!(
            "Unmounting {} from {}",
            self.dev.display(),
            self.host_mnt.target_path().display()
        );

        let res = self.host_mnt.unmount(UnmountFlags::DETACH);
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

fn create_and_mount_loop_device(
    mut file: File,
    partitions: &'static [Partition],
) -> Result<MountPoints, OciBootstrapError> {
    let mut gpt_builder = GuidPartitionTableBuilder::new();
    for partition in partitions {
        let mut part_builder = GuidPartitionBuilder::new(partition.uuid);

        if let Some(name) = partition.name {
            part_builder = part_builder.name(name);
        }

        if let Some(size) = partition.size {
            part_builder = part_builder.size(size);
        }

        let part = part_builder
            .bootable(partition.bootable)
            .platform_required(partition.platform_required)
            .build();

        gpt_builder = gpt_builder.add_partition(part);
    }

    gpt_builder.build().write(&file)?;
    file.flush()?;
    file.sync_all()?;

    let loop_control = LoopControl::open()?;
    let loop_device = LoopDevice::create(&loop_control, file)?;

    let temp_dir = TempDir::new()?;
    let output_dir = temp_dir.path().to_path_buf();
    debug!("Temp output dir is {}", output_dir.display());

    let mut mount_points = find_device_parts(&loop_device.path())?
        .into_iter()
        .enumerate()
        .map(|(idx, part)| {
            let part_desc = &partitions[idx];

            match part_desc.fs {
                Filesystem::Fat32 => {
                    let output = Command::new("mkfs.vfat").arg(part.as_os_str()).output()?;

                    if !output.status.success() {
                        unimplemented!();
                    }
                }
                Filesystem::Ext4 => {
                    let output = Command::new("mkfs.ext4").arg(part.as_os_str()).output()?;

                    if !output.status.success() {
                        unimplemented!();
                    }
                }
            };

            let mount_point = PathBuf::from(part_desc.mnt);
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

#[derive(Debug)]
enum Filesystem {
    Fat32,
    Ext4,
}

#[derive(Debug)]
struct Partition {
    uuid: Uuid,
    name: Option<&'static str>,
    mnt: &'static str,
    size: Option<u64>,
    fs: Filesystem,
    bootable: bool,
    platform_required: bool,
}

const PARTITIONS_LAYOUT: &[Partition] = &[
    Partition {
        uuid: EFI_SYSTEM_PART_GUID,
        name: Some(EFI_SYSTEM_PART_NAME),
        mnt: "/boot/efi",
        size: Some(256 << 20),
        fs: Filesystem::Fat32,
        bootable: true,
        platform_required: true,
    },
    Partition {
        uuid: EXTENDED_BOOTLOADER_PART_GUID,
        name: Some(BOOT_PART_NAME),
        mnt: "/boot",
        size: Some(512 << 20),
        fs: Filesystem::Ext4,
        bootable: false,
        platform_required: false,
    },
    Partition {
        uuid: ROOT_PART_GUID_ARM64,
        name: Some(ROOT_PART_NAME),
        mnt: "/",
        size: None,
        fs: Filesystem::Ext4,
        bootable: false,
        platform_required: false,
    },
];

fn write_manifest_to_dir(manifest: &Manifest<'_>, dir: &Path) -> Result<(), OciBootstrapError> {
    fs::create_dir_all(dir)?;

    for layer in manifest.layers() {
        info!("Found layer {}", layer.digest());
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

    match cli.command {
        CliSubcommand::Device { output, container } => {
            let container_spec = ContainerSpec::from_container_name(&container)?;

            info!(
                "Using container {} with output device {}",
                container_spec.to_oci_string(),
                output.display()
            );

            if !output.exists() {
                bail!("Output file doesn't exist.");
            }

            let metadata = output.metadata()?;
            let file_type = metadata.file_type();
            if !file_type.is_file() {
                bail!("Output argument isn't a file");
            }

            let registry = Registry::connect(&container_spec.domain)?;
            let image = registry.image(&container_spec.name)?;
            debug!("Found Image {}", image.name());

            let tag = image.latest()?;
            info!("Found Tag {}", tag.name());

            let manifest = tag.manifest_for_config(cli.arch, OperatingSystem::default())?;

            let file = File::options().read(true).write(true).open(&output)?;
            let mounts = create_and_mount_loop_device(file, PARTITIONS_LAYOUT)?;
            write_manifest_to_dir(&manifest, mounts.dir.path())?;

            drop(mounts);
            Ok(())
        }
        CliSubcommand::Directory { output, container } => {
            let container_spec = ContainerSpec::from_container_name(&container)?;

            info!(
                "Using container {} with output directory {}",
                container_spec.to_oci_string(),
                output.display()
            );

            if !output.exists() {
                debug!("Output directory doesn't exist, creating.");
                fs::create_dir_all(&output)?;
            }

            if !output.is_dir() {
                bail!("Output isn't a directory");
            }

            let registry = Registry::connect(&container_spec.domain)?;
            let image = registry.image(&container_spec.name)?;
            debug!("Found Image {}", image.name());

            let tag = image.latest()?;
            info!("Found Tag {}", tag.name());

            let manifest = tag.manifest_for_config(cli.arch, OperatingSystem::default())?;

            write_manifest_to_dir(&manifest, &output)?;
            Ok(())
        }
    }
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
