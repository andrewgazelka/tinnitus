//! Make some noise
#![allow(clippy::precedence)]

extern crate core;

use std::{sync::Arc, thread, time::SystemTime};

use assert_no_alloc::assert_no_alloc;
use clap::Parser;
use cpal::{
    FromSample,
    SizedSample, traits::{DeviceTrait, HostTrait, StreamTrait},
};
use crossterm::{
    event::{Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use fundsp::hacker::{AudioUnit64, highpole_hz, lowpole_hz, sine_hz, white};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
enum Sound {
    Sin(f64),
    White,
    Brown,
    Pink,
    None,
}

#[derive(Default, Debug, Clone)]
enum Error {
    #[default]
    Default,
    Err(Arc<dyn std::error::Error + Sync + Send>),
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Error::Default, Error::Default) | (Error::Err(_), Error::Err(_))
        )
    }
}

// impl from
impl<E: std::error::Error + Send + Sync + 'static> From<E> for Error {
    fn from(err: E) -> Self {
        Error::Err(Arc::new(err))
    }
}


#[derive(Parser)]
#[clap(version, author, about)]
struct Args {
    /// frequency of the tinnitus
    frequency: f64,

    /// the frequency radius to notch out
    radius: f64,
}

fn main() {
    // input separated by spaces
    let args = Args::parse();

    enable_raw_mode().unwrap();

    let host = cpal::default_host();

    let device = host
        .default_output_device()
        .expect("Failed to find a default output device");
    let config = device.default_output_config().unwrap();

    match config.sample_format() {
        cpal::SampleFormat::F32 => run::<f32>(&device, &config.into(), args).unwrap(),
        cpal::SampleFormat::I16 => run::<i16>(&device, &config.into(), args).unwrap(),
        cpal::SampleFormat::U16 => run::<u16>(&device, &config.into(), args).unwrap(),
        _ => panic!("Unsupported format"),
    }
}

fn sin_audio(args: &Args) -> impl AudioUnit64 {
    let hz = args.frequency;

    sine_hz(hz) * 0.1
}

fn main_audio(args: &Args) -> impl AudioUnit64 {
    let hz = args.frequency;
    let width = args.radius;
    let min = hz - width;
    let max = hz + width;

    let white_low = white() >> lowpole_hz(min);
    let white_high = white() >> highpole_hz(max);

    (white_low + white_high) * 0.1
}

fn run<T>(device: &cpal::Device, config: &cpal::StreamConfig, args: Args) -> anyhow::Result<()>
where
    T: SizedSample + FromSample<f64>,
{
    let sample_rate = config.sample_rate.0 as f64;
    let channels = config.channels as usize;

    let mut sin = sin_audio(&args);
    sin.set_sample_rate(sample_rate);
    sin.allocate();

    let mut main = main_audio(&args);
    main.set_sample_rate(sample_rate);
    main.allocate();

    let mut start = None;

    let mut next_value_sin = move || {
        assert_no_alloc(|| {
            let start = start.get_or_insert_with(SystemTime::now);
            let time_passed = start.elapsed().unwrap().as_millis();

            if time_passed > 500 {
                main.get_stereo()
            } else {
                sin.get_stereo()
            }
        })
    };

    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_data(data, channels, &mut next_value_sin);
        },
        err_fn,
        None,
    )?;
    stream.play()?;

    let handle = thread::current();

    ctrlc::set_handler({
        let handle = handle.clone();
        move || {
            println!("Received Ctrl-C, exiting...");
            handle.unpark();
        }
    })
    .expect("Error setting Ctrl-C handler");

    thread::spawn(move || loop {
        let event = crossterm::event::read().unwrap();

        let Event::Key(event) = event else {
            continue;
        };

        if event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('c') {
            handle.unpark();
            return;
        }

        match event.code {
            KeyCode::Char(' ') => {}
            KeyCode::Esc => {}
            _ => continue,
        }

        handle.unpark();
        return;
    });

    thread::park();

    disable_raw_mode().unwrap();

    Ok(())
}

fn write_data<T>(output: &mut [T], channels: usize, next_sample: &mut dyn FnMut() -> (f64, f64))
where
    T: SizedSample + FromSample<f64>,
{
    for frame in output.chunks_mut(channels) {
        let (left, right) = next_sample();

        let left = T::from_sample(left);
        let right: T = T::from_sample(right);

        for (channel, sample) in frame.iter_mut().enumerate() {
            if channel & 0b1 == 0 {
                *sample = left;
            } else {
                *sample = right;
            }
        }
    }
}
