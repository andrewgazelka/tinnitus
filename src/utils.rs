use cpal::{FromSample, SizedSample};

pub fn write_data<T>(output: &mut [T], channels: usize, next_sample: &mut dyn FnMut() -> (f64, f64))
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
