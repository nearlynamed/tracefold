//! Immutable TraceFold archive writer, reader, and verifier.

mod codec;
mod raw_codec;
mod reader;
mod writer;

pub use reader::{Archive, ArchiveError, InspectResult, RetainedClass, VerificationReport};
pub use writer::{EncodeOptions, EncodeResult, Layout, encode};

pub const ARCHIVE_FORMAT_VERSION: u16 = 2;
pub const VIEW_FORMAT_VERSION: u16 = 2;
