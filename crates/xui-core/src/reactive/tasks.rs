//! `resource!`: futures started by tracked code, whose results come back to
//! the main thread and are written to a source there.
//!
//! Sources are `Rc`s and stay on the main thread. A future only carries a
//! ticket; when it finishes, its result is sent over the app's channel as
//! `(ticket, Box<dyn Any + Send>)`, and [`Tasks::finish`] hands it to the
//! write registered for that ticket when the future was started.
//!
//! A `Cx` cannot reach the runtime, so `resource!` only queues the future
//! on the notifier; the runtime starts it when it next schedules.
use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use super::Cx;
use super::graph::SourceId;
use super::signal::{Signal, Source};

/// What a `resource!` holds: its last result, or `Pending` while one is coming.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsyncValue<T, E> {
    Pending,
    Ready(T),
    Error(E),
}

impl<T, E> AsyncValue<T, E> {
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }
}

impl<T, E> From<Result<T, E>> for AsyncValue<T, E> {
    fn from(result: Result<T, E>) -> Self {
        match result {
            Ok(value) => Self::Ready(value),
            Err(error) => Self::Error(error),
        }
    }
}

pub(crate) type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
pub(crate) type Finished = (u64, Box<dyn Any + Send>);
type Wake = Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>;
type Apply = Box<dyn FnOnce(Box<dyn Any + Send>, &mut Cx<'_>)>;

/// A future `resource!` asked for, waiting for the runtime to start it.
pub(crate) struct Spawn {
    /// The source its result is written to: a newer start for the same
    /// source drops this one's result.
    target: SourceId,
    future: Pin<Box<dyn Future<Output = Box<dyn Any + Send>> + Send>>,
    apply: Apply,
}

/// The app's side of `resource!`: how to start a future, and the writes
/// waiting for results.
#[derive(Default)]
pub(crate) struct Tasks {
    spawn: Option<Box<dyn Fn(BoxFuture)>>,
    sender: Option<mpsc::Sender<Finished>>,
    wake: Wake,
    next: u64,
    /// The latest ticket started for each source.
    latest: HashMap<SourceId, u64>,
    waiting: HashMap<u64, (SourceId, Apply)>,
}

impl Tasks {
    /// Where futures run, and where their results go.
    pub(crate) fn connect(&mut self, spawn: Box<dyn Fn(BoxFuture)>, sender: mpsc::Sender<Finished>) {
        self.spawn = Some(spawn);
        self.sender = Some(sender);
    }

    /// Called after a result is sent, so a sleeping loop drains it.
    pub(crate) fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.wake.lock().unwrap() = Some(wake);
    }

    /// Starts `spawn`'s future; its result is written with its `apply` on
    /// the main thread, unless another is started for the same source first.
    pub(crate) fn start(&mut self, Spawn { target, future, apply }: Spawn) {
        let (Some(spawn), Some(sender)) = (&self.spawn, &self.sender) else {
            panic!("`resource!` needs an app's async runtime: mount the component in an `App`");
        };
        self.next += 1;
        let ticket = self.next;
        if let Some(older) = self.latest.insert(target, ticket) {
            self.waiting.remove(&older);
        }
        self.waiting.insert(ticket, (target, apply));
        let (sender, wake) = (sender.clone(), self.wake.clone());
        spawn(Box::pin(async move {
            let result = future.await;
            if sender.send((ticket, result)).is_ok()
                && let Some(wake) = wake.lock().unwrap().clone()
            {
                wake();
            }
        }));
    }

    /// The write waiting for `ticket`'s result, if it still is.
    pub(crate) fn finish(&mut self, ticket: u64) -> Option<Apply> {
        let (target, apply) = self.waiting.remove(&ticket)?;
        self.latest.remove(&target);
        Some(apply)
    }
}

impl Cx<'_> {
    /// `resource!`'s effect: starts `future` and writes its result to `value`,
    /// unless it has started another since.
    #[doc(hidden)]
    pub fn __resource<T: 'static, E: 'static>(
        &mut self,
        value: &Signal<AsyncValue<T, E>>,
        future: impl Future<Output = Result<T, E>> + Send + 'static,
    ) where
        T: Send,
        E: Send,
    {
        if !value.read().is_pending() {
            value.set(self, AsyncValue::Pending);
        }
        let target = value.clone();
        self.__notifier().spawn(Spawn {
            target: value.id(),
            future: Box::pin(async move { Box::new(future.await) as Box<dyn Any + Send> }),
            apply: Box::new(move |result, cx| {
                let result = *result.downcast::<Result<T, E>>().expect("a ticket's result has its future's type");
                target.set(cx, result.into());
            }),
        });
    }
}
