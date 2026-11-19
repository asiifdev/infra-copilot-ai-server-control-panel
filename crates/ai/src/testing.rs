//! Test support: a scripted network transport and a scripted provider.
//!
//! Plain `pub` on purpose (no feature flag): the agent loop (M2-d) tests against
//! [`ScriptedProvider`], and anything that needs a provider without a network uses these.

use crate::error::AiError;
use crate::provider::{AiProvider, DeltaSink};
use crate::transport::{ByteStream, Transport, TransportError};
use crate::types::{ChatRequest, ChatResponse};
use bytes::Bytes;
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the fake server does for one request.
#[derive(Clone, Debug)]
pub enum Reply {
    /// The request fails before any answer.
    Fail(TransportError),
    /// An HTTP answer; the body pieces are delivered in order, then optionally an error.
    Http {
        /// Status code.
        status: u16,
        /// `Retry-After` seconds.
        retry_after: Option<u32>,
        /// Body pieces.
        chunks: Vec<Vec<u8>>,
        /// Fail with this after the last piece instead of ending cleanly.
        then_fail: Option<TransportError>,
    },
}

impl Reply {
    /// A 200 answer with the given body pieces.
    #[must_use]
    pub fn ok(chunks: Vec<&str>) -> Self {
        Self::Http {
            status: 200,
            retry_after: None,
            chunks: chunks.into_iter().map(|c| c.as_bytes().to_vec()).collect(),
            then_fail: None,
        }
    }

    /// An error status with a body.
    #[must_use]
    pub fn status(status: u16, body: &str) -> Self {
        Self::Http {
            status,
            retry_after: None,
            chunks: vec![body.as_bytes().to_vec()],
            then_fail: None,
        }
    }
}

/// One request as the fake saw it.
#[derive(Clone, Debug)]
pub struct Seen {
    /// URL.
    pub url: String,
    /// Body text.
    pub body: String,
    /// Headers.
    pub headers: Vec<(String, String)>,
}

/// A transport that records requests and replays queued replies.
#[derive(Debug, Default)]
pub struct FakeTransport {
    replies: Mutex<VecDeque<Reply>>,
    seen: Mutex<Vec<Seen>>,
}

impl FakeTransport {
    /// A fake with these replies, used in order.
    #[must_use]
    pub fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Requests seen so far.
    #[must_use]
    pub fn seen(&self) -> Vec<Seen> {
        lock(&self.seen).clone()
    }
}

/// The stream of a [`FakeTransport`] answer.
pub struct FakeStream {
    status: u16,
    retry_after: Option<u32>,
    chunks: VecDeque<Vec<u8>>,
    then_fail: Option<TransportError>,
}

impl ByteStream for FakeStream {
    fn status(&self) -> u16 {
        self.status
    }

    fn retry_after_secs(&self) -> Option<u32> {
        self.retry_after
    }

    async fn next_chunk(&mut self) -> Result<Option<Bytes>, TransportError> {
        match self.chunks.pop_front() {
            Some(chunk) => Ok(Some(Bytes::from(chunk))),
            None => self.then_fail.take().map_or(Ok(None), Err),
        }
    }
}

impl Transport for FakeTransport {
    type Stream = FakeStream;

    async fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<FakeStream, TransportError> {
        lock(&self.seen).push(Seen {
            url: url.to_owned(),
            body: String::from_utf8_lossy(body).into_owned(),
            headers: headers
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
        });
        match lock(&self.replies).pop_front() {
            None | Some(Reply::Fail(TransportError::Invalid)) => Err(TransportError::Invalid),
            Some(Reply::Fail(e)) => Err(e),
            Some(Reply::Http {
                status,
                retry_after,
                chunks,
                then_fail,
            }) => Ok(FakeStream {
                status,
                retry_after,
                chunks: chunks.into(),
                then_fail,
            }),
        }
    }
}

/// A provider that returns queued answers and records the requests it got. For testing code that
/// sits on top of [`AiProvider`].
#[derive(Debug, Default)]
pub struct ScriptedProvider {
    answers: Mutex<VecDeque<Result<ChatResponse, AiError>>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedProvider {
    /// A provider that answers with these, in order.
    #[must_use]
    pub fn new(answers: Vec<Result<ChatResponse, AiError>>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// Requests received so far.
    #[must_use]
    pub fn requests(&self) -> Vec<ChatRequest> {
        lock(&self.requests).clone()
    }
}

impl AiProvider for ScriptedProvider {
    async fn chat(
        &self,
        request: &ChatRequest,
        sink: &mut dyn DeltaSink,
    ) -> Result<ChatResponse, AiError> {
        lock(&self.requests).push(request.clone());
        let answer = lock(&self.answers)
            .pop_front()
            .unwrap_or(Err(AiError::Empty));
        if let Ok(response) = &answer
            && !response.content.is_empty()
        {
            sink.text(&response.content);
        }
        answer
    }
}
