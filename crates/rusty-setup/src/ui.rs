//! The installer's window: where rusty goes and a desktop shortcut, then
//! the progress with what is new going by under it, then Open — and, as
//! the uninstaller, a question, the progress and done.
//!
//! The work runs on a thread of its own and the window hears how far it
//! has got through the event loop, so the window keeps drawing while
//! four hundred megabytes unpack. The words are the system's language:
//! Chinese where the system's is, English otherwise.

use std::{path::PathBuf, rc::Rc, sync::Arc};

use rusty_setup::{Flags, payload};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::windows::{self, Step};

slint::include_modules!();

/// The window's words, in one language.
struct Text {
    tagline: &'static str,
    install_title: &'static str,
    update_title: &'static str,
    uninstall_title: &'static str,
    install_lead: &'static str,
    update_lead: &'static str,
    uninstall_lead: &'static str,
    location: &'static str,
    change: &'static str,
    desktop: &'static str,
    kept: &'static str,
    install: &'static str,
    update: &'static str,
    remove: &'static str,
    cancel: &'static str,
    installing: &'static str,
    removing: &'static str,
    installed: &'static str,
    installed_lead: &'static str,
    removed: &'static str,
    removed_lead: &'static str,
    open: &'static str,
    finish: &'static str,
    failed: &'static str,
    close: &'static str,
    whats_new: &'static str,
    step_files: &'static str,
    step_shortcuts: &'static str,
    step_removing: &'static str,
    highlights: [(&'static str, &'static str, &'static str); 6],
}

const ENGLISH: Text = Text {
    tagline: "The embedded Rust workbench",
    install_title: "Install rusty",
    update_title: "Update rusty",
    uninstall_title: "Remove rusty",
    install_lead: "For your account only, without administrator rights. The simulator, the debuggers and espflash come with it.",
    update_lead: "The rusty installed here is replaced with this version.",
    uninstall_lead: "rusty and the tools it came with are removed from this computer.",
    location: "Install location",
    change: "Change…",
    desktop: "Shortcut on the desktop",
    kept: "Your settings, drafts and projects are not touched.",
    install: "Install",
    update: "Update",
    remove: "Remove",
    cancel: "Cancel",
    installing: "Installing…",
    removing: "Removing…",
    installed: "rusty is installed",
    installed_lead: "It is in the Start menu. Open it now to pick up your last project where you left it.",
    removed: "rusty is removed",
    removed_lead: "Your settings, drafts and projects are where they were.",
    open: "Open rusty",
    finish: "Done",
    failed: "That did not work",
    close: "Close",
    whats_new: "WHAT'S NEW",
    step_files: "Unpacking the app and its tools",
    step_shortcuts: "Adding it to the Start menu",
    step_removing: "Removing files",
    highlights: [
        (
            "code",
            "A native window",
            "Rewritten in Slint: one process, no WebView. In a 25,000-line file a key reaches the screen in the next frame.",
        ),
        (
            "simulate",
            "Run firmware without a board",
            "rusty's own QEMU runs your ESP32 or CH32 firmware, and the LEDs, buttons and sensors you wire on the sheet follow it.",
        ),
        (
            "chip",
            "Five vendors' parts",
            "Espressif, STM32, RP2040 and RP2350, nRF52 and WCH CH32 — and Cargo, PlatformIO and CMake projects open as they are.",
        ),
        (
            "flash",
            "Build, flash and debug in one click",
            "Build and write the board, or run it in the simulator; stop at a breakpoint on either.",
        ),
        (
            "waves",
            "Signals and math",
            "A signal generator, a filter lab, plots and waves, and a worksheet that checks attitude quaternions before the drone does.",
        ),
        (
            "spark",
            "An assistant that reads your project",
            "Bring your own model. It calls rusty's own analyses instead of answering from memory.",
        ),
    ],
};

const CHINESE: Text = Text {
    tagline: "嵌入式 Rust 工作台",
    install_title: "安装 rusty",
    update_title: "更新 rusty",
    uninstall_title: "卸载 rusty",
    install_lead: "只为当前账户安装，不需要管理员权限。模拟器、调试器和 espflash 一并装好。",
    update_lead: "这里已安装的 rusty 会被替换为这个版本。",
    uninstall_lead: "从这台电脑上移除 rusty 和它附带的工具。",
    location: "安装位置",
    change: "更改…",
    desktop: "在桌面创建快捷方式",
    kept: "你的设置、草稿和项目不受影响。",
    install: "安装",
    update: "更新",
    remove: "卸载",
    cancel: "取消",
    installing: "正在安装…",
    removing: "正在卸载…",
    installed: "rusty 已安装",
    installed_lead: "开始菜单里可以找到它。现在打开，就能接着上次的项目继续。",
    removed: "rusty 已卸载",
    removed_lead: "你的设置、草稿和项目都还在原处。",
    open: "打开 rusty",
    finish: "完成",
    failed: "没有成功",
    close: "关闭",
    whats_new: "新版亮点",
    step_files: "正在解压程序和工具",
    step_shortcuts: "正在添加到开始菜单",
    step_removing: "正在删除文件",
    highlights: [
        (
            "code",
            "原生窗口，下一帧就响应",
            "用 Slint 重写：单个进程，不再依赖 WebView。在 2.5 万行的文件里打字，按键在下一帧就出现在屏幕上。",
        ),
        (
            "simulate",
            "没有开发板也能跑固件",
            "rusty 自带的 QEMU 运行你的 ESP32、CH32 固件；在电路图上连好的 LED、按键和传感器会跟着固件动起来。",
        ),
        (
            "chip",
            "五家厂商的芯片",
            "Espressif、STM32、RP2040/RP2350、nRF52、WCH CH32；Cargo、PlatformIO、CMake 项目直接打开。",
        ),
        (
            "flash",
            "一键构建、烧录、调试",
            "构建后直接写进开发板，或在模拟器里运行；两边都能在断点处停下。",
        ),
        (
            "waves",
            "信号与数学工具",
            "信号发生器、滤波器实验室、曲线和波形，还有在无人机起飞前核对姿态四元数的数学工作表。",
        ),
        (
            "spark",
            "读得懂项目的助手",
            "接入你自己的模型；它调用 rusty 自己的分析工具，而不是凭记忆回答。",
        ),
    ],
};

fn text() -> &'static Text {
    let chinese = sys_locale::get_locale().is_some_and(|tag| tag.to_lowercase().starts_with("zh"));
    if chinese { &CHINESE } else { &ENGLISH }
}

/// The window with its words in place.
fn window(mode: Mode, t: &'static Text) -> Result<SetupWindow, String> {
    let window = SetupWindow::new().map_err(|e| e.to_string())?;
    let words = window.global::<Words>();
    words.set_tagline(t.tagline.into());
    words.set_install_title(t.install_title.into());
    words.set_update_title(t.update_title.into());
    words.set_uninstall_title(t.uninstall_title.into());
    words.set_install_lead(t.install_lead.into());
    words.set_update_lead(t.update_lead.into());
    words.set_uninstall_lead(t.uninstall_lead.into());
    words.set_location(t.location.into());
    words.set_change(t.change.into());
    words.set_desktop(t.desktop.into());
    words.set_kept(t.kept.into());
    words.set_install(t.install.into());
    words.set_update(t.update.into());
    words.set_remove(t.remove.into());
    words.set_cancel(t.cancel.into());
    words.set_installing(t.installing.into());
    words.set_removing(t.removing.into());
    words.set_installed(t.installed.into());
    words.set_installed_lead(t.installed_lead.into());
    words.set_removed(t.removed.into());
    words.set_removed_lead(t.removed_lead.into());
    words.set_open(t.open.into());
    words.set_finish(t.finish.into());
    words.set_failed(t.failed.into());
    words.set_close(t.close.into());
    words.set_whats_new(t.whats_new.into());
    let icons = window.global::<Icons>();
    let highlights: Vec<Highlight> = t
        .highlights
        .iter()
        .map(|(icon, title, body)| Highlight {
            icon: match *icon {
                "code" => icons.get_code(),
                "simulate" => icons.get_simulate(),
                "chip" => icons.get_chip(),
                "flash" => icons.get_flash(),
                "waves" => icons.get_waves(),
                _ => icons.get_spark(),
            },
            title: (*title).into(),
            body: (*body).into(),
        })
        .collect();
    window.set_highlights(ModelRc::new(VecModel::from(highlights)));
    window.set_mode(mode);
    window.set_version(windows::VERSION.into());
    Ok(window)
}

/// Progress from the worker, onto the window — only when the shown percent
/// would change, so a fast unpack does not flood the event loop.
fn reporter(window: slint::Weak<SetupWindow>, t: &'static Text) -> impl FnMut(Step, f32) {
    let mut last = (-1i32, None::<Step>);
    move |step, done| {
        let percent = (done * 1000.0) as i32;
        if (percent, Some(step)) == last {
            return;
        }
        last = (percent, Some(step));
        let label = match step {
            Step::Files => t.step_files,
            Step::Shortcuts => t.step_shortcuts,
        };
        let _ = window.upgrade_in_event_loop(move |w| {
            w.set_progress(done);
            w.set_step(label.into());
        });
    }
}

/// The installer: the window until it is closed.
pub fn install(exe: Vec<u8>, flags: &Flags) -> Result<(), String> {
    let t = text();
    let exe = Arc::new(exe);
    let (own_len, payload_len) = payload::find(&exe)
        .map(|(own, payload)| (own.len(), payload.len()))
        .ok_or("no payload")?;
    let dir = windows::target_dir(flags);
    let update = windows::holds_rusty(&dir);
    let window = window(if update { Mode::Update } else { Mode::Install }, t)?;
    window.set_location(dir.display().to_string().into());
    window.set_desktop(!update || windows::desktop_link().exists());
    let chosen = Rc::new(std::cell::RefCell::new(dir));
    let args = flags.args.clone();

    {
        let weak = window.as_weak();
        let chosen = chosen.clone();
        window.on_change_location(move || {
            let Some(w) = weak.upgrade() else { return };
            let Some(picked) = rfd::FileDialog::new().pick_folder() else {
                return;
            };
            // A folder named for rusty is the install; any other is where
            // its folder goes.
            let dir = if picked
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("rusty"))
            {
                picked
            } else {
                picked.join("rusty")
            };
            w.set_location(dir.display().to_string().into());
            *chosen.borrow_mut() = dir;
        });
    }
    {
        let weak = window.as_weak();
        let chosen = chosen.clone();
        window.on_begin(move || {
            let Some(w) = weak.upgrade() else { return };
            w.set_page(Page::Working);
            w.set_progress(0.0);
            w.set_step(t.step_files.into());
            let dir = chosen.borrow().clone();
            let desktop = w.get_desktop();
            let exe = exe.clone();
            let weak = w.as_weak();
            let mut report = reporter(weak.clone(), t);
            let worker = std::thread::spawn(move || {
                let (own, payload) = (&exe[..own_len], &exe[own_len..own_len + payload_len]);
                windows::install_into(own, payload, &dir, desktop, &mut report)
            });
            // The worker's answer, back on the window's thread.
            std::thread::spawn(move || {
                let answer = worker
                    .join()
                    .unwrap_or_else(|_| Err("the installer stopped".into()));
                let _ = weak.upgrade_in_event_loop(move |w| match answer {
                    Ok(app) => {
                        w.set_progress(1.0);
                        w.set_page(Page::Done);
                        INSTALLED.with(|cell| *cell.borrow_mut() = Some(app));
                    }
                    Err(why) => {
                        w.set_error(why.into());
                        w.set_page(Page::Failed);
                    }
                });
            });
        });
    }
    {
        let weak = window.as_weak();
        window.on_open_app(move || {
            if let Some(app) = INSTALLED.with(|cell| cell.borrow().clone())
                && let Err(why) = windows::open(&app, &args)
            {
                if let Some(w) = weak.upgrade() {
                    w.set_error(why.into());
                    w.set_page(Page::Failed);
                }
                return;
            }
            let _ = slint::quit_event_loop();
        });
    }
    window.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    #[cfg(feature = "shots")]
    shots::photograph(&window, t);
    window.run().map_err(|e| e.to_string())
}

