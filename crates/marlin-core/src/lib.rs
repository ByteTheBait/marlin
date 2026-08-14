//! Shared hub types that break the module dependency cycles in the original
//! single-crate layout. These types are used by multiple crates on both sides
//! of a cycle (skills↔preflight, skills::daemon↔engine, engine↔tui), so they
//! live here where every crate can depend on them without forming a cycle.

pub mod skill;
pub mod tasks;
pub mod ui;
