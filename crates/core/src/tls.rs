//! The TLS server variables of a connection, named once for every PHP
//! execution path: embedded scripts, embedded applications and FastCGI.
//!
//! The names are mod_ssl's. A connection without TLS has none of them. With
//! TLS, `SSL_CLIENT_VERIFY` is always present and says `NONE` when no client
//! certificate was presented; the certificate's subject and issuer appear
//! only when the handshake verified one and its names could be parsed.

use std::ffi::CStr;

use claviron_http::ext::TlsInfo;

/// A TLS server variable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TlsVariable {
    /// `SSL_PROTOCOL`
    Protocol,
    /// `SSL_CIPHER`
    Cipher,
    /// `SSL_CLIENT_VERIFY`
    ClientVerify,
    /// `SSL_CLIENT_S_DN`
    ClientSubjectDn,
    /// `SSL_CLIENT_I_DN`
    ClientIssuerDn,
}

impl TlsVariable {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Protocol => "SSL_PROTOCOL",
            Self::Cipher => "SSL_CIPHER",
            Self::ClientVerify => "SSL_CLIENT_VERIFY",
            Self::ClientSubjectDn => "SSL_CLIENT_S_DN",
            Self::ClientIssuerDn => "SSL_CLIENT_I_DN",
        }
    }

    /// The same name, terminated for PHP's variable registration.
    pub const fn c_name(self) -> &'static CStr {
        match self {
            Self::Protocol => c"SSL_PROTOCOL",
            Self::Cipher => c"SSL_CIPHER",
            Self::ClientVerify => c"SSL_CLIENT_VERIFY",
            Self::ClientSubjectDn => c"SSL_CLIENT_S_DN",
            Self::ClientIssuerDn => c"SSL_CLIENT_I_DN",
        }
    }
}

/// A variable's text, borrowed either way: a name the transport gives as
/// static text, or text the connection's certificate owns. A consumer that
/// has to own the value copies only the second kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TlsValue<'a> {
    Static(&'static str),
    Connection(&'a str),
}

impl<'a> TlsValue<'a> {
    pub const fn as_str(self) -> &'a str {
        match self {
            Self::Static(text) => text,
            Self::Connection(text) => text,
        }
    }
}

/// The TLS variables of a connection, in a fixed order. Every value is
/// borrowed from what the connection already holds; nothing is formatted or
/// allocated here.
pub fn tls_variables(
    tls: Option<&TlsInfo>
) -> impl Iterator<Item = (TlsVariable, TlsValue<'_>)> {
    tls.into_iter().flat_map(|tls| {
        let cert = tls.client_cert.as_deref();
        [
            tls.ssl_protocol()
                .map(|text| (TlsVariable::Protocol, TlsValue::Static(text))),
            tls.ssl_cipher()
                .map(|text| (TlsVariable::Cipher, TlsValue::Static(text))),
            Some((
                TlsVariable::ClientVerify,
                TlsValue::Static(tls.ssl_client_verify()),
            )),
            cert.map(|cert| {
                (
                    TlsVariable::ClientSubjectDn,
                    TlsValue::Connection(&cert.subject_dn),
                )
            }),
            cert.map(|cert| {
                (
                    TlsVariable::ClientIssuerDn,
                    TlsValue::Connection(&cert.issuer_dn),
                )
            }),
        ]
        .into_iter()
        .flatten()
    })
}

#[cfg(test)]
#[path = "../tests/tls.rs"]
mod tests;
