//! Fuzzes the reading of a form's answer as a host could send it: the
//! passphrase form's two hidden fields compared, the recipient form's
//! key, file and armor. The body is `diro_crypt::fuzzing::form_answer`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    diro_crypt::fuzzing::form_answer(data);
});
