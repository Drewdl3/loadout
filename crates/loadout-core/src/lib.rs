//! Loadout pure resolution, planning and policy (no I/O).
//!
//! Everything here is a pure function of its inputs: no filesystem,
//! network, clock or environment access.

pub mod enabled;
pub mod frontmatter_edit;
pub mod hash;
pub mod links;
pub mod mcp;
pub mod resolve;
pub mod template;
