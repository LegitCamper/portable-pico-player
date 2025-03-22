use super::{DummyTimeSource, MAX_DIRS, MAX_FILES, MAX_VOLUMES, SD};
use audio_parser::AudioFile;
use core::mem;
use defmt::{error, info, panic, warn};
use embassy_futures::join::join;
use embassy_rp::{peripherals::PIO0, pio_programs::i2s::PioI2sOut};
use embassy_time::{Duration, Instant, Timer, with_timeout};

const BUFFER_SIZE: usize = 512;

pub async fn play_file<'a>(
    i2s: &mut PioI2sOut<'static, PIO0, 0>,
    audio_file: &mut AudioFile<'a, SD, DummyTimeSource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
) {
    // create two audio buffers (back and front) which will take turns being
    // filled with new audio data and being sent to the pio fifo using dma
    // *2 is buffer swapping not stereo
    let mut buf = [0u32; BUFFER_SIZE * 2];
    let (mut back_buffer, mut front_buffer) = buf.split_at_mut(BUFFER_SIZE);

    let sample_rate = audio_file.sample_rate;
    let bit_depth = audio_file.bit_depth;
    let channels = audio_file.num_channels;
    info!(
        "Audio info:  {}hz, {}bit, {} channels",
        sample_rate, bit_depth, channels
    );

    // Calculate the time needed to fill the buffer based on sample rate and buffer size
    let expected_fill_time =
        Duration::from_millis((BUFFER_SIZE * 1000) as u64 / sample_rate as u64);
    info!(
        "Expected time to fill audio buffer: {}ms",
        expected_fill_time.as_millis()
    );

    let gain = 5.0;

    fill_back(audio_file, &mut front_buffer, bit_depth, channels, gain).await;
    loop {
        let start = Instant::now();
        if audio_file.read >= audio_file.end {
            info!("Reached end of audio file");
            break;
        }

        // Read the next chunk of data into the back buffer asynchronously while sending front buffer.
        let back_buffer_fut = async {
            if let Err(_) = with_timeout(
                expected_fill_time,
                fill_back(audio_file, &mut back_buffer, bit_depth, channels, gain),
            )
            .await
            {
                info!("Filling with silence due to timeout.");
                // Fill with silence bc reading took too long
                back_buffer.fill(0);
            }
        };

        // Write the front buffer data to the i2s DMA while the back buffer is being filled.
        let dma_future = i2s.write(&mut front_buffer);

        // Execute the two tasks concurrently.
        join(back_buffer_fut, dma_future).await;

        // Synchronize the timing with the sample rate (e.g., 48kHz, 44.1kHz)
        // Calculate the time elapsed since starting this loop
        let elapsed = Instant::now().duration_since(start);

        // Adjust timing for any delays that have already occurred
        let delay_duration = if elapsed < expected_fill_time {
            expected_fill_time - elapsed
        } else {
            Duration::from_millis(0) // If we're behind, don't delay further
        };

        // Wait for the next buffer to be ready
        Timer::after(delay_duration).await;

        mem::swap(&mut back_buffer, &mut front_buffer);
    }
}

pub async fn fill_back(
    file_reader: &mut AudioFile<'_, SD, DummyTimeSource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    back_buffer: &mut [u32],
    bit_depth: u16,
    channels: u16,
    gain: f32,
) {
    let mut read_buf = [0u8; BUFFER_SIZE * 3]; // for 24 bit audio
    let mut read_slice = &mut read_buf[..BUFFER_SIZE * (bit_depth / 8) as usize];

    // read a frame of audio data from the sd card
    if let Err(e) = file_reader.read_exact(&mut read_slice).await {
        // Probably bc future was canceled
        error!("Failed to read next audio buffer: {}", e)
    }

    // decode if necisary
    // ...

    to_uniform_stereo_32(read_slice, back_buffer, bit_depth, channels);
    back_buffer.iter_mut().for_each(|sample| {
        let left = apply_gain((*sample >> 16) as u16, gain);
        let right = apply_gain(*sample as u16, gain);
        *sample = (left as u32) << 16 | right as u32;
    });
}

// converts any bit rate and channel into 32bit stereo audio
fn to_uniform_stereo_32(in_buf: &mut [u8], out_buf: &mut [u32], bit_depth: u16, channels: u16) {
    match bit_depth {
        8 => {
            if channels == 1 {
                out_buf
                    .iter_mut()
                    .zip(in_buf.as_ref())
                    .for_each(|(dma, read)| {
                        *dma = (*read as u32) << 16 | *read as u32;
                    });
            } else if channels == 2 {
                out_buf
                    .iter_mut()
                    .zip(in_buf.as_ref().chunks(2)) // get both L&R interleaved samples
                    .for_each(|(dma, read)| {
                        *dma = (read[0] as u32) << 16 | read[1] as u32;
                    });
            } else {
                panic!("unsupported number of channels")
            }
        }
        16 => {
            if channels == 1 {
                out_buf
                    .iter_mut()
                    .zip(in_buf.as_ref().chunks(2))
                    .for_each(|(dma, read)| {
                        let read = (read[0] as u16) << 8 | (read[1] as u16); // convert 2 bytes to 16bit
                        *dma = (read as u32) << 16 | read as u32;
                    });
            } else if channels == 2 {
                out_buf
                    .iter_mut()
                    .zip(in_buf.as_ref().chunks(4)) // get both L&R interleaved samples
                    .for_each(|(dma, read)| {
                        let l_read = (read[0] as u16) << 8 | (read[1] as u16); // convert 2 bytes to 16bit - Left
                        let r_read = (read[2] as u16) << 8 | (read[3] as u16); // convert 2 bytes to 16bit - Right
                        *dma = (l_read as u32) << 16 | r_read as u32;
                    });
            } else {
                panic!("unsupported number of channels")
            }
        }
        _ => {
            panic!("unsupported bit depth")
        }
    }
}

fn apply_gain(sample: u16, gain: f32) -> u16 {
    // Scale the sample by the gain value
    let scaled_sample = sample as f32 * gain;

    // Convert the scaled sample back to an integer using rounding
    let rounded_sample = (scaled_sample + 0.5) as u16;

    return rounded_sample;
}
