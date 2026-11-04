//! A small video player for the Changelog window, built on Media Foundation's
//! media engine. The engine decodes and plays the sound on its own threads;
//! a worker thread copies each new frame out so egui can draw it.
//!
//! The video is downloaded into the temp folder first and kept there, so
//! watching it again does not download it again.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICBitmap, IWICImagingFactory,
    WICBitmapCacheOnLoad, WICBitmapLockRead, WICRect,
};
use windows::Win32::Media::MediaFoundation::{
    CLSID_MFMediaEngineClassFactory, IMFAttributes, IMFMediaEngine, IMFMediaEngineClassFactory,
    IMFMediaEngineNotify, IMFMediaEngineNotify_Impl, MF_MEDIA_ENGINE_CALLBACK,
    MF_MEDIA_ENGINE_EVENT_ENDED, MF_MEDIA_ENGINE_EVENT_ERROR,
    MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA, MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MF_VERSION,
    MFCreateAttributes, MFSTARTUP_FULL, MFShutdown, MFStartup,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::{BSTR, implement};

use crate::i18n::{tr, trf};

/// DXGI_FORMAT_B8G8R8A8_UNORM, the layout of the WIC bitmap frames go into.
const BGRA: u32 = 87;
/// Frames wider than this are scaled down; the Changelog window is narrower.
const MAX_WIDTH: u32 = 1280;

/// Where the player is.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Downloading the file, with the percentage done.
    Downloading(u8),
    /// The file is opening.
    Loading,
    Playing,
    Paused,
    Ended,
    Failed(String),
}

/// What the controls show.
#[derive(Clone, Debug)]
pub struct Status {
    pub phase: Phase,
    /// Seconds.
    pub position: f64,
    pub duration: f64,
    pub muted: bool,
    /// 0.0 to 1.0.
    pub volume: f32,
}

/// One decoded picture, as RGBA.
pub struct Frame {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

enum Command {
    Play,
    Pause,
    Seek(f64),
    Mute(bool),
    Volume(f32),
}

struct Shared {
    status: Mutex<Status>,
    frame: Mutex<Option<Frame>>,
    stop: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A video that is playing (or getting ready to). Dropping it stops playback
/// and frees the engine.
pub struct Player {
    commands: Sender<Command>,
    shared: Arc<Shared>,
}

impl Player {
    /// Starts downloading `url` and plays it as soon as it is ready.
    /// `repaint` is called whenever there is something new to draw.
    pub fn start(url: &str, repaint: impl Fn() + Send + 'static) -> Self {
        let (commands, inbox) = channel();
        let shared = Arc::new(Shared {
            status: Mutex::new(Status {
                phase: Phase::Downloading(0),
                position: 0.0,
                duration: 0.0,
                muted: false,
                volume: 1.0,
            }),
            frame: Mutex::new(None),
            stop: AtomicBool::new(false),
        });
        let worker = shared.clone();
        let url = url.to_string();
        std::thread::spawn(move || {
            let result = cached_file(&url, &worker, &repaint)
                .and_then(|path| play(&path, &worker, &inbox, &repaint));
            if let Err(e) = result {
                lock(&worker.status).phase = Phase::Failed(e);
            }
            repaint();
        });
        Self { commands, shared }
    }

    pub fn status(&self) -> Status {
        lock(&self.shared.status).clone()
    }

    /// The newest frame, if one arrived since the last call.
    pub fn take_frame(&self) -> Option<Frame> {
        lock(&self.shared.frame).take()
    }

    pub fn play(&self) {
        let _ = self.commands.send(Command::Play);
    }

    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    pub fn seek(&self, seconds: f64) {
        lock(&self.shared.status).position = seconds;
        let _ = self.commands.send(Command::Seek(seconds));
    }

    pub fn set_muted(&self, muted: bool) {
        lock(&self.shared.status).muted = muted;
        let _ = self.commands.send(Command::Mute(muted));
    }

