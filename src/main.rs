use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use image::GenericImageView;
use pixels::{Pixels, SurfaceTexture};
use std::{
    path::Path,
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering},
};
use winit::{
    dpi::LogicalSize,
    event::{Event, KeyEvent, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::WindowBuilder,
};

fn load_image(path: &str, target_width: usize, target_height: usize) -> Option<Vec<u8>> {
    let img = image::open(path).ok()?;
    let img = img.resize_exact(
        target_width as u32,
        target_height as u32,
        image::imageops::FilterType::Lanczos3,
    );

    let (img_w, img_h) = img.dimensions();
    let rgba = img.to_rgba8();

    // RGBAバッファを作成
    let mut buffer = vec![0u8; target_width * target_height * 4];

    for y in 0..img_h as usize {
        for x in 0..img_w as usize {
            let pixel = rgba.get_pixel(x as u32, y as u32);
            let idx = (y * target_width + x) * 4;
            buffer[idx] = pixel[0];
            buffer[idx + 1] = pixel[1];
            buffer[idx + 2] = pixel[2];
            buffer[idx + 3] = pixel[3];
        }
    }

    Some(buffer)
}

fn find_loopback_device() -> Option<cpal::Device> {
    let host = cpal::default_host();

    // 利用可能な入力デバイスを表示
    log::info!("Available input devices:");
    if let Ok(devices) = host.input_devices() {
        for (i, device) in devices.enumerate() {
            if let Ok(name) = device.name() {
                log::info!("  {}: {}", i, name);
            }
        }
    }

    // BlackHole, Soundflower, Loopback, eqMacなどを探す
    if let Ok(devices) = host.input_devices() {
        for device in devices {
            if let Ok(name) = device.name() {
                let name_lower = name.to_lowercase();
                if name_lower.contains("blackhole")
                    || name_lower.contains("soundflower")
                    || name_lower.contains("loopback")
                    || name_lower.contains("eqmac")
                    || name_lower.contains("multi-output")
                {
                    log::info!("Found loopback device: {}", name);
                    return Some(device);
                }
            }
        }
    }

    None
}

fn setup_audio_capture(current_index: Arc<AtomicUsize>, _image_count: usize) -> Result<()> {
    let host = cpal::default_host();

    // ループバックデバイスを探すか、デフォルトの入力デバイスを使用
    let device = find_loopback_device()
        .or_else(|| host.default_input_device())
        .context("No input device available")?;

    let config = device.default_input_config()?;
    log::debug!("Input config: {:?}", config);

    let threshold = 0.001f32; // 音量閾値
    let last_switch = Arc::new(std::sync::Mutex::new(std::time::Instant::now()));
    let _cooldown = std::time::Duration::from_millis(20);

    let stream = device.build_input_stream(
        &config.into(),
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            // RMS音量を計算
            let sum: f32 = data.iter().map(|&s| s * s).sum();
            let rms = (sum / data.len() as f32).sqrt();

            let mut last = last_switch.lock().unwrap();

            // シンプルなロジック：音があれば画像1、なければ画像0
            if rms > threshold {
                current_index.store(1, Ordering::Relaxed);
                *last = std::time::Instant::now();
                // println!("Audio triggered! RMS: {:.4}, switching to image {}", rms, 1);
            } else {
                current_index.store(0, Ordering::Relaxed);
                // println!("Audio triggered! RMS: {:.4}, switching to image {}", rms, 0);
            }
        },
        |err| eprintln!("Audio stream error: {}", err),
        None,
    )?;

    stream.play()?;
    println!("Audio capture started. Listening...");

    // ストリームを維持
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn main() -> Result<()> {
    unsafe {
        std::env::set_var("RUST_LOG", "debug");
    }
    env_logger::init();

    // 画像ファイルのパス
    let image_paths = vec!["image1.jpg", "image2.jpg"];

    // 画面サイズ（フルスクリーン用）
    let width = 1664;
    let height = 1080;

    // 画像を読み込み (Pixelsはu8のRGBAバッファを使用)
    let mut images: Vec<Vec<u8>> = Vec::new();
    for path in &image_paths {
        log::debug!("Loading image from {}...", path);
        if Path::new(path).exists() {
            if let Some(buffer) = load_image(path, width as usize, height as usize) {
                images.push(buffer);
                log::debug!("Loaded image successfully");
            }
        } else {
            log::debug!("Cannot found image at {}", path);
        }
    }

    if images.is_empty() {
        // デモ用のダミー画像を作成
        println!("No images found. Creating demo images...");
        let size = (width * height * 4) as usize;
        let mut red_buffer = vec![0u8; size];
        let mut blue_buffer = vec![0u8; size];
        for i in (0..size).step_by(4) {
            // Red
            red_buffer[i] = 0x88;
            red_buffer[i + 3] = 0xff;
            // Blue
            blue_buffer[i + 2] = 0x88;
            blue_buffer[i + 3] = 0xff;
        }
        images.push(red_buffer);
        images.push(blue_buffer);
    }

    // 現在の画像インデックス
    let current_index = Arc::new(AtomicUsize::new(0));
    let image_count = images.len();

    // オーディオキャプチャをセットアップ
    let current_index_clone = current_index.clone();

    // Note: Audio thread needs to live as long as the app
    let _audio_thread = std::thread::spawn(move || {
        if let Err(e) = setup_audio_capture(current_index_clone, image_count) {
            eprintln!("Audio capture error: {}", e);
        }
    });

    // Winit セットアップ
    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new()
        .with_title("Image Viewer - ESC to exit, F to toggle fullscreen")
        .with_inner_size(LogicalSize::new(width, height))
        .build(&event_loop)?;

    let mut pixels = {
        let window_size = window.inner_size();
        let surface_texture = SurfaceTexture::new(window_size.width, window_size.height, &window);
        Pixels::new(width, height, surface_texture)?
    };

    let mut is_fullscreen = false;

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Poll);

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => elwt.exit(),

            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                if let Err(err) = pixels.resize_surface(size.width, size.height) {
                    eprintln!("pixels.resize_surface failed: {}", err);
                    elwt.exit();
                }
                window.request_redraw();
            }

            Event::WindowEvent {
                event:
                    WindowEvent::KeyboardInput {
                        event:
                            KeyEvent {
                                physical_key: PhysicalKey::Code(keycode),
                                state: winit::event::ElementState::Pressed,
                                ..
                            },
                        ..
                    },
                ..
            } => match keycode {
                KeyCode::Escape => elwt.exit(),
                KeyCode::KeyF => {
                    is_fullscreen = !is_fullscreen;
                    window.set_fullscreen(if is_fullscreen {
                        Some(winit::window::Fullscreen::Borderless(None))
                    } else {
                        None
                    });
                }
                _ => {}
            },

            Event::WindowEvent {
                event: WindowEvent::RedrawRequested,
                ..
            } => {
                let idx = current_index.load(Ordering::Relaxed);
                let frame = pixels.frame_mut();

                // Copy current image to frame
                if idx < images.len() {
                    let image_data = &images[idx];
                    if frame.len() == image_data.len() {
                        frame.copy_from_slice(image_data);
                    }
                }

                if let Err(e) = pixels.render() {
                    eprintln!("pixels.render() failed: {}", e);
                    elwt.exit();
                }
            }

            Event::AboutToWait => {
                window.request_redraw();
            }
            _ => {}
        }
    })?;

    Ok(())
}
