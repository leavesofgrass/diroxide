//! The bodies of the fuzz targets, here so normal tests replay their
//! seeds (`fixtures/sftp/seeds/sftp_packet/`) and the fuzz crate under
//! `plugins/diro-sftp/fuzz` stays a thin wrapper.

use super::codec::{DEFAULT_MAX_PACKET, Request, Response, frame};

/// Any bytes as a stream of packets, in both directions: nothing panics,
/// nothing is allocated beyond the bound, and every packet that decodes
/// encodes to bytes that decode to the same packet.
pub fn packet(data: &[u8]) {
    let mut rest = data;
    while let Ok(Some((body, used))) = frame(rest, DEFAULT_MAX_PACKET) {
        if let Ok(request) = Request::decode(body) {
            let again = request.encode();
            let (body2, _) = frame(&again, usize::MAX).ok().flatten().unwrap_or((&[], 0));
            assert_eq!(Request::decode(body2).as_ref(), Ok(&request));
        }
        if let Ok(response) = Response::decode(body) {
            let again = response.encode();
            let (body2, _) = frame(&again, usize::MAX).ok().flatten().unwrap_or((&[], 0));
            assert_eq!(Response::decode(body2).as_ref(), Ok(&response));
        }
        rest = &rest[used..];
    }
}
