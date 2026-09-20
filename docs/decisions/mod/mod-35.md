# MOD-35: Add Qdrant connection settings

## Context
The user requested "Add Qdrant connection settings. Like the Postgres DSN, add settings for Qdrant with base URL plus optional API key."

Initially this was modeled as a single `QdrantDsn` string (parsed for URL and API key). The user clarified they wanted two separate fields/values for base URL and API key, not a single one.

## Changes
- Updated the OS keyring secrets management in `htui-store::secret` to maintain two separate keys: `qdrant-url` and `qdrant-key`.
- Replaced `QdrantDsn` with `QdrantSettings` holding two distinct values (`url: String` and `api_key: Option<Zeroizing<String>>`).
- Rewrote the `QdrantSection` UI in the Settings tab to present two editable rows (URL and Key) using a wizard flow (`e` on the section opens the URL editor, Enter advances to the API Key editor).
- Updated the worker request payload (`StoreRequest::SetQdrantSettings`) to take `QdrantSettings` and spawn a blocking task to persist both properties to the OS keyring.
