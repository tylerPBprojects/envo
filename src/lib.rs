//! envo — A Nix-based developer environment runtime.
//!
//! This crate provides the core library for envo, a tool that wraps Nix
//! to deliver lazy package realization and sub-100ms environment activation.

pub mod manifest;
pub mod lockfile;
pub mod realize;
pub mod activate;
pub mod cli;
pub mod self_update;
pub mod nix_bootstrap;
pub mod lsp;
pub mod mcp;
pub mod templates;
pub mod telemetry;
