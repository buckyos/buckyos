//! Wish runs (许愿格详细设计): the ContextBuilder (snapshot, handles, profiles, map, read tools,
//! materialization), the read set, the result planner shared by every executor, and the Markdown outline of
//! control results and inputs.

pub mod analysis;
pub mod canvas;
pub mod context;
pub mod handles;
pub mod outline;
pub mod plan;
pub mod profile;
pub mod readset;
pub mod results;
pub mod runs;
pub mod snapshot;
