//! The pinned BGE-small model files (MOD-68, `docs/ANA-23.md` §7.1).
//!
//! [`crate::embed::RtenEmbedder`] loads exactly two files, `model.onnx` and `tokenizer.json`, from
//! `Xenova/bge-small-en-v1.5` at one commit. This module owns that pin and the tokenizer settings
//! that fastembed used to read from the repository's `config.json`, `tokenizer_config.json` and
//! `special_tokens_map.json`, so neither of those files is needed.
use std::path::{Path, PathBuf};

/// The Hugging Face repository the model files come from.
pub const REPO: &str = "Xenova/bge-small-en-v1.5";

/// The commit of [`REPO`] every file is fetched at (the one fastembed 3.14.1 resolved).
pub const REVISION: &str = "ea104dacec62c0de699686887e3f920caeb4f3e3";

/// sha256 of `onnx/model.onnx` at [`REVISION`].
pub const ONNX_SHA256: &str = "828e1496d7fabb79cfa4dcd84fa38625c0d3d21da474a00f08db0f559940cf35";

/// Size in bytes of `onnx/model.onnx` at [`REVISION`].
pub const ONNX_BYTES: u64 = 133_093_490;

/// sha256 of `tokenizer.json` at [`REVISION`].
pub const TOKENIZER_SHA256: &str =
    "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66";

/// Size in bytes of `tokenizer.json` at [`REVISION`].
pub const TOKENIZER_BYTES: u64 = 711_396;

/// Longest input in tokens, special tokens included; longer texts are truncated. fastembed took
/// `min(512, model_max_length)` from `tokenizer_config.json`, which is 512.
pub const MAX_TOKENS: usize = 512;

/// The `[PAD]` token's id (`config.json` `pad_token_id`): padding positions hold it, with an
/// attention mask of 0.
pub const PAD_ID: u32 = 0;

/// The special tokens of `special_tokens_map.json`, which fastembed re-added to the tokenizer.
pub const SPECIAL_TOKENS: [&str; 5] = ["[CLS]", "[MASK]", "[PAD]", "[SEP]", "[UNK]"];

/// How a text's vector is taken from the model's output: the `[CLS]` token's hidden state.
pub const POOLING: &str = "cls";

/// How that vector is normalised: to unit L2 length.
pub const NORMALISATION: &str = "l2";

/// The two files the embedder loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFiles {
    /// The ONNX graph and weights (`onnx/model.onnx` in [`REPO`]).
    pub onnx: PathBuf,
    /// The tokenizer (`tokenizer.json` in [`REPO`]).
    pub tokenizer: PathBuf,
}

impl ModelFiles {
    /// `dir/model.onnx` and `dir/tokenizer.json`.
    #[must_use]
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            onnx: dir.join("model.onnx"),
            tokenizer: dir.join("tokenizer.json"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_files_in_a_dir_end_in_onnx_and_json() {
        let files = ModelFiles::in_dir(Path::new("/cache/model"));
        assert_eq!(files.onnx, Path::new("/cache/model/model.onnx"));
        assert_eq!(files.tokenizer, Path::new("/cache/model/tokenizer.json"));
    }
}
