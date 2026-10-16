//! Session and SFTP events as iced `Subscription`s.
//!
//! The receiver lives in a shared slot until iced starts the stream, which takes it; later
//! `subscription()` calls hash to the same identity, so iced keeps the running stream.

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};

use fogu_session::{SessionOutput, SftpEvent};
use iced::Subscription;
use iced::futures::stream::{self, Stream};
use tokio::sync::mpsc::Receiver;

use crate::message::Message;
use crate::sftp::SftpMsg;
use crate::tab::PaneId;

/// Holds a pane's receiver until the subscription takes it.
#[derive(Debug)]
pub struct EventSlot<T> {
    id: u64,
    receiver: Arc<Mutex<Option<Receiver<T>>>>,
}

// Manual impl: a derive would require `T: Clone`.
impl<T> Clone for EventSlot<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            receiver: Arc::clone(&self.receiver),
        }
    }
}

impl<T> EventSlot<T> {
    /// Wraps `receiver`; `id` identifies the pane's subscription.
    pub fn new(id: u64, receiver: Receiver<T>) -> Self {
        Self {
            id,
            receiver: Arc::new(Mutex::new(Some(receiver))),
        }
    }

    fn take(&self) -> Option<Receiver<T>> {
        self.receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

impl<T> Hash for EventSlot<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Streams the pane's `SessionOutput`s as [`Message::Session`] (tagged with the pane id) until
/// the session ends.
pub fn session_events(slot: &EventSlot<SessionOutput>) -> Subscription<Message> {
    Subscription::run_with(slot.clone(), |slot| stream_of(slot, Message::Session))
}

/// Streams an SSH pane's SFTP engine events as [`SftpMsg::Event`]. Always subscribed, panel
/// open or not: the engine blocks on a full channel, so the UI must keep draining it.
pub fn sftp_events(slot: &EventSlot<SftpEvent>) -> Subscription<Message> {
    Subscription::run_with(slot.clone(), |slot| {
        stream_of(slot, |pane, event| {
            Message::Sftp(SftpMsg::Event(pane, Box::new(event)))
        })
    })
}

fn stream_of<T: Send + 'static>(
    slot: &EventSlot<T>,
    wrap: fn(PaneId, T) -> Message,
) -> impl Stream<Item = Message> + use<T> {
    let receiver = slot.take();
    let pane = PaneId(slot.id);
    stream::unfold(receiver, move |receiver| async move {
        let mut receiver = receiver?;
        let event = receiver.recv().await?;
        Some((wrap(pane, event), Some(receiver)))
    })
}
