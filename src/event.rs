//! Terminal event plumbing.
//!
//! [`EventHandler`] spawns a background tokio task that multiplexes
//! crossterm input events and a fixed-rate tick timer onto a single
//! MPSC channel. The main loop `.await`s [`EventHandler::next`] for a
//! unified [`Event`] stream.

use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, EventStream, KeyEvent, MouseEvent};
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

/// Unified event stream delivered by [`EventHandler`]: terminal input
/// forwarded from crossterm, plus periodic [`Event::Tick`]s for animation
/// (e.g. the loading spinner).
#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize { width: u16, height: u16 },
    Tick,
}

/// Background event pump. Constructed with a tick rate; exposes
/// [`EventHandler::next`] to `.await` the next [`Event`].
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
}

impl EventHandler {
    /// Spawns the background input/tick task and returns a handle.
    ///
    /// The task runs until the channel is dropped or the crossterm stream
    /// errors/ends. Ticks fire every `tick_rate`.
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut reader = EventStream::new();
            let mut tick = tokio::time::interval(tick_rate);

            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        if tx.send(Event::Tick).is_err() {
                            break;
                        }
                    }
                    event = reader.next() => {
                        let send_result = match event {
                            Some(Ok(CrosstermEvent::Key(key))) => tx.send(Event::Key(key)),
                            Some(Ok(CrosstermEvent::Mouse(mouse))) => tx.send(Event::Mouse(mouse)),
                            Some(Ok(CrosstermEvent::Resize(w, h))) => {
                                tx.send(Event::Resize { width: w, height: h })
                            }
                            Some(Err(_)) | None => break,
                            _ => continue,
                        };
                        if send_result.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Self { rx }
    }

    /// Awaits the next [`Event`].
    ///
    /// # Errors
    ///
    /// Returns an error when the MPSC channel closes, which means the
    /// background input/tick task has terminated (crossterm stream
    /// errored or hit EOF).
    pub async fn next(&mut self) -> Result<Event> {
        self.rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("Event channel closed"))
    }
}
