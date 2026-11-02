//! Core protocol definitions and binary framing for Spine Wire Protocol (`SPWP`).

pub mod frame;
pub mod messages;

pub use frame::{
    compute_checksum, Frame, FrameFlags, FrameHeader, FrameType, ProtocolError, SessionState,
    HEADER_SIZE, MAX_PAYLOAD_SIZE, SPWP_MAGIC, SPWP_VERSION_1,
};
pub use messages::*;

#[cfg(test)]
mod tests {
    use super::*;
    use spine_core::{HashAlgorithm, ObjectId};
    use std::collections::BTreeMap;

    #[test]
    fn test_frame_header_roundtrip() {
        let flags = FrameFlags::END_OF_STREAM | FrameFlags::COMPRESSED;
        let checksum = [1, 2, 3, 4];
        let header = FrameHeader::new(FrameType::Hello, flags, 128, checksum);
        let bytes = header.serialize();
        assert_eq!(bytes.len(), HEADER_SIZE);

        let parsed = FrameHeader::deserialize(&bytes).unwrap();
        assert_eq!(parsed.magic, *SPWP_MAGIC);
        assert_eq!(parsed.version, SPWP_VERSION_1);
        assert_eq!(parsed.frame_type, FrameType::Hello);
        assert_eq!(parsed.flags, flags);
        assert_eq!(parsed.payload_len, 128);
        assert_eq!(parsed.checksum, checksum);
    }

    #[test]
    fn test_frame_serialization_and_checksum_verification() {
        let payload = b"Hello Spine Distributed Wire Protocol!".to_vec();
        let frame = Frame::new(FrameType::Hello, payload.clone()).unwrap();

        let mut wire_bytes = Vec::new();
        frame.write_to_sync(&mut wire_bytes).unwrap();

        let decoded = Frame::read_from_sync(&wire_bytes[..]).unwrap();
        assert_eq!(decoded.header.frame_type, FrameType::Hello);
        assert_eq!(decoded.payload, payload);

        // Test corruption detection
        let mut corrupted = wire_bytes.clone();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0xFF;
        let err = Frame::read_from_sync(&corrupted[..]).unwrap_err();
        match err {
            ProtocolError::ChecksumMismatch(_, _) => {}
            other => panic!("Expected ChecksumMismatch, got {:?}", other),
        }
    }

    #[test]
    fn test_message_typed_roundtrips() {
        // Hello
        let hello = HelloMessage {
            client_version: "0.1.0".to_string(),
            capabilities: vec!["pack_zstd".to_string(), "cas_push".to_string()],
            agent: "spine-cli/0.1.0".to_string(),
        };
        let frame = hello.to_frame().unwrap();
        let decoded = HelloMessage::from_frame(&frame).unwrap();
        assert_eq!(hello, decoded);

        // RefAdvertise
        let mut refs = BTreeMap::new();
        let oid1 = ObjectId::compute(HashAlgorithm::Blake3, spine_core::ObjectType::Commit, b"c1");
        let oid2 = ObjectId::compute(HashAlgorithm::Blake3, spine_core::ObjectType::Commit, b"c2");
        refs.insert("refs/heads/main".to_string(), oid1);
        refs.insert("refs/tags/v1.0".to_string(), oid2);

        let ref_adv = RefAdvertise { refs };
        let frame = ref_adv.to_frame().unwrap();
        let decoded = RefAdvertise::from_frame(&frame).unwrap();
        assert_eq!(ref_adv, decoded);

        // FetchNegotiate
        let fetch = FetchNegotiate {
            want: vec![oid1],
            have: vec![oid2],
        };
        let frame = fetch.to_frame().unwrap();
        let decoded = FetchNegotiate::from_frame(&frame).unwrap();
        assert_eq!(fetch, decoded);
    }

    #[test]
    fn test_frame_payload_too_large_rejection() {
        let header = FrameHeader::new(
            FrameType::PackData,
            FrameFlags::empty(),
            (MAX_PAYLOAD_SIZE + 1) as u32,
            [0; 4],
        );
        let bytes = header.serialize();
        let err = FrameHeader::deserialize(&bytes).unwrap_err();
        match err {
            ProtocolError::PayloadTooLarge(_, _) => {}
            other => panic!("Expected PayloadTooLarge, got {:?}", other),
        }
    }
}
