//! Async frame I/O on top of the pure codec in `opsbox_proto::frame`.

use std::io;

use opsbox_proto::frame::{self, FrameError};
use opsbox_proto::{PtyFrame, PtyFrameError};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Read one frame body. `Ok(None)` means the peer closed the connection cleanly
/// between frames.
pub(crate) async fn read_body<R>(r: &mut R) -> Result<Option<Vec<u8>>, ReadError>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0u8; 4];
    // Distinguish "closed between frames" from "closed mid-header".
    let first = r.read(&mut header[..1]).await?;
    if first == 0 {
        return Ok(None);
    }
    r.read_exact(&mut header[1..]).await?;
    let len = frame::body_len(header)?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    Ok(Some(body))
}

pub(crate) async fn write_frame<W, T>(w: &mut W, msg: &T) -> Result<(), WriteError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = frame::encode(msg)?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

/// Read one PTY stream frame. `Ok(None)` is a clean close between frames.
pub(crate) async fn read_pty_frame<R>(r: &mut R) -> Result<Option<PtyFrame>, PtyIoError>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0u8; 4];
    if r.read(&mut header[..1]).await? == 0 {
        return Ok(None);
    }
    r.read_exact(&mut header[1..]).await?;
    let len = PtyFrame::body_len(header)?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    Ok(Some(PtyFrame::decode(&body)?))
}

pub(crate) async fn write_pty_frame<W>(w: &mut W, frame: &PtyFrame) -> Result<(), io::Error>
where
    W: AsyncWrite + Unpin,
{
    w.write_all(&frame.encode()).await?;
    w.flush().await
}

#[derive(Debug, thiserror::Error)]
pub enum PtyIoError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Frame(#[from] PtyFrameError),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReadError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Frame(#[from] FrameError),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum WriteError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Frame(#[from] FrameError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use opsbox_proto::{PrivOp, PrivReply, PrivRequest, PrivResponse};

    #[tokio::test]
    async fn priv_messages_round_trip_over_a_stream() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let req = PrivRequest {
            id: "r1".into(),
            actor: Some("admin".into()),
            op: PrivOp::HostInfo,
        };
        let resp = PrivResponse::success(
            "r1",
            PrivReply::Pong {
                version: "9".into(),
            },
        );

        write_frame(&mut a, &req).await.unwrap();
        write_frame(&mut a, &resp).await.unwrap();
        drop(a);

        let body = read_body(&mut b).await.unwrap().unwrap();
        assert_eq!(frame::decode::<PrivRequest>(&body).unwrap(), req);
        let body = read_body(&mut b).await.unwrap().unwrap();
        assert_eq!(frame::decode::<PrivResponse>(&body).unwrap(), resp);
        assert!(read_body(&mut b).await.unwrap().is_none(), "clean EOF");
    }

    #[tokio::test]
    async fn every_op_survives_a_round_trip() {
        for op in [PrivOp::Ping, PrivOp::HostInfo] {
            let (mut a, mut b) = tokio::io::duplex(4096);
            let req = PrivRequest {
                id: "x".into(),
                actor: None,
                op,
            };
            write_frame(&mut a, &req).await.unwrap();
            let body = read_body(&mut b).await.unwrap().unwrap();
            assert_eq!(frame::decode::<PrivRequest>(&body).unwrap(), req);
        }
    }

    #[tokio::test]
    async fn oversized_frames_are_refused_without_reading_the_body() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        let err = read_body(&mut b).await.unwrap_err();
        assert!(matches!(err, ReadError::Frame(FrameError::TooLarge(_))));
    }

    #[tokio::test]
    async fn truncated_frame_is_an_error() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&10u32.to_be_bytes()).await.unwrap();
        a.write_all(b"abc").await.unwrap();
        drop(a);
        assert!(matches!(read_body(&mut b).await, Err(ReadError::Io(_))));
    }
}
