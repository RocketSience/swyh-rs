use crossbeam_channel::{unbounded, Receiver, Sender};
use flac_bound::{FlacEncoder, FlacEncoderConfig, WriteWrapper};
use std::{
    cell::{Ref, RefCell},
    io::{Cursor, Read, Seek, SeekFrom, Write},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc,
    },
};

use super::{
    i24::{I24Sample, I24},
    rwstream::ChannelStream,
};

#[derive(Clone)]
pub struct FlacWriter {
    flac_s: Sender<Vec<u8>>,
}

impl FlacWriter {
    pub fn new(s: Sender<Vec<u8>>) -> FlacWriter {
        FlacWriter { flac_s: s }
    }
}

impl Write for FlacWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.flac_s.send(buf.to_vec()) {
            Ok(()) => Ok(buf.len()),
            Err(e) => Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "FlacWriter channel send error",
            )),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct FlacChannel {
    f32_r: Receiver<Vec<f32>>,
    flac_r: Receiver<Vec<u8>>,
    active: Arc<AtomicBool>,
    writer: FlacWriter,
}

impl FlacChannel {
    pub fn new(r: Receiver<Vec<f32>>) -> FlacChannel {
        let (outs, outr): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = unbounded();
        FlacChannel {
            f32_r: r,
            flac_r: outr,
            writer: FlacWriter::new(outs),
            active: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn run(&self) {
        let l_active = self.active.clone();
        let l_samples = self.f32_r.clone();
        let mut l_writer = self.writer.clone();

        let _ = std::thread::Builder::new()
            .name("flac_encoder".into())
            .stack_size(4 * 1024 * 1024)
            .spawn(move || {
                let mut outw = WriteWrapper(&mut l_writer);
                let mut enc = FlacEncoder::new()
                    .unwrap()
                    .channels(2)
                    .bits_per_sample(24)
                    .compression_level(0)
                    .init_write(&mut outw)
                    .unwrap();
                while l_active.load(Relaxed) {
                    let f32_samples = l_samples.recv().unwrap();
                    let mut left_samples: Vec<i32> = Vec::with_capacity(f32_samples.len() / 2);
                    let mut right_samples: Vec<i32> = Vec::with_capacity(f32_samples.len() / 2);

                    left_samples = f32_samples
                        .iter()
                        .enumerate()
                        .filter(|&(i, _)| i % 2 == 0)
                        .map(|(i, s)| to_i32_sample(*s))
                        .collect::<Vec<i32>>();
                    right_samples = f32_samples
                        .iter()
                        .enumerate()
                        .filter(|&(i, _)| i % 2 == 1)
                        .map(|(i, s)| to_i32_sample(*s))
                        .collect::<Vec<i32>>();
                    enc.process(&[left_samples.as_slice(), right_samples.as_slice()]);
                }
            })
            .unwrap();
    }

    pub fn stop(&self) {
        self.active.store(true, Relaxed);
    }
}

fn to_i32_sample(mut f32_sample: f32) -> i32 {
    if f32_sample > 1.0 {
        f32_sample = 1.0;
    } else if f32_sample < -1.0 {
        f32_sample = -1.0;
    }
    if f32_sample >= 0.0 {
        ((f32_sample as f64 * i32::MAX as f64) + 0.5) as i32
    } else {
        ((-f32_sample as f64 * i32::MIN as f64) - 0.5) as i32
    }
}
