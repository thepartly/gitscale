//! gitscale's integration tests: one binary, one module per feature.
//!
//! A test's name is `<kind>_<id>[<variant>]_<sentence>`, inside the module of
//! the feature whose behaviour it holds — so `cargo test sync::` runs one
//! feature, `cargo test sync::edge_` one kind of one feature, and
//! `cargo test ::error_` every error case. docs/testing.md has the rules, and
//! docs/test-catalog.md lists every test; `catalog` keeps that list current.

#[allow(dead_code)]
mod support;

mod artefact;
mod cache;
mod catalog;
mod check;
mod ci_auth;
mod ci_checkout;
mod clean;
mod cli;
mod config;
mod exclude;
mod forward;
mod hash;
mod hook;
mod links;
mod ls;
mod man;
mod network;
mod output;
mod placement;
mod post_sync;
mod prefer;
mod registry;
mod require;
mod resolution;
mod skill;
mod stores;
mod sync;
mod topic;
mod upgrade;
mod workspace;
