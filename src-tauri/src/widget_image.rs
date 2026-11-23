use image::{
    codecs::{gif::GifDecoder, jpeg::JpegDecoder, png::PngDecoder, webp::WebPDecoder},
    imageops::{self, FilterType},
    metadata::LoopCount,
    AnimationDecoder, ColorType, DynamicImage, ImageDecoder, ImageFormat, Limits, RgbaImage,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};
use tauri::{
    ipc::{InvokeBody, Request, Response},
    AppHandle, Manager, WebviewWindow,
};
use uuid::Uuid;

const MAX_IMPORT: usize = 24 * 1024 * 1024;
const MAX_ANIMATION_WORK: u64 = 240_000_000;
const MAX_CROP_FILE: usize = 64 * 1024 * 1024;
const TILE_HEIGHT: u32 = 192;

#[derive(Clone, Copy, Deserialize, Serialize)]
pub struct Crop {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    ratio: u32,
}

impl Crop {
    fn valid(self) -> bool {
        (1..=3).contains(&self.ratio)
            && self.x >= 0.0
            && self.y >= 0.0
            && self.width > 0.0
            && self.height > 0.0
            && self.x + self.width <= 1.0 + 1e-9
            && self.y + self.height <= 1.0 + 1e-9
    }

    fn render(self, image: &RgbaImage) -> RgbaImage {
        let (width, height) = image.dimensions();
        let x = ((self.x * f64::from(width)).round() as u32).min(width - 1);
        let y = ((self.y * f64::from(height)).round() as u32).min(height - 1);
        let w = ((self.width * f64::from(width)).round() as u32).clamp(1, width - x);
        let h = ((self.height * f64::from(height)).round() as u32).clamp(1, height - y);
        let output_height = h.min(w.div_ceil(self.ratio)).min(TILE_HEIGHT);
        imageops::resize(
            &imageops::crop_imm(image, x, y, w, h).to_image(),
            output_height * self.ratio,
            output_height,
            FilterType::Triangle,
        )
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ImageWidget {
    asset: Uuid,
    source: Uuid,
    name: String,
    crop: Crop,
}

impl ImageWidget {
    pub(crate) fn valid(&self) -> bool {
        self.crop.valid() && !self.name.is_empty() && self.name.chars().count() <= 255
    }
}

#[derive(Serialize)]
pub struct SavedImage {
    asset: Uuid,
    source: Uuid,
}

fn image_directory(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|root| root.join("widget-images"))
        .map_err(|error| error.to_string())
}

fn image_path(dir: &Path, id: Uuid, original: bool) -> PathBuf {
    dir.join(format!("{id}.{}", if original { "source" } else { "png" }))
}

fn read_image(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    if fs::metadata(path)
        .map_err(|_| "The saved image could not be found.")?
        .len()
        > limit as u64
    {
        return Err("The saved image is too large.".into());
    }
    fs::read(path).map_err(|error| format!("Couldn’t read the saved image. {error}"))
}

fn write_image(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let save = || -> std::io::Result<()> {
        let dir = path.parent().expect("widget image directory");
        fs::create_dir_all(dir)?;
        let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
        temporary.write_all(bytes)?;
        temporary.persist_noclobber(path)?;
        Ok(())
    };
    save().map_err(|error| format!("Couldn’t save the image. {error}"))
}

fn decode_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits
}

fn check_dimensions(decoder: &mut impl ImageDecoder) -> Result<(), String> {
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 32_000_000 {
        return Err("The image is too large. Choose an image under 32 megapixels.".into());
    }
    decoder.set_limits(decode_limits()).map_err(image_error)
}

fn image_error(error: impl std::fmt::Display) -> String {
    format!("Couldn’t process this image. {error}")
}

fn static_crop(mut decoder: impl ImageDecoder, crop: Crop) -> Result<Vec<u8>, String> {
    check_dimensions(&mut decoder)?;
    let orientation = decoder.orientation().map_err(image_error)?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(image_error)?;
    image.apply_orientation(orientation);
    encode_png(vec![(crop.render(&image.into_rgba8()), (0, 1))], 0)
}

