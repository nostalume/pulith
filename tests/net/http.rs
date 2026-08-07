#![cfg(feature = "net")]

use std::path::PathBuf;

use pulith::net::{RemoteUrl, RemoteUrlError};

#[test]
fn remote_url_uses_resource_specific_errors() {
    assert!(matches!(
        RemoteUrl::parse("file:///tmp/pulith"),
        Err(RemoteUrlError::UnsupportedScheme { .. })
    ));
    assert!(matches!(
        RemoteUrl::parse("not a URL"),
        Err(RemoteUrlError::Invalid { .. })
    ));
}

#[cfg(feature = "http-sync")]
#[test]
fn sync_http_inspect_implements_inspection_contract() {
    use pulith::Inspect;
    use pulith::net::SyncHttpInspect;

    fn assert_inspect<T: Inspect<RemoteUrl>>(_: &T) {}

    assert_inspect(&SyncHttpInspect::default());
}

#[cfg(feature = "http-async")]
#[test]
fn async_http_inspect_implements_inspection_contract() {
    use pulith::AsyncInspect;
    use pulith::net::AsyncHttpInspect;

    fn assert_async_inspect<T: AsyncInspect<RemoteUrl>>(_: &T) {}

    assert_async_inspect(&AsyncHttpInspect::default());
}

#[cfg(all(feature = "http-sync", feature = "local"))]
#[test]
fn sync_http_acquire_implements_destination_free_contract() {
    use pulith::net::{RemoteSource, SyncHttpAcquire};
    use pulith::{Acquire, Materialize};

    fn assert_acquire<T: Acquire<Materialize<(), RemoteSource, PathBuf>>>(_: &T) {}

    assert_acquire(&SyncHttpAcquire::default());
}

#[cfg(all(feature = "http-async", feature = "local"))]
#[test]
fn async_http_acquire_implements_destination_free_contract() {
    use pulith::net::{AsyncHttpAcquire, RemoteSource};
    use pulith::{AsyncAcquire, Materialize};

    fn assert_async_acquire<T: AsyncAcquire<Materialize<(), RemoteSource, PathBuf>>>(_: &T) {}

    assert_async_acquire(&AsyncHttpAcquire::default());
}
