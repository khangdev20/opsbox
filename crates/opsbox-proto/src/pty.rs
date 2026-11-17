//! The terminal: Linux users a shell can be opened as, and the framing of the byte stream
//! between `serve` and `priv` once a PTY is open.
//!
//! After `PtyOpen` succeeds, the connection stops speaking JSON. Both directions then carry
//! frames of `u32` big-endian length (of kind + payload), one `kind` byte and the payload.

use serde::{Deserialize, Serialize};

/// A local account, as far as the terminal and the Access page need it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemUser {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
    pub shell: String,
    /// The account has a real login shell (not `nologin` or `false`).
    pub login: bool,
}

/// Largest payload of a single stream frame. Terminal output is cut into pieces of this
/// size, so one slow reader never makes the other side buffer a lot.
pub const MAX_PTY_PAYLOAD: usize = 16 * 1024;

/// Largest terminal size accepted, in either direction.
pub const MAX_PTY_DIM: u16 = 1000;

/// One message on a PTY stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyFrame {
    /// Bytes for the terminal (towards priv) or from it (towards serve).
    Data(Vec<u8>),
    /// The terminal was resized. Towards priv only.
    Resize { cols: u16, rows: u16 },
    /// The shell ended. Towards serve only. `code` is its exit status, or `None` when a
    /// signal ended it.
    Exit { code: Option<i32> },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PtyFrameError {
    #[error("stream frame of {0} bytes is not valid")]
    BadLength(usize),
    #[error("unknown stream frame kind {0}")]
    UnknownKind(u8),
    #[error("stream frame is malformed")]
    Malformed,
}

const K_DATA: u8 = 0;
const K_RESIZE: u8 = 1;
const K_EXIT: u8 = 2;

/// Check a terminal size and turn it into something usable.
pub fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(2, MAX_PTY_DIM), rows.clamp(2, MAX_PTY_DIM))
}

impl PtyFrame {
    /// Serialise into a complete frame (length prefix included).
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Vec::new();
        match self {
            Self::Data(bytes) => {
                body.push(K_DATA);
                body.extend_from_slice(bytes);
            }
            Self::Resize { cols, rows } => {
                body.push(K_RESIZE);
                body.extend_from_slice(&cols.to_be_bytes());
                body.extend_from_slice(&rows.to_be_bytes());
            }
            Self::Exit { code } => {
                body.push(K_EXIT);
                match code {
                    Some(c) => {
                        body.push(1);
                        body.extend_from_slice(&c.to_be_bytes());
                    }
                    None => body.push(0),
                }
            }
        }
        // Frames are at most MAX_PTY_PAYLOAD + 5 bytes, far below u32::MAX.
        let len = u32::try_from(body.len()).unwrap_or(u32::MAX);
        let mut out = Vec::with_capacity(4 + body.len());
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Read the length from a header, enforcing the size limit before anything is allocated.
    pub fn body_len(header: [u8; 4]) -> Result<usize, PtyFrameError> {
        let len = u32::from_be_bytes(header) as usize;
        if len == 0 || len > MAX_PTY_PAYLOAD + 1 {
            return Err(PtyFrameError::BadLength(len));
        }
        Ok(len)
    }

    /// Parse a frame body (kind byte and payload, without the length prefix).
    pub fn decode(body: &[u8]) -> Result<Self, PtyFrameError> {
        let (&kind, rest) = body.split_first().ok_or(PtyFrameError::BadLength(0))?;
        match kind {
            K_DATA => Ok(Self::Data(rest.to_vec())),
            K_RESIZE => {
                let b: [u8; 4] = rest.try_into().map_err(|_| PtyFrameError::Malformed)?;
                Ok(Self::Resize {
                    cols: u16::from_be_bytes([b[0], b[1]]),
                    rows: u16::from_be_bytes([b[2], b[3]]),
                })
            }
            K_EXIT => match rest {
                [0] => Ok(Self::Exit { code: None }),
                [1, a, b, c, d] => Ok(Self::Exit {
                    code: Some(i32::from_be_bytes([*a, *b, *c, *d])),
                }),
                _ => Err(PtyFrameError::Malformed),
            },
            other => Err(PtyFrameError::UnknownKind(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(f: PtyFrame) {
        let bytes = f.encode();
        let header: [u8; 4] = bytes[..4].try_into().unwrap();
        assert_eq!(PtyFrame::body_len(header).unwrap(), bytes.len() - 4);
        assert_eq!(PtyFrame::decode(&bytes[4..]).unwrap(), f);
    }

    #[test]
    fn every_kind_round_trips() {
        round(PtyFrame::Data(b"ls -la\r".to_vec()));
        round(PtyFrame::Data(Vec::new()));
        round(PtyFrame::Data(vec![0u8; MAX_PTY_PAYLOAD]));
        round(PtyFrame::Resize {
            cols: 120,
            rows: 40,
        });
        round(PtyFrame::Exit { code: Some(0) });
        round(PtyFrame::Exit { code: Some(-1) });
        round(PtyFrame::Exit { code: None });
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(PtyFrame::body_len(0u32.to_be_bytes()).is_err());
        assert!(PtyFrame::body_len(((MAX_PTY_PAYLOAD + 2) as u32).to_be_bytes()).is_err());
        assert!(PtyFrame::body_len(u32::MAX.to_be_bytes()).is_err());
        assert_eq!(
            PtyFrame::decode(&[9, 1]),
            Err(PtyFrameError::UnknownKind(9))
        );
        assert_eq!(
            PtyFrame::decode(&[K_RESIZE, 0, 1]),
            Err(PtyFrameError::Malformed)
        );
        assert_eq!(
            PtyFrame::decode(&[K_EXIT, 1, 0]),
            Err(PtyFrameError::Malformed)
        );
        assert_eq!(PtyFrame::decode(&[]), Err(PtyFrameError::BadLength(0)));
    }

    #[test]
    fn sizes_are_kept_sane() {
        assert_eq!(clamp_size(0, 0), (2, 2));
        assert_eq!(clamp_size(80, 24), (80, 24));
        assert_eq!(clamp_size(u16::MAX, u16::MAX), (MAX_PTY_DIM, MAX_PTY_DIM));
    }
}
