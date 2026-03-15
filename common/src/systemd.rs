/// Signal to systemd that the service is fully initialized and ready to serve
/// traffic.  No-ops gracefully when not running under systemd.
pub fn notify_ready() {
  let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);
}

/// Spawn a background task that sends the systemd watchdog keepalive at half
/// the configured interval.  No-ops gracefully when the watchdog is not
/// configured (i.e. `WatchdogSec` is absent from the unit).
pub fn spawn_watchdog() {
  let mut usec: u64 = 0;
  if sd_notify::watchdog_enabled(false, &mut usec) {
    let interval = std::time::Duration::from_micros(usec);
    tokio::spawn(async move {
      let mut ticker = tokio::time::interval(interval / 2);
      loop {
        ticker.tick().await;
        let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]);
      }
    });
  }
}
