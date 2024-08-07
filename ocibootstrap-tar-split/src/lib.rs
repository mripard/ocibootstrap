#![allow(missing_docs)]

use std::{
    ffi::OsString,
    fs::File,
    io::{self, BufReader},
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

use base64::Engine;
use crc::{Crc, Digest as CrcDigest, CRC_64_GO_ISO};
use flate2::bufread::GzDecoder;
use log::debug;
use serde::{de, Deserialize};
use serde_json::{de::IoRead, StreamDeserializer, Value};

#[derive(Debug, Deserialize)]
pub enum EntryName {
    Raw(Vec<u8>),
    String(String),
}

fn base64_decode<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Ok(Vec::new());
    }

    base64::engine::general_purpose::STANDARD
        .decode(
            value
                .as_str()
                .ok_or(de::Error::custom("Value isn't a string"))?,
        )
        .map_err(de::Error::custom)
}

fn u64_base64_decode<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let bytes = base64_decode(deserializer)?;
    if bytes.len() == 0 {
        return Ok(None);
    }

    Ok(Some(u64::from_be_bytes(bytes.try_into().unwrap())))
}

#[derive(Debug, Deserialize)]
pub struct FileEntry {
    name: Option<String>,
    name_raw: Option<Vec<u8>>,
    size: Option<u64>,
    #[serde(deserialize_with = "u64_base64_decode", rename = "payload")]
    checksum: Option<u64>,
    position: usize,
}

#[derive(Debug, Deserialize)]
pub struct SegmentEntry {
    name: Option<String>,
    name_raw: Option<Vec<u8>>,
    #[serde(deserialize_with = "base64_decode")]
    payload: Vec<u8>,
    position: usize,
}

#[derive(Debug)]
pub enum Entry {
    File(FileEntry),
    Segment(SegmentEntry),
}

