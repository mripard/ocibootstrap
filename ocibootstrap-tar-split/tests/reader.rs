use std::{
    fs::File,
    io::{self, Seek},
    path::{Path, PathBuf},
};

use flate2::read::GzDecoder;
use log::debug;
use ocibootstrap_tar_split::from_path;
use tar::Archive;
use tempfile::{NamedTempFile, TempDir};
use test_log::test;

fn test_archive_fn(archive_path: &Path) {
    debug!("Running with archive {}", archive_path.display());

    let archive_gz = File::open(&archive_path).unwrap();
    let mut archive_gz_reader = GzDecoder::new(archive_gz);

    let temp_dir = TempDir::new().unwrap();

    debug!("Temporary Dir is {}", temp_dir.path().display());

    let archive_dec_path = temp_dir
        .path()
        .join(archive_path.with_extension("").file_name().unwrap());

    debug!("Unzipping archive to {}", archive_dec_path.display());

    let mut archive_dec = File::create_new(&archive_dec_path).unwrap();
    io::copy(&mut archive_gz_reader, &mut archive_dec).unwrap();

    archive_dec.seek(io::SeekFrom::Start(0)).unwrap();
    let expected_sha = sha256::try_digest(&archive_dec_path).unwrap();

    debug!("Expected Archive SHA-256 is {}", expected_sha);

    let base_dir = temp_dir.path().join("base");

    debug!("Extracting archive content to {}", base_dir.display());

    let mut archive = Archive::new(&archive_dec);
    archive.unpack(&base_dir).unwrap();

    let json_path = archive_path.parent().unwrap().join("tar-data.json.gz");
    let mut reader = from_path(&base_dir, &json_path).unwrap();

    let mut archive = NamedTempFile::new().unwrap();
    io::copy(&mut reader, &mut archive).unwrap();

    assert_eq!(sha256::try_digest(&archive.path()).unwrap(), expected_sha);
}

#[test]
fn test_t() {
    test_archive_fn(&PathBuf::from("./tests/data/t/t.tar.gz"));
}

#[test]
fn test_longlink() {
    test_archive_fn(&PathBuf::from("./tests/data/longlink/longlink.tar.gz"));
}

#[test]
fn test_fatlonglink() {
    test_archive_fn(&PathBuf::from(
        "./tests/data/fatlonglink/fatlonglink.tar.gz",
    ));
}

#[test]
fn test_iso_8859() {
    test_archive_fn(&PathBuf::from("./tests/data/iso-8859/iso-8859.tar.gz"));
}

#[test]
fn test_extranils() {
    test_archive_fn(&PathBuf::from("./tests/data/extranils/extranils.tar.gz"));
}

#[test]
fn test_notenoughnils() {
    test_archive_fn(&PathBuf::from(
        "./tests/data/notenoughnils/notenoughnils.tar.gz",
    ));
}

#[test]
fn test_1c51fc286aa95d9413226599576bafa38490b1e292375c90de095855b64caea6() {
    test_archive_fn(&PathBuf::from(
        "./tests/data/1c51fc286aa95d9413226599576bafa38490b1e292375c90de095855b64caea6/1c51fc286aa95d9413226599576bafa38490b1e292375c90de095855b64caea6",
    ));
}
