//! Diff, merge, and snapshot DAG — pure and I/O-free (§1, §11).
//!
//! `merge()` is a pure function of three `CharacterBuild`s: no database, no
//! async, no adapter (§11.3). This is what makes the highest-risk code in the
//! project exhaustively property-testable (§16) and is the concrete reason
//! sync-engine is I/O-free.
//!
//! Phase 0.1: scaffold only. Engine + proptest suite land in 0.5.
