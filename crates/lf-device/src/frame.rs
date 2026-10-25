//! A tonic codec that keeps each received frame's bytes next to its decoded
//! message.
//!
//! The schema was recovered from Creator 2.2.12, and every field number in it
//! is a large hash. If a device's firmware used different numbers, prost would
//! drop the unknown fields and decode an empty message without any error. So
//! every frame keeps its raw bytes, and [`Frame::is_lossless`] tells whether
//! decoding accounted for all of them.

use bytes::{Buf, BufMut, Bytes};
use prost::Message;
use tonic::Status;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};

use crate::proto::MetaRpcMessage;

/// One received frame.
#[derive(Debug, Clone)]
pub struct Frame {
    /// The frame exactly as received.
    pub raw: Bytes,
    /// `None` when the bytes aren't a valid protobuf message at all.
    pub message: Option<MetaRpcMessage>,
}

impl Frame {
    pub fn decode(raw: Bytes) -> Frame {
        let message = MetaRpcMessage::decode(raw.clone()).ok();
        Frame { raw, message }
    }

    /// True when re-encoding the decoded message reproduces the frame, i.e. no
    /// field was dropped as unknown. Protobuf encoders write fields in field
    /// order, so a canonical frame from a matching schema round-trips.
    pub fn is_lossless(&self) -> bool {
        self.message
            .as_ref()
            .is_some_and(|m| m.encode_to_vec() == self.raw)
    }

    /// The raw bytes as hex, for logs.
    pub fn hex(&self) -> String {
        self.raw.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// The field number of the first field in an encoded message, which for a
/// oneof wrapper is the tag of the variant that is set.
pub(crate) fn first_field(encoded: &[u8]) -> Option<u32> {
    let mut buf = encoded;
    let key = prost::encoding::decode_varint(&mut buf).ok()?;
    u32::try_from(key >> 3).ok()
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RawCodec;

impl Codec for RawCodec {
    type Encode = MetaRpcMessage;
    type Decode = Frame;
    type Encoder = RawEncoder;
    type Decoder = RawDecoder;

    fn encoder(&mut self) -> RawEncoder {
        RawEncoder
    }

    fn decoder(&mut self) -> RawDecoder {
        RawDecoder
    }
}

pub(crate) struct RawEncoder;

impl Encoder for RawEncoder {
    type Item = MetaRpcMessage;
    type Error = Status;

    fn encode(&mut self, item: MetaRpcMessage, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        dst.put_slice(&item.encode_to_vec());
        Ok(())
    }
}

pub(crate) struct RawDecoder;

impl Decoder for RawDecoder {
    type Item = Frame;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Frame>, Status> {
        let raw = src.copy_to_bytes(src.remaining());
        Ok(Some(Frame::decode(raw)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{
        MetaRpcClientRequestMessage, PlaybackStateChangedRequestMessage,
        meta_rpc_client_request_message as push, meta_rpc_message,
    };

    fn playback_push() -> MetaRpcMessage {
        MetaRpcMessage {
            message_id: 7,
            data: Some(meta_rpc_message::Data::ClientRequest(
                MetaRpcClientRequestMessage {
                    data: Some(push::Data::PlaybackStateChangedRequest(
                        PlaybackStateChangedRequestMessage {
                            slide: 2,
                            global_brightness: 0.5,
                            ..Default::default()
                        },
                    )),
                },
            )),
        }
    }

    #[test]
    fn canonical_frame_is_lossless() {
        let frame = Frame::decode(playback_push().encode_to_vec().into());
        assert!(frame.is_lossless());
        assert_eq!(frame.message.unwrap().message_id, 7);
    }

    #[test]
    fn unknown_field_numbers_are_detected() {
        // messageId 7, then a length-delimited field 6 the schema doesn't have.
        let raw = Bytes::from_static(&[0x08, 0x07, 0x32, 0x02, 0xaa, 0xbb]);
        let frame = Frame::decode(raw);
        let message = frame.message.as_ref().expect("still valid protobuf");
        assert_eq!(message.data, None, "prost silently drops the unknown field");
        assert!(!frame.is_lossless(), "but the frame is flagged");
    }

    #[test]
    fn garbage_keeps_raw_bytes() {
        let frame = Frame::decode(Bytes::from_static(&[0xff, 0xff]));
        assert!(frame.message.is_none());
        assert_eq!(frame.hex(), "ffff");
    }

    #[test]
    fn first_field_reads_the_oneof_tag() {
        let push = MetaRpcClientRequestMessage {
            data: Some(push::Data::PlaybackStateChangedRequest(Default::default())),
        };
        assert_eq!(first_field(&push.encode_to_vec()), Some(384_320_591));
    }
}
