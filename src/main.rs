//! Make some noise
#![allow(clippy::precedence)]

extern crate core;

use std::{sync::Arc, thread};

use anyhow::bail;
use assert_no_alloc::assert_no_alloc;
use cpal::{
    FromSample,
    SizedSample, traits::{DeviceTrait, HostTrait, StreamTrait},
};
use fundsp::{
    hacker::{AudioNode, AudioUnit64, Net64, white},
    prelude::sine_hz,
};
use fundsp::hacker::{brown, pink};
use itertools::Itertools;
use logos::{Lexer, Logos};
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
        matches!((self, other), (Error::Default, Error::Default) | (Error::Err(_), Error::Err(_)))
    }
}

// impl from
impl<E: std::error::Error + Send + Sync + 'static> From<E> for Error {
    fn from(err: E) -> Self {
        Error::Err(Arc::new(err))
    }
}

type Result<T> = std::result::Result<T, Error>;

fn parse_int(lex: &Lexer<Token>) -> Result<f64> {
    let slice = lex.slice();
    let n: u64 = slice.parse()?;
    Ok(n as f64)
}

fn parse_float(lex: &Lexer<Token>) -> Result<f64> {
    let slice = lex.slice();
    let n: f64 = slice.parse()?;
    Ok(n)
}

fn main() {
    // input separated by spaces
    let input = std::env::args().skip(1).join(" ");
    let lex = Token::lexer(&input);
    let tokens: Vec<_> = lex.try_collect().unwrap();
    let ast = to_ast(tokens).unwrap();

    println!("{:?}", ast);

    let host = cpal::default_host();

    let device = host
        .default_output_device()
        .expect("Failed to find a default output device");
    let config = device.default_output_config().unwrap();

    match config.sample_format() {
        cpal::SampleFormat::F32 => run::<f32>(&device, &config.into(), ast).unwrap(),
        cpal::SampleFormat::I16 => run::<i16>(&device, &config.into(), ast).unwrap(),
        cpal::SampleFormat::U16 => run::<u16>(&device, &config.into(), ast).unwrap(),
        _ => panic!("Unsupported format"),
    }
}

#[derive(Logos, Debug, PartialEq, Copy, Clone)]
#[logos(skip r"[ \t\n\f]+")] // Ignore this regex pattern between tokens
#[logos(error = Error)]
enum Token {
    // Tokens can be literal strings, of any length.
    #[token("sin")]
    Sin,

    // Tokens can be literal strings, of any length.
    #[token("white")]
    White,

    #[token("brown")]
    Brown,

    #[token("pink")]
    Pink,

    #[token("|")]
    Pipe,


    #[regex("[0-9]+", parse_int)]
    #[regex("[0-9]+\\.[0-9]*", parse_float)]
    Number(f64),
}

fn to_ast(tokens: impl IntoIterator<Item = Token>) -> anyhow::Result<Vec<Vec<Sound>>> {
    let tokens = tokens.into_iter();
    let mut peekable = tokens.peekable();

    let mut sounds = vec![Vec::new()];

    while let Some(on) = peekable.next() {
        let next = peekable.peek().copied();

        match on {
            Token::Sin => {
                let Some(Token::Number(num)) = next else {
                    bail!("need hz for sin");
                };
                sounds.last_mut().unwrap().push(Sound::Sin(num));

                peekable.next();
            }
            Token::Pipe => {
                sounds.push(vec![]);
            }
            Token::White => sounds.last_mut().unwrap().push(Sound::White),
            Token::Brown => sounds.last_mut().unwrap().push(Sound::Brown),
            Token::Pink => sounds.last_mut().unwrap().push(Sound::Pink),
            Token::Number(_) => {
                bail!("unused number");
            }
        }
    }

    Ok(sounds)
}

trait ValidSound: AudioNode + AudioUnit64 {}

impl<T> ValidSound for T where T: AudioNode + AudioUnit64 {}

fn run<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    ast: Vec<Vec<Sound>>,
) -> anyhow::Result<()>
where
    T: SizedSample + FromSample<f64>,
{
    let sample_rate = config.sample_rate.0 as f64;
    let channels = config.channels as usize;

    fn combine(sounds: Vec<Sound>) -> Net64 {
        sounds.into_iter()
            .map(|sound| {
                let sound: Box<dyn AudioUnit64> = match sound {
                    Sound::Sin(freq) => Box::new(sine_hz(freq)),
                    Sound::White => Box::new(white()),
                    Sound::Brown => Box::new(brown()),
                    Sound::Pink => Box::new(pink()),
                    Sound::None => Box::<Net64>::default(),
                };
                Net64::wrap(sound)
            })
            .reduce(|a, b| a + b)
            .unwrap_or_else(Net64::default)
    }

    // todo: edge case pipe and nothing on end

    let res = ast
        .into_iter()
        .map(combine)
        .reduce(|a,b| {
            a | b
        })
        .unwrap();


    println!("channels {channels}");

    let mut c = res;

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
