extern crate ed25519_dalek;
extern crate rand;
extern crate serde;

mod eddsa_ed25519;
mod dlog_ed25519;

pub use crate::eddsa_ed25519::*;
pub use crate::dlog_ed25519::*;
