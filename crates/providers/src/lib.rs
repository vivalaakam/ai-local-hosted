//! OpenAI-compatible HTTP providers. Ollama is the first one; any `/v1` base URL uses the same adapter.

mod openai;

pub use openai::{OpenAiProvider, ProviderConfig};
