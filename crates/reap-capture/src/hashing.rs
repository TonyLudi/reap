pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    reap_capture_framing::sha256_hex(bytes)
}

pub(crate) fn digest_hex(bytes: impl AsRef<[u8]>) -> String {
    reap_capture_framing::digest_hex(bytes)
}
