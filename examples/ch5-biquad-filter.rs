// This code is derived from these examples on RustAudio:
//
// - https://github.com/RustAudio/dasp/blob/master/examples/synth.rs
// - https://github.com/RustAudio/cpal/blob/master/examples/record_wav.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use dasp::{signal, Sample, Signal};
use std::sync::mpsc;

const ATTACK: usize = 1000;
const RELEASE: usize = 1000;

#[rustfmt::skip]
const SEQ: [bool; 8] = [true; 8];

struct Env {
    seq: Vec<bool>,
    cur_frame: usize,
    note_on: bool,
    step_length: usize,
    attack_frames: usize,
    release_frames: usize,
}

impl Env {
    fn new(
        mut seq: Vec<bool>,
        step_length: usize,
        attack_frames: usize,
        release_frames: usize,
    ) -> Self {
        let note_on = seq.pop().unwrap_or(false);
        Self {
            seq,
            cur_frame: 0,
            note_on,
            step_length,
            attack_frames,
            release_frames,
        }
    }
}

impl Signal for Env {
    type Frame = f64;

    fn next(&mut self) -> Self::Frame {
        self.cur_frame += 1;

        // proceed to the next step
        if self.cur_frame > self.step_length {
            self.cur_frame -= self.step_length;
            self.note_on = self.seq.pop().unwrap_or(false);
        }

        if !self.note_on {
            return 0.0;
        }

        // release phase
        if self.cur_frame > self.step_length - self.release_frames {
            return (self.step_length - self.cur_frame) as f64 / self.release_frames as f64;
        }

        // attack phase
        if self.cur_frame <= self.attack_frames {
            return self.cur_frame as f64 / self.attack_frames as f64;
        }

        // sustain phase
        1.0
    }
}

struct Lpf<S: Signal<Frame = f64>> {
    signal: S,
    fs: f64, // sampling rate
    fc: f64,
    q: f64,
    before: dasp::ring_buffer::Fixed<[f64; 2]>,
    after: dasp::ring_buffer::Fixed<[f64; 2]>,
}

impl<S: Signal<Frame = f64>> Lpf<S> {
    fn new(signal: S, fs: f64, fc: f64, q: f64) -> Self {
        println!("central frequency: {fc}");
        println!("Q: {q}");

        Self {
            signal,
            fs,
            fc,
            q,
            before: dasp::ring_buffer::Fixed::from([0.0; 2]),
            after: dasp::ring_buffer::Fixed::from([0.0; 2]),
        }
    }
}

impl<S: Signal<Frame = f64>> Signal for Lpf<S> {
    type Frame = f64;

    // c.f. https://webaudio.github.io/Audio-EQ-Cookbook/audio-eq-cookbook.html
    fn next(&mut self) -> Self::Frame {
        let orig = self.signal.next();

        let pi = std::f64::consts::PI as Self::Frame;
        let omega0 = 2.0 * pi * self.fc / self.fs;
        let alpha = omega0.sin() / 2.0 / self.q;

        // Since `push()` pushes on to the back of the queue,
        //
        //   - before[1]: input of 1-step before
        //   - before[0]: input of 2-step before
        //   - after[1]:  output of 1-step before
        //   - after[0]:  output of 2-step before
        //
        let mut out = (1.0 - omega0.cos()) / 2.0 * orig
            + (1.0 - omega0.cos()) * self.before[1]
            + (1.0 - omega0.cos()) / 2.0 * self.before[0]
            - (-2.0 * omega0.cos()) * self.after[1]
            - (1.0 - alpha) * self.after[0];
        out /= 1.0 + alpha;

        self.before.push(orig);
        self.after.push(out);

        out
    }
}

fn main() -> Result<(), anyhow::Error> {
    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();
    let config = device.default_output_config()?;

    println!("host: {}", host.id().name());

    match config.sample_format() {
        cpal::SampleFormat::F32 => run::<f32>(&device, &config.into())?,
        cpal::SampleFormat::I16 => run::<i16>(&device, &config.into())?,
        cpal::SampleFormat::U16 => run::<u16>(&device, &config.into())?,
    }

    Ok(())
}

fn run<T>(device: &cpal::Device, config: &cpal::StreamConfig) -> Result<(), anyhow::Error>
where
    T: cpal::Sample,
{
    println!("sample rate: {}", config.sample_rate.0);
    println!("channels: {}", config.channels);

    let square = signal::rate(config.sample_rate.0 as f64)
        .const_hz(500.0)
        .square();

    let step_length = config.sample_rate.0 as usize;

    let env = Env::new(SEQ.to_vec(), step_length, ATTACK, RELEASE);

    // taking the same number of samples as the sample rate = 1 second
    let mut frames = Lpf::new(
        square,
        config.sample_rate.0 as _,
        500.0,
        std::f64::consts::FRAC_1_SQRT_2,
    )
    .mul_amp(env)
    .take(step_length * SEQ.len())
    // To prevent click noise at the end, fill some silence
    .chain(signal::equilibrium().take(1000));

    let (complete_tx, complete_rx) = mpsc::sync_channel::<()>(1);

    let channels = config.channels as usize;
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_data(data, channels, &complete_tx, &mut frames);
        },
        |err| eprintln!("{err}"),
    )?;

    stream.play()?;

    complete_rx.recv().unwrap();
    stream.pause()?;

    Ok(())
}

fn write_data<T>(
    output: &mut [T],
    channels: usize,
    complete_rx: &mpsc::SyncSender<()>,
    frames: &mut dyn Iterator<Item = f64>,
) where
    T: cpal::Sample,
{
    for frame in output.chunks_mut(channels) {
        let sample = match frames.next() {
            Some(sample) => sample.to_sample::<f32>(),
            None => {
                complete_rx.try_send(()).ok();
                0.0
            }
        };
        let value: T = cpal::Sample::from::<f32>(&sample);
        for sample in frame.iter_mut() {
            *sample = value;
        }
    }
}
