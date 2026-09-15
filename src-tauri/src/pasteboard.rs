//! System-clipboard access the web layer can't reach: reading file references
//! and raw image bytes (paste), and writing images (the editor's one-click image
//! copy).
//! macOS-only for now (NSPasteboard); other platforms return nothing / false
//! and the frontend falls back (paste: DOM `File`s; copy: the button reports a failure).

#[cfg(target_os = "macos")]
pub use imp::{file_paths, image_data, image_kind, string_text, write_image};

#[cfg(target_os = "macos")]
mod imp {
    use objc2::ClassType;
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeTIFF};
    use objc2_foundation::{NSArray, NSURL};
    use std::path::PathBuf;

    /// Absolute paths of files on the clipboard (Finder ⌘C), if any.
    pub fn file_paths() -> Vec<PathBuf> {
        let pb = NSPasteboard::generalPasteboard();
        let classes = NSArray::from_slice(&[NSURL::class()]);
        let Some(objects) = (unsafe { pb.readObjectsForClasses_options(&classes, None) }) else {
            return vec![];
        };
        objects
            .iter()
            .filter_map(|o| o.downcast::<NSURL>().ok())
            .filter(|url| url.isFileURL())
            .filter_map(|url| url.path().map(|p| PathBuf::from(p.to_string())))
            .collect()
    }

    /// Which image `image_data` would read — the extension alone, without copying
    /// the bytes (a probe before a paste whose bytes are only wanted later).
    pub fn image_kind() -> Option<&'static str> {
        let pb = NSPasteboard::generalPasteboard();
        let png = unsafe { NSPasteboardTypePNG };
        let types = NSArray::from_slice(&[png, unsafe { NSPasteboardTypeTIFF }]);
        let found = pb.availableTypeFromArray(&types)?;
        Some(if found.isEqualToString(png) { "png" } else { "tiff" })
    }

    /// Raw image bytes on the clipboard (screenshots, copied images) with the
    /// extension they should be saved under.
    pub fn image_data() -> Option<(Vec<u8>, &'static str)> {
        let pb = NSPasteboard::generalPasteboard();
        if let Some(data) = pb.dataForType(unsafe { NSPasteboardTypePNG }) {
            return Some((data.to_vec(), "png"));
        }
        if let Some(data) = pb.dataForType(unsafe { NSPasteboardTypeTIFF }) {
            return Some((data.to_vec(), "tiff"));
        }
        None
    }

    /// Plain text on the clipboard. The fallback leg of the editor's right-click
    /// "paste": ordinary text rather than a file or a screenshot (WKWebView's
    /// navigator.clipboard.readText is unreliable, so this is the main path).
    pub fn string_text() -> Option<String> {
        use objc2_app_kit::NSPasteboardTypeString;
        let pb = NSPasteboard::generalPasteboard();
        let s = unsafe { pb.stringForType(NSPasteboardTypeString) }?;
        Some(s.to_string())
    }

    /// Put encoded image bytes on the clipboard as a bitmap. Decoding is
    /// NSImage's job — png/jpg/gif/webp/tiff, whatever the OS reads — so we
    /// carry no image-decoding dependency. Returns false when the bytes don't
    /// carry no image-decoding dependency. Returns false when the bytes don't
    /// decode. A bitmap form: the paste target gets pixels, and an animation loses
    /// its frames (drag the original file from the file manager for that).
    pub fn write_image(bytes: &[u8]) -> bool {
        use objc2::runtime::ProtocolObject;
        use objc2::AnyThread;
        use objc2_app_kit::NSImage;
        use objc2_foundation::NSData;
        let data = NSData::with_bytes(bytes);
        let Some(img) = NSImage::initWithData(NSImage::alloc(), &data) else {
            return false;
        };
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let objects = NSArray::from_slice(&[ProtocolObject::from_ref(&*img)]);
        pb.writeObjects(&objects)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn file_paths() -> Vec<std::path::PathBuf> {
    vec![]
}

#[cfg(not(target_os = "macos"))]
pub fn image_kind() -> Option<&'static str> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn image_data() -> Option<(Vec<u8>, &'static str)> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn string_text() -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn write_image(_bytes: &[u8]) -> bool {
    false
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// Manual check (mutates the user's clipboard, so ignored by default):
    /// `osascript -e 'set the clipboard to POSIX file "/tmp/x"'` then
    /// `cargo test -p vemry-app --lib pasteboard -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn read_clipboard_manual() {
        println!("files: {:?}", super::file_paths());
        println!("image: {:?}", super::image_data().map(|(b, ext)| (b.len(), ext)));
    }

    /// Manual round-trip (mutates the user's clipboard, so ignored by
    /// default): write a 1×1 PNG via NSImage, read it back with image_data.
    #[test]
    #[ignore]
    fn write_image_roundtrip_manual() {
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78,
            0xda, 0x63, 0xfc, 0xcf, 0xc0, 0x50, 0x0f, 0x00, 0x04, 0x85, 0x01, 0x80, 0x84, 0xa9,
            0x8c, 0x21, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        assert!(super::write_image(PNG), "NSImage must recognise a PNG");
        assert!(super::image_data().is_some(), "the clipboard should hold an image");
        assert!(!super::write_image(b"not an image"), "non-image bytes must fail");
    }
}
