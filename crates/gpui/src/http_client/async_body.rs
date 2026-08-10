use std::{
    io::{Cursor, Read},
    pin::Pin,
    task::Poll,
};

use bytes::Bytes;
use futures::AsyncRead;
use http_body::{Body, Frame};
use serde::Serialize;

/// An asynchronous HTTP request or response body.
pub struct AsyncBody(pub Inner);

/// The storage backing an [`AsyncBody`].
pub enum Inner {
    /// An empty body.
    Empty,
    /// A body stored in memory.
    Bytes(Cursor<Bytes>),
    /// A streaming asynchronous reader.
    AsyncReader(Pin<Box<dyn AsyncRead + Send + Sync>>),
}

impl AsyncBody {
    /// Creates an empty body.
    pub fn empty() -> Self {
        Self(Inner::Empty)
    }

    /// Creates a streaming body backed by an asynchronous reader.
    pub fn from_reader<R>(reader: R) -> Self
    where
        R: AsyncRead + Send + Sync + 'static,
    {
        Self(Inner::AsyncReader(Box::pin(reader)))
    }

    /// Creates an in-memory body.
    pub fn from_bytes(bytes: Bytes) -> Self {
        Self(Inner::Bytes(Cursor::new(bytes)))
    }
}

impl Default for AsyncBody {
    fn default() -> Self {
        Self::empty()
    }
}

impl From<()> for AsyncBody {
    fn from(_: ()) -> Self {
        Self::empty()
    }
}

impl From<Bytes> for AsyncBody {
    fn from(bytes: Bytes) -> Self {
        Self::from_bytes(bytes)
    }
}

impl From<Vec<u8>> for AsyncBody {
    fn from(bytes: Vec<u8>) -> Self {
        Self::from_bytes(bytes.into())
    }
}

impl From<String> for AsyncBody {
    fn from(string: String) -> Self {
        Self::from_bytes(string.into())
    }
}

impl From<&'static [u8]> for AsyncBody {
    fn from(bytes: &'static [u8]) -> Self {
        Self::from_bytes(Bytes::from_static(bytes))
    }
}

impl From<&'static str> for AsyncBody {
    fn from(string: &'static str) -> Self {
        Self::from_bytes(Bytes::from_static(string.as_bytes()))
    }
}

/// Wraps a serializable value so it can be converted into a JSON body.
pub struct Json<T: Serialize>(pub T);

impl<T: Serialize> From<Json<T>> for AsyncBody {
    fn from(json: Json<T>) -> Self {
        Self::from_bytes(
            serde_json::to_vec(&json.0)
                .expect("failed to serialize JSON")
                .into(),
        )
    }
}

impl<T: Into<Self>> From<Option<T>> for AsyncBody {
    fn from(body: Option<T>) -> Self {
        match body {
            Some(body) => body.into(),
            None => Self::empty(),
        }
    }
}

impl AsyncRead for AsyncBody {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        // SAFETY: `Inner` is never moved while its `AsyncReader` variant is pinned.
        let inner = unsafe { &mut self.get_unchecked_mut().0 };
        match inner {
            Inner::Empty => Poll::Ready(Ok(0)),
            Inner::Bytes(cursor) => Poll::Ready(cursor.read(buffer)),
            Inner::AsyncReader(reader) => AsyncRead::poll_read(reader.as_mut(), cx, buffer),
        }
    }
}

impl Body for AsyncBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let mut buffer = vec![0; 8192];
        match AsyncRead::poll_read(self.as_mut(), cx, &mut buffer) {
            Poll::Ready(Ok(0)) => Poll::Ready(None),
            Poll::Ready(Ok(length)) => Poll::Ready(Some(Ok(Frame::data(Bytes::copy_from_slice(
                &buffer[..length],
            ))))),
            Poll::Ready(Err(error)) => Poll::Ready(Some(Err(error))),
            Poll::Pending => Poll::Pending,
        }
    }
}
