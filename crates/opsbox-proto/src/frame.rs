//! Wire framing for the privileged socket: a 4-byte big-endian length, then that
//! many bytes of JSON. Pure functions only; the async read/write loops live in
//! `opsbox-priv`.

use serde::{Serialize, de::DeserializeOwned};

/// Largest accepted frame body. It must hold the biggest thing sent inline: a file the
/// editor opens or saves (up to 5 MB of text, which can double or more once JSON-escaped).
/// Truly large payloads (uploads, downloads, PTY bytes) never travel in frames: they go
/// through the transfer folder or their own streams.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame of {0} bytes exceeds the {MAX_FRAME_LEN} byte limit")]
    TooLarge(usize),
    #[error("invalid JSON in frame: {0}")]
    Json(#[from] serde_json::Error),
}

/// Serialize `msg` into a complete frame (length prefix included).
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge(body.len()));
    }
    let len = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge(body.len()))?;
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Read the body length from a frame header, enforcing [`MAX_FRAME_LEN`] *before*
/// the caller allocates a buffer for it.
pub fn body_len(header: [u8; 4]) -> Result<usize, FrameError> {
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge(len));
    }
    Ok(len)
}

/// Parse a frame body.
pub fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, FrameError> {
    Ok(serde_json::from_slice(body)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privilege::{PrivOp, PrivRequest};

    #[test]
    fn round_trip() {
        let req = PrivRequest {
            id: "req-1".into(),
            actor: Some("admin".into()),
            op: PrivOp::HostInfo,
        };
        let frame = encode(&req).unwrap();
        let header: [u8; 4] = frame[..4].try_into().unwrap();
        assert_eq!(body_len(header).unwrap(), frame.len() - 4);
        let back: PrivRequest = decode(&frame[4..]).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn oversized_header_is_rejected_before_allocation() {
        let header = ((MAX_FRAME_LEN as u32) + 1).to_be_bytes();
        assert!(matches!(body_len(header), Err(FrameError::TooLarge(_))));
        assert!(matches!(
            body_len(u32::MAX.to_be_bytes()),
            Err(FrameError::TooLarge(_))
        ));
    }

    #[test]
    fn garbage_body_is_an_error_not_a_panic() {
        assert!(decode::<PrivRequest>(b"\xff\x00not json").is_err());
    }
}
