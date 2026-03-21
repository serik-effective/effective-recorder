use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use crossbeam_channel::Sender;
use log::{error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[allow(dead_code)]
pub struct AudioSamples {
    pub data: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
    pub timestamp_us: i64,
}

pub struct AudioCapture {
    device: Device,
    config: StreamConfig,
    sample_format: SampleFormat,
}

impl AudioCapture {
    pub fn new(device_name: Option<&str>) -> Result<Self> {
        let host = cpal::default_host();

        let device = if let Some(name) = device_name {
            // If "default" is passed or the name matches the default device, use default
            if name == "__default__" {
                host.default_input_device()
                    .context("No default input device available")?
            } else {
                host.input_devices()
                    .context("Failed to enumerate input devices")?
                    .find(|d| d.name().map(|n| n == name).unwrap_or(false))
                    .unwrap_or_else(|| {
                        warn!("Device '{}' not found, using default", name);
                        host.default_input_device().expect("No default input device")
                    })
            }
        } else {
            host.default_input_device()
                .context("No default input device available")?
        };

        let dev_name = device.name().unwrap_or_else(|_| "unknown".into());
        info!("Using audio device: {}", dev_name);

        let supported_config = device
            .default_input_config()
            .context("Failed to get default input config")?;

        let sample_format = supported_config.sample_format();
        let mut config: StreamConfig = supported_config.into();

        // Use a larger buffer to reduce crackling
        // Request a buffer size that's a multiple of common audio frame sizes
        config.buffer_size = cpal::BufferSize::Fixed(4096);

        info!(
            "Audio config: {} Hz, {} channels, format: {:?}, buffer: {:?}",
            config.sample_rate.0, config.channels, sample_format, config.buffer_size
        );

        Ok(Self {
            device,
            config,
            sample_format,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate.0
    }

    pub fn channels(&self) -> u16 {
        self.config.channels
    }

    pub fn start(
        &self,
        is_recording: Arc<AtomicBool>,
        sender: Sender<AudioSamples>,
        _start_time: Instant,
    ) -> Result<Stream> {
        let channels = self.config.channels;
        let sample_rate = self.config.sample_rate.0;
        let is_rec = is_recording.clone();

        let err_fn = |err: cpal::StreamError| {
            error!("Audio stream error: {}", err);
        };

        let stream = self.build_stream(channels, sample_rate, is_rec, sender, err_fn)
            .map_err(|e| anyhow::anyhow!("Failed to build audio stream: {}", e))?;

        stream.play().context("Failed to start audio stream")?;
        info!("Audio capture started");

        Ok(stream)
    }

    fn build_stream(
        &self,
        channels: u16,
        sample_rate: u32,
        is_rec: Arc<AtomicBool>,
        sender: Sender<AudioSamples>,
        err_fn: impl Fn(cpal::StreamError) + Send + 'static,
    ) -> Result<Stream, cpal::BuildStreamError> {
        match self.sample_format {
            SampleFormat::F32 => self.device.build_input_stream(
                &self.config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if !is_rec.load(Ordering::Relaxed) {
                        return;
                    }
                    let timestamp_us = Instant::now().elapsed().as_micros() as i64;
                    let samples = AudioSamples {
                        data: data.to_vec(),
                        channels,
                        sample_rate,
                        timestamp_us,
                    };
                    let _ = sender.try_send(samples);
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => {
                let is_rec2 = is_rec.clone();
                self.device.build_input_stream(
                    &self.config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if !is_rec2.load(Ordering::Relaxed) {
                            return;
                        }
                        let timestamp_us = Instant::now().elapsed().as_micros() as i64;
                        let float_data: Vec<f32> =
                            data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                        let samples = AudioSamples {
                            data: float_data,
                            channels,
                            sample_rate,
                            timestamp_us,
                        };
                        let _ = sender.try_send(samples);
                    },
                    err_fn,
                    None,
                )
            }
            SampleFormat::U16 => {
                let is_rec2 = is_rec.clone();
                self.device.build_input_stream(
                    &self.config,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        if !is_rec2.load(Ordering::Relaxed) {
                            return;
                        }
                        let timestamp_us = Instant::now().elapsed().as_micros() as i64;
                        let float_data: Vec<f32> = data
                            .iter()
                            .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                            .collect();
                        let samples = AudioSamples {
                            data: float_data,
                            channels,
                            sample_rate,
                            timestamp_us,
                        };
                        let _ = sender.try_send(samples);
                    },
                    err_fn,
                    None,
                )
            }
            _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
        }
    }

    /// List audio devices. Returns default device first with a marker.
    pub fn list_devices() -> Result<Vec<String>> {
        let host = cpal::default_host();
        let default_name = host
            .default_input_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_default();

        let mut devices: Vec<String> = Vec::new();

        // Add default device first
        if !default_name.is_empty() {
            devices.push(default_name.clone());
        }

        // Add other devices
        if let Ok(input_devices) = host.input_devices() {
            for d in input_devices {
                if let Ok(name) = d.name() {
                    if name != default_name && !devices.contains(&name) {
                        devices.push(name);
                    }
                }
            }
        }

        Ok(devices)
    }
}
