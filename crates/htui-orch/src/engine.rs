//! The six-stage walk of ANA-2 §4.2, in its fixed order, plus the `Clock` and `AgentSelector`
//! seams (plan D6, D8) and the stage-1 capability interlock (`docs/ANA-2.md:475-483`).
//!
//! **Empty on purpose.** T4 fills this file. Two rules it inherits from the plan and which nothing
//! here may pre-empt: the engine holds no state across a call and re-derives position, attempt and
//! completion from the store on every one (D16), and it never assumes a `run` row passed through
//! `can_move_to` — MOD-2's chat path inserts rows outside §4.3 on purpose (D17).
