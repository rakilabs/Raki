//! The memory layer: embedding pipeline, memory lifecycle, and context assembly.

mod context;

pub use context::{assemble_context, AssembledContext, Candidate, ContextItem};
