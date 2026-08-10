use anyhow::anyhow;
use futures::{AsyncReadExt as _, future::BoxFuture};
use gpui::http_client::{AsyncBody, HttpClient, RedirectPolicy, Request, Response};
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(catch, js_name = "fetch")]
    fn global_fetch(input: &web_sys::Request) -> Result<js_sys::Promise, JsValue>;
}

pub struct FetchHttpClient;

impl Default for FetchHttpClient {
    fn default() -> Self {
        Self
    }
}

#[cfg(feature = "multithreaded")]
impl FetchHttpClient {
    pub unsafe fn new() -> Self {
        Self
    }
}

#[cfg(not(feature = "multithreaded"))]
impl FetchHttpClient {
    pub fn new() -> Self {
        Self
    }
}

/// Wraps a `!Send` future to satisfy the `Send` bound on `BoxFuture`.
struct AssertSend<F>(F);

unsafe impl<F> Send for AssertSend<F> {}

impl<F: Future> Future for AssertSend<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let inner = unsafe { self.map_unchecked_mut(|this| &mut this.0) };
        inner.poll(cx)
    }
}

impl HttpClient for FetchHttpClient {
    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        Box::pin(AssertSend(async move {
            let redirect_policy = request
                .extensions()
                .get::<RedirectPolicy>()
                .cloned()
                .unwrap_or_default();
            let (parts, mut body) = request.into_parts();
            let url = parts.uri.to_string();
            let init = web_sys::RequestInit::new();
            init.set_method(parts.method.as_str());
            init.set_redirect(match redirect_policy {
                RedirectPolicy::NoFollow => web_sys::RequestRedirect::Manual,
                RedirectPolicy::FollowAll => web_sys::RequestRedirect::Follow,
                RedirectPolicy::FollowLimit(limit) => {
                    anyhow::bail!(
                        "the browser Fetch API cannot enforce a redirect limit of {limit}"
                    );
                }
            });

            let request_headers = web_sys::Headers::new()
                .map_err(|error| anyhow!("failed to create fetch Headers: {error:?}"))?;
            for (name, value) in &parts.headers {
                let value = value
                    .to_str()
                    .map_err(|error| anyhow!("request header {name} is not valid text: {error}"))?;
                request_headers
                    .append(name.as_str(), value)
                    .map_err(|error| {
                        anyhow!("failed to append request header {name}: {error:?}")
                    })?;
            }
            init.set_headers(request_headers.as_ref());

            let mut request_body = Vec::new();
            body.read_to_end(&mut request_body).await?;
            let request_body = (!request_body.is_empty()).then(|| {
                let bytes = js_sys::Uint8Array::from(request_body.as_slice());
                init.set_body(bytes.as_ref());
                bytes
            });

            let web_request = web_sys::Request::new_with_str_and_init(&url, &init)
                .map_err(|error| anyhow!("failed to create fetch Request: {error:?}"))?;
            drop(request_body);

            let promise = global_fetch(&web_request)
                .map_err(|error| anyhow!("fetch threw an error: {error:?}"))?;
            let response_value = wasm_bindgen_futures::JsFuture::from(promise)
                .await
                .map_err(|error| anyhow!("fetch failed: {error:?}"))?;

            let web_response: web_sys::Response = response_value
                .dyn_into()
                .map_err(|error| anyhow!("fetch result is not a Response: {error:?}"))?;

            let status_code = http::StatusCode::from_u16(web_response.status())
                .map_err(|_| anyhow!("invalid status code"))?;
            let mut response = Response::builder().status(status_code);
            let response_headers = response
                .headers_mut()
                .ok_or_else(|| anyhow!("failed to initialize response headers"))?;
            let entries = web_response.headers().entries();
            loop {
                let next = entries
                    .next()
                    .map_err(|error| anyhow!("failed to read response headers: {error:?}"))?;
                if next.done() {
                    break;
                }
                let entry: js_sys::Array = next
                    .value()
                    .dyn_into()
                    .map_err(|error| anyhow!("response header entry is not an array: {error:?}"))?;
                let name = entry
                    .get(0)
                    .as_string()
                    .ok_or_else(|| anyhow!("response header name is not a string"))?;
                let value = entry
                    .get(1)
                    .as_string()
                    .ok_or_else(|| anyhow!("response header value is not a string"))?;
                let name = http::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|error| anyhow!("invalid response header name: {error}"))?;
                let value = http::HeaderValue::from_str(&value)
                    .map_err(|error| anyhow!("invalid response header value: {error}"))?;
                response_headers.append(name, value);
            }

            let body_promise = web_response
                .array_buffer()
                .map_err(|error| anyhow!("failed to initiate response body read: {error:?}"))?;
            let body_value = wasm_bindgen_futures::JsFuture::from(body_promise)
                .await
                .map_err(|error| anyhow!("failed to read response body: {error:?}"))?;
            let array_buffer: js_sys::ArrayBuffer = body_value
                .dyn_into()
                .map_err(|error| anyhow!("response body is not an ArrayBuffer: {error:?}"))?;
            let body = js_sys::Uint8Array::new(&array_buffer).to_vec();

            Ok(response.body(AsyncBody::from(body))?)
        }))
    }
}
