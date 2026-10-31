//! Message structures and payload serialization for Spine Wire Protocol.

use crate::frame::{Frame, FrameType, ProtocolError};
use serde::{Deserialize, Serialize};
use spine_core::{HashAlgorithm, ObjectId};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelloMessage {
    pub client_version: String,
    pub capabilities: Vec<String>,
    pub agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelloAckMessage {
    pub server_version: String,
    pub selected_version: u16,
    pub capabilities: Vec<String>,
    pub server_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthChallenge {
    pub nonce: [u8; 32],
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthResponse {
    pub user_id: String,
    pub signature: Vec<u8>,
    pub public_key: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthResult {
    pub authenticated: bool,
    pub user: String,
    pub role: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoOpen {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoInfo {
    pub algorithm: HashAlgorithm,
    pub default_branch: String,
    pub head_oid: Option<ObjectId>,
    pub storage_format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefAdvertise {
    pub refs: BTreeMap<String, ObjectId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefUpdateProposal {
    pub ref_name: String,
    pub expected_old_oid: Option<ObjectId>,
    pub new_oid: ObjectId,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefUpdateReq {
    pub updates: Vec<RefUpdateProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefUpdateStatus {
    pub ref_name: String,
    pub status: String, // "ok", "stale_ref", "non_fast_forward", "permission_denied"
    pub current_oid: Option<ObjectId>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefUpdateRes {
    pub success: bool,
    pub results: Vec<RefUpdateStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FetchNegotiate {
    pub want: Vec<ObjectId>,
    pub have: Vec<ObjectId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FetchPlan {
    pub objects_to_send: u64,
    pub estimated_bytes: u64,
    pub already_up_to_date: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushBegin {
    pub ref_updates: Vec<RefUpdateProposal>,
    pub expected_pack_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackProgress {
    pub phase: String,
    pub current: u64,
    pub total: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackEnd {
    pub pack_checksum: [u8; 32],
    pub object_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorMessage {
    pub code: u16,
    pub category: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PingMessage {
    pub token: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PongMessage {
    pub token: u32,
}

// Helpers for encoding/decoding typed messages into frames
pub trait ProtocolMessage: Serialize + for<'de> Deserialize<'de> {
    const FRAME_TYPE: FrameType;

    fn to_frame(&self) -> Result<Frame, ProtocolError> {
        let payload = serde_json::to_vec(self)?;
        Frame::new(Self::FRAME_TYPE, payload)
    }

    fn from_frame(frame: &Frame) -> Result<Self, ProtocolError> {
        if frame.header.frame_type != Self::FRAME_TYPE {
            return Err(ProtocolError::Protocol(format!(
                "Mismatched frame type: expected {:?}, got {:?}",
                Self::FRAME_TYPE,
                frame.header.frame_type
            )));
        }
        let msg = serde_json::from_slice(&frame.payload)?;
        Ok(msg)
    }
}

impl ProtocolMessage for HelloMessage {
    const FRAME_TYPE: FrameType = FrameType::Hello;
}
impl ProtocolMessage for HelloAckMessage {
    const FRAME_TYPE: FrameType = FrameType::HelloAck;
}
impl ProtocolMessage for AuthChallenge {
    const FRAME_TYPE: FrameType = FrameType::AuthChallenge;
}
impl ProtocolMessage for AuthResponse {
    const FRAME_TYPE: FrameType = FrameType::AuthResponse;
}
impl ProtocolMessage for AuthResult {
    const FRAME_TYPE: FrameType = FrameType::AuthResult;
}
impl ProtocolMessage for RepoOpen {
    const FRAME_TYPE: FrameType = FrameType::RepoOpen;
}
impl ProtocolMessage for RepoInfo {
    const FRAME_TYPE: FrameType = FrameType::RepoInfo;
}
impl ProtocolMessage for RefAdvertise {
    const FRAME_TYPE: FrameType = FrameType::RefAdvertise;
}
impl ProtocolMessage for RefUpdateReq {
    const FRAME_TYPE: FrameType = FrameType::RefUpdateReq;
}
impl ProtocolMessage for RefUpdateRes {
    const FRAME_TYPE: FrameType = FrameType::RefUpdateRes;
}
impl ProtocolMessage for FetchNegotiate {
    const FRAME_TYPE: FrameType = FrameType::FetchNegotiate;
}
impl ProtocolMessage for FetchPlan {
    const FRAME_TYPE: FrameType = FrameType::FetchPlan;
}
impl ProtocolMessage for PushBegin {
    const FRAME_TYPE: FrameType = FrameType::PushBegin;
}
impl ProtocolMessage for PackProgress {
    const FRAME_TYPE: FrameType = FrameType::PackProgress;
}
impl ProtocolMessage for PackEnd {
    const FRAME_TYPE: FrameType = FrameType::PackEnd;
}
impl ProtocolMessage for ErrorMessage {
    const FRAME_TYPE: FrameType = FrameType::Error;
}
impl ProtocolMessage for PingMessage {
    const FRAME_TYPE: FrameType = FrameType::Ping;
}
impl ProtocolMessage for PongMessage {
    const FRAME_TYPE: FrameType = FrameType::Pong;
}
