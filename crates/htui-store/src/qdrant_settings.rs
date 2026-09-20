use zeroize::Zeroizing;
use htui_core::store::StoreError;

#[derive(Clone)]
pub struct QdrantSettings {
    pub url: String,
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
    pub fn new(url: String, api_key: Option<String>) -> Result<Self, StoreError> {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(StoreError::Backend("Qdrant URL must start with http:// or https://".to_owned()));
        }
        let key = api_key.filter(|k| !k.trim().is_empty()).map(Zeroizing::new);
        Ok(Self {
            url,
            api_key: key,
        })
    }
}
