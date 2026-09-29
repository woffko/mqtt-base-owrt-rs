// SPDX-License-Identifier: GPL-2.0-or-later
//! Byte stream under the MQTT client: plain TCP, or TLS (rustls with the
//! `ring` provider) when the `tls` feature is compiled in.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

/// System CA bundle (OpenWrt `ca-bundle` package).
pub const DEFAULT_CA_BUNDLE: &str = "/etc/ssl/certs/ca-certificates.crt";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsSettings {
    /// PEM file with trusted CA certificates; `None` uses the system bundle.
    pub ca_file: Option<String>,
    /// `false` accepts any server certificate (self-signed brokers without a
    /// CA file). The connection is still encrypted but not authenticated.
    pub verify: bool,
    pub cert_file: Option<String>,
    pub key_file: Option<String>,
}

pub enum Transport {
    Plain(TcpStream),
    #[cfg(feature = "tls")]
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Transport {
    pub fn establish(
        tcp: TcpStream,
        host: &str,
        tls: Option<&TlsSettings>,
    ) -> io::Result<Transport> {
        match tls {
            None => Ok(Transport::Plain(tcp)),
            #[cfg(feature = "tls")]
            Some(settings) => tls::establish(tcp, host, settings),
            #[cfg(not(feature = "tls"))]
            Some(_) => {
                let _ = host;
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "this build has no TLS support",
                ))
            }
        }
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Transport::Plain(s) => s.set_read_timeout(timeout),
            #[cfg(feature = "tls")]
            Transport::Tls(s) => s.sock.set_read_timeout(timeout),
        }
    }

    pub fn shutdown(&mut self, how: Shutdown) {
        match self {
            Transport::Plain(s) => {
                let _ = s.shutdown(how);
            }
            #[cfg(feature = "tls")]
            Transport::Tls(s) => {
                s.conn.send_close_notify();
                let _ = s.flush();
                let _ = s.sock.shutdown(how);
            }
        }
    }
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Transport::Plain(s) => s.read(buf),
            #[cfg(feature = "tls")]
            Transport::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Transport::Plain(s) => s.write(buf),
            #[cfg(feature = "tls")]
            Transport::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Transport::Plain(s) => s.flush(),
            #[cfg(feature = "tls")]
            Transport::Tls(s) => s.flush(),
        }
    }
}

#[cfg(feature = "tls")]
mod tls {
    use std::io;
    use std::net::TcpStream;
    use std::sync::Arc;

    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
    use rustls::pki_types::pem::{Error as PemError, PemObject};
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
    use rustls::{
        ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, SignatureScheme,
        StreamOwned,
    };

    use super::{TlsSettings, Transport, DEFAULT_CA_BUNDLE};

    fn invalid(what: String) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidInput, what)
    }

    /// The PEM parser quotes the line it rejects; in a key file that line can
    /// be the key. Errors end up in the log, the status file and LuCI, so
    /// only the kind of failure is reported.
    fn pem_error(path: &str, what: &str, e: &PemError) -> io::Error {
        let reason = match e {
            PemError::Io(e) => e.to_string(),
            PemError::NoItemsFound => format!("no {what} found"),
            _ => format!("not a valid PEM {what}"),
        };
        invalid(format!("{path}: {reason}"))
    }

    /// Accepts any server certificate but still checks handshake signatures.
    #[derive(Debug)]
    struct AnyCertificate(Arc<CryptoProvider>);

    impl ServerCertVerifier for AnyCertificate {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls12_signature(
                message,
                cert,
                dss,
                &self.0.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls13_signature(
                message,
                cert,
                dss,
                &self.0.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    fn certificates(path: &str) -> io::Result<Vec<CertificateDer<'static>>> {
        let certs: Result<Vec<_>, _> = CertificateDer::pem_file_iter(path)
            .map_err(|e| pem_error(path, "certificate", &e))?
            .collect();
        let certs = certs.map_err(|e| pem_error(path, "certificate", &e))?;
        if certs.is_empty() {
            return Err(invalid(format!("{path}: no certificate found")));
        }
        Ok(certs)
    }

    pub fn config(s: &TlsSettings) -> io::Result<Arc<ClientConfig>> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .map_err(|e| invalid(e.to_string()))?;
        let builder = if s.verify {
            let path = s.ca_file.as_deref().unwrap_or(DEFAULT_CA_BUNDLE);
            let mut roots = RootCertStore::empty();
            let (added, _ignored) = roots.add_parsable_certificates(certificates(path)?);
            if added == 0 {
                return Err(invalid(format!("{path}: no usable CA certificate")));
            }
            builder.with_root_certificates(roots)
        } else {
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
        };
        let config = match (&s.cert_file, &s.key_file) {
            (Some(cert), Some(key)) => {
                let key = PrivateKeyDer::from_pem_file(key)
                    .map_err(|e| pem_error(key, "private key", &e))?;
                builder
                    .with_client_auth_cert(certificates(cert)?, key)
                    .map_err(|e| invalid(e.to_string()))?
            }
            _ => builder.with_no_client_auth(),
        };
        Ok(Arc::new(config))
    }

    pub fn establish(tcp: TcpStream, host: &str, s: &TlsSettings) -> io::Result<Transport> {
        let name =
            ServerName::try_from(host.to_string()).map_err(|e| invalid(format!("{host}: {e}")))?;
        let conn =
            ClientConnection::new(config(s)?, name).map_err(|e| io::Error::other(e.to_string()))?;
        let mut stream = StreamOwned::new(conn, tcp);
        // Complete the handshake now so certificate errors surface as
        // connection errors, not as a failed CONNECT.
        while stream.conn.is_handshaking() {
            stream.conn.complete_io(&mut stream.sock)?;
        }
        Ok(Transport::Tls(Box::new(stream)))
    }
}