thread_local! {
    /// The app the install put in place, for Open.
    static INSTALLED: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// The uninstaller: the window until it is closed, and the directory taken
/// away a moment after.
pub fn uninstall(_flags: &Flags) -> Result<(), String> {
    let t = text();
    let dir = windows::own_dir()?;
    let window = window(Mode::Uninstall, t)?;
    window.set_location(dir.display().to_string().into());
    let finished = Rc::new(std::cell::Cell::new(false));
    {
        let weak = window.as_weak();
        let dir = dir.clone();
        let finished = finished.clone();
        window.on_begin(move || {
            let Some(w) = weak.upgrade() else { return };
            w.set_page(Page::Working);
            w.set_progress(0.0);
            w.set_step(t.step_removing.into());
            let dir = dir.clone();
            let weak = w.as_weak();
            let progress_to = weak.clone();
            let worker = std::thread::spawn(move || {
                let mut last = -1i32;
                windows::uninstall_from(&dir, &mut |done| {
                    let percent = (done * 1000.0) as i32;
                    if percent != last {
                        last = percent;
                        let _ = progress_to.upgrade_in_event_loop(move |w| w.set_progress(done));
                    }
                })
            });
            finished.set(true);
            std::thread::spawn(move || {
                let answer = worker
                    .join()
                    .unwrap_or_else(|_| Err("the uninstaller stopped".into()));
                let _ = weak.upgrade_in_event_loop(move |w| match answer {
                    Ok(()) => {
                        w.set_progress(1.0);
                        w.set_page(Page::Done);
                    }
                    Err(why) => {
                        w.set_error(why.into());
                        w.set_page(Page::Failed);
                    }
                });
            });
        });
    }
    window.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    window.on_change_location(|| {});
    window.on_open_app(|| {});
    #[cfg(feature = "shots")]
    shots::photograph(&window, t);
    window.run().map_err(|e| e.to_string())?;
    if finished.get() {
        windows::remove_after_exit(&dir);
    }
    Ok(())
}

/// Each page photographed into `RUSTY_SETUP_SHOTS`, then the window shut —
/// for looking at the window without clicking through it, or installing
/// anything. A development build's only (`--features shots`).
#[cfg(feature = "shots")]
mod shots {
    use std::time::Duration;

    use slint::ComponentHandle;

    use super::{Page, SetupWindow, Text};

    /// A page's name and what puts the window on it.
    type Steps = std::rc::Rc<Vec<(&'static str, Box<dyn Fn(&SetupWindow)>)>>;

    pub fn photograph(window: &SetupWindow, t: &'static Text) {
        let Some(dir) = std::env::var_os("RUSTY_SETUP_SHOTS").map(std::path::PathBuf::from) else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        let weak = window.as_weak();
        let mode = format!("{:?}", window.get_mode()).to_lowercase();
        let steps: Steps = std::rc::Rc::new(vec![
            ("welcome", Box::new(|_| {})),
            (
                "working",
                Box::new(move |w| {
                    w.set_page(Page::Working);
                    w.set_progress(0.42);
                    w.set_step(t.step_files.into());
                    w.set_shown(1);
                }),
            ),
            ("done", Box::new(|w| w.set_page(Page::Done))),
            (
                "failed",
                Box::new(|w| {
                    w.set_error("rusty.exe: Access is denied. (os error 5)".into());
                    w.set_page(Page::Failed);
                }),
            ),
        ]);
        fn next(
            weak: slint::Weak<SetupWindow>,
            steps: Steps,
            at: usize,
            dir: std::path::PathBuf,
            mode: String,
        ) {
            slint::Timer::single_shot(Duration::from_millis(900), move || {
                let Some(w) = weak.upgrade() else { return };
                if at >= steps.len() {
                    let _ = slint::quit_event_loop();
                    return;
                }
                (steps[at].1)(&w);
                let weak2 = weak.clone();
                let steps2 = steps.clone();
                let dir2 = dir.clone();
                let mode2 = mode.clone();
                slint::Timer::single_shot(Duration::from_millis(900), move || {
                    let Some(w) = weak2.upgrade() else { return };
                    if let Ok(shot) = w.window().take_snapshot() {
                        let path = dir2.join(format!("{mode2}-{}.png", steps2[at].0));
                        let _ = image::save_buffer(
                            &path,
                            shot.as_bytes(),
                            shot.width(),
                            shot.height(),
                            image::ColorType::Rgba8,
                        );
                        eprintln!("shot: {}", path.display());
                    }
                    next(weak2, steps2, at + 1, dir2, mode2);
                });
            });
        }
        next(weak, steps, 0, dir, mode);
    }
}
