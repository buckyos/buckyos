//! Crate-internal tests of the `LLMContext` loop, grouped by theme. The
//! function call / behavior loop groups share the scripted mocks of
//! [`mocks`]; the X7 suspension groups share [`suspension::mocks`].

mod behavior_loop;
mod budgets;
mod cancel;
mod error_policy;
mod function_call_loop;
mod mocks;
mod suspension;
mod thinking;
