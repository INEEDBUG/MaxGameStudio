use super::HttpResponse;
use http_body_util::BodyExt;
use hyper::{body::Incoming, header::RANGE, Request};
use std::{
    io,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};
use tower_http::services::ServeFile;

pub(super) async fn serve(
    file: PathBuf,
    mut request: Request<Incoming>,
    extended_request: &AtomicBool,
) -> io::Result<HttpResponse> {
    // Browser seeking sends one range. Use the full file for multipart ranges
    // to avoid building a multipart response in memory.
    if request
        .headers()
        .get(RANGE)
        .is_some_and(|range| range.as_bytes().contains(&b','))
    {
        request.headers_mut().remove(RANGE);
    }
    let response = ServeFile::new(file).try_call(request).await?;
    if response.status().is_success() {
        extended_request.store(true, Ordering::Relaxed);
    }
    Ok(response.map(BodyExt::boxed_unsync))
}
