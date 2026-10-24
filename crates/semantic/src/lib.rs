//! Search on meaning: finding mail about what the person means, also when the words differ
//! ("verbouwing" finds a message about "de aannemer en de nieuwe keuken").
//!
//! A small multilingual embedding model (multilingual-e5-small, MIT licence) turns the subject
//! and the start of the text of every message into a vector; a search turns its words into one
//! too, and the messages whose vectors point the same way are the ones about the same thing.
//! Everything runs on this computer, on the CPU, in pure Rust (candle). Nothing is sent
//! anywhere; the model is downloaded once, only when the person switches this on, from a fixed
//! revision and checked against its SHA-256 before it is used.
//!
//! The downloaded model knows a hundred languages. Before it is kept, everything Dutch and
//! English mail cannot use is cut away (words in other scripts) and the weights are stored at
//! half precision, which brings 470 MB down to about a fifth.

pub mod model;
pub mod setup;
pub mod vectors;
pub mod worker;

pub use model::{DIM, Embedder, Kind};
pub use setup::Stage;
pub use vectors::Vectors;
pub use worker::{Reply, Request, Worker};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Model(String),
    #[error("download: {0}")]
    Download(String),
    #[error("the downloaded file {0} is not the one expected")]
    Mismatch(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl From<candle_core::Error> for Error {
    fn from(e: candle_core::Error) -> Self {
        Error::Model(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
