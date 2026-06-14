//! The memory layer: embedding pipeline, memory lifecycle, and context assembly.

mod answer;
mod chunk;
mod context;
pub mod indexing;
pub mod signals;

pub use answer::{AnswerConfig, AnswerResult, AnswerService, GenerateError};
pub use chunk::chunk_note;
pub use context::{assemble_context, AssembledContext, Candidate, ContextItem};
pub use signals::DefaultSignalBooster;
