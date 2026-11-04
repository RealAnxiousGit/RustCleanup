//! The What's New video player, on FFmpeg: `ffprobe` reads the size and
//! length, `ffmpeg` decodes frames to RGBA through a pipe, and `ffplay`
//! (without a window) plays the sound. Pausing and seeking restart the
//! helpers at the right position; volume and mute change the sound as it
//! plays (see [`Sound`]).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::os::unix::process::CommandExt;
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
    /// 0.0 to 1.0.
    pub volume: f32,
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
        let _ = self.commands.send(PlayerCommand::Play);
    }

    pub fn pause(&self) {
        let _ = self.commands.send(PlayerCommand::Pause);
    }

    pub fn seek(&self, seconds: f64) {
        lock(&self.shared.status).position = seconds;
        let _ = self.commands.send(PlayerCommand::Seek(seconds));
    }

    /// The sound thread reads mute and volume from the status as it plays.
    pub fn set_muted(&self, muted: bool) {
        lock(&self.shared.status).muted = muted;
    }

    /// `volume` from 0.0 (silent) to 1.0 (full).
    pub fn set_volume(&self, volume: f32) {
        lock(&self.shared.status).volume = volume.clamp(0.0, 1.0);
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
    audio: Option<Sound>,
    /// When `start_pos` was on screen.
    started: Instant,
    start_pos: f64,
    frames: u64,
}

impl Helpers {
    /// `shared` is `None` for a silent decoder, such as the still frame
    /// shown after seeking while paused.
    fn start(path: &Path, info: &Info, pos: f64, shared: Option<&Arc<Shared>>) -> Result<Self, String> {
        let video = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-ss", &format!("{pos:.3}"), "-i"])
            .arg(path)
            .args(["-an", "-vf", &format!("scale={}:{}", info.width, info.height)])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .with_parent_death()
            .spawn()
            .map_err(|e| trf("Could not play the video: {0}", &[&e]))?;
        let started = Instant::now();
        let audio = shared.and_then(|s| Sound::start(path, pos, started, s.clone()));
        Ok(Self {
            video,
            audio,
            started,
            start_pos: pos,
            frames: 0,
        })
    }
}

/// The sound: `ffmpeg` decodes it to WAV, a thread scales each sample by
/// the volume (zero when muted) and `ffplay` plays the result from its
/// input. Changing the volume changes the samples on the way, so the sound
/// keeps playing instead of starting again. ffplay itself only takes a
/// volume when it starts.
struct Sound {
    decoder: Child,
    output: Child,
}

impl Sound {
    fn start(path: &Path, pos: f64, started: Instant, shared: Arc<Shared>) -> Option<Self> {
        which("ffplay")?;
        let mut decoder = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-ss", &format!("{pos:.3}"), "-i"])
            .arg(path)
            .args(["-vn", "-ac", "2", "-ar", "48000", "-c:a", "pcm_s16le", "-f", "wav", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .with_parent_death()
            .spawn()
            .ok()?;
        // Without the probe limits ffplay waits for about 4 seconds of
        // sound before it starts playing.
        let output = Command::new("ffplay")
            .args(["-v", "quiet", "-nodisp", "-autoexit", "-probesize", "32", "-analyzeduration", "0", "-i", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .with_parent_death()
            .spawn();
        let mut output = match output {
            Ok(o) => o,
            Err(_) => {
                let _ = decoder.kill();
                let _ = decoder.wait();
                return None;
            }
        };
        let (input, sink) = (decoder.stdout.take()?, output.stdin.take()?);
        std::thread::spawn(move || pump(input, sink, started, &shared));
        Some(Self { decoder, output })
    }
}

impl Drop for Sound {
    fn drop(&mut self) {
        // The pump thread then sees its pipes close and ends.
        for child in [&mut self.decoder, &mut self.output] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Copies the WAV from the decoder to ffplay, scaling the samples by the
/// volume. It stays only a little ahead of the video's clock, so a change
/// is heard within a fraction of a second, and it ramps between two levels
/// so a change does not click.
fn pump(mut input: impl Read, mut output: impl Write, started: Instant, shared: &Shared) {
    /// Bytes per second: 48 kHz, two channels, 16 bits.
    const RATE: f64 = 48_000.0 * 4.0;
    /// How far ahead of the clock the sound is written.
    const LEAD: f64 = 0.1;

    // The header goes through as it is: "RIFF", then chunks up to "data".
    let mut riff = [0u8; 12];
    if input.read_exact(&mut riff).is_err() || output.write_all(&riff).is_err() {
        return;
    }
    loop {
        let mut head = [0u8; 8];
        if input.read_exact(&mut head).is_err() || output.write_all(&head).is_err() {
            return;
        }
        if &head[..4] == b"data" {
            break;
        }
        let size = u64::from(u32::from_le_bytes([head[4], head[5], head[6], head[7]]));
        let size = size + (size & 1);
        if std::io::copy(&mut (&mut input).take(size), &mut output).is_err() {
            return;
        }
    }

    let gain = || {
        let s = lock(&shared.status);
        if s.muted { 0.0 } else { s.volume }
    };
    let mut last = gain();
    let mut buf = [0u8; 4096];
    let mut written = 0u64;
    while !shared.stop.load(Ordering::Relaxed) {
        let mut n = 0;
        while n < buf.len() {
            match input.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let n = n - n % 4;
        if n == 0 {
            return;
        }
        let next = gain();
        let frames = (n / 4) as f32;
        for (i, frame) in buf[..n].as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let g = last + (next - last) * (i + 1) as f32 / frames;
            for s in frame.as_chunks_mut::<2>().0 {
                let v = f32::from(i16::from_le_bytes(*s)) * g;
                *s = (v.round() as i16).to_le_bytes();
            }
        }
        last = next;
        if output.write_all(&buf[..n]).is_err() {
            return;
        }
        written += n as u64;
        let ahead = written as f64 / RATE - started.elapsed().as_secs_f64();
        if ahead > LEAD {
            std::thread::sleep(Duration::from_secs_f64(ahead - LEAD));
        }
    }
}

trait ParentDeath {
    fn with_parent_death(&mut self) -> &mut Self;
}

impl ParentDeath for Command {
    /// The helper is killed when the player's thread ends, even when
    /// RustCleanup quits without running destructors (tray Exit calls
    /// `process::exit`) or is killed. Otherwise ffplay kept playing the sound.
    fn with_parent_death(&mut self) -> &mut Self {
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            self.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // The parent may have died before prctl took effect.
                if libc::getppid() == 1 {
                    libc::_exit(0);
                }
                Ok(())
            })
        }
    }
}

impl Drop for Helpers {
    fn drop(&mut self) {
        self.audio = None;
        let _ = self.video.kill();
        let _ = self.video.wait();
    }
}

/// Plays `path` until the player is dropped.
fn play(
    path: &Path,
    shared: &Arc<Shared>,
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

    let mut helpers = Some(Helpers::start(path, &info, 0.0, Some(shared))?);
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
                        if let Ok(mut h) = Helpers::start(path, &info, position, None)
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
            }
        }
        if let Some(pos) = restart_at {
            // Stop the old helpers before starting new ones.
            drop(helpers.take());
            helpers = Some(Helpers::start(path, &info, pos, Some(shared))?);
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
