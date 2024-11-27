#![doc = include_str!("../README.md")]

use core::fmt;
use std::{
    ffi::{OsStr, OsString},
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
    if bytes.is_empty() {
        return Ok(None);
    }

    let array: [u8; size_of::<u64>()] = bytes
        .try_into()
        .map_err(|_err| de::Error::custom("Couldn't convert bytes array to u64"))?;
    let val = u64::from_be_bytes(array);

    Ok(Some(val))
}

fn decode_name<'de, D>(deserializer: D) -> Result<Option<OsString>, D::Error>
where
    D: de::Deserializer<'de>,
{
    Ok(Value::deserialize(deserializer)?.as_str().map(Into::into))
}

fn decode_name_raw<'de, D>(deserializer: D) -> Result<Option<OsString>, D::Error>
where
    D: de::Deserializer<'de>,
{
    Ok(Value::deserialize(deserializer)?
        .as_str()
        .map(|s| base64::engine::general_purpose::STANDARD.decode(s))
        .transpose()
        .map_err(de::Error::custom)?
        .map(OsString::from_vec))
}

#[derive(Debug, Deserialize)]
struct FileEntry {
    #[serde(default, deserialize_with = "decode_name")]
    name: Option<OsString>,
    #[serde(default, deserialize_with = "decode_name_raw")]
    name_raw: Option<OsString>,
    size: Option<u64>,
    #[serde(deserialize_with = "u64_base64_decode", rename = "payload")]
    checksum: Option<u64>,
    position: usize,
}

#[derive(Debug, Deserialize)]
struct SegmentEntry {
    #[serde(deserialize_with = "base64_decode")]
    payload: Vec<u8>,
    position: usize,
}

#[derive(Debug)]
enum Entry {
    File(FileEntry),
    Segment(SegmentEntry),
}

impl Entry {
    #[must_use]
    fn name(&self) -> &OsStr {
        match self {
            Entry::File(f) => {
                if let Some(name) = &f.name {
                    return name;
                }

                if let Some(raw) = &f.name_raw {
                    return raw;
                }

                unreachable!()
            }
            Entry::Segment(_) => {
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

impl fmt::Debug for TarSplitRemainer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TarSplitRemainer::File(_file, size, _digest, checksum) => {
                if let Some(checksum) = checksum {
                    write!(
                        f,
                        "Remainer: File remaining size {size}, expected checksum {checksum}",
                    )
                } else {
                    write!(f, "Remainer: File remaining size {size}")
                }
            }
            TarSplitRemainer::Segment(bytes) => write!(f, "Remainer: {bytes:#x?}"),
        }
    }
}

enum TarSplitRemainerStatus {
    Continue,
    Handled(usize),
    None,
}

/// Reader over a checksum-reproducible tar archive
pub struct TarSplitReader<'de, R>
where
    R: io::Read,
{
    base: PathBuf,
    iter: StreamDeserializer<'de, IoRead<R>, Entry>,
    rem: Option<TarSplitRemainer>,
}

impl<R> TarSplitReader<'_, R>
where
    R: io::Read,
{
    fn handle_remainder(&mut self, buf: &mut [u8]) -> io::Result<TarSplitRemainerStatus> {
        let buf_len = buf.len();

        if let Some(rem) = self.rem.take() {
            match rem {
                TarSplitRemainer::File(mut f, mut size, mut digest, expected_checksum) => {
                    debug!("Remainder: Found a remaining file ({size} bytes left).");

                    let used = io::Read::read(&mut f, buf)?;
                    if used > 0 {
                        size -= used as u64;
                        digest.update(&buf[0..used]);
                        debug!("Remainder: Read {used} bytes, {size} left.");

                        self.rem = Some(TarSplitRemainer::File(f, size, digest, expected_checksum));
                        return Ok(TarSplitRemainerStatus::Handled(used));
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
                    Ok(TarSplitRemainerStatus::Continue)
                }
                TarSplitRemainer::Segment(mut s) => {
                    debug!(
                        "Remainder: Found a remaining segment ({} bytes left)",
                        s.len()
                    );

                    let len = if buf_len < s.len() { buf_len } else { s.len() };
                    debug!("Remainder: Draining {len} bytes");

                    if len == 0 {
                        return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                    }

                    let bytes: Vec<_> = s.drain(0..len).collect();
                    buf[0..len].clone_from_slice(&bytes);

                    if !s.is_empty() {
                        debug!("Remainder: {} bytes left, postponing the rest", s.len());
                        self.rem = Some(TarSplitRemainer::Segment(s));
                    }

                    debug!("Remainder: Read {len} bytes");
                    Ok(TarSplitRemainerStatus::Handled(len))
                }
            }
        } else {
            Ok(TarSplitRemainerStatus::None)
        }
    }
}

impl<R> fmt::Debug for TarSplitReader<'_, R>
where
    R: io::Read,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TarSplitReader")
            .field("base", &self.base)
            .field("remainder", &self.rem)
            .finish_non_exhaustive()
    }
}

impl<R> io::Read for TarSplitReader<'_, R>
where
    R: io::Read,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let buf_len = buf.len();

        debug!("Reading next entry to a buffer of {buf_len} bytes");

        match self.handle_remainder(buf)? {
            TarSplitRemainerStatus::Handled(used) => return Ok(used),
            TarSplitRemainerStatus::Continue | TarSplitRemainerStatus::None => {}
        }

        for o in self.iter.by_ref() {
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
                            if metadata.len() != size {
                                return Err(io::Error::from(io::ErrorKind::InvalidData));
                            }
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
                    }

                    debug!(
                        "Found File Entry {} with no size. Skipping.",
                        entry.name().to_string_lossy()
                    );
                    continue;
                }
                Entry::Segment(f) => {
                    debug!("Position {}: Found Segment entry. Extracting.", f.position);

                    let payload_len = f.payload.len();

                    if buf_len >= payload_len {
                        debug!("Position {}: Buffer is large enough to hold the payload (buf {buf_len}, payload {payload_len})", f.position);
                        buf[0..payload_len].copy_from_slice(&f.payload);
                        debug!("Position {}: Returned length {payload_len}", f.position);

                        return Ok(payload_len);
                    }

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

        debug!("No entries left");
        Ok(0)
    }
}

/// Returns a `TarSplitReader` from a Reader to the tar-split file
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

/// Returns a `TarSplitReader` from a Path to the tar-split File
///
/// # Errors
///
/// If the file isn't accessible, or in an unsupported format
pub fn from_path<'de>(
    base: &Path,
    path: &Path,
) -> Result<TarSplitReader<'de, Box<dyn io::Read>>, io::Error> {
    let kind = infer::get_from_path(path)?.ok_or(io::Error::from(io::ErrorKind::Unsupported))?;

    let file = File::open(path)?;
    let bufread = BufReader::new(file);
    let reader: Box<dyn io::Read> = Box::new(match kind.mime_type() {
        "application/gzip" => GzDecoder::new(bufread),
        _ => return Err(io::Error::from(io::ErrorKind::Unsupported)),
    });

    Ok(from_reader(base, reader))
}
