//! OpenDAN — the Agent Loader.
//!
//! The agent kernel (inference loop, context scheduling, Turns, the input
//! bus, runtimes, the Agent State file protocol) lives in `llm_context`,
//! `agent_tool` and `libopendan`. This process only: starts as the runtime
//! app of one agent, keeps the agent's sessions hosted
//! ([`libopendan::host::Supervisor`]), bridges system sources into session
//! inputs and replies back out ([`ui`]), and serves the Agent State
//! ([`service`]).
//!
//! Nothing here knows a prompt, a behavior name or the shape of an LLM
//! answer, and nothing here writes a session's state: only a session's
//! driver does, through `drive`.

pub mod config;
pub mod home;
pub mod loader;
pub mod rootfs;
pub mod service;
pub mod ui;
