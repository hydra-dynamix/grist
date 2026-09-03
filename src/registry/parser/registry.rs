use super::super::{ParserDescriptor, ParserOrigin, RegistrySnapshot, UnavailableParser};
use super::Parser;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum ParserSelection {
    Available(Box<ParserDescriptor>),
    Unsupported {
        format: String,
        unavailable: Vec<UnavailableParser>,
    },
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ParserRegistryError {
    #[error("invalid parser registration: {0}")]
    Invalid(String),
    #[error("duplicate parser registration: {0}")]
    DuplicateId(String),
    #[error("selector conflicts with another canonical format: {0}")]
    SelectorConflict(String),
    #[error("format already has a parser at this priority: {0}")]
    PriorityConflict(String),
    #[error("registration must have caller origin: {0}")]
    InvalidCallerOrigin(String),
}

pub(super) struct ParserEntry {
    pub descriptor: ParserDescriptor,
    pub parser: Arc<dyn Parser>,
}

#[derive(Default)]
pub struct ParserRegistry {
    pub(super) entries: BTreeMap<String, ParserEntry>,
    pub(super) unavailable: BTreeMap<String, UnavailableParser>,
    format_selectors: BTreeMap<String, String>,
    media_type_selectors: BTreeMap<String, String>,
    extension_selectors: BTreeMap<String, String>,
}

impl ParserRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn register_caller(
        &mut self,
        descriptor: ParserDescriptor,
        parser: Arc<dyn Parser>,
    ) -> Result<(), ParserRegistryError> {
        if descriptor.origin != ParserOrigin::Caller {
            return Err(ParserRegistryError::InvalidCallerOrigin(descriptor.id));
        }
        self.register(descriptor, parser)
    }

    pub(crate) fn register_builtin(
        &mut self,
        descriptor: ParserDescriptor,
        parser: Arc<dyn Parser>,
    ) -> Result<(), ParserRegistryError> {
        self.register(descriptor, parser)
    }

    pub(crate) fn register_unavailable(
        &mut self,
        unavailable: UnavailableParser,
    ) -> Result<(), ParserRegistryError> {
        validate_descriptor(&unavailable.descriptor)?;
        let id = unavailable.descriptor.id.clone();
        if self.entries.contains_key(&id) || self.unavailable.contains_key(&id) {
            return Err(ParserRegistryError::DuplicateId(id));
        }
        self.install_selectors(&unavailable.descriptor)?;
        self.unavailable.insert(id, unavailable);
        Ok(())
    }

    fn register(
        &mut self,
        descriptor: ParserDescriptor,
        parser: Arc<dyn Parser>,
    ) -> Result<(), ParserRegistryError> {
        validate_descriptor(&descriptor)?;
        if self.entries.contains_key(&descriptor.id)
            || self.unavailable.contains_key(&descriptor.id)
        {
            return Err(ParserRegistryError::DuplicateId(descriptor.id));
        }
        let format = normalize_format(&descriptor.format.id);
        if self.entries.values().any(|entry| {
            normalize_format(&entry.descriptor.format.id) == format
                && entry.descriptor.priority == descriptor.priority
        }) {
            return Err(ParserRegistryError::PriorityConflict(format));
        }
        self.install_selectors(&descriptor)?;
        self.entries
            .insert(descriptor.id.clone(), ParserEntry { descriptor, parser });
        Ok(())
    }

    fn install_selectors(
        &mut self,
        descriptor: &ParserDescriptor,
    ) -> Result<(), ParserRegistryError> {
        // Build the new indexes off to the side so a conflict cannot leave a
        // partially registered alias, media type, or extension behind.
        let mut format_selectors = self.format_selectors.clone();
        let mut media_type_selectors = self.media_type_selectors.clone();
        let mut extension_selectors = self.extension_selectors.clone();
        let format = normalize_format(&descriptor.format.id);
        install_selector(&mut format_selectors, format.clone(), &format)?;
        for alias in &descriptor.format.aliases {
            install_selector(&mut format_selectors, normalize_format(alias), &format)?;
        }
        for media_type in &descriptor.format.media_types {
            install_selector(
                &mut media_type_selectors,
                normalize_media_type(media_type),
                &format,
            )?;
        }
        for extension in &descriptor.format.extensions {
            install_selector(
                &mut extension_selectors,
                normalize_extension(extension),
                &format,
            )?;
        }
        self.format_selectors = format_selectors;
        self.media_type_selectors = media_type_selectors;
        self.extension_selectors = extension_selectors;
        Ok(())
    }

    pub fn select_format(&self, format: &str) -> ParserSelection {
        let normalized = normalize_format(format);
        let canonical = self
            .format_selectors
            .get(&normalized)
            .map(String::as_str)
            .unwrap_or(&normalized);
        self.select_canonical(canonical)
    }

    pub fn select_media_type(&self, media_type: &str) -> ParserSelection {
        let normalized = normalize_media_type(media_type);
        let canonical = self
            .media_type_selectors
            .get(&normalized)
            .map(String::as_str)
            .unwrap_or(&normalized);
        self.select_canonical(canonical)
    }

    pub fn select_extension(&self, extension: &str) -> ParserSelection {
        let normalized = normalize_extension(extension);
        let canonical = self
            .extension_selectors
            .get(&normalized)
            .map(String::as_str)
            .unwrap_or(&normalized);
        self.select_canonical(canonical)
    }

    pub fn select_parser(&self, parser_id: &str) -> ParserSelection {
        if let Some(entry) = self.entries.get(parser_id) {
            return ParserSelection::Available(Box::new(entry.descriptor.clone()));
        }
        ParserSelection::Unsupported {
            format: parser_id.to_string(),
            unavailable: self
                .unavailable
                .get(parser_id)
                .cloned()
                .into_iter()
                .collect(),
        }
    }

    fn select_canonical(&self, canonical: &str) -> ParserSelection {
        if let Some(entry) = self
            .entries
            .values()
            .filter(|entry| normalize_format(&entry.descriptor.format.id) == canonical)
            .max_by_key(|entry| entry.descriptor.priority)
        {
            return ParserSelection::Available(Box::new(entry.descriptor.clone()));
        }
        ParserSelection::Unsupported {
            format: canonical.to_string(),
            unavailable: self
                .unavailable
                .values()
                .filter(|entry| normalize_format(&entry.descriptor.format.id) == canonical)
                .cloned()
                .collect(),
        }
    }

    pub fn grammar_probes(&self, text: &str) -> Vec<super::GrammarProbe> {
        self.entries
            .values()
            .filter_map(|entry| entry.parser.grammar_probe(text))
            .collect()
    }

    pub fn parsers(&self) -> Vec<ParserDescriptor> {
        self.entries
            .values()
            .map(|entry| entry.descriptor.clone())
            .collect()
    }

    pub fn unavailable_parsers(&self) -> Vec<UnavailableParser> {
        self.unavailable.values().cloned().collect()
    }

    pub fn snapshot(&self) -> RegistrySnapshot {
        RegistrySnapshot {
            parsers: self.parsers(),
            unavailable_parsers: self.unavailable_parsers(),
            providers: Vec::new(),
        }
    }
}

