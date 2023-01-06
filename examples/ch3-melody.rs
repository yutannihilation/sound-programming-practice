// This code is derived from these examples on RustAudio:
//
// - https://github.com/RustAudio/dasp/blob/master/examples/synth.rs
// - https://github.com/RustAudio/cpal/blob/master/examples/record_wav.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use dasp::{
    signal::{self, from_iter},
    Sample, Signal,
};
use std::sync::mpsc;

#[rustfmt::skip]
const TRACK1: [f64; 8] = [659.26, 587.33, 523.25, 493.88, 440.00, 392.00, 440.00, 493.88];
#[rustfmt::skip]
const TRACK2: [f64; 8] = [261.63, 196.00, 220.00, 164.81, 174.61, 130.81, 174.61, 196.00];

const ATTACK: usize = 1000;
const RELEASE: usize = 1000;

struct Env {
    cur_frame: usize,
    total_frames: usize,
    attack_frames: usize,
    release_frames: usize,
}

impl Env {
    fn new(total_frames: usize, attack_frames: usize, release_frames: usize) -> Self {
        Self {
            cur_frame: 0,
            total_frames,
            attack_frames,
            release_frames,
        }
    }
}

impl Iterator for Env {
    type Item = f64;

    fn next(&mut self) -> Option<Self::Item> {
        self.cur_frame += 1;

        // already ended
        if self.cur_frame > self.total_frames {
            return None;
        }

        // release phase
        if self.cur_frame > self.total_frames - self.release_frames {
            return Some((self.total_frames - self.cur_frame) as f64 / self.release_frames as f64);
        }

        // attack phase
        if self.cur_frame <= self.attack_frames {
            return Some(self.cur_frame as f64 / self.attack_frames as f64);
        }

        // sustain phase
        Some(1.0)
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

    let total_frames = config.sample_rate.0 as usize;

    let f = |f| {
        let env = signal::from_iter(Env::new(total_frames, ATTACK, RELEASE));
        signal::rate(config.sample_rate.0 as f64)
            .const_hz(f)
            .sine()
            .mul_amp(env)
            .take(total_frames)
    };

    let track1 = signal::from_iter(TRACK1.map(f).into_iter().flatten());
    let track2 = signal::from_iter(TRACK2.map(f).into_iter().flatten());

    let mut frames = track1
        .add_amp(track2)
        .until_exhausted()
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
