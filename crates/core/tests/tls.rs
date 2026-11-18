use std::sync::Arc;

use claviron_http::ext::{ClientAuthStatus, TlsInfo};
use claviron_types::PeerIdentity;

use super::*;

fn names(tls: Option<&TlsInfo>) -> Vec<(&'static str, &str)> {
    tls_variables(tls)
        .map(|(variable, value)| (variable.name(), value.as_str()))
        .collect()
}

#[test]
fn client_certificate_variables_follow_the_connection() {
    assert!(names(None).is_empty(), "plain connections carry no TLS variables");

    let mut tls = TlsInfo {
        protocol_version: None,
        cipher_suite: None,
        client_auth: ClientAuthStatus::NotProvided,
        client_cert: None,
    };
    assert_eq!(names(Some(&tls)), [("SSL_CLIENT_VERIFY", "NONE")]);

    tls.client_auth = ClientAuthStatus::Verified;
    tls.client_cert = Some(Arc::new(PeerIdentity {
        certificate: Arc::from([0u8; 0]),
        subject_dn: "CN=client.example.test".into(),
        issuer_dn: "CN=Example Test CA".into(),
        common_names: Box::new([]),
        subject_uris: Box::new([]),
    }));
    assert_eq!(
        names(Some(&tls)),
        [
            ("SSL_CLIENT_VERIFY", "SUCCESS"),
            ("SSL_CLIENT_S_DN", "CN=client.example.test"),
            ("SSL_CLIENT_I_DN", "CN=Example Test CA"),
        ]
    );
    // The certificate's text is lent by the connection, never copied.
    let (_, subject) = tls_variables(Some(&tls))
        .find(|(variable, _)| *variable == TlsVariable::ClientSubjectDn)
        .unwrap();
    let cert = tls.client_cert.as_deref().unwrap();
    assert!(std::ptr::eq(subject.as_str(), &*cert.subject_dn));
}
