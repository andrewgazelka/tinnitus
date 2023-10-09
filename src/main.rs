//! Make some noise
#![allow(clippy::precedence)]

extern crate core;

use std::{sync::Arc, thread};

use anyhow::bail;
use assert_no_alloc::assert_no_alloc;
use clap::Parser;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, SizedSample,
};
use crossterm::{
    event::{Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use fundsp::hacker::{highpole_hz, lowpole_hz, white, AudioNode, AudioUnit64};
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

type Result<T> = std::result::Result<T, Error>;

#[derive(Parser)]
struct Args {
    frequency: f64,
    width: f64,
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

fn run<T>(device: &cpal::Device, config: &cpal::StreamConfig, args: Args) -> anyhow::Result<()>
where
    T: SizedSample + FromSample<f64>,
{
    let sample_rate = config.sample_rate.0 as f64;
    let channels = config.channels as usize;

    let hz = args.frequency;
    let width = args.width;
    let min = hz - width;
    let max = hz + width;

    let white_low = white() >> lowpole_hz(min);
    let white_high = white() >> highpole_hz(max);

    let mut res = white_low + white_high;
    // let mut res = res * 0.0001;

    res.set_sample_rate(sample_rate);
    res.allocate();

    let mut next_value = move || assert_no_alloc(|| res.get_stereo());

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
