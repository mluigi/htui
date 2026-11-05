//! htui's own log (MOD-122): what every `tracing` event that passed the global filter becomes.
//!
//! [`buffer`] keeps the last [`CAPACITY`] lines in memory for the Logs tab; [`source`] maps an
//! event's target to the part of htui it came from.

pub mod buffer;
pub mod source;

pub use buffer::{CAPACITY, Counts, LogBuffer, LogLine, MemoryLayer, Since, SourceCounts};
pub use source::Source;
