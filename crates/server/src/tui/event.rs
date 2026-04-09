use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use std::time::Duration;
use tokio::sync::mpsc;

/// Application events.
pub enum AppEvent {
    /// A key was pressed.
    Key(KeyEvent),
    /// Periodic tick for data refresh.
    Tick,
}

/// Polls terminal events and sends them as AppEvents.
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<AppEvent>,
}

impl EventHandler {
    /// Create a new event handler with the given tick rate.
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut tick_interval = tokio::time::interval(tick_rate);
            loop {
                tokio::select! {
                    _ = tick_interval.tick() => {
                        if tx.send(AppEvent::Tick).is_err() {
                            break;
                        }
                    }
                    _ = tokio::task::spawn_blocking({
                        let tx = tx.clone();
                        move || {
                            if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                                if let Ok(Event::Key(key)) = event::read() {
                                    if key.kind == KeyEventKind::Press {
                                        let _ = tx.send(AppEvent::Key(key));
                                    }
                                }
                            }
                        }
                    }) => {}
                }
            }
        });

        Self { rx }
    }

    /// Wait for the next event.
    pub async fn next(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }
}