    /// `volume` from 0.0 (silent) to 1.0 (full).
    pub fn set_volume(&self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        lock(&self.shared.status).volume = volume;
        let _ = self.commands.send(Command::Volume(volume));
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

/// The video in the temp folder, downloaded first if it is not there yet.
fn cached_file(url: &str, shared: &Shared, repaint: &dyn Fn()) -> Result<PathBuf, String> {
    let name = url
        .rsplit('/')
        .next()
        .filter(|n| n.ends_with(".mp4") && !n.contains(['\\', ':', '?']))
        .ok_or_else(|| trf("{0} is not a video RustCleanup can play.", &[&url]))?;
    let dir = std::env::temp_dir().join("RustCleanup");
    let path = dir.join(name);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 0) {
        return Ok(path);
    }
    let last = AtomicU8::new(0);
    let bytes = super::updater::download(url, |done, total| {
        if let Some(total) = total.filter(|t| *t > 0) {
            let percent = (done * 100 / total).min(99) as u8;
            if percent != last.swap(percent, Ordering::Relaxed) {
                lock(&shared.status).phase = Phase::Downloading(percent);
                repaint();
            }
        }
    })
    .map_err(|e| trf("Could not download the video: {0}", &[&e]))?;
    if shared.stop.load(Ordering::Relaxed) {
        return Err(String::new());
    }
    // Written under another name first, so a cut-off download is never played.
    let part = dir.join(format!("{name}.part"));
    std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(&part, &bytes))
        .and_then(|_| std::fs::rename(&part, &path))
        .map_err(|e| trf("Could not save the video: {0}", &[&e]))?;
    Ok(path)
}

/// What the engine reported through its callback.
#[derive(Default)]
struct Events {
    metadata: AtomicBool,
    ended: AtomicBool,
    error: AtomicBool,
}

#[implement(IMFMediaEngineNotify)]
struct Notify(Arc<Events>);

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _param1: usize, _param2: u32) -> windows::core::Result<()> {
        let flag = match event as i32 {
            e if e == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0 => &self.0.metadata,
            e if e == MF_MEDIA_ENGINE_EVENT_ENDED.0 => &self.0.ended,
            e if e == MF_MEDIA_ENGINE_EVENT_ERROR.0 => &self.0.error,
            _ => return Ok(()),
        };
        flag.store(true, Ordering::Relaxed);
        Ok(())
    }
}

/// Keeps COM and Media Foundation started on this thread while alive.
struct Runtime;

impl Runtime {
    fn start() -> Result<Self, String> {
        // SAFETY: a worker thread of our own; balanced in Drop.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| super::com::message(&e))?;
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(super::com::message(&e));
            }
        }
        Ok(Self)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: balances the successful calls in `start`, on the same thread.
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

/// Plays `path` until the player is dropped.
fn play(
    path: &std::path::Path,
    shared: &Shared,
    inbox: &Receiver<Command>,
    repaint: &dyn Fn(),
) -> Result<(), String> {
    lock(&shared.status).phase = Phase::Loading;
    repaint();
    let _runtime = Runtime::start()?;
    let fail = |e: windows::core::Error| trf("Could not play the video: {0}", &[&super::com::message(&e)]);
    let events = Arc::new(Events::default());

    // SAFETY: COM calls on interfaces created here; the engine is shut down
    // before the runtime is.
    unsafe {
        let mut attributes: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attributes, 2).map_err(fail)?;
        let attributes = attributes.ok_or_else(|| tr("Could not play the video."))?;
        let notify: IMFMediaEngineNotify = Notify(events.clone()).into();
        attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify).map_err(fail)?;
        attributes.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, BGRA).map_err(fail)?;
        let factory: IMFMediaEngineClassFactory =
            CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(fail)?;
        let engine = factory.CreateInstance(0, &attributes).map_err(fail)?;
        let result = run(&engine, path, shared, &events, inbox, repaint);
        let _ = engine.Shutdown();
        result
    }
}

