use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::MAX_FILE_BYTES;

/// Why the configuration could not be used. Every variant names the file,
/// because the host shows it to someone editing that file.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The file exists but could not be read.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The file is larger than [`MAX_FILE_BYTES`].
    #[error("{}: larger than {MAX_FILE_BYTES} bytes", path.display())]
    TooLarge {
        /// The file.
        path: PathBuf,
    },
    /// The text is not UTF-8 TOML of the shape the schema describes.
    #[error("{}: {message}", path.display())]
    Invalid {
        /// The file.
        path: PathBuf,
        /// The parser's or the validator's complaint, with line and column
        /// where it has them.
        message: String,
    },
}
