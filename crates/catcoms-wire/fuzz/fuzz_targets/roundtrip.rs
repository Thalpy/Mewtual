#![no_main]
//! Decoding arbitrary, attacker-controlled bytes must never panic; it may only
//! return a value or a `WireError`.

use catcoms_wire::Decoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut dec = Decoder::new(data);
    // Exercise each reader; all failures must be graceful errors, never panics.
    let _ = dec.get_u8();
    let _ = dec.get_u16();
    let _ = dec.get_u32();
    let _ = dec.get_u64();
    let _ = dec.get_u128();
    let _ = dec.get_bytes();
    let _ = dec.get_str();
});
