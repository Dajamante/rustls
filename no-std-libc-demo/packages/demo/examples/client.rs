#![no_main]
#![no_std]

extern crate alloc;

use alloc::{
    borrow::Cow,
    format,
    string::{String, ToString},
    sync::Arc,
    vec,
};
use core::str;
use ministd::{
    dbg, entry, eprintln,
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    println,
};
use rustls::{
    client::{low_level::LlClientConnection, InvalidDnsNameError},
    low_level::{AppDataRecord, EncodeError, InsufficientSizeError, State, Status},
    server::danger::DnsName,
    ClientConfig, RootCertStore,
};
use rustls_pemfile::Item;

const SERVER_NAME: &str = "localhost";
const PORT: u16 = 1443;

entry!(main);

fn main() -> Result<(), Error> {
    let mut root_store = RootCertStore::empty();
    root_store.extend(
        webpki_roots::TLS_SERVER_ROOTS
            .iter()
            .cloned(),
    );

    let mut certfile: &[_] = include_bytes!("/home/japaric/.local/share/mkcert/rootCA.pem");
    let mut certs = vec![];
    while let Ok(Some((item, rest))) = rustls_pemfile::read_one_from_slice(certfile) {
        certfile = rest;
        if let Item::X509Certificate(cert) = item {
            certs.push(cert);
        }
    }
    dbg!(certs.len());
    root_store.add_parsable_certificates(certs);

    let mut config = ClientConfig::builder_with_provider(demo::CRYPTO_PROVIDER)
        .with_safe_defaults()
        .dangerous()
        .with_custom_certificate_verifier(demo::certificate_verifier(root_store))
        .with_no_client_auth();

    config.time_provider = demo::time_provider();

    let sock_addr = (SERVER_NAME, PORT)
        .to_socket_addrs()?
        .next()
        .ok_or(io::Error::AddressLookup)?;
    dbg!(sock_addr);

    let mut sock = TcpStream::connect(&sock_addr)?;
    let mut conn = LlClientConnection::new(
        Arc::new(config),
        rustls::ServerName::DnsName(DnsName::try_from(SERVER_NAME.to_string())?),
    )?;
    let request = http_request(SERVER_NAME);

    let mut incoming_tls = [0; 16 * 1024];
    let mut incoming_used = 0;

    let mut outgoing_tls = vec![];
    let mut outgoing_used = 0;

    let mut open_connection = true;

    while open_connection {
        let Status { discard, state } =
            conn.process_tls_records(&mut incoming_tls[..incoming_used])?;
        match state {
            // logic similar to the one presented in the 'handling InsufficientSizeError' section is
            // used in these states
            State::MustEncodeTlsData(mut state) => {
                let written = match state.encode(&mut outgoing_tls[outgoing_used..]) {
                    Ok(written) => written,
                    Err(EncodeError::InsufficientSize(InsufficientSizeError { required_size })) => {
                        let new_len = outgoing_used + required_size;
                        outgoing_tls.resize(new_len, 0);
                        eprintln!("resized `outgoing_tls` buffer to {}B", new_len)?;

                        // don't forget to encrypt the handshake record after resizing!
                        state
                            .encode(&mut outgoing_tls[outgoing_used..])
                            .expect("should not fail this time")
                    }
                    Err(err) => return Err(err.into()),
                };
                outgoing_used += written;
            }
            State::MustTransmitTlsData(state) => {
                sock.write_all(&outgoing_tls[..outgoing_used])?;

                outgoing_used = 0;

                state.done();
            }

            State::NeedsMoreTlsData { .. } => {
                // NOTE real code needs to handle the scenario where `incoming_tls` is not big enough
                let read = sock.read(&mut incoming_tls[incoming_used..])?;
                incoming_used += read;
            }

            State::AppDataAvailable(mut records) => {
                while let Some(result) = records.next_record() {
                    let AppDataRecord { payload, .. } = result?;

                    println!("response:\n{:?}", str::from_utf8(payload))?;
                }
            }

            State::TrafficTransit(mut traffic_transit) => {
                // post-handshake logic
                let request = request.as_bytes();
                let len = traffic_transit.encrypt(request, outgoing_tls.as_mut_slice())?;
                sock.write_all(&outgoing_tls[..len])?;

                let read = sock.read(&mut incoming_tls[incoming_used..])?;
                incoming_used += read;
            }

            State::ConnectionClosed => open_connection = false,
        }

        // discard TLS records
        if discard != 0 {
            assert!(discard <= incoming_used);

            incoming_tls.copy_within(discard..incoming_used, 0);
            incoming_used -= discard;
        }
    }

    Ok(())
}

#[derive(Debug)]
enum Error {
    Minstd(io::Error),
    Rustls(rustls::Error),
    InvalidDnsName(InvalidDnsNameError),
    EncodeError(EncodeError),
}

impl From<EncodeError> for Error {
    fn from(v: EncodeError) -> Self {
        Self::EncodeError(v)
    }
}

impl From<InvalidDnsNameError> for Error {
    fn from(v: InvalidDnsNameError) -> Self {
        Self::InvalidDnsName(v)
    }
}

impl From<rustls::Error> for Error {
    fn from(v: rustls::Error) -> Self {
        Self::Rustls(v)
    }
}

impl From<io::Error> for Error {
    fn from(v: io::Error) -> Self {
        Self::Minstd(v)
    }
}

fn http_request(server_name: &str) -> String {
    const HTTP_SEPARATOR: &str = "\r\n";

    let lines = [
        Cow::Borrowed("GET / HTTP/1.1"),
        format!("Host: {server_name}").into(),
        "Connection: close".into(),
        "Accept-Encoding: identity".into(),
        "".into(), // body
    ];

    let mut req = String::new();
    for line in lines {
        req.push_str(&line);
        req.push_str(HTTP_SEPARATOR);
    }

    req
}
