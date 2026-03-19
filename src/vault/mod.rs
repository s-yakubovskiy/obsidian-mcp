//! Vault layer — pure filesystem operations on an Obsidian vault.
//!
//! Knows nothing about MCP; provides the data model and I/O primitives
//! that tool handlers delegate to.

pub mod frontmatter;
pub mod fs;
pub mod index;
pub mod parser;
pub mod patch;
pub mod periodic;
pub mod watcher;
pub mod wikilink;
