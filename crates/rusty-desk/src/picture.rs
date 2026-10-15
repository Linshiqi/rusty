//! A picture from the project — a file opened from the tree, or one a
//! Markdown page names — read the one way both use, so a figure on a page
//! and the same file opened on its own decode alike, and fail alike.
//!
//! The file is read and a raster decoded off the UI thread (`read`); the
//! `slint::Image` is made on it (`image`), since an image is not `Send`. An
//! SVG stays vector to the end, so it is drawn crisply at any size. A
//! picture that cannot be decoded says why, in the decoder's own words: the
//! window's own loader says only that it could not.

use std::path::Path;

use slint::{Rgba8Pixel, SharedPixelBuffer};

/// The extensions read as pictures. Anything else opened from the tree is
/// text, or refused as binary.
const PICTURES: [&str; 9] = [
    "svg", "png", "jpg", "jpeg", "gif", "bmp", "webp", "ico", "svgz",
];

fn extension(path: &str) -> String {
    path.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether a file is a picture, by its name.
pub fn is_picture(path: &str) -> bool {
    PICTURES.contains(&extension(path).as_str())
}

/// Whether a picture is an SVG, which is text as well: it opens in the
/// editor, with its picture over it.
pub fn is_svg(path: &str) -> bool {
    extension(path) == "svg"
}

/// A picture's content, read and checked.
pub enum Content {
    Pixels(SharedPixelBuffer<Rgba8Pixel>),
    Svg(Vec<u8>),
}

/// A picture as read off the disk: what it holds, and how many bytes the
/// file is.
pub struct Read {
    pub content: Content,
    pub bytes: u64,
}

/// Read a picture: an SVG checked as the window will draw it, anything else
/// decoded to pixels. Off the UI thread; the error is the reason.
pub fn read(path: &Path) -> Result<Read, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let bytes = data.len() as u64;
    let name = path.to_string_lossy();
    let content = if matches!(extension(&name).as_str(), "svg" | "svgz") {
        check_svg(&data)?;
        Content::Svg(data)
    } else {
        Content::Pixels(decode(&data)?)
    };
    Ok(Read { content, bytes })
}

/// Raster bytes as RGBA pixels, the format taken from the bytes themselves.
fn decode(data: &[u8]) -> Result<SharedPixelBuffer<Rgba8Pixel>, String> {
    let picture = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?
        .into_rgba8();
    Ok(SharedPixelBuffer::clone_from_slice(
        picture.as_raw(),
        picture.width(),
        picture.height(),
    ))
}

/// Why an SVG would not draw, in the parser's words.
fn check_svg(data: &[u8]) -> Result<(), String> {
    usvg::Tree::from_data(data, &usvg::Options::default())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The picture to draw, on the UI thread.
pub fn image(read: &Read) -> Result<slint::Image, String> {
    match &read.content {
        Content::Pixels(pixels) => Ok(slint::Image::from_rgba8(pixels.clone())),
        Content::Svg(data) => svg(data),
    }
}

/// An SVG's text as a picture: what a `.svg` open in the editor looks like
/// now, its unsaved edits included.
pub fn svg(data: &[u8]) -> Result<slint::Image, String> {
    slint::Image::load_from_svg_data(data).map_err(|_| match check_svg(data) {
        Err(reason) => reason,
        // The parser read it and the window still would not: say so rather
        // than nothing.
        Ok(()) => "the SVG could not be drawn".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pictures are known by their extension, whatever its case; an SVG is
    /// also text.
    #[test]
    fn a_picture_is_known_by_its_name() {
        for path in [
            "a.png", "b/c.JPG", "x.svg", "d.webp", "e.ico", "f.gif", "g.bmp",
        ] {
            assert!(is_picture(path), "{path}");
        }
        for path in ["a.rs", "png", "notes.md", "svg.txt"] {
            assert!(!is_picture(path), "{path}");
        }
        assert!(is_svg("fig.SVG") && !is_svg("fig.png"));
    }

    /// A picture that cannot be decoded comes back as the decoder's reason,
    /// not as an empty picture.
    #[test]
    fn a_picture_that_does_not_decode_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("broken.png");
        std::fs::write(&png, b"\x89PNG\r\n\x1a\n not really").unwrap();
        let reason = read(&png).err().expect("refused");
        assert!(!reason.is_empty());
        let svg = dir.path().join("broken.svg");
        std::fs::write(&svg, "<svg xmlns='http://www.w3.org/2000/svg'><g></svg>").unwrap();
        assert!(read(&svg).is_err());
        let missing = read(&dir.path().join("gone.png")).err().unwrap();
        assert!(!missing.is_empty());
    }

    /// A picture that decodes is its pixels and its size on disk, whatever
    /// its extension says: the bytes decide the format.
    #[test]
    fn a_picture_decodes_to_its_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dot.png");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 128]))
            .save(&path)
            .unwrap();
        let read = read(&path).unwrap();
        let Content::Pixels(pixels) = &read.content else {
            panic!("pixels")
        };
        assert_eq!((pixels.width(), pixels.height()), (3, 2));
        assert_eq!(read.bytes, std::fs::metadata(&path).unwrap().len());
        let good = dir.path().join("ok.svg");
        std::fs::write(
            &good,
            "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4'/></svg>",
        )
        .unwrap();
        assert!(matches!(
            super::read(&good).unwrap().content,
            Content::Svg(_)
        ));
    }
}
