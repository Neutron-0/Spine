use std::io;
use thiserror::Error;

pub const PKT_FLUSH: &[u8; 4] = b"0000";
pub const PKT_DELIM: &[u8; 4] = b"0001";
pub const PKT_END: &[u8; 4] = b"0002";
pub const MAX_PKT_LINE_LEN: usize = 65520;
pub const MAX_SIDEBAND_PAYLOAD_LEN: usize = 65515; // 65520 - 4 (len) - 1 (channel)

#[derive(Error, Debug, PartialEq, Eq)]
pub enum PktLineError {
    #[error("Packet-line too short: {0} bytes")]
    TooShort(usize),
    #[error("Invalid packet-line hex length: {0}")]
    InvalidHexLength(String),
    #[error("Packet-line length {0} exceeds maximum allowed size of {MAX_PKT_LINE_LEN}")]
    PayloadTooLarge(usize),
    #[error("Unexpected end of packet-line stream: expected {expected} bytes, found {actual}")]
    UnexpectedEof { expected: usize, actual: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PktLine {
    Data(Vec<u8>),
    Flush,
    Delimiter,
    ResponseEnd,
}

impl PktLine {
    pub fn is_flush(&self) -> bool {
        matches!(self, PktLine::Flush)
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            PktLine::Data(b) => b.as_slice(),
            _ => &[],
        }
    }

    pub fn to_string_lossy(&self) -> String {
        match self {
            PktLine::Data(b) => String::from_utf8_lossy(b).trim_end_matches('\n').to_string(),
            PktLine::Flush => "0000 (flush)".to_string(),
            PktLine::Delimiter => "0001 (delim)".to_string(),
            PktLine::ResponseEnd => "0002 (end)".to_string(),
        }
    }
}

/// Encodes an arbitrary payload as a data packet-line (`<4-hex-len><payload>`).
pub fn encode_pkt_line(payload: &[u8]) -> Result<Vec<u8>, PktLineError> {
    if payload.len() + 4 > MAX_PKT_LINE_LEN {
        return Err(PktLineError::PayloadTooLarge(payload.len() + 4));
    }
    let total_len = payload.len() + 4;
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(format!("{:04x}", total_len).as_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// Encodes a text line ending with `\n` as a packet-line.
pub fn encode_pkt_line_str(text: &str) -> Result<Vec<u8>, PktLineError> {
    let mut s = text.to_string();
    if !s.ends_with('\n') {
        s.push('\n');
    }
    encode_pkt_line(s.as_bytes())
}

/// Returns the flush packet-line (`0000`).
pub fn encode_flush() -> &'static [u8] {
    PKT_FLUSH
}

/// Returns the delimiter packet-line (`0001`).
pub fn encode_delim() -> &'static [u8] {
    PKT_DELIM
}

/// Returns the response-end packet-line (`0002`).
pub fn encode_end() -> &'static [u8] {
    PKT_END
}

/// Frames payload data into one or more sideband packet-lines.
/// Channel: 1 = data, 2 = progress, 3 = error.
pub fn encode_sideband(channel: u8, payload: &[u8]) -> Vec<u8> {
    if payload.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for chunk in payload.chunks(MAX_SIDEBAND_PAYLOAD_LEN) {
        let total_len = chunk.len() + 5; // 4 bytes hex len + 1 byte channel
        out.extend_from_slice(format!("{:04x}", total_len).as_bytes());
        out.push(channel);
        out.extend_from_slice(chunk);
    }
    out
}

/// Parses a single packet-line from `input`.
/// Returns `Ok(Some((pkt, bytes_consumed)))` or `Ok(None)` if input is incomplete.
pub fn parse_pkt_line(input: &[u8]) -> Result<Option<(PktLine, usize)>, PktLineError> {
    if input.len() < 4 {
        return Ok(None);
    }

    let len_str = match std::str::from_utf8(&input[..4]) {
        Ok(s) => s,
        Err(_) => return Err(PktLineError::InvalidHexLength("Non-UTF8 hex prefix".into())),
    };

    let total_len = match usize::from_str_radix(len_str, 16) {
        Ok(n) => n,
        Err(_) => return Err(PktLineError::InvalidHexLength(len_str.to_string())),
    };

    match total_len {
        0 => Ok(Some((PktLine::Flush, 4))),
        1 => Ok(Some((PktLine::Delimiter, 4))),
        2 => Ok(Some((PktLine::ResponseEnd, 4))),
        3 => Err(PktLineError::InvalidHexLength(
            "Packet length 3 is invalid".into(),
        )),
        len if len > MAX_PKT_LINE_LEN => Err(PktLineError::PayloadTooLarge(len)),
        len => {
            if input.len() < len {
                // Incomplete packet in buffer
                return Ok(None);
            }
            let payload = input[4..len].to_vec();
            Ok(Some((PktLine::Data(payload), len)))
        }
    }
}

/// Parses all packet-lines from a stream slice up to a flush or until buffer exhaustion.
pub fn parse_pkt_lines(mut input: &[u8]) -> Result<(Vec<PktLine>, usize), PktLineError> {
    let mut lines = Vec::new();
    let mut total_consumed = 0;

    while let Some((pkt, consumed)) = parse_pkt_line(input)? {
        let is_flush = pkt.is_flush();
        lines.push(pkt);
        input = &input[consumed..];
        total_consumed += consumed;
        if is_flush {
            break;
        }
    }

    Ok((lines, total_consumed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkt_line_encode_decode_roundtrip() {
        let text = "want 1234567890123456789012345678901234567890\n";
        let encoded = encode_pkt_line_str(text).unwrap();
        assert_eq!(&encoded[..4], format!("{:04x}", text.len() + 4).as_bytes());

        let (pkt, consumed) = parse_pkt_line(&encoded).unwrap().unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(pkt.as_bytes(), text.as_bytes());
    }

    #[test]
    fn test_pkt_line_special_packets() {
        assert_eq!(parse_pkt_line(b"0000").unwrap().unwrap(), (PktLine::Flush, 4));
        assert_eq!(parse_pkt_line(b"0001").unwrap().unwrap(), (PktLine::Delimiter, 4));
        assert_eq!(parse_pkt_line(b"0002").unwrap().unwrap(), (PktLine::ResponseEnd, 4));
    }

    #[test]
    fn test_sideband_encoding() {
        let data = b"Sideband channel 1 payload";
        let encoded = encode_sideband(1, data);
        assert_eq!(&encoded[4..5], &[1]);
        assert_eq!(&encoded[5..], data);
    }
}
