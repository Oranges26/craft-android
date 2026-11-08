//! Program monitor and playback: a worker thread renders the requested frame of the sequence with
//! upstream's renderer (from a snapshot of the project, so editing never waits for it), and the
//! sound plays through cpal (AAudio on Android) from upstream's mixer, mixed ahead on its own
//! thread. While the sound plays, its clock drives the playhead.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use filmcraft_engine::Session;
use filmcraft_project::{ItemId, Project};
use filmcraft_time::Tick;

// ---------------------------------------------------------------------------------------------
// frames

pub struct FrameJob {
    pub project: Arc<Project>,
    pub pool: Arc<filmcraft_engine::media_pool::MediaPool>,
    pub services: Arc<dyn filmcraft_engine::Services>,
    pub seq: ItemId,
    pub t: Tick,
    pub scale: f32,
}

pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

pub struct FrameWorker {
    tx: Sender<FrameJob>,
    pub rx: Receiver<Frame>,
    last: Option<(Tick, u64, u32)>,
}

impl FrameWorker {
    pub fn new(ctx: egui::Context) -> FrameWorker {
        let (tx, jobs) = channel::<FrameJob>();
        let (done, rx) = channel::<Frame>();
        std::thread::Builder::new()
            .name("fc-frames".into())
            .spawn(move || {
                while let Ok(mut job) = jobs.recv() {
                    while let Ok(next) = jobs.try_recv() {
                        job = next;
                    }
                    let provider = job.pool.provider(job.project.clone(), job.services.clone());
                    let opts = filmcraft_render::RenderOptions { scale: job.scale, captions: true, ..Default::default() };
                    let (w, h, rgba) = filmcraft_render::render_sequence_rgba8(&job.project, job.seq, job.t, opts, &provider);
                    if done.send(Frame { w, h, rgba }).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("frame thread");
        FrameWorker { tx, rx, last: None }
    }

    /// Ask for the frame at `t` unless that exact frame is already on its way.
    pub fn request(&mut self, s: &Session, seq: ItemId, t: Tick, scale: f32) {
        let key = (t, s.revision, (scale * 1000.0) as u32);
        if self.last == Some(key) {
            return;
        }
        self.last = Some(key);
        let _ = self.tx.send(FrameJob {
            project: s.project.clone(),
            pool: s.media.clone(),
            services: s.services.clone(),
            seq,
            t,
            scale,
        });
    }

    pub fn forget(&mut self) {
        self.last = None;
    }
}

// ---------------------------------------------------------------------------------------------
// sound

type Mix = Box<dyn FnMut(i64, &mut [f32], usize) + Send>;

/// The sequence's sound at `device_rate`, interleaved into `buf` (`ch` channels) from `cursor`
/// (in device frames). Mixed by upstream's preview mixer, resampled when the sequence's rate is
/// not the device's.
pub fn sequence_mix(s: &Session, seq: ItemId, device_rate: u32) -> Mix {
    let project = s.project.clone();
    let provider = s.media.provider(project.clone(), s.services.clone());
    let previews = s.previews.clone();
    let mut resampler: Option<(u32, filmcraft_audio_dsp::resample::StreamResampler)> = None;
    Box::new(move |cursor: i64, buf: &mut [f32], ch: usize| {
        let ch = ch.max(1);
        let n = buf.len() / ch;
        let Some(q) = project.sequence(seq) else {
            buf.fill(0.0);
            return;
        };
        let seq_sr = q.settings.sample_rate;
        let planes = if seq_sr == device_rate {
            previews.mix(&project, seq, cursor, n, &provider).channels
        } else {
            if resampler.as_ref().is_none_or(|(r, _)| *r != seq_sr) {
                resampler = Some((seq_sr, filmcraft_audio_dsp::resample::StreamResampler::new(seq_sr, device_rate)));
            }
            let (_, r) = resampler.as_mut().expect("resampler");
            r.process(cursor, n, |s0, m| previews.mix(&project, seq, s0, m, &provider).channels)
        };
        for (i, frame) in buf.chunks_exact_mut(ch).enumerate() {
            for (c, out) in frame.iter_mut().enumerate() {
                let plane = planes.get(c.min(planes.len().saturating_sub(1)));
                *out = plane.and_then(|p| p.get(i)).copied().filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-1.0, 1.0);
            }
        }
    })
}

/// Sound output while playing. Dropped = silence.
pub struct Sound {
    _stream: cpal::Stream,
    stop: Arc<AtomicBool>,
    /// Frames the device has played (the playback clock).
    played: Arc<AtomicU64>,
    pub rate: u32,
}

impl Drop for Sound {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Sound {
    /// Start playing the sequence from `start` (sequence time).
    pub fn start(s: &Session, seq: ItemId, start: Tick) -> Result<Sound, String> {
        let host = cpal::default_host();
        let dev = host.default_output_device().ok_or("no audio output")?;
        let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
        let rate = cfg.sample_rate().0;
        let ch = cfg.channels() as usize;
        let mut mix = sequence_mix(s, seq, rate);
        let queue: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::with_capacity(rate as usize * ch)));
        let stop = Arc::new(AtomicBool::new(false));
        let played = Arc::new(AtomicU64::new(0));

        // mix ahead (~0.35 s) so the device callback never waits for decoding
        let (q, st) = (queue.clone(), stop.clone());
        let mut cursor = (start.seconds() * f64::from(rate)).round() as i64;
        let ahead = (rate as usize * ch * 35) / 100;
        std::thread::Builder::new()
            .name("fc-mix".into())
            .spawn(move || {
                let block = 1024usize;
                let mut buf = vec![0.0f32; block * ch];
                while !st.load(Ordering::Relaxed) {
                    let len = q.lock().map(|g| g.len()).unwrap_or(0);
                    if len >= ahead {
                        std::thread::sleep(std::time::Duration::from_millis(4));
                        continue;
                    }
                    mix(cursor, &mut buf, ch);
                    cursor += block as i64;
                    if let Ok(mut g) = q.lock() {
                        g.extend(buf.iter().copied());
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        let (q, p) = (queue.clone(), played.clone());
        let err = |e: cpal::StreamError| log::warn!("audio stream: {e}");
        let config: cpal::StreamConfig = cfg.clone().into();
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => dev.build_output_stream(
                &config,
                move |out: &mut [f32], _| {
                    let mut got = 0;
                    if let Ok(mut g) = q.try_lock() {
                        for o in out.iter_mut() {
                            match g.pop_front() {
                                Some(v) => {
                                    *o = v;
                                    got += 1;
                                }
                                None => *o = 0.0,
                            }
                        }
                    } else {
                        out.fill(0.0);
                    }
                    p.fetch_add((got / ch) as u64, Ordering::Relaxed);
                },
                err,
                None,
            ),
            cpal::SampleFormat::I16 => dev.build_output_stream(
                &config,
                move |out: &mut [i16], _| {
                    let mut got = 0;
                    if let Ok(mut g) = q.try_lock() {
                        for o in out.iter_mut() {
                            match g.pop_front() {
                                Some(v) => {
                                    *o = (v * 32767.0) as i16;
                                    got += 1;
                                }
                                None => *o = 0,
                            }
                        }
                    } else {
                        out.fill(0);
                    }
                    p.fetch_add((got / ch) as u64, Ordering::Relaxed);
                },
                err,
                None,
            ),
            f => return Err(format!("unsupported sample format {f:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Sound { _stream: stream, stop, played, rate })
    }

    /// Seconds of sound played since `start`.
    pub fn elapsed(&self) -> f64 {
        self.played.load(Ordering::Relaxed) as f64 / f64::from(self.rate.max(1))
    }
}

/// Playback state: where it started and which clock drives it.
pub struct Playing {
    pub from: Tick,
    pub wall: std::time::Instant,
    pub sound: Option<Sound>,
}

impl Playing {
    pub fn now(&self) -> Tick {
        let secs = match &self.sound {
            // the sound clock, once the device has started consuming
            Some(s) if s.elapsed() > 0.0 => s.elapsed(),
            // a device that has not started within half a second: the wall clock takes over
            Some(_) if self.wall.elapsed().as_secs_f64() < 0.5 => 0.0,
            _ => self.wall.elapsed().as_secs_f64(),
        };
        Tick::from_seconds_f64(self.from.seconds() + secs)
    }
}
