//! Native, provider-derived, and reconciled representation separation.

use super::{ProviderConfidence, ProviderContractError, ProviderResponse, ProviderResult};
use crate::core::{Diagnostic, canonical_json_sha256};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NativeRepresentation<N> {
    pub identity: String,
    pub value: N,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

impl<N: Serialize> NativeRepresentation<N> {
    pub fn new(value: N) -> Result<Self, ProviderContractError> {
        let identity = canonical_json_sha256(&value)
            .map_err(|error| ProviderContractError::Digest(error.to_string()))?;
        Ok(Self {
            identity,
            value,
            diagnostics: Vec::new(),
        })
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReconciledRepresentation<R> {
    pub identity: String,
    pub value: R,
    pub algorithm: String,
    pub algorithm_version: String,
    pub configuration_digest: String,
    pub native_input_identity: String,
    pub provider_output_identities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ProviderConfidence>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

impl<R: Serialize> ReconciledRepresentation<R> {
    pub fn new(
        value: R,
        algorithm: impl Into<String>,
        algorithm_version: impl Into<String>,
        configuration_digest: impl Into<String>,
        native_input_identity: impl Into<String>,
        provider_output_identities: Vec<String>,
    ) -> Result<Self, ProviderContractError> {
        let algorithm = algorithm.into();
        let algorithm_version = algorithm_version.into();
        let configuration_digest = configuration_digest.into();
        if algorithm.trim().is_empty() || algorithm_version.trim().is_empty() {
            return Err(ProviderContractError::InvalidField(
                "reconciliation.algorithm",
            ));
        }
        if provider_output_identities.is_empty() {
            return Err(ProviderContractError::InvalidResult(
                "reconciliation requires provider output".into(),
            ));
        }
        let identity = canonical_json_sha256(&value)
            .map_err(|error| ProviderContractError::Digest(error.to_string()))?;
        Ok(Self {
            identity,
            value,
            algorithm,
            algorithm_version,
            configuration_digest,
            native_input_identity: native_input_identity.into(),
            provider_output_identities,
            confidence: None,
            diagnostics: Vec::new(),
        })
    }
}

/// A representation set whose native value has no mutating accessor. Provider
/// attempts, including failures, are appended separately. Reconciliation is a
/// third, explicitly derived value and can never overwrite source-native facts.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepresentationSet<N, R> {
    native: NativeRepresentation<N>,
    #[serde(default)]
    provider_attempts: Vec<ProviderResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reconciled: Option<ReconciledRepresentation<R>>,
}

impl<N, R> RepresentationSet<N, R> {
    pub fn new(native: NativeRepresentation<N>) -> Self {
        Self {
            native,
            provider_attempts: Vec::new(),
            reconciled: None,
        }
    }

    pub fn native(&self) -> &NativeRepresentation<N> {
        &self.native
    }

    pub fn provider_attempts(&self) -> &[ProviderResponse] {
        &self.provider_attempts
    }

    pub fn reconciled(&self) -> Option<&ReconciledRepresentation<R>> {
        self.reconciled.as_ref()
    }

    pub fn record_provider_attempt(&mut self, response: ProviderResponse) {
        self.provider_attempts.push(response);
    }

    pub fn set_reconciled(
        &mut self,
        reconciled: ReconciledRepresentation<R>,
    ) -> Result<(), ProviderContractError> {
        if reconciled.native_input_identity != self.native.identity {
            return Err(ProviderContractError::InvalidResult(
                "reconciliation native input identity does not match".into(),
            ));
        }
        let available: Vec<_> = self
            .provider_attempts
            .iter()
            .filter_map(|response| response.metadata.output_identity.as_deref())
            .collect();
        if reconciled
            .provider_output_identities
            .iter()
            .any(|identity| !available.contains(&identity.as_str()))
        {
            return Err(ProviderContractError::InvalidResult(
                "reconciliation references an unavailable provider output".into(),
            ));
        }
        self.reconciled = Some(reconciled);
        Ok(())
    }

    pub fn provider_results(&self) -> impl Iterator<Item = &ProviderResult> {
        self.provider_attempts
            .iter()
            .filter_map(ProviderResponse::result)
    }
}
