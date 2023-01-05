// This code is derived from these examples on RustAudio:
//
// - https://github.com/RustAudio/dasp/blob/master/examples/synth.rs
// - https://github.com/RustAudio/cpal/blob/master/examples/record_wav.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use dasp::{signal, Sample, Signal};
use std::sync::mpsc;

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
    let hz = signal::rate(config.sample_rate.0 as f64).const_hz(440.0);
    let one_sec = config.sample_rate.0 as usize;

    let channels = config.channels as usize;

    println!("sample rate: {}", config.sample_rate.0);
    println!("channels: {}", config.channels);

    let mut synth = hz.sine().take(one_sec);

    let (complete_tx, complete_rx) = mpsc::sync_channel::<()>(1);

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_data(data, channels, &complete_tx, &mut synth);
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
    signal: &mut dyn Iterator<Item = f64>,
) where
    T: cpal::Sample,
{
    for frame in output.chunks_mut(channels) {
        let sample = match signal.next() {
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
