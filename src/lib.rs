//! Memory allocators backed by singletons that own statically allocated memory
//!
//! # References
//!
//! - Kenwright, Ben. “Fast Efficient Fixed-Size Memory Pool.” (2012).

#![cfg_attr(feature = "nightly", feature(const_fn))]
#![cfg_attr(feature = "nightly", feature(maybe_uninit))]
#![cfg_attr(not(test), no_std)]
#![deny(missing_docs)]
#![deny(warnings)]

extern crate as_slice;
extern crate owned_singleton;
extern crate stable_deref_trait;

#[cfg(feature = "nightly")]
pub mod nightly;
pub mod stable;
