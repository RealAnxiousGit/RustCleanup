//! The What's New video player, on FFmpeg: `ffprobe` reads the size and
//! length, `ffmpeg` decodes frames to RGBA through a pipe, and `ffplay`
//! (without a window) plays the sound. Pausing, seeking and muting restart
//! the helpers at the right position.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::command::which;
use crate::i18n::{tr, trf};

/// Frames wider than this are scaled down; the Changelog window is narrower.
const MAX_WIDTH: usize = 1280;

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
}

/// One decoded picture, as RGBA.
pub struct Frame {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

enum PlayerCommand {
    Play,
    Pause,
    Seek(f64),
    Mute(bool),
}

struct Shared {
    status: Mutex<Status>,
    frame: Mutex<Option<Frame>>,
    stop: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A video that is playing (or getting ready to). Dropping it stops playback.
pub struct Player {
    commands: Sender<PlayerCommand>,
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
        let _ = self.commands.send(PlayerCommand::Play);
    }

    pub fn pause(&self) {
        let _ = self.commands.send(PlayerCommand::Pause);
    }

    pub fn seek(&self, seconds: f64) {
        lock(&self.shared.status).position = seconds;
        let _ = self.commands.send(PlayerCommand::Seek(seconds));
    }

    pub fn set_muted(&self, muted: bool) {
        lock(&self.shared.status).muted = muted;
        let _ = self.commands.send(PlayerCommand::Mute(muted));
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

/// The video in the cache folder, downloaded first if it is not there yet.
fn cached_file(url: &str, shared: &Shared, repaint: &dyn Fn()) -> Result<PathBuf, String> {
    let name = url
        .rsplit('/')
        .next()
        .filter(|n| n.ends_with(".mp4") && !n.contains(['\\', ':', '?']))
        .ok_or_else(|| trf("{0} is not a video RustCleanup can play.", &[&url]))?;
    let dir = std::env::temp_dir().join(format!("RustCleanup-{}", super::user::desktop_user().uid));
    let path = dir.join(name);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 0) {
        return Ok(path);
    }
    let last = AtomicU8::new(0);
    let bytes = crate::sys::updater::download(url, |done, total| {
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

/// Size, frame rate and length from `ffprobe`.
struct Info {
    width: usize,
    height: usize,
    fps: f64,
    duration: f64,
}

fn probe(path: &Path) -> Result<Info, String> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .arg("stream=width,height,avg_frame_rate:format=duration")
        .args(["-of", "default=noprint_wrappers=1"])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let value = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(str::trim)
    };
    let num = |key: &str| value(key).and_then(|v| v.parse::<f64>().ok());
    let fps = value("avg_frame_rate")
        .and_then(|r| {
            let (a, b) = r.split_once('/')?;
            let (a, b) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
            (b > 0.0 && a > 0.0).then(|| a / b)
        })
        .unwrap_or(30.0);
    let (w, h) = (num("width").unwrap_or(0.0), num("height").unwrap_or(0.0));
    if w < 1.0 || h < 1.0 {
        return Err(tr("Could not play the video.").to_string());
    }
    // Scaled to at most MAX_WIDTH, even sizes for the scaler.
    let width = (w as usize).min(MAX_WIDTH) & !1;
    let height = ((h * width as f64 / w) as usize).max(2) & !1;
    Ok(Info {
        width,
        height,
        fps,
        duration: num("duration").unwrap_or(0.0),
    })
}

/// The running decoder (and sound) from one position.
struct Helpers {
    video: Child,
    audio: Option<Child>,
    /// When `start_pos` was on screen.
    started: Instant,
    start_pos: f64,
    frames: u64,
}

impl Helpers {
    fn start(path: &Path, info: &Info, pos: f64, muted: bool) -> Result<Self, String> {
        let video = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-ss", &format!("{pos:.3}"), "-i"])
            .arg(path)
            .args(["-an", "-vf", &format!("scale={}:{}", info.width, info.height)])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| trf("Could not play the video: {0}", &[&e]))?;
        let audio = if muted || which("ffplay").is_none() {
            None
        } else {
            Command::new("ffplay")
                .args(["-v", "quiet", "-nodisp", "-autoexit", "-vn", "-ss", &format!("{pos:.3}")])
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok()
        };
        Ok(Self {
            video,
            audio,
            started: Instant::now(),
            start_pos: pos,
            frames: 0,
        })
    }

    fn stop_audio(&mut self) {
        if let Some(mut a) = self.audio.take() {
            let _ = a.kill();
            let _ = a.wait();
        }
    }
}

impl Drop for Helpers {
    fn drop(&mut self) {
        self.stop_audio();
        let _ = self.video.kill();
        let _ = self.video.wait();
    }
}

/// Plays `path` until the player is dropped.
fn play(
    path: &Path,
    shared: &Shared,
    inbox: &Receiver<PlayerCommand>,
    repaint: &dyn Fn(),
) -> Result<(), String> {
    lock(&shared.status).phase = Phase::Loading;
    repaint();
    if which("ffmpeg").is_none() || which("ffprobe").is_none() {
        return Err(tr("The video needs FFmpeg. Install the ffmpeg package and try again.").to_string());
    }
    let info = probe(path)?;
    lock(&shared.status).duration = info.duration;
    let frame_len = info.width * info.height * 4;
    let frame_time = 1.0 / info.fps;

    let mut helpers = Some(Helpers::start(path, &info, 0.0, lock(&shared.status).muted)?);
    lock(&shared.status).phase = Phase::Playing;
    let mut playing = true;
    let mut position = 0.0f64;

    while !shared.stop.load(Ordering::Relaxed) {
        // Commands first; each one that moves playback restarts the helpers.
        let mut restart_at = None;
        while let Ok(cmd) = inbox.try_recv() {
            match cmd {
                PlayerCommand::Play => {
                    if !playing {
                        playing = true;
                        let ended = lock(&shared.status).phase == Phase::Ended;
                        restart_at = Some(if ended { 0.0 } else { position });
                    }
                }
                PlayerCommand::Pause => {
                    if playing {
                        playing = false;
                        helpers = None;
                        lock(&shared.status).phase = Phase::Paused;
                    }
                }
                PlayerCommand::Seek(s) => {
                    position = s.clamp(0.0, info.duration.max(0.0));
                    if playing {
                        restart_at = Some(position);
                    } else {
                        // Show the frame at the new spot while paused.
                        helpers = None;
                        if let Ok(mut h) = Helpers::start(path, &info, position, true)
                            && let Some(rgba) = read_frame(&mut h.video, frame_len)
                        {
                            *lock(&shared.frame) = Some(Frame {
                                size: [info.width, info.height],
                                rgba,
                            });
                        }
                    }
                    lock(&shared.status).position = position;
                    if lock(&shared.status).phase == Phase::Ended {
                        lock(&shared.status).phase = Phase::Paused;
                    }
                }
                PlayerCommand::Mute(m) => {
                    if playing {
                        if m {
                            if let Some(h) = helpers.as_mut() {
                                h.stop_audio();
                            }
                        } else {
                            restart_at = Some(position);
                        }
                    }
                }
            }
        }
        if let Some(pos) = restart_at {
            // Stop the old helpers before starting new ones.
            drop(helpers.take());
            helpers = Some(Helpers::start(path, &info, pos, lock(&shared.status).muted)?);
            position = pos;
            lock(&shared.status).phase = Phase::Playing;
        }

        let Some(h) = helpers.as_mut().filter(|_| playing) else {
            repaint();
            std::thread::sleep(Duration::from_millis(30));
            continue;
        };

        // Wait until this frame is due, then hand it over.
        let due = h.frames as f64 * frame_time;
        let elapsed = h.started.elapsed().as_secs_f64();
        if due > elapsed {
            std::thread::sleep(Duration::from_secs_f64((due - elapsed).min(0.05)));
            continue;
        }
        match read_frame(&mut h.video, frame_len) {
            Some(rgba) => {
                h.frames += 1;
                position = h.start_pos + due;
                *lock(&shared.frame) = Some(Frame {
                    size: [info.width, info.height],
                    rgba,
                });
                lock(&shared.status).position = position.min(info.duration);
                repaint();
            }
            None => {
                helpers = None;
                playing = false;
                let mut s = lock(&shared.status);
                s.phase = Phase::Ended;
                s.position = info.duration;
                drop(s);
                repaint();
            }
        }
    }
    Ok(())
}

fn read_frame(video: &mut Child, len: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; len];
    video.stdout.as_mut()?.read_exact(&mut buf).ok()?;
    Some(buf)
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
        tx.send(PlayerCommand::Seek(5.0)).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let status = lock(&shared.status).clone();
        shared.stop.store(true, Ordering::Relaxed);
        handle.join().unwrap().unwrap();
        println!("{frames} frames of {size:?}, {status:?}");
        assert!(frames > 10, "only {frames} frames");
        assert!(status.duration > 1.0 && status.position >= 5.0);
    }
}
