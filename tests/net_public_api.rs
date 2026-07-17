#![cfg(feature = "net")]

use pulith::net::{RemoteUrl, RemoteUrlError};

#[test]
fn public_remote_url_uses_resource_specific_errors() {
    assert!(matches!(
        RemoteUrl::parse("file:///tmp/pulith"),
        Err(RemoteUrlError::UnsupportedScheme { .. })
    ));
    assert!(matches!(
        RemoteUrl::parse("not a URL"),
        Err(RemoteUrlError::Invalid { .. })
    ));
}

#[cfg(feature = "ureq")]
#[test]
fn public_ureq_inspect_implements_sync_inspection_contract() {
    use pulith::Inspect;
    use pulith::net::UreqInspect;

    fn assert_inspect<T: Inspect<RemoteUrl>>(_: &T) {}

    assert_inspect(&UreqInspect::default());
}

#[cfg(feature = "reqwest")]
#[test]
fn public_reqwest_inspect_implements_async_inspection_contract() {
    use pulith::AsyncInspect;
    use pulith::net::ReqwestInspect;

    fn assert_async_inspect<T: AsyncInspect<RemoteUrl>>(_: &T) {}

    assert_async_inspect(&ReqwestInspect::default());
}

#[cfg(all(feature = "ureq", feature = "local"))]
#[test]
fn public_ureq_acquire_implements_destination_free_contract() {
    use pulith::local::LocalTarget;
    use pulith::net::{RemoteSource, UreqAcquire};
    use pulith::{Acquire, Materialize};

    fn assert_acquire<T: Acquire<Materialize<(), RemoteSource, LocalTarget>>>(_: &T) {}

    assert_acquire(&UreqAcquire::default());
}

#[cfg(all(feature = "reqwest", feature = "local"))]
#[test]
fn public_reqwest_acquire_implements_destination_free_contract() {
    use pulith::local::LocalTarget;
    use pulith::net::{RemoteSource, ReqwestAcquire};
    use pulith::{AsyncAcquire, Materialize};

    fn assert_async_acquire<T: AsyncAcquire<Materialize<(), RemoteSource, LocalTarget>>>(_: &T) {}

    assert_async_acquire(&ReqwestAcquire::default());
}
