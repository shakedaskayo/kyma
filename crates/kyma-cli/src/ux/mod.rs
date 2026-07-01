//! Shared terminal-output toolkit for the kyma CLI. Commands use these
//! helpers instead of hand-rolled `println!` formatting, so color, tables,
//! spinners, and error rendering stay consistent across every subcommand.

pub(crate) mod theme;
