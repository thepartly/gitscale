//! gitscale's integration tests: one binary, one module per feature.
//!
//! A test's name is `<kind>_<id>[<variant>]_<sentence>`, inside the module of
//! the feature whose behaviour it holds — so `cargo test pull::` runs one
//! feature, `cargo test pull::edge_` one kind of one feature, and
//! `cargo test ::error_` every error case. docs/testing.md has the rules, and
//! docs/test-catalog.md lists every test; `catalog` keeps that list current.

#[allow(dead_code)]
mod support;

mod add;
mod artefact;
mod cache;
mod catalog;
mod check;
mod ci_auth;
mod ci_checkout;
mod clean;
mod cli;
mod commit;
mod config;
mod develop;
mod fetch;
mod hook;
mod links;
mod post_sync;
mod pull;
mod push;
mod registry;
mod remove;
mod resolution;
mod skill;
mod status;
mod stores;
mod sync;
mod upgrade;
