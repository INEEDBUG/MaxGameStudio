//! The Demo library's coalesced change notifications for desktop EventSource.

use super::HttpBody;
use futures_util::{
    future::{select, Either},
    stream::{self, StreamExt},
};
use http_body_util::{BodyExt, StreamBody};
use hyper::body::{Bytes, Frame};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::{
    sync::{broadcast, watch, Semaphore},
    time::timeout,
};

pub(super) const CONTENT_TYPE: &str = "text/event-stream; charset=utf-8";
const MAX_SUBSCRIBERS: usize = 4;
const KEEPALIVE: Duration = Duration::from_secs(20);
const LIBRARY_EVENT: &str = "event: library\ndata: {\"reason\": \"changed\"}\n\n";

pub(super) struct DemoEvents {
    changes: watch::Sender<()>,
    published: broadcast::Sender<()>,
    slots: Arc<Semaphore>,
}

impl DemoEvents {
    pub(super) fn new() -> Self {
        Self {
            changes: watch::channel(()).0,
            published: broadcast::channel(64).0,
            slots: Arc::new(Semaphore::new(MAX_SUBSCRIBERS)),
        }
    }

    pub(super) fn changed(&self) {
        self.changes.send_replace(());
    }

    pub(super) fn publisher(
        &self,
        mut shutdown: watch::Receiver<bool>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let mut changes = self.changes.subscribe();
        let published = self.published.clone();
        let quiet_period = debounce();
        async move {
            let publish = async move {
                while changes.changed().await.is_ok() {
                    loop {
                        match timeout(quiet_period, changes.changed()).await {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => return,
                            Err(_) => {
                                let _ = published.send(());
                                break;
                            }
                        }
                    }
                }
            };
            let _ = select(Box::pin(shutdown.changed()), Box::pin(publish)).await;
        }
    }

    pub(super) fn stream(&self, shutdown: watch::Receiver<bool>) -> Option<HttpBody> {
        let permit = Arc::clone(&self.slots).try_acquire_owned().ok()?;
        let updates = self.published.subscribe();
        let frames = stream::unfold(
            (updates, shutdown, permit),
            |(mut updates, mut shutdown, permit)| async move {
                if *shutdown.borrow() {
                    return None;
                }
                let received = match select(
                    Box::pin(shutdown.changed()),
                    Box::pin(timeout(KEEPALIVE, updates.recv())),
                )
                .await
                {
                    Either::Left(_) => return None,
                    Either::Right((received, _)) => received,
                };
                let data = match received {
                    Ok(Err(broadcast::error::RecvError::Closed)) => return None,
                    Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                        // A slow client needs one fresh snapshot rather than a backlog of identical refreshes.
                        updates = updates.resubscribe();
                        LIBRARY_EVENT
                    }
                    Ok(Ok(())) => LIBRARY_EVENT,
                    Err(_) => ": keepalive\n\n",
                };
                Some((
                    Ok(Frame::data(Bytes::from_static(data.as_bytes()))),
                    (updates, shutdown, permit),
                ))
            },
        );
        let hello = stream::once(async { Ok(Frame::data(Bytes::from_static(b": ok\n\n"))) });
        Some(StreamBody::new(hello.chain(frames)).boxed_unsync())
    }
}

fn debounce() -> Duration {
    let seconds = std::env::var("CS2_INSIGHT_LIBRARY_SSE_DEBOUNCE_SEC")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .unwrap_or_default();
    let seconds = if seconds.is_nan() || seconds <= 0.0 {
        0.55
    } else {
        seconds.clamp(0.05, 5.0)
    };
    Duration::from_secs_f64(seconds)
}
