mod async_body;

pub use async_body::{AsyncBody, Inner, Json};
use futures::{AsyncReadExt as _, future::BoxFuture};
use http::HeaderValue;
pub use http::{self, Method, Request, Response, StatusCode, Uri, request::Builder};
pub use url::{Host, Url};

/// A simple HTTP response.
#[derive(Debug)]
pub struct HttpResponse {
    /// The HTTP status code.
    pub status: StatusCode,
    /// The response body bytes.
    pub body: Vec<u8>,
}

/// Controls how an HTTP client follows redirects.
#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum RedirectPolicy {
    /// Do not follow redirects.
    #[default]
    NoFollow,
    /// Follow at most the specified number of redirects.
    FollowLimit(u32),
    /// Follow redirects without an application-level limit.
    FollowAll,
}

/// Extension methods for attaching GPUI HTTP metadata to a request.
pub trait HttpRequestExt {
    /// Attaches a redirect policy to this request builder.
    fn follow_redirects(self, follow: RedirectPolicy) -> Self;
}

impl HttpRequestExt for Builder {
    fn follow_redirects(self, follow: RedirectPolicy) -> Self {
        self.extension(follow)
    }
}

/// A trait for making HTTP requests.
pub trait HttpClient: 'static + Send + Sync {
    /// Returns the user agent configured for this client.
    fn user_agent(&self) -> Option<&HeaderValue> {
        None
    }

    /// Returns the proxy configured for this client.
    fn proxy(&self) -> Option<&Url> {
        None
    }

    /// Performs an HTTP request.
    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>>;

    /// Performs a GET request using the legacy GPUI HTTP client contract.
    fn get(
        &self,
        uri: &str,
        body: AsyncBody,
        follow_redirects: bool,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let request = Builder::new()
            .uri(uri)
            .follow_redirects(if follow_redirects {
                RedirectPolicy::FollowAll
            } else {
                RedirectPolicy::NoFollow
            })
            .body(body);

        match request {
            Ok(request) => self.send(request),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }

    /// Performs a GET request and buffers the full response body.
    fn get_bytes(
        &self,
        uri: &str,
        follow_redirects: bool,
    ) -> BoxFuture<'static, anyhow::Result<HttpResponse>> {
        let response = self.get(uri, AsyncBody::empty(), follow_redirects);
        Box::pin(async move {
            let response = response.await?;
            let status = response.status();
            let mut body = response.into_body();
            let mut bytes = Vec::new();
            body.read_to_end(&mut bytes).await?;
            Ok(HttpResponse {
                status,
                body: bytes,
            })
        })
    }
}

/// An HTTP client that always returns an error.
pub struct NullHttpClient;

impl HttpClient for NullHttpClient {
    fn send(
        &self,
        _request: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        Box::pin(async { anyhow::bail!("No HttpClient available") })
    }
}

/// An HTTP client that blocks all requests.
pub struct BlockedHttpClient;

impl BlockedHttpClient {
    /// Create a new `BlockedHttpClient`.
    pub fn new() -> Self {
        Self
    }
}

impl Default for BlockedHttpClient {
    fn default() -> Self {
        Self
    }
}

impl HttpClient for BlockedHttpClient {
    fn send(
        &self,
        _request: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        Box::pin(async {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "BlockedHttpClient disallowed request",
            )
            .into())
        })
    }
}

/// A fake HTTP client for testing.
#[cfg(any(test, feature = "test-support"))]
pub struct FakeHttpClient {
    status: StatusCode,
}

#[cfg(any(test, feature = "test-support"))]
impl FakeHttpClient {
    /// Create a fake client that returns 404 responses.
    pub fn with_404_response() -> std::sync::Arc<dyn HttpClient> {
        std::sync::Arc::new(Self {
            status: StatusCode::NOT_FOUND,
        })
    }

    /// Create a fake client that returns 200 responses.
    pub fn with_200_response() -> std::sync::Arc<dyn HttpClient> {
        std::sync::Arc::new(Self {
            status: StatusCode::OK,
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl HttpClient for FakeHttpClient {
    fn send(
        &self,
        _request: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let status = self.status;
        Box::pin(async move {
            Ok(Response::builder()
                .status(status)
                .body(AsyncBody::empty())?)
        })
    }
}
