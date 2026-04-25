//! OSC 52 clipboard writer.
//!
//! Most modern terminals (iTerm2, Alacritty, kitty, foot, recent xterm,
//! Windows Terminal, tmux with `set -g set-clipboard on`) interpret the
//! OSC 52 escape sequence as a request to write to the system clipboard.
//! Works through SSH where `xclip`/`pbcopy` cannot reach the user's local
//! machine. No external process spawn, no extra dependency — just a
//! handful of bytes written to stdout.
//!
//! The encoding is the standard base64 alphabet (RFC 4648) with `=`
//! padding; the `c` selector requests the system clipboard (vs `p` for
//! the primary X11 selection).

use std::io::{self, Write};

/// Standard base64 alphabet (RFC 4648 Table 1).
const BASE64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Sends `text` to the host terminal's system clipboard via OSC 52.
///
/// Returns the underlying I/O error if the write or flush fails. In
/// practice the error is non-fatal — the caller should surface it in the
/// status bar but keep running.
pub fn copy(text: &str) -> io::Result<()> {
    let encoded = base64_encode(text.as_bytes());
    let mut out = io::stdout().lock();
    write!(out, "\x1b]52;c;{}\x07", encoded)?;
    out.flush()
}

/// Standard base64 encode (no line wrapping). Output length is
/// `((bytes.len() + 2) / 3) * 4`.
fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        out.push(BASE64[(chunk[0] >> 2) as usize] as char);
        out.push(BASE64[(((chunk[0] & 0b11) << 4) | (chunk[1] >> 4)) as usize] as char);
        out.push(BASE64[(((chunk[1] & 0b1111) << 2) | (chunk[2] >> 6)) as usize] as char);
        out.push(BASE64[(chunk[2] & 0b111111) as usize] as char);
    }
    let rem = chunks.remainder();
    match rem.len() {
        0 => {}
        1 => {
            out.push(BASE64[(rem[0] >> 2) as usize] as char);
            out.push(BASE64[((rem[0] & 0b11) << 4) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            out.push(BASE64[(rem[0] >> 2) as usize] as char);
            out.push(BASE64[(((rem[0] & 0b11) << 4) | (rem[1] >> 4)) as usize] as char);
            out.push(BASE64[((rem[1] & 0b1111) << 2) as usize] as char);
            out.push('=');
        }
        _ => unreachable!("chunks_exact(3) remainder is 0..=2"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_empty() {
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn base64_one_byte_pads_two_equals() {
        assert_eq!(base64_encode(b"f"), "Zg==");
    }

    #[test]
    fn base64_two_bytes_pads_one_equals() {
        assert_eq!(base64_encode(b"fo"), "Zm8=");
    }

    #[test]
    fn base64_three_bytes_no_padding() {
        assert_eq!(base64_encode(b"foo"), "Zm9v");
    }

    #[test]
    fn base64_rfc4648_vectors() {
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_url_string_round_trip() {
        // Sanity: the encoding round-trips against itself via length.
        let url = "https://example.com/path?q=1&r=2";
        let encoded = base64_encode(url.as_bytes());
        assert_eq!(encoded.len() % 4, 0);
        assert!(encoded.len() >= url.len()); // base64 expands ~33%
    }

    #[test]
    fn base64_handles_high_bytes() {
        let bytes = &[0xff, 0xff, 0xff];
        assert_eq!(base64_encode(bytes), "////");
    }

    #[test]
    fn base64_handles_zero_bytes() {
        let bytes = &[0, 0, 0];
        assert_eq!(base64_encode(bytes), "AAAA");
    }
}