impl Entry {
    pub fn name(&self) -> OsString {
        match self {
            Entry::File(f) => {
                if let Some(name) = &f.name {
                    return OsString::from(name);
                }

                if let Some(raw) = &f.name_raw {
                    return OsString::from_vec(raw.clone());
                }

                unreachable!()
            }
            Entry::Segment(s) => {
                if let Some(name) = &s.name {
                    return OsString::from(name);
                }

                if let Some(raw) = &s.name_raw {
                    return OsString::from_vec(raw.clone());
                }

                unreachable!()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Entry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        const TYPE_KEY: &str = "type";

        let mut value = Value::deserialize(deserializer)?;
        let map = value
            .as_object_mut()
            .ok_or(de::Error::invalid_type(de::Unexpected::Seq, &"a map"))?;

        let entry_kind = map
            .remove(TYPE_KEY)
            .ok_or(de::Error::missing_field(TYPE_KEY))?
            .as_u64()
            .ok_or(de::Error::invalid_type(
                de::Unexpected::Other("something other than a u64"),
                &"a u64",
            ))?;

        Ok(match entry_kind {
            1 => Self::File(FileEntry::deserialize(value).map_err(de::Error::custom)?),
            2 => Self::Segment(SegmentEntry::deserialize(value).map_err(de::Error::custom)?),
            _ => unimplemented!(),
        })
    }
}

const TAR_SPLIT_CRC: Crc<u64> = Crc::<u64>::new(&CRC_64_GO_ISO);

enum TarSplitRemainer {
    File(File, u64, CrcDigest<'static, u64>, Option<u64>),
    Segment(Vec<u8>),
}

#[allow(missing_debug_implementations)]
pub struct TarSplitReader<'de, R>
where
    R: io::Read,
{
    base: PathBuf,
    iter: StreamDeserializer<'de, IoRead<R>, Entry>,
    rem: Option<TarSplitRemainer>,
}

impl<R> io::Read for TarSplitReader<'_, R>
where
    R: io::Read,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let buf_len = buf.len();

        debug!("Reading next entry to a buffer of {buf_len} bytes");

        if let Some(rem) = self.rem.take() {
            match rem {
                TarSplitRemainer::File(mut f, mut size, mut digest, expected_checksum) => {
                    debug!("Remainder: Found a remaining file ({size} bytes left).");

                    let used = f.read(buf)?;
                    if used > 0 {
                        size -= used as u64;
                        digest.update(&buf[0..used]);
                        debug!("Remainder: Read {used} bytes, {size} left.");

                        self.rem = Some(TarSplitRemainer::File(f, size, digest, expected_checksum));
                        return Ok(used);
                    }
                    debug!("Remainder: Reached End-of-File, checking integrity");

                    if let Some(cksum) = expected_checksum {
                        let crc = digest.finalize();

                        if crc != cksum {
                            debug!(
                                "Remainder: CRC mismatch: actual {:x} vs expected {:x}",
                                crc, cksum
                            );

                            unimplemented!();
                        }
                    }

                    debug!("Remainder: Done. Going to the next entry.");
                }
                TarSplitRemainer::Segment(mut s) => {
                    debug!(
                        "Remainder: Found a remaining segment ({} bytes left)",
                        s.len()
                    );

                    let len = if buf_len < s.len() { buf_len } else { s.len() };
                    assert!(len > 0);

                    let bytes: Vec<_> = s.drain(0..len).collect();
                    buf[0..len].clone_from_slice(&bytes);

                    if s.len() != 0 {
                        debug!("Remainder: {} bytes left, postponing the rest", s.len());
                        self.rem = Some(TarSplitRemainer::Segment(s));
                    }

                    debug!("Remainder: Read {len} bytes");
                    return Ok(len);
                }
            }
        }

        while let Some(o) = self.iter.next() {
            let entry = o?;

            match &entry {
                Entry::File(f) => {
                    if let Some(len) = f.size {
                        debug!(
                            "Position {}: File Entry {}, size {len} bytes",
                            f.position,
                            entry.name().to_string_lossy()
                        );

                        let path = self.base.join(entry.name());
                        debug!("Opening File {}", path.display());

                        let mut file = File::open(&path)?;
                        let metadata = file.metadata()?;

                        debug!(
                            "Position {}: File opened, actual size {}",
                            f.position,
                            metadata.len()
                        );

                        if let Some(size) = f.size {
                            assert_eq!(metadata.len(), size);
                        }

                        let used = file.read(buf)?;
                        debug!("Position {}: Read {used} bytes", f.position);

                        let mut digest = TAR_SPLIT_CRC.digest();
                        digest.update(&buf[0..used]);

                        if used == buf_len {
                            debug!("File is larger than buffer, postponing the rest");
                            self.rem = Some(TarSplitRemainer::File(
                                file,
                                metadata.len() - used as u64,
                                digest,
                                f.checksum,
                            ));

                            return Ok(used);
                        }

                        if let Some(checksum) = f.checksum {
                            let crc = digest.finalize();

                            if crc != checksum {
                                debug!(
                                    "Position {}: CRC mismatch: actual {:x} vs expected {:x}",
                                    f.position, crc, checksum
                                );

                                unimplemented!();
                            }
                        }

                        return Ok(used);
                    } else {
                        debug!(
                            "Found File Entry {} with no size. Skipping.",
                            entry.name().to_string_lossy()
                        );
                        continue;
                    }
                }
                Entry::Segment(f) => {
                    debug!("Position {}: Found Segment entry. Extracting.", f.position);

                    let payload_len = f.payload.len();

                    if buf_len >= payload_len {
                        debug!("Position {}: Buffer is large enough to hold the payload (buf {buf_len}, payload {payload_len})", f.position);
                        buf[0..payload_len].copy_from_slice(&f.payload);
                        debug!("Position {}: Returned length {payload_len}", f.position);

                        return Ok(payload_len);
                    } else {
                        debug!("Position {}: Buffer is too small to hold the payload (buf {buf_len}, payload {payload_len}). Postponing the rest.", f.position);

                        let mut payload = f.payload.clone();
                        let bytes: Vec<_> = payload.drain(0..buf_len).collect();
                        buf.copy_from_slice(&bytes);
                        self.rem = Some(TarSplitRemainer::Segment(payload));
                        debug!("Position {}: Returned length {buf_len}", f.position);
                        return Ok(buf_len);
                    }
                }
            }
        }

        debug!("No entries left");
        Ok(0)
    }
}

pub fn from_reader<'de, R>(base: &Path, reader: R) -> TarSplitReader<'de, R>
where
    R: io::Read,
{
    TarSplitReader {
        base: base.to_path_buf(),
        iter: StreamDeserializer::new(IoRead::new(reader)),
        rem: None,
    }
}

pub fn from_path<'de>(base: &Path, path: &Path) -> TarSplitReader<'de, Box<dyn io::Read>> {
    let kind = infer::get_from_path(path).unwrap().unwrap();

    let file = File::open(path).unwrap();
    let bufread = BufReader::new(file);
    let reader: Box<dyn io::Read> = match kind.mime_type() {
        "application/gzip" => {
            let reader = GzDecoder::new(bufread);

            Box::new(reader)
        }
        _ => unimplemented!(),
    };

    from_reader(base, reader)
}