fn animated_crop<'a>(
    decoder: impl AnimationDecoder<'a>,
    crop: Crop,
    plays: u32,
) -> Result<Vec<u8>, String> {
    let mut frames = Vec::new();
    let mut decoded_pixels = 0;
    let mut crop_bytes = 0;
    for frame in decoder.into_frames() {
        let frame = frame.map_err(image_error)?;
        let original = frame.buffer();
        decoded_pixels += u64::from(original.width()) * u64::from(original.height());
        if decoded_pixels > MAX_ANIMATION_WORK || frames.len() >= 600 {
            return Err("This animation is too large. Choose a shorter or smaller file.".into());
        }
        let (numer, denom) = frame.delay().numer_denom_ms();
        let image = crop.render(original);
        crop_bytes += image.len();
        if crop_bytes > 96 * 1024 * 1024 {
            return Err("This animation is too large. Zoom in or choose a shorter file.".into());
        }
        let millis = (f64::from(numer) / f64::from(denom)).round() as u32;
        let delay = if millis <= u32::from(u16::MAX) {
            (millis as u16, 1000)
        } else {
            ((millis / 100).min(u32::from(u16::MAX)) as u16, 10)
        };
        frames.push((image, delay));
    }
    encode_png(frames, plays)
}

fn encode_png(frames: Vec<(RgbaImage, (u16, u16))>, plays: u32) -> Result<Vec<u8>, String> {
    let first = frames.first().ok_or("The image has no frames.")?;
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, first.0.width(), first.0.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_compression(png::Compression::Fast);
    let animated = frames.len() > 1;
    if animated {
        encoder
            .set_animated(frames.len() as u32, plays)
            .map_err(image_error)?;
    }
    let mut writer = encoder.write_header().map_err(image_error)?;
    for (frame, (numer, denom)) in frames {
        if animated {
            writer.set_frame_delay(numer, denom).map_err(image_error)?;
        }
        writer
            .write_image_data(frame.as_raw())
            .map_err(image_error)?;
    }
    writer.finish().map_err(image_error)?;
    if bytes.len() > MAX_CROP_FILE {
        return Err("The cropped animation is too large to save.".into());
    }
    Ok(bytes)
}

fn loop_count(count: LoopCount) -> u32 {
    match count {
        LoopCount::Finite(plays) => plays.get(),
        LoopCount::Infinite => 0,
    }
}

fn crop_image(bytes: &[u8], crop: Crop) -> Result<Vec<u8>, String> {
    let input = Cursor::new(bytes);
    match image::guess_format(bytes) {
        Ok(ImageFormat::Gif) => {
            let mut decoder = GifDecoder::new(input.clone()).map_err(image_error)?;
            check_dimensions(&mut decoder)?;
            let plays = match gif::Decoder::new(input).map_err(image_error)?.repeat() {
                gif::Repeat::Infinite => 0,
                gif::Repeat::Finite(repeats) => u32::from(repeats) + 1,
            };
            animated_crop(decoder, crop, plays)
        }
        Ok(ImageFormat::Png) => {
            let mut decoder =
                PngDecoder::with_limits(input, decode_limits()).map_err(image_error)?;
            if !decoder.is_apng().map_err(image_error)? {
                return static_crop(decoder, crop);
            }
            check_dimensions(&mut decoder)?;
            if matches!(
                decoder.color_type(),
                ColorType::L16 | ColorType::La16 | ColorType::Rgb16 | ColorType::Rgba16
            ) {
                return Err("Choose an 8-bit PNG animation.".into());
            }
            let animation = decoder.apng().map_err(image_error)?;
            let plays = loop_count(animation.loop_count());
            animated_crop(animation, crop, plays)
        }
        Ok(ImageFormat::WebP) => {
            let mut decoder = WebPDecoder::new(input).map_err(image_error)?;
            if !decoder.has_animation() {
                return static_crop(decoder, crop);
            }
            check_dimensions(&mut decoder)?;
            let plays = loop_count(decoder.loop_count());
            animated_crop(decoder, crop, plays)
        }
        Ok(ImageFormat::Jpeg) => static_crop(JpegDecoder::new(input).map_err(image_error)?, crop),
        _ => Err("Choose a PNG, JPEG, GIF, or WebP image.".into()),
    }
}

