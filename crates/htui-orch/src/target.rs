//! MOD-132: preparing a target and collecting its work as a git bundle.
//!
//! M1 lands [`bundle`], the header parser every bundle is read through before any of it reaches
//! a `git` on this box (plan P7). [`seed`] makes prepare's clone on a target from a bundle made
//! here (P5), and [`collect`] brings its run branch back as a bundle and moves a local branch to
//! it (P6). The verbs the seed and the collect are built from — `bundle create`, `bundle verify`,
//! `index-pack`, the connectivity check and the temp ref — are [`crate::isolate::git`]'s, beside
//! the crate's other git verbs.

pub mod bundle;
pub mod collect;
pub mod seed;
