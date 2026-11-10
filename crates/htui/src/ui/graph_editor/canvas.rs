//! MOD-141 D7, D11: a `GraphBody` drawn as a rataflow `Flow`. Forward edges (`forward_edges`) are
//! layered by longest path; back edges leave and enter by the sides (`changes_requested` and a
//! back `asks_you` on the right, `fails` on the left); palette nodes sit in a band below. The flow
//! keeps no selection of its own and never sees a key; the mouse reaches it only through
//! `Canvas::on_mouse`, and `FlowAction::Delete` is never routed to it.
