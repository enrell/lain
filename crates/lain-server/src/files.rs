//! A response body read from a file in chunks, for artifacts too large
//! to hold in memory (backups). It may own a scratch directory that goes
//! away with the body, after the last byte or a dropped connection.

use std::fs::File;
use std::io::Read;
use std::pin::Pin;
use std::task::{Context, Poll};

use hyper::body::{Bytes, Frame, SizeHint};

const CHUNK: usize = 256 << 10;

pub struct FileBody {
    // Field order is drop order: the file closes before its directory
    // is removed (Windows refuses to delete an open file).
    file: File,
    remaining: u64,
    _scratch: Option<lain_admin::backup::TempDir>,
}

impl FileBody {
    pub fn open(path: &std::path::Path, scratch: Option<lain_admin::backup::TempDir>) -> std::io::Result<FileBody> {
        let file = File::open(path)?;
        let remaining = file.metadata()?.len();
        Ok(FileBody { file, remaining, _scratch: scratch })
    }

    pub fn len(&self) -> u64 {
        self.remaining
    }

    pub fn is_empty(&self) -> bool {
        self.remaining == 0
    }
}

impl hyper::body::Body for FileBody {
    type Data = Bytes;
    type Error = std::io::Error;

    // Local-disk reads of one chunk block briefly; this body serves
    // admin downloads, never the hot media path.
    fn poll_frame(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if self.remaining == 0 {
            return Poll::Ready(None);
        }
        let want = CHUNK.min(self.remaining as usize);
        let mut buf = vec![0u8; want];
        match self.file.read(&mut buf) {
            Ok(0) => Poll::Ready(Some(Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "file shrank")))),
            Ok(n) => {
                buf.truncate(n);
                self.remaining -= n as u64;
                Poll::Ready(Some(Ok(Frame::data(Bytes::from(buf)))))
            }
            Err(e) => Poll::Ready(Some(Err(e))),
        }
    }

    fn is_end_stream(&self) -> bool {
        self.remaining == 0
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.remaining)
    }
}
