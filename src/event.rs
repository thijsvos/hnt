use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, EventStream, KeyEvent, MouseEvent};
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(#[allow(dead_code)] u16, u16),
    Tick,
}

pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
    _tx: mpsc::UnboundedSender<Event>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let _tx = tx.clone();

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
                            Some(Ok(CrosstermEvent::Resize(w, h))) => tx.send(Event::Resize(w, h)),
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

        Self { rx, _tx }
    }

    pub async fn next(&mut self) -> Result<Event> {
        self.rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("Event channel closed"))
    }
}
