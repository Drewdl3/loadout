//! Loadout target table, linking and config renderers.

pub mod fsutil;
pub mod link;
pub mod mcp;
pub mod owned;
pub mod plugins;
pub mod table;
pub mod testhook;

pub use table::{TargetDef, TargetTable, expand_path};
