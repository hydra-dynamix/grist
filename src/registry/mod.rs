//! Deterministic parser, provider, and isolated-backend registration.
//!
//! The registry owns selection, budget/cancellation gates, provider access, and
//! envelope construction. Extensions return parser output, never an unchecked
//! public envelope.

mod builtins;
mod metadata;
mod parser;
mod provider;

pub use builtins::builtin_parser_registry;
pub use metadata::{
    Capability, Determinism, FormatMetadata, IsolationMetadata, NetworkPolicy, OptionsMetadata,
    ParserDescriptor, ParserOrigin, ProviderDescriptor, RegistrySnapshot, SchemaMetadata,
    UnavailableParser, UnavailableReason,
};
pub use parser::{
    Parser, ParserContext, ParserDispatchError, ParserError, ParserOutput, ParserRegistry,
    ParserRegistryError, ParserSelection,
};
pub use provider::{
    IsolatedBackendRegistry, ProviderRegistry, ProviderRegistryError, ProviderSelection,
    builtin_provider_registry,
};

impl RegistrySnapshot {
    pub fn from_registries(parsers: &ParserRegistry, providers: &ProviderRegistry) -> Self {
        Self {
            parsers: parsers.parsers(),
            unavailable_parsers: parsers.unavailable_parsers(),
            providers: providers.providers(),
        }
    }
}
