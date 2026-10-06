//! Named key actions (MOD-67, `docs/ANA-26.md` §7): the compiled-in catalogue, the strict chord
//! parser, context stacks and the resolver that turns a chord into ordered candidate actions, and
//! the hints and help generated from them.

pub mod catalogue;
pub mod chord;
pub mod hint;
pub mod stack;
