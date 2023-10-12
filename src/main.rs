extern crate core;

use std::{sync::Mutex, thread, time::SystemTime};

use assert_no_alloc::assert_no_alloc;
use clap::Parser;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, SizedSample,
};
use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use fundsp::hacker::{bandpass_hz, notch_hz, prelude::*, white};

use crate::utils::write_data;

mod utils;

#[derive(Parser)]
#[clap(version, author, about)]
struct Args {
    /// frequency of the tinnitus
    frequency: f64,

    /// the frequency radius to notch out
    #[clap(default_value = "50")]
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
        cpal::SampleFormat::F32 => run::<f32>(&device, &config.into(), &args).unwrap(),
        cpal::SampleFormat::I16 => run::<i16>(&device, &config.into(), &args).unwrap(),
        cpal::SampleFormat::U16 => run::<u16>(&device, &config.into(), &args).unwrap(),
        _ => panic!("Unsupported format"),
    }
}

const Q: f64 = 1000.0;

fn removed_audio(args: &Args) -> impl AudioUnit64 {
    white() >> bandpass_hz(args.frequency, Q)
}

fn main_audio(args: &Args) -> impl AudioUnit64 {
    white() >> notch_hz(args.frequency, Q)
}

static LOUDNESS: Mutex<f64> = Mutex::new(0.1);

fn run<T>(device: &cpal::Device, config: &cpal::StreamConfig, args: &Args) -> anyhow::Result<()>
where
    T: SizedSample + FromSample<f64>,
{
    let sample_rate = f64::from(config.sample_rate.0);
    let channels = config.channels as usize;

    let mut sin = removed_audio(args);
    sin.set_sample_rate(sample_rate);
    sin.allocate();

    let mut main = main_audio(args);
    main.set_sample_rate(sample_rate);
    main.allocate();

    let mut start = None;

    let mut next_value_sin = move || {
        assert_no_alloc(|| {
            let start = start.get_or_insert_with(SystemTime::now);
            let time_passed = start.elapsed().unwrap().as_millis();

            let (l, r) = match time_passed > 5000 {
                true => main.get_stereo(),
                false => sin.get_stereo(),
            };

            let loudness = *LOUDNESS.lock().unwrap();
            (l * loudness, r * loudness)
        })
    };

    let err_fn = |err| eprintln!("an error occurred on stream: {err}");

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
            handle.unpark();
        }
    })
    .expect("Error setting Ctrl-C handler");

    thread::spawn(move || loop {
        let event = crossterm::event::read().unwrap();

        match event {
            Event::Key(
                KeyEvent {
                    code: KeyCode::Esc | KeyCode::Char(' '),
                    ..
                }
                | KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                },
            ) => {
                handle.unpark();
                return;
            }
            Event::Key(KeyEvent {
                code: KeyCode::Up, ..
            }) => {
                // max out at 1
                let loudness = *LOUDNESS.lock().unwrap();

                *LOUDNESS.lock().unwrap() = (loudness * 2.0).min(1.0);
            }
            Event::Key(KeyEvent {
                code: KeyCode::Down,
                ..
            }) => {
                *LOUDNESS.lock().unwrap() *= 0.5;
            }
            _ => {}
        }
    });

    thread::park();

    disable_raw_mode().unwrap();

    Ok(())
}
