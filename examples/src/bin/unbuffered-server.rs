use std::error::Error;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::UnbufferedServerConnection;
use rustls::unbuffered::{
    AppDataRecord, ConnectionState, EncodeError, EncryptError, InsufficientSizeError,
    UnbufferedStatus,
};
use rustls::ServerConfig;
use rustls_pemfile::Item;

const KB: usize = 1024;
const INCOMING_TLS_BUFSIZ: usize = 16 * KB;
const OUTGOING_TLS_INITIAL_BUFSIZ: usize = 0;
const MAX_EARLY_DATA_SIZE: Option<u32> = Some(128);
const MAX_FRAGMENT_SIZE: Option<usize> = None;

const PORT: u16 = 1443;
const MAX_ITERATIONS: usize = 30;
const CERTFILE: &str = "localhost.pem";
const PRIV_KEY_FILE: &str = "localhost-key.pem";

fn main() -> Result<(), Box<dyn Error>> {
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(load_certs()?, load_private_key()?)?;

    if let Some(max_early_data_size) = MAX_EARLY_DATA_SIZE {
        config.max_early_data_size = max_early_data_size;
    }

    config.max_fragment_size = MAX_FRAGMENT_SIZE;

    let config = Arc::new(config);

    let listener = TcpListener::bind(format!("[::]:{PORT}"))?;

    let mut incoming_tls = [0; INCOMING_TLS_BUFSIZ];
    let mut outgoing_tls = vec![0; OUTGOING_TLS_INITIAL_BUFSIZ];
    for stream in listener.incoming() {
        handle(stream?, &config, &mut incoming_tls, &mut outgoing_tls)?;
    }

    Ok(())
}

fn handle(
    mut sock: TcpStream,
    config: &Arc<ServerConfig>,
    incoming_tls: &mut [u8],
    outgoing_tls: &mut Vec<u8>,
) -> Result<(), Box<dyn Error>> {
    eprintln!("\n---- new client ----");

    dbg!(sock.peer_addr()?);

    let mut conn = UnbufferedServerConnection::new(config.clone())?;

    let mut incoming_used = 0;
    let mut outgoing_used = 0;

    let mut open_connection = true;
    let mut received_request = false;
    let mut sent_response = false;

    let mut iter_count = 0;
    while open_connection {
        let UnbufferedStatus { mut discard, state } =
            conn.process_tls_records(&mut incoming_tls[..incoming_used])?;

        match dbg!(state) {
            ConnectionState::AppDataAvailable(mut state) => {
                while let Some(res) = state.next_record() {
                    let AppDataRecord {
                        discard: new_discard,
                        payload,
                    } = res?;
                    discard += new_discard;

                    if payload.starts_with(b"GET") {
                        let response = core::str::from_utf8(payload)?;
                        let header = response
                            .lines()
                            .next()
                            .unwrap_or(response);

                        println!("{header}");
                    } else {
                        println!("(.. continued HTTP request ..)");
                    }

                    received_request = true;
                }
            }

            ConnectionState::EarlyDataAvailable(mut state) => {
                while let Some(res) = state.next_record() {
                    let AppDataRecord {
                        discard: new_discard,
                        payload,
                    } = res?;
                    discard += new_discard;

                    println!("early data: {:?}", core::str::from_utf8(payload));

                    received_request = true;
                }
            }

            ConnectionState::MustEncodeTlsData(mut state) => {
                try_or_resize_and_retry(
                    |out_buffer| state.encode(out_buffer),
                    |e| {
                        if let EncodeError::InsufficientSize(is) = &e {
                            Ok(*is)
                        } else {
                            Err(e.into())
                        }
                    },
                    outgoing_tls,
                    &mut outgoing_used,
                )?;
            }

            ConnectionState::MustTransmitTlsData(state) => {
                send_tls(&mut sock, outgoing_tls, &mut outgoing_used)?;
                state.done();
            }

            ConnectionState::NeedsMoreTlsData { .. } => {
                recv_tls(&mut sock, incoming_tls, &mut incoming_used)?;
            }

            ConnectionState::TrafficTransit(mut state) => {
                if !received_request {
                    recv_tls(&mut sock, incoming_tls, &mut incoming_used)?;
                } else {
                    let map_err = |e| {
                        if let EncryptError::InsufficientSize(is) = &e {
                            Ok(*is)
                        } else {
                            Err(e.into())
                        }
                    };

                    let http_response = build_http_response();
                    try_or_resize_and_retry(
                        |out_buffer| state.encrypt(http_response, out_buffer),
                        map_err,
                        outgoing_tls,
                        &mut outgoing_used,
                    )?;
                    sent_response = true;

                    try_or_resize_and_retry(
                        |out_buffer| state.queue_close_notify(out_buffer),
                        map_err,
                        outgoing_tls,
                        &mut outgoing_used,
                    )?;
                    open_connection = false;

                    send_tls(&mut sock, outgoing_tls, &mut outgoing_used)?;
                }
            }

            _ => unreachable!(),
        }

        if discard != 0 {
            assert!(discard <= incoming_used);

            incoming_tls.copy_within(discard..incoming_used, 0);
            incoming_used -= discard;

            eprintln!("discarded {discard}B from `incoming_tls`");
        }

        iter_count += 1;
        assert!(
            iter_count < MAX_ITERATIONS,
            "did not get a HTTP response within {MAX_ITERATIONS} iterations"
        );
    }

    assert!(received_request);
    assert!(sent_response);
    assert_eq!(0, incoming_used);
    assert_eq!(0, outgoing_used);

    Ok(())
}

