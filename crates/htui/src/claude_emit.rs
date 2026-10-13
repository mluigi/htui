//! `htui claude emit` (MOD-93, `R-ID-4`, `R-ID-6`, `R-STO-1`).

/// How `htui claude emit` ends when it does not write; `main` maps it to the exit code and prints
/// it on stderr. Neither arm reaches Sentry: its sentences carry home paths (MOD-93 D8).
#[derive(Debug)]
pub enum EmitExit {
    /// Exit 2: the target is not one `emit` may write (foreign, a file, ours without `--force`),
    /// or this box has no default directory.
    Refused(String),
    /// Exit 1: a write, a rename or stdout failed.
    Failed(String),
}

impl EmitExit {
    /// 2 refused, 1 failed.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Refused(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl core::fmt::Display for EmitExit {
    /// The sentence, and nothing else.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused(sentence) | Self::Failed(sentence) => f.write_str(sentence),
        }
    }
}

impl std::error::Error for EmitExit {}

#[cfg(test)]
mod tests {
    use super::EmitExit;

    /// D8: refused is 2, failed is 1, and each prints its sentence alone.
    #[test]
    fn emit_exits_are_2_and_1() {
        assert_eq!(EmitExit::Refused("r".into()).code(), 2);
        assert_eq!(EmitExit::Failed("f".into()).code(), 1);
        assert_eq!(EmitExit::Refused("no".into()).to_string(), "no");
        assert_eq!(EmitExit::Failed("broke".into()).to_string(), "broke");
    }
}
