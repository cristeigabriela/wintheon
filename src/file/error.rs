//! Errors surfaced by [`FileEntry`](super::FileEntry) operations.

use core::result;
use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T> = result::Result<T, Error>;
