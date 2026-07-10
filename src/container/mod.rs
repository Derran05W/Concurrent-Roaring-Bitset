//! `Container` enum, dispatch, conversion policy, normalization, and set-op kernels.

pub mod array;
pub mod bitmap;
pub mod run;

use array::ArrayContainer;

/// A roaring container. Additional variants (`Bitmap`, `Run`) are introduced in P2/P3;
/// growing a crate-private enum is a non-breaking internal change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Container {
    Array(ArrayContainer),
}

impl Container {
    pub fn insert(&mut self, v: u16) -> bool {
        match self {
            Container::Array(a) => a.insert(v),
        }
    }

    pub fn remove(&mut self, v: u16) -> bool {
        match self {
            Container::Array(a) => a.remove(v),
        }
    }

    pub fn contains(&self, v: u16) -> bool {
        match self {
            Container::Array(a) => a.contains(v),
        }
    }

    pub fn cardinality(&self) -> u32 {
        match self {
            Container::Array(a) => a.cardinality(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Container::Array(a) => a.is_empty(),
        }
    }

    pub fn num_runs(&self) -> u32 {
        match self {
            Container::Array(a) => a.num_runs(),
        }
    }
}
