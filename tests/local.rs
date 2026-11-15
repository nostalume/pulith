#[path = "local/activation.rs"]
mod activation;
mod common;
#[path = "local/deactivate.rs"]
mod deactivate;
#[path = "local/link.rs"]
mod link;
#[path = "local/materialization.rs"]
mod materialization;
#[path = "local/observation.rs"]
mod observation;
#[path = "local/record.rs"]
mod record;

#[path = "local/switch.rs"]
mod switch;

#[cfg(all(windows, feature = "windows-private-preview"))]
#[path = "local/private_windows.rs"]
mod private_windows;
