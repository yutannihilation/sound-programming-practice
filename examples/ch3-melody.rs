// This code is derived from these examples on RustAudio:
//
// - https://github.com/RustAudio/dasp/blob/master/examples/synth.rs
// - https://github.com/RustAudio/cpal/blob/master/examples/record_wav.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use dasp::{signal, Sample, Signal};
use std::sync::mpsc;

#[rustfmt::skip]
const SEQ: [bool; 8] = [true; 8];
#[rustfmt::skip]
const TRACK1: [f64; 8] = [659.26, 587.33, 523.25, 493.88, 440.00, 392.00, 440.00, 493.88];
#[rustfmt::skip]
const TRACK2: [f64; 8] = [261.63, 196.00, 220.00, 164.81, 174.61, 130.81, 174.61, 196.00];

const ATTACK: usize = 1000;
const RELEASE: usize = 1000;

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
        seq: Vec<bool>,
        step_length: usize,
        attack_frames: usize,
        release_frames: usize,
    ) -> Self {
        Self {
            seq,
            cur_frame: 0,
            note_on: false,
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

struct Track {
    seq: Vec<f64>,
    step_length: usize,
    cur_frame: usize,
    note: f64,
}

impl Track {
    fn new(seq: Vec<f64>, step_length: usize) -> Self {
        Self {
            seq,
            step_length,
            cur_frame: 0,
            note: 0.0,
        }
    }
}

impl Signal for Track {
    type Frame = f64;

    fn next(&mut self) -> Self::Frame {
        self.cur_frame += 1;

        // proceed to the next step
        if self.cur_frame > self.step_length {
            self.cur_frame -= self.step_length;
            self.note = self.seq.pop().unwrap_or(0.0);
            println!("note: {}", self.note);
        }

        self.note
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

    let step_length = config.sample_rate.0 as usize;

    let track1 = signal::rate(config.sample_rate.0 as f64)
        .hz(Track::new(TRACK1.to_vec(), step_length))
        .sine();

    let track2 = signal::rate(config.sample_rate.0 as f64)
        .hz(Track::new(TRACK2.to_vec(), step_length))
        .sine();

    let env = Env::new(SEQ.to_vec(), step_length, ATTACK, RELEASE);

    let mut frames = track1
        .add_amp(track2)
        .mul_amp(env)
        .take(step_length * SEQ.len())
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
