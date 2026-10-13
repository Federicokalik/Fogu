//! Session events as an iced `Subscription`.
//!
//! The receiver lives in a shared slot until iced starts the stream, which takes it; later
//! `subscription()` calls hash to the same identity, so iced keeps the running stream.

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};

use fogu_session::SessionOutput;
use iced::Subscription;
use iced::futures::stream::{self, Stream};
use tokio::sync::mpsc::Receiver;

use crate::message::Message;

/// Holds a pane's session receiver until the subscription takes it.
#[derive(Clone, Debug)]
pub struct EventSlot {
    id: u64,
    receiver: Arc<Mutex<Option<Receiver<SessionOutput>>>>,
}

impl EventSlot {
    /// Wraps `receiver`; `id` identifies the pane's subscription.
    pub fn new(id: u64, receiver: Receiver<SessionOutput>) -> Self {
        Self {
            id,
            receiver: Arc::new(Mutex::new(Some(receiver))),
        }
    }
}

impl Hash for EventSlot {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Streams the pane's `SessionOutput`s as [`Message::Session`] until the session ends.
pub fn session_events(slot: &EventSlot) -> Subscription<Message> {
    Subscription::run_with(slot.clone(), stream_of)
}

fn stream_of(slot: &EventSlot) -> impl Stream<Item = Message> + use<> {
    let receiver = slot
        .receiver
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    stream::unfold(receiver, |receiver| async move {
        let mut receiver = receiver?;
        let event = receiver.recv().await?;
        Some((Message::Session(event), Some(receiver)))
    })
}
