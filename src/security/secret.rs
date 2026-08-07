//! Exact secret leak detection and text redaction.

use serde::Serialize;

#[derive(Debug, Default)]
pub struct SecretRedactor<'a> {
    values: Vec<&'a [u8]>,
}

impl<'a> SecretRedactor<'a> {
    pub fn new(values: impl IntoIterator<Item = &'a [u8]>) -> Self {
        let mut values = values
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        values.dedup();
        Self { values }
    }

    pub fn contains_bytes(&self, candidate: &[u8]) -> bool {
        self.values.iter().any(|secret| {
            candidate
                .windows(secret.len())
                .any(|window| window == *secret)
        })
    }

    pub fn contains_serialized<T: Serialize + ?Sized>(
        &self,
        value: &T,
    ) -> Result<bool, serde_json::Error> {
        serde_json::to_vec(value).map(|bytes| self.contains_bytes(&bytes))
    }

    pub fn redact_text(&self, text: &str) -> String {
        let mut output = text.to_string();
        for secret in &self.values {
            if let Ok(secret) = std::str::from_utf8(secret) {
                output = output.replace(secret, "[REDACTED]");
            }
        }
        output
    }
}
