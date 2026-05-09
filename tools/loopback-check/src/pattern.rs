use crate::types::{AudioConfig, SampleWidth};

pub fn generate_pattern(config: &AudioConfig) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(config.payload_frames() * config.bytes_per_frame());
    for frame in 0..config.payload_frames() as u32 {
        let left = sample_value(frame, 0, config.sample_width);
        let right = sample_value(frame, 1, config.sample_width);
        push_sample(&mut bytes, left, config.sample_width);
        push_sample(&mut bytes, right, config.sample_width);
    }
    bytes
}

fn sample_value(frame: u32, channel: u32, width: SampleWidth) -> i32 {
    let mut x = frame
        .wrapping_mul(0x9e37_79b9)
        .wrapping_add(channel.wrapping_mul(0x7f4a_7c15))
        .wrapping_add(0x1357_2468);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;

    match width {
        SampleWidth::Bits16 => {
            let value = (x & 0xffff) as i32 - 0x8000;
            value.saturating_mul(3) / 4
        }
        SampleWidth::Bits24 => {
            let value = (x & 0x00ff_ffff) as i32 - 0x0080_0000;
            value.saturating_mul(3) / 4
        }
        SampleWidth::Bits32 => x as i32,
    }
}

fn push_sample(bytes: &mut Vec<u8>, sample: i32, width: SampleWidth) {
    match width {
        SampleWidth::Bits16 => bytes.extend_from_slice(&(sample as i16).to_le_bytes()),
        SampleWidth::Bits24 => {
            let sample = sample as u32;
            bytes.push(sample as u8);
            bytes.push((sample >> 8) as u8);
            bytes.push((sample >> 16) as u8);
        }
        SampleWidth::Bits32 => bytes.extend_from_slice(&sample.to_le_bytes()),
    }
}
