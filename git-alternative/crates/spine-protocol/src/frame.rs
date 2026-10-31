//! Binary framing implementation for Spine Wire Protocol (`SPWP`).

use std::io::{self, Read, Write};
use std::ops::{BitOr, BitOrAssign};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SPWP_MAGIC: &[u8; 4] = b"SPWP";
pub const SPWP_VERSION_1: u16 = 1;
pub const HEADER_SIZE: usize = 20;
pub const MAX_PAYLOAD_SIZE: usize = 16 * 1024 * 1024; // 16 MB

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("Invalid magic: expected b\"SPWP\", got {0:?}")]
    InvalidMagic([u8; 4]),
    #[error("Unsupported protocol version: {0}")]
    UnsupportedVersion(u16),
    #[error("Unknown frame type: 0x{0:04x}")]
    UnknownFrameType(u16),
    #[error("Frame payload too large: {0} bytes (max {1} bytes)")]
    PayloadTooLarge(usize, usize),
    #[error("Checksum mismatch: expected {0:02x?}, got {1:02x?}")]
    ChecksumMismatch([u8; 4], [u8; 4]),
    #[error("Unexpected EOF reading frame")]
    UnexpectedEof,
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Protocol error: {0}")]
    Protocol(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum FrameType {
    Hello = 0x0001,
    HelloAck = 0x0002,
    AuthChallenge = 0x0010,
    AuthResponse = 0x0011,
    AuthResult = 0x0012,
    RepoOpen = 0x0020,
    RepoInfo = 0x0021,
    RefAdvertise = 0x0030,
    RefUpdateReq = 0x0031,
    RefUpdateRes = 0x0032,
    FetchNegotiate = 0x0040,
    FetchPlan = 0x0041,
    PushBegin = 0x0050,
    PackData = 0x0051,
    PackProgress = 0x0052,
    PackEnd = 0x0053,
    Error = 0x0060,
    Ping = 0x0070,
    Pong = 0x0071,
}

impl FrameType {
    pub fn from_u16(val: u16) -> Result<Self, ProtocolError> {
        match val {
            0x0001 => Ok(Self::Hello),
            0x0002 => Ok(Self::HelloAck),
            0x0010 => Ok(Self::AuthChallenge),
            0x0011 => Ok(Self::AuthResponse),
            0x0012 => Ok(Self::AuthResult),
            0x0020 => Ok(Self::RepoOpen),
            0x0021 => Ok(Self::RepoInfo),
            0x0030 => Ok(Self::RefAdvertise),
            0x0031 => Ok(Self::RefUpdateReq),
            0x0032 => Ok(Self::RefUpdateRes),
            0x0040 => Ok(Self::FetchNegotiate),
            0x0041 => Ok(Self::FetchPlan),
            0x0050 => Ok(Self::PushBegin),
            0x0051 => Ok(Self::PackData),
            0x0052 => Ok(Self::PackProgress),
            0x0053 => Ok(Self::PackEnd),
            0x0060 => Ok(Self::Error),
            0x0070 => Ok(Self::Ping),
            0x0071 => Ok(Self::Pong),
            other => Err(ProtocolError::UnknownFrameType(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameFlags(pub u16);

impl FrameFlags {
    pub const EMPTY: Self = Self(0);
    pub const END_OF_STREAM: Self = Self(0x0001);
    pub const COMPRESSED: Self = Self(0x0002);
    pub const HEARTBEAT: Self = Self(0x0004);

    pub fn empty() -> Self {
        Self::EMPTY
    }

    pub fn bits(&self) -> u16 {
        self.0
    }

    pub fn from_bits_truncate(bits: u16) -> Self {
        Self(bits)
    }

    pub fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

impl BitOr for FrameFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for FrameFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// 20-byte fixed frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub magic: [u8; 4],
    pub version: u16,
    pub frame_type: FrameType,
    pub flags: FrameFlags,
    pub reserved: u16,
    pub payload_len: u32,
    pub checksum: [u8; 4],
}

impl FrameHeader {
    pub fn new(
        frame_type: FrameType,
        flags: FrameFlags,
        payload_len: u32,
        checksum: [u8; 4],
    ) -> Self {
        Self {
            magic: *SPWP_MAGIC,
            version: SPWP_VERSION_1,
            frame_type,
            flags,
            reserved: 0,
            payload_len,
            checksum,
        }
    }

    pub fn serialize(&self) -> [u8; HEADER_SIZE] {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(&self.magic);
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6..8].copy_from_slice(&(self.frame_type as u16).to_le_bytes());
        buf[8..10].copy_from_slice(&self.flags.bits().to_le_bytes());
        buf[10..12].copy_from_slice(&self.reserved.to_le_bytes());
        buf[12..16].copy_from_slice(&self.payload_len.to_le_bytes());
        buf[16..20].copy_from_slice(&self.checksum);
        buf
    }

    pub fn deserialize(buf: &[u8; HEADER_SIZE]) -> Result<Self, ProtocolError> {
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&buf[0..4]);
        if &magic != SPWP_MAGIC {
            return Err(ProtocolError::InvalidMagic(magic));
        }

        let version = u16::from_le_bytes([buf[4], buf[5]]);
        if version != SPWP_VERSION_1 {
            return Err(ProtocolError::UnsupportedVersion(version));
        }

        let frame_type_raw = u16::from_le_bytes([buf[6], buf[7]]);
        let frame_type = FrameType::from_u16(frame_type_raw)?;

        let flags_raw = u16::from_le_bytes([buf[8], buf[9]]);
        let flags = FrameFlags::from_bits_truncate(flags_raw);

        let reserved = u16::from_le_bytes([buf[10], buf[11]]);
        let payload_len = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);

        if payload_len as usize > MAX_PAYLOAD_SIZE {
            return Err(ProtocolError::PayloadTooLarge(
                payload_len as usize,
                MAX_PAYLOAD_SIZE,
            ));
        }

        let mut checksum = [0u8; 4];
        checksum.copy_from_slice(&buf[16..20]);

        Ok(Self {
            magic,
            version,
            frame_type,
            flags,
            reserved,
            payload_len,
            checksum,
        })
    }
}

/// Compute 4-byte truncated BLAKE3 checksum.
pub fn compute_checksum(payload: &[u8]) -> [u8; 4] {
    if payload.is_empty() {
        return [0u8; 4];
    }
    let hash = blake3::hash(payload);
    let mut cs = [0u8; 4];
    cs.copy_from_slice(&hash.as_bytes()[0..4]);
    cs
}

/// Complete in-memory protocol frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub header: FrameHeader,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(frame_type: FrameType, payload: Vec<u8>) -> Result<Self, ProtocolError> {
        Self::with_flags(frame_type, FrameFlags::empty(), payload)
    }

    pub fn with_flags(
        frame_type: FrameType,
        flags: FrameFlags,
        payload: Vec<u8>,
    ) -> Result<Self, ProtocolError> {
        if payload.len() > MAX_PAYLOAD_SIZE {
            return Err(ProtocolError::PayloadTooLarge(
                payload.len(),
                MAX_PAYLOAD_SIZE,
            ));
        }
        let checksum = compute_checksum(&payload);
        let header = FrameHeader::new(frame_type, flags, payload.len() as u32, checksum);
        Ok(Self { header, payload })
    }

    pub fn serialize_to_vec(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_SIZE + self.payload.len());
        out.extend_from_slice(&self.header.serialize());
        out.extend_from_slice(&self.payload);
        out
    }

    pub fn write_to_sync<W: Write>(&self, mut writer: W) -> io::Result<()> {
        writer.write_all(&self.header.serialize())?;
        writer.write_all(&self.payload)?;
        writer.flush()
    }

    pub async fn write_to_async<W: AsyncWrite + Unpin>(&self, mut writer: W) -> io::Result<()> {
        writer.write_all(&self.header.serialize()).await?;
        writer.write_all(&self.payload).await?;
        writer.flush().await
    }

    pub fn read_from_sync<R: Read>(mut reader: R) -> Result<Self, ProtocolError> {
        let mut hdr_buf = [0u8; HEADER_SIZE];
        reader.read_exact(&mut hdr_buf).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                ProtocolError::UnexpectedEof
            } else {
                ProtocolError::Io(e)
            }
        })?;

        let header = FrameHeader::deserialize(&hdr_buf)?;
        let mut payload = vec![0u8; header.payload_len as usize];
        reader.read_exact(&mut payload).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                ProtocolError::UnexpectedEof
            } else {
                ProtocolError::Io(e)
            }
        })?;

        let actual_checksum = compute_checksum(&payload);
        if actual_checksum != header.checksum {
            return Err(ProtocolError::ChecksumMismatch(
                header.checksum,
                actual_checksum,
            ));
        }

        Ok(Self { header, payload })
    }

    pub async fn read_from_async<R: AsyncRead + Unpin>(
        mut reader: R,
    ) -> Result<Self, ProtocolError> {
        let mut hdr_buf = [0u8; HEADER_SIZE];
        reader.read_exact(&mut hdr_buf).await.map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                ProtocolError::UnexpectedEof
            } else {
                ProtocolError::Io(e)
            }
        })?;

        let header = FrameHeader::deserialize(&hdr_buf)?;
        let mut payload = vec![0u8; header.payload_len as usize];
        reader.read_exact(&mut payload).await.map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                ProtocolError::UnexpectedEof
            } else {
                ProtocolError::Io(e)
            }
        })?;

        let actual_checksum = compute_checksum(&payload);
        if actual_checksum != header.checksum {
            return Err(ProtocolError::ChecksumMismatch(
                header.checksum,
                actual_checksum,
            ));
        }

        Ok(Self { header, payload })
    }
}
