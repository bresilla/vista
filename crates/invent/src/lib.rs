//! Semantic candidate retrieval, for repairing an item when literal matching
//! finds nothing.
//!
//! This crate is a peer of `vista-recall`, not an extension of it. Neither
//! depends on the other; a consumer may use either alone or compose both. The
//! interface between them is plain text and caller-owned identifiers, so no
//! type is shared across the boundary.
//!
//! Unimplemented. The intended shape is an embedding index keyed by caller
//! identifiers:
//!
//! ```text
//! insert(id, text)          embed and retain a vector
//! remove(id)                drop it
//! search(text, limit)       nearest identifiers by cosine similarity
//! ```
//!
//! It holds vectors and a model, never the caller's history.

#![forbid(unsafe_code)]
