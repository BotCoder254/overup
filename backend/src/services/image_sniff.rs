//! Server-side image type detection by magic bytes (file signatures).
//!
//! OWASP File Upload guidance: the client's Content-Type header and filename
//! are trivially spoofable, so uploads are classified exclusively from their
//! leading bytes against a closed allow-list of raster image formats. Anything
//! that does not match is rejected wholesale — there is no fallback type.

/// A detected image format: `(content_type, file_extension)`.
pub type DetectedImage = (&'static str, &'static str);

/// Classify `bytes` as one of the allowed image formats, or `None`.
///
/// Allow-list: PNG, JPEG, GIF (87a/89a), WebP. SVG is deliberately excluded —
/// it is an XML document that can carry script.
pub fn detect(bytes: &[u8]) -> Option<DetectedImage> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(("image/png", "png"));
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Some(("image/jpeg", "jpg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(("image/gif", "gif"));
    }
    // RIFF container: bytes 0-3 "RIFF", 8-11 "WEBP" (4-7 are the size field).
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(("image/webp", "webp"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_png() {
        let bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        assert_eq!(detect(bytes), Some(("image/png", "png")));
    }

    #[test]
    fn detects_jpeg() {
        let bytes = b"\xff\xd8\xff\xe0\x00\x10JFIF";
        assert_eq!(detect(bytes), Some(("image/jpeg", "jpg")));
    }

    #[test]
    fn detects_gif_both_versions() {
        assert_eq!(detect(b"GIF87a\x01\x00"), Some(("image/gif", "gif")));
        assert_eq!(detect(b"GIF89a\x01\x00"), Some(("image/gif", "gif")));
    }

    #[test]
    fn detects_webp() {
        let bytes = b"RIFF\x24\x00\x00\x00WEBPVP8 ";
        assert_eq!(detect(bytes), Some(("image/webp", "webp")));
    }

    #[test]
    fn rejects_riff_that_is_not_webp() {
        // A WAV file is also RIFF — must not classify as an image.
        assert_eq!(detect(b"RIFF\x24\x00\x00\x00WAVEfmt "), None);
    }

    #[test]
    fn rejects_non_images_and_truncated_input() {
        assert_eq!(detect(b"<svg xmlns=\"http://www.w3.org/2000/svg\">"), None);
        assert_eq!(detect(b"#!/bin/sh\nrm -rf /"), None);
        assert_eq!(detect(b"MZ\x90\x00"), None);
        assert_eq!(detect(b""), None);
        assert_eq!(detect(b"RIFF"), None);
        assert_eq!(detect(b"\x89PN"), None);
    }
}
