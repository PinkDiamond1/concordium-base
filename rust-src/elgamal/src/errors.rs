// -*- mode: rust; -*-
  //
  // This file is part of concordium_crypto
  // Copyright (c) 2019 -
  // See LICENSE for licensing information.
  //
  // Authors:
  // - bm@concordium.com


// rustc seems to think the typenames in match statements (e.g. in
// Display) should be snake cased, for some reason.
#![allow(non_snake_case)]

use core::fmt;
use core::fmt::Display;
use pairing::{PrimeFieldDecodingError, GroupDecodingError};

/// Internal errors.  

#[derive(Debug )]
pub(crate) enum InternalError {
    DecodingError(PrimeFieldDecodingError),
    GDecodingError(GroupDecodingError),
    DivisionByZero,
    PublicKeyLengthError,
    MessageLengthError,
    CipherLengthError
}

impl Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            InternalError::GDecodingError(ref p)
                => write!(f,"{}", p),
            InternalError::DecodingError(ref p)
                => write!(f,"{}", p),
            InternalError::PublicKeyLengthError
                => write!(f, "Wrong PublicKey length"),
            InternalError::MessageLengthError
                => write!(f, "Wrong message length"),
            InternalError::CipherLengthError
                => write!(f, "Wrong cipher length"),
            InternalError::DivisionByZero
                => write!(f, "Division By Zero"),
        }
    }
}

impl ::failure::Fail for InternalError {}

/// Errors which may occur while processing proofs and keys.
///
/// This error may arise due to:
///
/// * Being given bytes with a length different to what was expected.
///
/// * A problem decompressing to a scalar, 
///
/// * A problem with the format of `s`, a scalar, 

#[derive(Debug)]
pub struct ElgamalError(pub(crate) InternalError);

impl Display for ElgamalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ::failure::Fail for ElgamalError {
    fn cause(&self) -> Option<&dyn (::failure::Fail)> {
        Some(&self.0)
    }
}