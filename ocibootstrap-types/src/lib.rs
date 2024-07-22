use std::io;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Connection Failure")]
    Connection(#[from] reqwest::Error),

    #[error("I/O Error")]
    Io(#[from] io::Error),

    #[error("JSON Parsing Failure")]
    Json(#[from] serde_json::Error),

    #[error("Configuration File Format Error")]
    Toml(#[from] toml::de::Error),

    #[error("Invalid URL")]
    Url(#[from] url::ParseError),

    #[error("Error: {0}")]
    Custom(String),
}
