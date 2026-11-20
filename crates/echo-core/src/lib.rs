//! Echo's engine: watches a repo's remote branches and keeps one synced
//! branch fast-forwarded in the local checkout. No UI dependencies; the Tauri
//! app (`src-tauri`) is a thin shell over [`core::Core`].

pub mod branches;
pub mod core;
pub mod flags;
pub mod git;
pub mod github;
pub mod interval;
pub mod notify;
pub mod repo;
pub mod store;
pub mod sync;
pub mod types;

pub use crate::core::{CmdError, CmdErrorKind, Core, CoreEvent, EventSink, TrayIconState, TrayModel};
