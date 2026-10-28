//! Settings for connecting to a Qdrant vector database.

use htui_core::qdrant::QdrantUrl;
use zeroize::Zeroizing;

/// Settings for connecting to a Qdrant vector database.
#[derive(Clone)]
pub struct QdrantSettings {
    /// The base URL of the Qdrant server, checked by [`QdrantUrl::parse`] (MOD-158 D2).
    pub url: QdrantUrl,
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
    /// MOD-158 D2: only a checked URL reaches the client. A blank key is no key.
    #[must_use]
    pub fn new(url: QdrantUrl, api_key: Option<String>) -> Self {
        let api_key = api_key.filter(|k| !k.trim().is_empty()).map(Zeroizing::new);
        Self { url, api_key }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url() -> QdrantUrl {
        QdrantUrl::parse("http://localhost:6334").expect("a valid URL")
    }

    #[test]
    fn a_blank_key_is_no_key() {
        assert!(QdrantSettings::new(url(), None).api_key.is_none());
        assert!(
            QdrantSettings::new(url(), Some(" \t".into()))
                .api_key
                .is_none()
        );
        let settings = QdrantSettings::new(url(), Some("k".into()));
        assert_eq!(settings.api_key.as_deref().map(String::as_str), Some("k"));
        assert_eq!(settings.url.as_str(), "http://localhost:6334");
    }

    #[test]
    fn debug_never_prints_the_key() {
        let text = format!("{:?}", QdrantSettings::new(url(), Some("SENTINEL".into())));
        assert!(!text.contains("SENTINEL"), "{text}");
        assert!(text.contains("<redacted>"), "{text}");
        assert!(text.contains("http://localhost:6334"), "{text}");
    }
}
