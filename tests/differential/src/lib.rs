// ABOUTME: The differential harness: runs git worktree add and git-sprout add over
// ABOUTME: the same repository and asserts nothing observable differs between them.

pub mod case;
pub mod compare;
pub mod env;
pub mod files;
pub mod fixtures;
pub mod flags;
pub mod index;
pub mod inject;
pub mod normalize;
pub mod run;
pub mod snapshot;
pub mod stats;
