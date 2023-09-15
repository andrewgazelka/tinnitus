//! Make some noise via cpal.
#![allow(clippy::precedence)]

use std::thread;

use assert_no_alloc::*;
use cpal::{
    FromSample,
    SizedSample, traits::{DeviceTrait, HostTrait, StreamTrait},
};
use fundsp::hacker::*;

fn main() {
    let host = cpal::default_host();

    let device = host
        .default_output_device()
        .expect("Failed to find a default output device");
    let config = device.default_output_config().unwrap();

    match config.sample_format() {
        cpal::SampleFormat::F32 => run::<f32>(&device, &config.into()).unwrap(),
        cpal::SampleFormat::I16 => run::<i16>(&device, &config.into()).unwrap(),
        cpal::SampleFormat::U16 => run::<u16>(&device, &config.into()).unwrap(),
        _ => panic!("Unsupported format"),
    }
}

fn run<T>(device: &cpal::Device, config: &cpal::StreamConfig) -> Result<(), anyhow::Error>
where
    T: SizedSample + FromSample<f64>,
{
    let sample_rate = config.sample_rate.0 as f64;
    let channels = config.channels as usize;

    let hz = 2110.0 / 1.0;
    let mut c = saw_hz(hz) * 0.02 + sine_hz(hz);

    c.set_sample_rate(sample_rate);
    c.allocate();

    let mut next_value = move || assert_no_alloc(|| c.get_stereo());

    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_data(data, channels, &mut next_value);
        },
        err_fn,
        None,
    )?;
    stream.play()?;

    let handle = thread::current();

    ctrlc::set_handler(move || {
        println!("Received Ctrl-C, exiting...");
        handle.unpark();
    })
    .expect("Error setting Ctrl-C handler");

    println!("Waiting for Ctrl-C...");
    thread::park();
    println!("Exited successfully.");

    Ok(())
}

fn write_data<T>(output: &mut [T], channels: usize, next_sample: &mut dyn FnMut() -> (f64, f64))
where
    T: SizedSample + FromSample<f64>,
{
    for frame in output.chunks_mut(channels) {
        let sample = next_sample();
        let left = T::from_sample(sample.0);
        let right: T = T::from_sample(sample.1);

        for (channel, sample) in frame.iter_mut().enumerate() {
            if channel & 1 == 0 {
                *sample = left;
            } else {
                *sample = right;
            }
        }
    }
}
