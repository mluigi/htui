use htui_core::store::StoreError;
/// Settings for connecting to a Qdrant vector database.
use zeroize::Zeroizing;

/// Settings for connecting to a Qdrant vector database.
#[derive(Clone)]
pub struct QdrantSettings {
    /// The base URL of the Qdrant server.
    pub url: String,
    /// An optional API key for authentication.
    pub api_key: Option<Zeroizing<String>>,
}

impl std::fmt::Debug for QdrantSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QdrantSettings")
            .field("url", &self.url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl QdrantSettings {
    /// Creates a new `QdrantSettings` instance, validating the URL.
    pub fn new(url: String, api_key: Option<String>) -> Result<Self, StoreError> {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(StoreError::Backend(
                "Qdrant URL must start with http:// or https://".to_owned(),
            ));
        }
        let key = api_key.filter(|k| !k.trim().is_empty()).map(Zeroizing::new);
        Ok(Self { url, api_key: key })
    }
}
