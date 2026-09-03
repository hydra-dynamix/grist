//! OCR request, attributed region result, and adapter trait.

use super::{
    ProviderConfidence, ProviderContractError, ProviderError, ProviderMetadata, ProviderRequest,
    ProviderRequestContext, ProviderRequestManifest, ProviderResult, parameters_digest,
};
use crate::core::{BoundingBox, Diagnostic, Provider, ProviderKind, SourceLocator};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OcrOptions {
    #[serde(default)]
    pub language_hints: Vec<String>,
    #[serde(default)]
    pub recognize_layout: bool,
    #[serde(default)]
    pub recognize_tables: bool,
    /// Optional page/frame/region scope within a compound provider input.
    ///
    /// The raw provider input remains unchanged; this locator makes repeated
    /// calls over one PDF or multi-frame image explicit and digest-bearing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_locator: Option<SourceLocator>,
}

pub struct OcrRequest<'a> {
    pub context: ProviderRequestContext<'a>,
    pub options: OcrOptions,
}

impl<'a> OcrRequest<'a> {
    pub fn new(context: ProviderRequestContext<'a>, options: OcrOptions) -> Self {
        Self { context, options }
    }

    pub fn manifest(&self) -> Result<ProviderRequestManifest, ProviderContractError> {
        Ok(ProviderRequestManifest {
            kind: ProviderKind::Ocr,
            input_identity: self.context.input().identity().clone(),
            network_access: self.context.network_access(),
            configuration_digest: self.context.configuration_digest().into(),
            parameters_digest: parameters_digest(&self.options)?,
        })
    }
}

impl std::fmt::Debug for OcrRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OcrRequest")
            .field("context", &self.context)
            .field("options", &self.options)
            .finish()
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrRegion {
    pub text: String,
    pub bounding_box: BoundingBox,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ProviderConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading_order: Option<u64>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OcrResult {
    pub text: String,
    #[serde(default)]
    pub regions: Vec<OcrRegion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ProviderConfidence>,
    /// Provider-assigned confidence in the returned region ordering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading_order_confidence: Option<ProviderConfidence>,
    /// Provider-assigned confidence in region boundaries and layout geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_confidence: Option<ProviderConfidence>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

impl OcrResult {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        for region in &self.regions {
            region
                .bounding_box
                .validate("ocr_region")
                .map_err(|error| ProviderContractError::InvalidResult(error.to_string()))?;
        }
        let mut orders: Vec<_> = self
            .regions
            .iter()
            .filter_map(|region| region.reading_order)
            .collect();
        let before = orders.len();
        orders.sort_unstable();
        orders.dedup();
        if orders.len() != before {
            return Err(ProviderContractError::InvalidResult(
                "OCR reading order values must be unique".into(),
            ));
        }
        Ok(())
    }
}

pub trait OcrProvider: Send + Sync + 'static {
    fn recognize(&self, request: &OcrRequest<'_>) -> Result<OcrResult, ProviderError>;
}

pub struct OcrProviderAdapter<P> {
    metadata: ProviderMetadata,
    implementation: P,
}

impl<P> OcrProviderAdapter<P> {
    pub fn new(
        metadata: ProviderMetadata,
        implementation: P,
    ) -> Result<Self, ProviderContractError> {
        metadata.validate()?;
        Ok(Self {
            metadata,
            implementation,
        })
    }
}

impl<P: OcrProvider> Provider for OcrProviderAdapter<P> {
    fn name(&self) -> &str {
        &self.metadata.name
    }

    fn kind(&self) -> Option<ProviderKind> {
        Some(ProviderKind::Ocr)
    }

    fn metadata(&self) -> Option<&ProviderMetadata> {
        Some(&self.metadata)
    }

    fn invoke(&self, request: &ProviderRequest<'_>) -> Result<ProviderResult, ProviderError> {
        let ProviderRequest::Ocr(request) = request else {
            return Err(ProviderError::failure(
                &self.metadata.name,
                "OCR provider received a different request kind",
            ));
        };
        self.implementation
            .recognize(request)
            .map(ProviderResult::Ocr)
    }
}
