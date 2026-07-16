#![cfg(feature = "net")]

use pulith::{RemoteUrl, RemoteUrlError};

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
    use pulith::{HttpInspectPolicy, InspectNode, UreqInspect};

    fn assert_inspect<T: InspectNode<RemoteUrl>>(_: &T) {}

    let inspect = UreqInspect::new().with_policy(HttpInspectPolicy::default());
    assert_inspect(&inspect);
}

#[cfg(feature = "reqwest")]
#[test]
fn public_reqwest_inspect_implements_async_inspection_contract() {
    use pulith::{AsyncInspectNode, HttpInspectPolicy, ReqwestInspect};

    fn assert_async_inspect<T: AsyncInspectNode<RemoteUrl>>(_: &T) {}

    let inspect = ReqwestInspect::new().with_policy(HttpInspectPolicy::default());
    assert_async_inspect(&inspect);
}
