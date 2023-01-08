// This code is derived from these examples on RustAudio:
//
// - https://github.com/RustAudio/dasp/blob/master/examples/synth.rs
// - https://github.com/RustAudio/cpal/blob/master/examples/record_wav.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use dasp::{
    signal::{self, Phase, Step},
    Sample, Signal,
};
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

pub struct PolyBlepSaw<S> {
    phase: Phase<S>,
    prev_phase: f64,
}

impl<S: Step> PolyBlepSaw<S> {
    fn new(phase: Phase<S>) -> Self {
        Self {
            phase,
            // TODO: The initial phase is not always 0.0?
            prev_phase: 0.0,
        }
    }
}

// This implementation is derived from https://github.com/electro-smith/DaisySP/blob/master/Source/Synthesis/oscillator.cpp
impl<S: Step> Signal for PolyBlepSaw<S> {
    type Frame = f64;

    fn next(&mut self) -> Self::Frame {
        let phase = self.phase.next_phase();
        let mut out = phase * -2.0 + 1.0;

        let delta = if phase > self.prev_phase {
            phase - self.prev_phase
        } else {
            // if the phase decreased, it should be because the phase got wrapped at 1.0.
            1.0 + phase - self.prev_phase
        };

        if phase < delta {
            let t = phase / delta;
            out += -t * t + 2.0 * t - 1.0;
        } else if phase > 1.0 - delta {
            let t = (phase - 1.0) / delta;
            out += t * t + 2.0 * t + 1.0;
        }

        self.prev_phase = phase;

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

    let hz = signal::rate(config.sample_rate.0 as f64).const_hz(220.0);
    let saw = PolyBlepSaw::new(hz.phase());

    let step_length = config.sample_rate.0 as usize;

    let env = Env::new(SEQ.to_vec(), step_length, ATTACK, RELEASE);

    // taking the same number of samples as the sample rate = 1 second
    let mut frames = saw
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
