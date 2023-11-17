#![no_main]
#[macro_use]
extern crate libfuzzer_sys;
extern crate rustls;

use rustls::internal::msgs::deframer;
use rustls::internal::msgs::message::Message;
use rustls::internal::record_layer::RecordLayer;
use std::io;

fuzz_target!(|data: &[u8]| {
    let mut buf = <_>::default();
    let mut dfm = deframer::MessageDeframer::default();
    if dfm
        .read(&mut io::Cursor::new(data), &mut buf)
        .is_err()
    {
        return;
    }
    buf.has_pending();

    let mut rl = RecordLayer::new();
    let mut to_discard = 0;
    while let Ok(Some(decrypted)) = dfm.pop(&mut rl, None, &mut buf.borrow(&mut to_discard)) {
        Message::try_from(decrypted.message).ok();
    }
    buf.discard(to_discard);
});