fn try_or_resize_and_retry<E>(
    mut f: impl FnMut(&mut [u8]) -> Result<usize, E>,
    map_err: impl FnOnce(E) -> Result<InsufficientSizeError, Box<dyn Error>>,
    outgoing_tls: &mut Vec<u8>,
    outgoing_used: &mut usize,
) -> Result<usize, Box<dyn Error>>
where
    E: Error + 'static,
{
    let written = match f(&mut outgoing_tls[*outgoing_used..]) {
        Ok(written) => written,

        Err(e) => {
            let InsufficientSizeError { required_size } = map_err(e)?;
            let new_len = *outgoing_used + required_size;
            outgoing_tls.resize(new_len, 0);
            eprintln!("resized `outgoing_tls` buffer to {new_len}B");

            f(&mut outgoing_tls[*outgoing_used..])?
        }
    };

    *outgoing_used += written;

    Ok(written)
}
fn recv_tls(
    sock: &mut TcpStream,
    incoming_tls: &mut [u8],
    incoming_used: &mut usize,
) -> Result<(), Box<dyn Error>> {
    let read = sock.read(&mut incoming_tls[*incoming_used..])?;
    eprintln!("received {read}B of data");
    *incoming_used += read;
    Ok(())
}

fn send_tls(
    sock: &mut TcpStream,
    outgoing_tls: &[u8],
    outgoing_used: &mut usize,
) -> Result<(), Box<dyn Error>> {
    sock.write_all(&outgoing_tls[..*outgoing_used])?;
    eprintln!("sent {outgoing_used}B of data");
    *outgoing_used = 0;
    Ok(())
}

fn build_http_response() -> &'static [u8] {
    b"HTTP/1.0 200 OK\r\nConnection: close\r\n\r\nHello world from rustls unbuffered server\r\n"
}

fn load_certs() -> Result<Vec<CertificateDer<'static>>, io::Error> {
    let mut reader = BufReader::new(File::open(CERTFILE)?);
    rustls_pemfile::certs(&mut reader).collect()
}

fn load_private_key() -> Result<PrivateKeyDer<'static>, io::Error> {
    let mut reader = BufReader::new(File::open(PRIV_KEY_FILE)?);

    loop {
        match rustls_pemfile::read_one(&mut reader)? {
            Some(Item::Pkcs1Key(key)) => return Ok(key.into()),
            Some(Item::Pkcs8Key(key)) => return Ok(key.into()),
            Some(Item::Sec1Key(key)) => return Ok(key.into()),
            None => break,
            _ => continue,
        }
    }

    panic!("no keys found in {PRIV_KEY_FILE}")
}