/// The playback loop: commands in, frames and status out.
///
/// # Safety
/// `engine` must be a live media engine on a thread with COM started.
unsafe fn run(
    engine: &IMFMediaEngine,
    path: &std::path::Path,
    shared: &Shared,
    events: &Events,
    inbox: &Receiver<Command>,
    repaint: &dyn Fn(),
) -> Result<(), String> {
    let fail = |e: windows::core::Error| trf("Could not play the video: {0}", &[&super::com::message(&e)]);
    // SAFETY: see the function's contract.
    unsafe {
        engine
            .SetSource(&BSTR::from(path.to_string_lossy().as_ref()))
            .map_err(fail)?;
        let (muted, volume) = {
            let s = lock(&shared.status);
            (s.muted, s.volume)
        };
        engine.SetMuted(muted).map_err(fail)?;
        engine.SetVolume(f64::from(volume)).map_err(fail)?;
        engine.Play().map_err(fail)?;

        let wic: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(fail)?;
        let mut target: Option<(IWICBitmap, u32, u32)> = None;
        let mut last_pts = i64::MIN;
        let mut last_second = -1;

        while !shared.stop.load(Ordering::Relaxed) {
            match inbox.recv_timeout(Duration::from_millis(5)) {
                Ok(Command::Play) => {
                    if engine.IsEnded().as_bool() {
                        let _ = engine.SetCurrentTime(0.0);
                    }
                    let _ = engine.Play();
                }
                Ok(Command::Pause) => {
                    let _ = engine.Pause();
                }
                Ok(Command::Seek(t)) => {
                    let _ = engine.SetCurrentTime(t);
                }
                Ok(Command::Mute(m)) => {
                    let _ = engine.SetMuted(m);
                }
                Ok(Command::Volume(v)) => {
                    let _ = engine.SetVolume(f64::from(v));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if events.error.swap(false, Ordering::Relaxed) {
                let code = engine.GetError().map(|e| e.GetErrorCode()).unwrap_or(0);
                return Err(trf("Could not play the video (error {0}).", &[&code]));
            }

            if target.is_none() && events.metadata.load(Ordering::Relaxed) {
                let (mut w, mut h) = (0u32, 0u32);
                engine.GetNativeVideoSize(Some(&mut w), Some(&mut h)).map_err(fail)?;
                if w > MAX_WIDTH {
                    h = h * MAX_WIDTH / w;
                    w = MAX_WIDTH;
                }
                if w == 0 || h == 0 {
                    return Err(tr("The video has no picture.").into());
                }
                let bitmap = wic
                    .CreateBitmap(w, h, &GUID_WICPixelFormat32bppBGRA, WICBitmapCacheOnLoad)
                    .map_err(fail)?;
                target = Some((bitmap, w, h));
            }

            let mut fresh = false;
            if let Some((bitmap, w, h)) = &target
                && let Ok(pts) = engine.OnVideoStreamTick()
                && pts != last_pts
            {
                last_pts = pts;
                let rect = RECT {
                    left: 0,
                    top: 0,
                    right: *w as i32,
                    bottom: *h as i32,
                };
                if engine.TransferVideoFrame(bitmap, None, &rect, None).is_ok()
                    && let Some(frame) = copy_frame(bitmap, *w, *h)
                {
                    *lock(&shared.frame) = Some(frame);
                    fresh = true;
                }
            }

            let ended = events.ended.load(Ordering::Relaxed) && engine.IsEnded().as_bool();
            let position = engine.GetCurrentTime();
            let duration = engine.GetDuration();
            let phase = if ended {
                Phase::Ended
            } else if engine.IsPaused().as_bool() {
                Phase::Paused
            } else if target.is_some() {
                Phase::Playing
            } else {
                Phase::Loading
            };
            let changed = {
                let mut status = lock(&shared.status);
                let changed = status.phase != phase;
                status.phase = phase;
                status.position = position;
                status.duration = if duration.is_finite() { duration } else { 0.0 };
                changed
            };
            // The clock under the video moves once a second even when the
            // picture does not change.
            let second = position as i64;
            if fresh || changed || second != last_second {
                last_second = second;
                repaint();
            }
        }
        Ok(())
    }
}

/// Copies the WIC bitmap out as RGBA.
///
/// # Safety
/// `bitmap` must be a 32bpp BGRA bitmap of `w` by `h`.
unsafe fn copy_frame(bitmap: &IWICBitmap, w: u32, h: u32) -> Option<Frame> {
    let rect = WICRect {
        X: 0,
        Y: 0,
        Width: w as i32,
        Height: h as i32,
    };
    // SAFETY: see the function's contract; the lock is held while reading.
    unsafe {
        let pixels = bitmap.Lock(&rect, WICBitmapLockRead.0 as u32).ok()?;
        let stride = pixels.GetStride().ok()? as usize;
        let mut len = 0u32;
        let mut data = std::ptr::null_mut();
        pixels.GetDataPointer(&mut len, &mut data).ok()?;
        if data.is_null() || (len as usize) < stride * (h as usize - 1) + w as usize * 4 {
            return None;
        }
        let src = std::slice::from_raw_parts(data, len as usize);
        let (w, h) = (w as usize, h as usize);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in src.chunks(stride).take(h) {
            for px in row[..w * 4].chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        }
        Some(Frame { size: [w, h], rgba })
    }
}

/// `75.4` -> `1:15`.
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_reads_like_a_player() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(75.4), "1:15");
        assert_eq!(clock(-3.0), "0:00");
    }

    /// Plays a local video for two seconds and checks frames come out.
    /// Run with `RUSTCLEANUP_TEST_VIDEO=<path to an .mp4>`.
    #[test]
    #[ignore]
    fn plays_a_local_video() {
        let path = PathBuf::from(std::env::var("RUSTCLEANUP_TEST_VIDEO").expect("set the path"));
        let shared = Arc::new(Shared {
            status: Mutex::new(Status {
                phase: Phase::Loading,
                position: 0.0,
                duration: 0.0,
                muted: true,
                volume: 1.0,
            }),
            frame: Mutex::new(None),
            stop: AtomicBool::new(false),
        });
        let (tx, rx) = channel();
        let worker = shared.clone();
        let handle = std::thread::spawn(move || play(&path, &worker, &rx, &|| {}));
        let mut frames = 0;
        let mut size = [0, 0];
        for _ in 0..200 {
            std::thread::sleep(Duration::from_millis(10));
            if let Some(f) = lock(&shared.frame).take() {
                frames += 1;
                size = f.size;
                assert_eq!(f.rgba.len(), size[0] * size[1] * 4);
            }
        }
        tx.send(Command::Seek(5.0)).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let status = lock(&shared.status).clone();
        shared.stop.store(true, Ordering::Relaxed);
        handle.join().unwrap().unwrap();
        println!("{frames} frames of {size:?}, {status:?}");
        assert!(frames > 10, "only {frames} frames");
        assert!(status.duration > 1.0 && status.position >= 5.0);
    }
}
