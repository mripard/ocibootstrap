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
