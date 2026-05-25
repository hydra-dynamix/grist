//! Grist library crate.

/// Returns the crate version from Cargo metadata.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn version_matches_manifest() {
        assert_eq!(version(), "0.1.0");
    }
}
