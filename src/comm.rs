//! Communication protocol for a generic UDS service.

use std::marker::PhantomData;

use serde::{Serialize, de::DeserializeOwned};
use tokio_util::bytes::{Buf, BufMut};

/// Errors produced by codec.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// IO error.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// Encoding/decoding failed.
    #[error("Encoding/decoding failed: {0}")]
    Postcard(#[from] postcard::Error),
}

/// A codec that uses [postcard] crate to serialize/deserialize.
///
/// It fits when the decoded messages are not too large, which is the case here.
pub struct PostcardCodec<E, D> {
    _ser_phantom: PhantomData<E>,
    _de_phantom: PhantomData<D>,
}

impl<E, D> Default for PostcardCodec<E, D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E, D> PostcardCodec<E, D> {
    /// Construct a codec from the options.
    pub const fn new() -> Self {
        Self {
            _ser_phantom: PhantomData,
            _de_phantom: PhantomData,
        }
    }
}

impl<E, D: DeserializeOwned> tokio_util::codec::Decoder for PostcardCodec<E, D> {
    type Item = D;
    type Error = CodecError;

    fn decode(
        &mut self,
        src: &mut tokio_util::bytes::BytesMut,
    ) -> Result<Option<Self::Item>, Self::Error> {
        match postcard::take_from_bytes(src) {
            Err(postcard::Error::DeserializeUnexpectedEnd) => Ok(None),
            Ok((res, rest)) => {
                let frame_len = src.len().saturating_sub(rest.len());
                src.advance(frame_len);
                Ok(Some(res))
            }
            Err(e) => Err(Self::Error::from(e)),
        }
    }
}

impl<E: Serialize, D> tokio_util::codec::Encoder<&E> for PostcardCodec<E, D> {
    type Error = CodecError;

    fn encode(
        &mut self,
        item: &E,
        dst: &mut tokio_util::bytes::BytesMut,
    ) -> Result<(), Self::Error> {
        postcard::to_io(item, dst.writer())?;
        Ok(())
    }
}
