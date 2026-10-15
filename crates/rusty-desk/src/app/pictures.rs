//! Pictures in the window: a picture file opened from the tree, as a page
//! of its own (`Tab::Picture`); an SVG's picture over its source, drawn from
//! the document so an unsaved edit shows the moment the page is turned back
//! to; and the pictures a Markdown page names. All three read through
//! `crate::picture`, and the files are kept here, read once, until the
//! watcher says one changed.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use rusty_i18n::t;
use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer};

use super::Tab;
use crate::picture;

/// A picture file as the window has it.
pub(super) enum Pic {
    Ready {
        image: slint::Image,
        width: u32,
        height: u32,
        bytes: u64,
    },
    Failed(String),
    Pending,
}

/// Picture files read, by where they are.
#[derive(Default)]
pub(super) struct Pictures {
    files: HashMap<PathBuf, Pic>,
}

/// Pictures kept beyond the ones in front.
const KEEP: usize = 200;

impl Pictures {
    pub(super) fn get(&self, path: &Path) -> Option<&Pic> {
        self.files.get(path)
    }
}

/// A checkerboard tile, the ground a picture's transparency shows through.
fn checker(light: [u8; 3], dark: [u8; 3]) -> slint::Image {
    const SQUARE: u32 = 8;
    let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(SQUARE * 2, SQUARE * 2);
    let width = pixels.width();
    for (at, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = (at as u32 % width, at as u32 / width);
        let [r, g, b] = if (x / SQUARE + y / SQUARE).is_multiple_of(2) {
            light
        } else {
            dark
        };
        *pixel = Rgba8Pixel { r, g, b, a: 255 };
    }
    slint::Image::from_rgba8(pixels)
}

/// What a picture is, in a line: its size in pixels and on disk.
fn info(width: u32, height: u32, bytes: Option<u64>) -> String {
    match bytes {
        Some(bytes) => t!(
            "picture.info",
            width = width.to_string(),
            height = height.to_string(),
            size = crate::format::bytes(bytes)
        ),
        None => t!(
            "picture.size",
            width = width.to_string(),
            height = height.to_string()
        ),
    }
}

impl super::App {
    /// The checkerboards, once, as the window starts.
    pub(super) fn picture_ground(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<crate::Shell>();
        shell.set_picture_checker(checker([0xff, 0xff, 0xff], [0xe4, 0xe4, 0xe4]));
        shell.set_picture_checker_dark(checker([0x3a, 0x38, 0x36], [0x2c, 0x2a, 0x28]));
    }

    /// Open a picture file as a page of its own. A binary picture is never
    /// opened as text: no document holds it and no server hears of it.
    pub(super) fn open_picture(self: &Rc<Self>, path: String) {
        self.open_tab(Tab::Picture(path));
    }

    /// Read a picture file, unless it is read or being read.
    pub(super) fn want_picture(self: &Rc<Self>, path: PathBuf) {
        {
            let mut state = self.state.borrow_mut();
            if state.pictures.files.contains_key(&path) {
                return;
            }
            state.pictures.files.insert(path.clone(), Pic::Pending);
        }
        crate::work::spawn(
            move || {
                let read = picture::read(&path);
                (path, read)
            },
            |app, (path, read)| {
                let pic = match read.and_then(|read| {
                    let image = picture::image(&read)?;
                    Ok((image, read.bytes))
                }) {
                    Ok((image, bytes)) => {
                        let size = image.size();
                        Pic::Ready {
                            image,
                            width: size.width,
                            height: size.height,
                            bytes,
                        }
                    }
                    Err(reason) => Pic::Failed(reason),
                };
                app.state.borrow_mut().pictures.files.insert(path, pic);
                app.pictures_arrived();
            },
        );
    }

    /// A picture read: whatever shows one shows it now.
    fn pictures_arrived(self: &Rc<Self>) {
        match self.active_tab() {
            Some(Tab::Picture(_)) => self.sync_picture(),
            Some(Tab::File(_)) => self.sync_markdown(),
            _ => {}
        }
    }

    /// Files the watcher saw change: a picture among them is read again
    /// where it shows.
    pub(super) fn pictures_changed(self: &Rc<Self>, changed: &[String]) {
        let Some(root) = self.root() else { return };
        let mut dropped = false;
        {
            let mut state = self.state.borrow_mut();
            for path in changed.iter().filter(|p| picture::is_picture(p)) {
                if let Ok(full) = rusty_edit::absolute(&root, path) {
                    dropped |= state.pictures.files.remove(&full).is_some();
                }
                dropped |= state.pictures.files.remove(&root.join(path)).is_some();
            }
            let files = &mut state.pictures.files;
            if files.len() > KEEP {
                files.retain(|_, pic| matches!(pic, Pic::Pending));
            }
        }
        if dropped {
            self.pictures_arrived();
        }
    }

    /// The picture page in front: the file, read or being read, or why it
    /// could not be.
    pub(super) fn sync_picture(self: &Rc<Self>) {
        let Some(Tab::Picture(path)) = self.active_tab() else {
            return;
        };
        let Some(window) = self.window() else { return };
        let shell = window.global::<crate::Shell>();
        let Some(root) = self.root() else { return };
        let full = match rusty_edit::absolute(&root, &path) {
            Ok(full) => full,
            Err(e) => {
                shell.set_picture_shown(false);
                shell.set_picture_note(t!("picture.failed", reason = e.to_string()).into());
                return;
            }
        };
        let (shown, note, info_line, image) = match self.state.borrow().pictures.get(&full) {
            Some(Pic::Ready {
                image,
                width,
                height,
                bytes,
            }) => (
                true,
                String::new(),
                info(*width, *height, Some(*bytes)),
                image.clone(),
            ),
            Some(Pic::Failed(reason)) => (
                false,
                t!("picture.failed", reason = reason.clone()),
                String::new(),
                slint::Image::default(),
            ),
            Some(Pic::Pending) => (
                false,
                t!("picture.reading"),
                String::new(),
                slint::Image::default(),
            ),
            None => (false, String::new(), String::new(), slint::Image::default()),
        };
        if self.state.borrow().pictures.get(&full).is_none() {
            self.want_picture(full);
            shell.set_picture_note(t!("picture.reading").into());
            shell.set_picture_shown(false);
            return;
        }
        shell.set_picture(image);
        shell.set_picture_shown(shown);
        shell.set_picture_note(note.into());
        shell.set_picture_info(info_line.into());
    }

    /// An SVG open in the editor, drawn from its text as it stands.
    pub(super) fn show_svg(&self, text: &str) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<crate::Shell>();
        match picture::svg(text.as_bytes()) {
            Ok(image) => {
                let size = image.size();
                shell.set_picture_info(info(size.width, size.height, None).into());
                shell.set_picture(image);
                shell.set_picture_shown(true);
                shell.set_picture_note("".into());
            }
            Err(reason) => {
                shell.set_picture_shown(false);
                shell.set_picture_note(t!("picture.failed", reason = reason).into());
            }
        }
    }
}
