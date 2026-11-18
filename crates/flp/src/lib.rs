//! Reader/writer for FL Studio project files (`.flp`).
//!
//! The format is undocumented. The layout implemented here follows the
//! community reverse-engineering effort (most notably PyFLP).
//!
//! Two layers are provided:
//! * [`FlpFile`] - a lossless list of raw events that can be written back
//!   byte-for-byte.
//! * [`FlpProject`] - a high-level, typed view (channels, patterns, notes,
//!   playlist, mixer) extracted from those events.

mod event;
mod project;

pub use event::*;
pub use project::*;
