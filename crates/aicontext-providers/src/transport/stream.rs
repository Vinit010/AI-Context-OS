//! Turns a streaming HTTP response into a `StreamEvent` stream, for every adapter.
//!
//! The bytes arrive on the reader's task, so a small task reads them and forwards parsed events
//! over a channel; dropping the returned stream drops the receiver, the next send fails, and the
//! task ends with the response. That is what makes cancellation and an abandoned stream both
//! bounded (`docs/AI_PROVIDER_SPEC.md` §6).
//!
//! The pump is identical for all three adapters — SSE over a chunked read, one channel, one
//! cancellation race — and only the meaning of a payload differs, so an adapter passes its own
//! `translate` in. A `None` from `translate` ends the stream, which is how OpenAI-compatible
//! servers' `data: [DONE]` sentinel is handled without the pump knowing it exists.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use tokio::sync::mpsc;

use crate::cancel::CancellationToken;
use crate::provider::BoxStream;
use crate::types::StreamEvent;

use super::sse::SseParser;

/// How many parsed events may wait before the reader is back-pressured.
const CAPACITY: usize = 32;

/// A [`StreamEvent`] stream backed by the reader task; the task owns the HTTP response.
pub(crate) fn open<T>(
    response: reqwest::Response,
    cancel: CancellationToken,
    translate: T,
) -> BoxStream<StreamEvent>
where
    T: FnMut(&str) -> Option<Vec<StreamEvent>> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel::<StreamEvent>(CAPACITY);
    tokio::spawn(async move {
        let mut response = response;
        let mut parser = SseParser::new();
        let mut translate = translate;
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                chunk = response.chunk() => match chunk {
                    Ok(Some(bytes)) => {
                        let text = String::from_utf8_lossy(&bytes);
                        parser.push(&text);
                        while let Some(payload) = parser.next() {
                            let Some(events) = translate(&payload) else {
                                return;
                            };
                            for event in events {
                                if sender.send(event).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    // A clean end, a transport error, and a dropping receiver all end the stream:
                    // `StreamEvent` has no error item, so there is nothing to report and nothing to
                    // invent (docs/AI_PROVIDER_SPEC.md §2).
                    Ok(None) | Err(_) => return,
                },
            }
        }
    });
    Box::pin(EventReceiver { receiver })
}

/// A `Stream` over the receiver end of the reader channel.
struct EventReceiver {
    receiver: mpsc::Receiver<StreamEvent>,
}

impl Stream for EventReceiver {
    type Item = StreamEvent;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(context)
    }
}