#[tauri::command]
pub async fn read_widget_image(
    app: AppHandle,
    webview: WebviewWindow,
    id: String,
    original: bool,
) -> Result<Response, String> {
    let label = webview.label();
    if (original && label != "image-editor")
        || (!original && !matches!(label, "settings" | "dock" | "folder" | "drag"))
    {
        return Err("Widget images are unavailable here.".into());
    }
    let id = Uuid::parse_str(&id).map_err(image_error)?;
    let path = image_path(&image_directory(&app)?, id, original);
    tauri::async_runtime::spawn_blocking(move || {
        read_image(&path, if original { MAX_IMPORT } else { MAX_CROP_FILE }).map(Response::new)
    })
    .await
    .map_err(image_error)?
}

#[tauri::command]
pub async fn save_widget_image(
    app: AppHandle,
    webview: WebviewWindow,
    request: Request<'_>,
) -> Result<SavedImage, String> {
    if webview.label() != "image-editor" {
        return Err("Open the image editor to save widget images.".into());
    }
    let headers = request.headers();
    let crop: Crop = headers
        .get("x-spacebar-crop")
        .and_then(|value| serde_json::from_slice(value.as_bytes()).ok())
        .filter(|crop: &Crop| crop.valid())
        .ok_or("Choose a valid crop.")?;
    let source = headers
        .get("x-spacebar-source")
        .map(|id| Uuid::try_parse_ascii(id.as_bytes()))
        .transpose()
        .map_err(image_error)?;
    let uploaded = match (source, request.body()) {
        (Some(_), _) => None,
        (None, InvokeBody::Raw(bytes)) if !bytes.is_empty() && bytes.len() <= MAX_IMPORT => {
            Some(bytes.clone())
        }
        (None, InvokeBody::Json(bytes))
            if bytes
                .as_array()
                .is_some_and(|bytes| !bytes.is_empty() && bytes.len() <= MAX_IMPORT) =>
        {
            // A WebView can fall back to postMessage, where binary input arrives as a JSON array.
            Some(Vec::<u8>::deserialize(bytes).map_err(image_error)?)
        }
        _ => return Err("Choose an image under 24 MB.".into()),
    };
    let dir = image_directory(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let source_id = source.unwrap_or_else(Uuid::new_v4);
        let original = image_path(&dir, source_id, true);
        let bytes = match uploaded {
            Some(bytes) => bytes,
            None => read_image(&original, MAX_IMPORT)?,
        };
        let cropped = crop_image(&bytes, crop)?;
        let asset = Uuid::new_v4();
        if source.is_none() {
            write_image(&original, &bytes)?;
        }
        let result = write_image(&image_path(&dir, asset, false), &cropped);
        if result.is_err() && source.is_none() {
            let _ = fs::remove_file(&original);
        }
        result?;
        Ok(SavedImage {
            asset,
            source: source_id,
        })
    })
    .await
    .map_err(image_error)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_crop_keeps_frame_timing_loop_count_and_composited_positions() {
        let mut gif = Vec::new();
        {
            let mut encoder = gif::Encoder::new(&mut gif, 6, 2, &[255, 0, 0, 0, 0, 255]).unwrap();
            encoder.set_repeat(gif::Repeat::Finite(2)).unwrap();
            encoder
                .write_frame(&gif::Frame {
                    width: 6,
                    height: 2,
                    delay: 8,
                    dispose: gif::DisposalMethod::Keep,
                    buffer: vec![0; 12].into(),
                    ..Default::default()
                })
                .unwrap();
            encoder
                .write_frame(&gif::Frame {
                    width: 2,
                    height: 2,
                    left: 4,
                    delay: 14,
                    buffer: vec![1; 4].into(),
                    ..Default::default()
                })
                .unwrap();
        }

        let cropped = crop_image(
            &gif,
            Crop {
                x: 1.0 / 3.0,
                y: 0.0,
                width: 2.0 / 3.0,
                height: 1.0,
                ratio: 2,
            },
        )
        .unwrap();
        let mut reader = png::Decoder::new(Cursor::new(cropped)).read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (4, 2));
        let animation = reader.info().animation_control.unwrap();
        assert_eq!((animation.num_frames, animation.num_plays), (2, 3));
        let mut pixels = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(reader.info().frame_control.unwrap().delay_num, 80);
        assert_eq!(&pixels[8..12], &[255, 0, 0, 255]);
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(reader.info().frame_control.unwrap().delay_num, 140);
        assert_eq!(&pixels[..4], &[255, 0, 0, 255]);
        assert_eq!(&pixels[8..12], &[0, 0, 255, 255]);
    }
}
