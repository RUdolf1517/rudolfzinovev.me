//! Cryptographic primitives, password hashing (PBKDF2-HMAC-SHA256), SHA-256,
//! CSPRNG tokens, and gzip compression via system `libcrypto` and `libz`.

use std::ffi::{c_int, c_uchar, c_uint, c_ulong, c_void};

#[link(name = "crypto")]
unsafe extern "C" {
    fn RAND_bytes(buf: *mut c_uchar, num: c_int) -> c_int;
    fn SHA256(d: *const c_uchar, n: usize, md: *mut c_uchar) -> *mut c_uchar;
    fn PKCS5_PBKDF2_HMAC(
        pass: *const c_uchar,
        passlen: c_int,
        salt: *const c_uchar,
        saltlen: c_int,
        iter: c_int,
        digest: *const c_void,
        keylen: c_int,
        out: *mut c_uchar,
    ) -> c_int;
    fn EVP_sha256() -> *const c_void;
    fn CRYPTO_memcmp(a: *const c_void, b: *const c_void, len: usize) -> c_int;
}

#[repr(C)]
struct ZStream {
    next_in: *const c_uchar,
    avail_in: c_uint,
    total_in: c_ulong,
    next_out: *mut c_uchar,
    avail_out: c_uint,
    total_out: c_ulong,
    msg: *const i8,
    state: *mut c_void,
    zalloc: Option<extern "C" fn(*mut c_void, c_uint, c_uint) -> *mut c_void>,
    zfree: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    opaque: *mut c_void,
    data_type: c_int,
    adler: c_ulong,
    reserved: c_ulong,
}

#[link(name = "z")]
unsafe extern "C" {
    fn zlibVersion() -> *const i8;
    fn deflateInit2_(
        strm: *mut ZStream,
        level: c_int,
        method: c_int,
        window_bits: c_int,
        mem_level: c_int,
        strategy: c_int,
        version: *const i8,
        stream_size: c_int,
    ) -> c_int;
    fn deflate(strm: *mut ZStream, flush: c_int) -> c_int;
    fn deflateEnd(strm: *mut ZStream) -> c_int;
}

const PBKDF2_ITERATIONS: i32 = 210_000;

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    let rc = unsafe { RAND_bytes(buf.as_mut_ptr(), N as c_int) };
    assert_eq!(rc, 1, "OpenSSL RAND_bytes failed");
    buf
}

pub fn random_token_hex(byte_len: usize) -> String {
    let mut buf = vec![0u8; byte_len];
    let rc = unsafe { RAND_bytes(buf.as_mut_ptr(), byte_len as c_int) };
    assert_eq!(rc, 1, "OpenSSL RAND_bytes failed");
    hex_encode(&buf)
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut out = [0u8; 32];
    unsafe {
        SHA256(data.as_ptr(), data.len(), out.as_mut_ptr());
    }
    hex_encode(&out)
}

pub fn hash_password(password: &str) -> String {
    let salt = random_bytes::<16>();
    let dk = pbkdf2_sha256(password.as_bytes(), &salt, PBKDF2_ITERATIONS);
    format!(
        "pbkdf2_sha256${}${}${}",
        PBKDF2_ITERATIONS,
        hex_encode(&salt),
        hex_encode(&dk)
    )
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    if parts.len() != 4 || parts[0] != "pbkdf2_sha256" {
        return false;
    }
    let Ok(iter) = parts[1].parse::<i32>() else {
        return false;
    };
    if iter < 10_000 || iter > 5_000_000 {
        return false;
    }
    let Some(salt) = hex_decode(parts[2]) else {
        return false;
    };
    let Some(expected) = hex_decode(parts[3]) else {
        return false;
    };
    if expected.len() != 32 {
        return false;
    }
    let actual = pbkdf2_sha256(password.as_bytes(), &salt, iter);
    constant_time_eq(&actual, &expected)
}

fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: i32) -> [u8; 32] {
    let mut out = [0u8; 32];
    let rc = unsafe {
        PKCS5_PBKDF2_HMAC(
            password.as_ptr(),
            password.len() as c_int,
            salt.as_ptr(),
            salt.len() as c_int,
            iterations as c_int,
            EVP_sha256(),
            32,
            out.as_mut_ptr(),
        )
    };
    assert_eq!(rc, 1, "PKCS5_PBKDF2_HMAC failed");
    out
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = unsafe { CRYPTO_memcmp(a.as_ptr().cast(), b.as_ptr().cast(), a.len()) };
    diff == 0
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let hi = hex_val(bytes[i])?;
        let lo = hex_val(bytes[i + 1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Standard Base64 with padding — used for CSP source hashes.
pub fn base64_encode(input: &[u8]) -> String {
    const TBL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TBL[(n >> 18) as usize & 63] as char);
        out.push(TBL[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TBL[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TBL[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Compress bytes into GZIP format (windowBits = 15 + 16 = 31) using system zlib.
pub fn gzip_compress(input: &[u8]) -> Option<Vec<u8>> {
    const Z_DEFLATED: c_int = 8;
    const Z_FINISH: c_int = 4;
    const Z_STREAM_END: c_int = 1;
    const Z_OK: c_int = 0;

    let mut out = vec![0u8; input.len() + 64 + (input.len() / 100)];
    let mut strm = ZStream {
        next_in: input.as_ptr(),
        avail_in: input.len() as c_uint,
        total_in: 0,
        next_out: out.as_mut_ptr(),
        avail_out: out.len() as c_uint,
        total_out: 0,
        msg: std::ptr::null(),
        state: std::ptr::null_mut(),
        zalloc: None,
        zfree: None,
        opaque: std::ptr::null_mut(),
        data_type: 0,
        adler: 0,
        reserved: 0,
    };

    unsafe {
        let rc = deflateInit2_(
            &mut strm,
            6, // balanced fast compression
            Z_DEFLATED,
            15 + 16, // gzip header + trailer
            8,
            0,
            zlibVersion(),
            std::mem::size_of::<ZStream>() as c_int,
        );
        if rc != Z_OK {
            return None;
        }
        let drc = deflate(&mut strm, Z_FINISH);
        let total = strm.total_out as usize;
        deflateEnd(&mut strm);
        if drc != Z_STREAM_END {
            return None;
        }
        out.truncate(total);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_and_verify() {
        let hash = hash_password("correct-horse-battery-staple");
        assert!(verify_password("correct-horse-battery-staple", &hash));
        assert!(!verify_password("wrong-password", &hash));
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn gzip_produces_valid_header() {
        let raw = "hello ".repeat(200);
        let gz = gzip_compress(raw.as_bytes()).expect("gzip succeeded");
        assert_eq!(gz[0], 0x1f);
        assert_eq!(gz[1], 0x8b);
        assert!(gz.len() < raw.len());
    }
}
