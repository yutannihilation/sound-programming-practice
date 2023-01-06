// This code is derived from these examples on RustAudio:
//
// - https://github.com/RustAudio/dasp/blob/master/examples/synth.rs
// - https://github.com/RustAudio/cpal/blob/master/examples/record_wav.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use dasp::{signal, Sample, Signal};
use std::sync::mpsc;

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

    let sine = signal::rate(config.sample_rate.0 as f64)
        .const_hz(440.0)
        .sine();

    let total_frames = config.sample_rate.0 as usize;

    let env = signal::from_iter(Env::new(total_frames, ATTACK, RELEASE));

    // taking the same number of samples as the sample rate = 1 second
    let mut frames = sine.mul_amp(env).take(total_frames + 1000);

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
