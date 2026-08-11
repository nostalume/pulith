use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Control(Arc<AtomicBool>);

impl Control {
    pub fn arm() -> Result<Self, String> {
        use signal_hook::consts::{SIGINT, SIGTERM};
        let stopped = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(SIGTERM, stopped.clone()).map_err(|error| error.to_string())?;
        signal_hook::flag::register(SIGINT, stopped.clone()).map_err(|error| error.to_string())?;
        Ok(Self(stopped))
    }

    pub fn ready(&self) -> Result<(), String> {
        sd_notify::notify(&[sd_notify::NotifyState::Ready]).map_err(|error| error.to_string())
    }

    pub fn stop_requested(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
