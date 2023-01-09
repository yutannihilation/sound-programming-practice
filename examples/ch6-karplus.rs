// This code is derived from these examples on RustAudio:
//
// - https://github.com/RustAudio/dasp/blob/master/examples/synth.rs
// - https://github.com/RustAudio/cpal/blob/master/examples/record_wav.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use dasp::{
    signal::{self, Noise},
    Sample, Signal,
};
use std::sync::mpsc;

const SEED: u64 = 1234;

#[rustfmt::skip]
const SEQ: [bool; 8] = [true; 8];

struct KarplusStrong {
    cur_frame: usize,
    noise_source: Noise,
    fs: f64, // sampling rate
    g: f64,
    c: f64,
    d: f64,
    delay_line_length: usize,
    delay_line: dasp::ring_buffer::Bounded<[f64; 1024]>,
    last_delayed_sample: f64,
    last_all_passed_feedback: f64,
}

impl KarplusStrong {
    fn new(fs: f64, f0: f64, d: f64, decay: f64) -> Self {
        println!("central frequency: {f0}");

        let num = 10.0_f64.powf(-3.0 / f0 / decay);
        let den = ((1.0 - d) * (1.0 - d)
            + 2.0 * d * (1.0 - d) * (2.0 * std::f64::consts::PI * f0 / fs).cos())
        .sqrt();
        let c = (num / den).clamp(0.0, 1.0);

        let delay = fs / f0 - d;
        let delay_line_length = delay.floor() as usize + 1;
        let e = delay.fract();
        let g = (1.0 - e) / (1.0 + e);

        println!("delay line length: {delay_line_length}");
        let delay_line =
            dasp::ring_buffer::Bounded::from_raw_parts(0, delay_line_length, [0.0; 1024]);

        Self {
            cur_frame: 0,
            noise_source: signal::noise(SEED),
            fs,
            g,
            c,
            d,
            delay_line_length,
            delay_line,
            last_delayed_sample: 0.0,
            last_all_passed_feedback: 0.0,
        }
    }
}

impl Signal for KarplusStrong {
    type Frame = f64;

    // c.f. https://webaudio.github.io/Audio-EQ-Cookbook/audio-eq-cookbook.html
    fn next(&mut self) -> Self::Frame {
        self.cur_frame += 1;

        let cur_delayed_sample = self.delay_line.pop().unwrap_or(0.0);

        let all_passed_feedback = -self.g * self.last_all_passed_feedback
            + self.g * cur_delayed_sample
            + self.last_delayed_sample;

        // trigger once per second with the same lenght as the delay line
        let orig_noise = if self.cur_frame % (self.fs as usize) < self.delay_line_length {
            self.noise_source.next_sample()
        } else {
            0.0
        };

        let out = orig_noise
            + self.c
                * ((1.0 - self.d) * all_passed_feedback + self.d * self.last_all_passed_feedback);

        self.last_all_passed_feedback = all_passed_feedback;
        self.last_delayed_sample = cur_delayed_sample;
        self.delay_line.push(out);

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

    let step_length = config.sample_rate.0 as usize;

    // taking the same number of samples as the sample rate = 1 second
    let mut frames = KarplusStrong::new(step_length as _, 220.0, 0.05, 2.0)
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