fn validate_descriptor(descriptor: &ParserDescriptor) -> Result<(), ParserRegistryError> {
    let required = [
        descriptor.id.as_str(),
        descriptor.format.id.as_str(),
        descriptor.payload_schema.name.as_str(),
        descriptor.payload_schema.version.as_str(),
        descriptor.options.schema.name.as_str(),
        descriptor.options.schema.version.as_str(),
    ];
    if required
        .iter()
        .any(|value| value.trim().is_empty() || value.chars().any(char::is_control))
    {
        return Err(ParserRegistryError::Invalid(
            "required metadata is empty or contains control characters".to_string(),
        ));
    }
    descriptor
        .parser
        .validate()
        .map_err(|error| ParserRegistryError::Invalid(error.to_string()))?;
    if !descriptor.options.default.is_object() {
        return Err(ParserRegistryError::Invalid(
            "default options must be a JSON object".to_string(),
        ));
    }
    if !descriptor
        .required_providers
        .is_subset(&descriptor.allowed_providers)
    {
        return Err(ParserRegistryError::Invalid(
            "required providers must also be allowed".to_string(),
        ));
    }
    Ok(())
}

fn install_selector(
    selectors: &mut BTreeMap<String, String>,
    selector: String,
    format: &str,
) -> Result<(), ParserRegistryError> {
    if selector.is_empty() {
        return Err(ParserRegistryError::Invalid(selector));
    }
    if let Some(existing) = selectors.get(&selector)
        && existing != format
    {
        return Err(ParserRegistryError::SelectorConflict(selector));
    }
    selectors.insert(selector, format.to_string());
    Ok(())
}

pub(super) fn normalize_format(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

fn normalize_media_type(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or(value)
        .trim()
        .to_ascii_lowercase()
}

fn normalize_extension(value: &str) -> String {
    value.trim().trim_start_matches('.').to_ascii_lowercase()
}
