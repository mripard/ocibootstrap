#![allow(clippy::multiple_crate_versions)]
#![doc = include_str!("../../README.md")]

use std::{
    fs::{self, File},
    io::{self, Write},
    os::fd::AsFd,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use gpt::{GuidPartitionBuilder, GuidPartitionTableBuilder};
use layout::{Filesystem, GptPartitionTable, MbrPartitionTable, PartitionTable};
use local::{LocalManifest, LocalRegistry};
use log::{debug, error, info, log_enabled, trace, Level};
use loopdev::LoopControl;
use mbr::{MasterBootRecordPartitionBuilder, MasterBootRecordPartitionTableBuilder};
use serde::Deserialize;
use sys_mount::{FilesystemType, Mount, Unmount, UnmountFlags};
use tar::Archive;
use temp_dir::TempDir;
use types::{Architecture, OciBootstrapError, OperatingSystem};

mod config;
mod container;
mod layout;
mod local;

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

fn create_gpt(
    table: &GptPartitionTable,
    file: &mut File,
) -> Result<Vec<(Filesystem, PathBuf)>, OciBootstrapError> {
    let mut builder = GuidPartitionTableBuilder::new();
    for partition in table.partitions() {
        let mut part_builder = GuidPartitionBuilder::new(partition.uuid);

        if let Some(name) = &partition.name {
            part_builder = part_builder.name(name);
        }

        if let Some(size) = partition.size {
            part_builder = part_builder.size(size);
        }

        let part = part_builder
            .bootable(partition.bootable)
            .platform_required(partition.platform_required)
            .build();

        builder = builder.add_partition(part);
    }

    builder.build().write(file)?;
    file.flush()?;
    file.sync_all()?;

    Ok(table
        .partitions()
        .iter()
        .map(|p| (p.fs, p.mnt.clone()))
        .collect())
}

fn create_mbr(
    table: &MbrPartitionTable,
    file: &mut File,
) -> Result<Vec<(Filesystem, PathBuf)>, OciBootstrapError> {
    let mut builder = MasterBootRecordPartitionTableBuilder::new();
    for partition in table.partitions() {
        let mut part_builder = MasterBootRecordPartitionBuilder::new(partition.kind);

        if let Some(size) = partition.size {
            part_builder = part_builder.size(size);
        }

        let part = part_builder.bootable(partition.bootable).build();

        builder = builder.add_partition(part);
    }

    builder.build().write(file)?;
    file.flush()?;
    file.sync_all()?;

    Ok(table
        .partitions()
        .iter()
        .map(|p| (p.fs, p.mnt.clone()))
        .collect())
}

fn create_and_mount_loop_device(
    mut file: File,
    partition_table: PartitionTable,
) -> Result<MountPoints, OciBootstrapError> {
    let partitions = match partition_table {
        PartitionTable::Gpt(table) => create_gpt(&table, &mut file)?,
        PartitionTable::Mbr(table) => create_mbr(&table, &mut file)?,
    };

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

            match part_desc.0 {
                Filesystem::Fat32(p) => {
                    let mut command = Command::new("mkfs.vfat");
                    let mut command_ref = &mut command;

                    debug!("Creating FAT32 partition on {}", part.display());

                    if let (Some(heads), Some(spt)) = (p.heads, p.sectors_per_track) {
                        let geometry = format!("{heads}/{spt}");

                        debug!("FAT32 Geometry uses {heads} heads, {spt} sectors per track");

                        command_ref = command_ref.args(["-g", &geometry]);
                    }

                    if let Some(vol_id) = p.volume_id {
                        let id = format!("{vol_id:x}");

                        debug!("FAT32 Volume ID is {id}");

                        command_ref = command_ref.args(["-i", &id]);
                    }

                    let output = command_ref.arg(part.as_os_str()).output()?;
                    if !output.status.success() {
                        unimplemented!();
                    }
                }
                Filesystem::Ext4(p) => {
                    let mut command = Command::new("mkfs.ext4");
                    let mut command_ref = &mut command;

                    debug!("Creating EXT4 partition on {}", part.display());

                    if let Some(uuid) = p.uuid {
                        let uuid = uuid.to_string();

                        debug!("EXT4 UUID is {uuid}");

                        command_ref = command_ref.args(["-U", &uuid]);
                    }

                    let output = command_ref.arg(part.as_os_str()).output()?;

                    if !output.status.success() {
                        unimplemented!();
                    }
                }
            };

            let mount_point = part_desc.1.clone();
            debug!(
                "Partition {} Mounted on {}",
                part.display(),
                mount_point.display()
            );

            Ok((part, mount_point))
        })
        .collect::<Result<Vec<_>, io::Error>>()?;

    mount_points.sort_by(|a, b| Ord::cmp(&a.1, &b.1));

    let mounts = mount_points
        .into_iter()
        .map(|(part, target_mnt)| {
            let mount_dir = join_path(&output_dir, &target_mnt)?;
            MountPoint::new(&part, &mount_dir)
        })
        .collect::<Result<Vec<_>, io::Error>>()?;

    Ok(MountPoints {
        _loopdev: loop_device,
        dir: temp_dir,
        mnts: mounts,
    })
}

fn write_manifest_to_dir(
    manifest: &LocalManifest<'_>,
    dir: &Path,
) -> Result<(), OciBootstrapError> {
    fs::create_dir_all(dir)?;

    for layer in manifest.layers()? {
        info!("Found layer {}, extracting...", layer.digest());
        let reader = layer.archive()?;

        debug!("Got the archive. Extracting...");

        let mut archive = Archive::new(reader);
        archive.set_overwrite(true);
        archive.set_preserve_mtime(true);
        archive.set_preserve_ownerships(true);
        archive.set_preserve_permissions(true);
        archive.set_unpack_xattrs(true);

        archive.unpack(dir)?;

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

            let registry = LocalRegistry::new()?;
            let image = registry
                .image_by_spec(&container_spec)
                .context("Couldn't find image in registry")?;

            debug!("Found Image {} in our local storage", container_spec);

            let manifest = image
                .manifest_for_platform(cli.arch, OperatingSystem::default())?
                .context("Couldn't find manifest")?;

            let file = File::options().read(true).write(true).open(&output)?;
            let mounts = create_and_mount_loop_device(file, manifest.configuration().try_into()?)?;
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

            let registry = LocalRegistry::new()?;
            let image = registry
                .image_by_spec(&container_spec)
                .context("Couldn't find image in registry")?;

            debug!("Found Image {} in our local storage", container_spec);

            let manifest = image
                .manifest_for_platform(cli.arch, OperatingSystem::default())?
                .context("Couldn't find manifest")?;

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
