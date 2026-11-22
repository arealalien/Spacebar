use base64::{engine::general_purpose::STANDARD, Engine};
use std::{mem::size_of, ptr::null_mut, slice};
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetDC, ReleaseDC,
        SelectObject, SetStretchBltMode, StretchBlt, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS, HALFTONE, SRCCOPY,
    },
};

pub(crate) fn capture(bounds: RECT) -> Option<String> {
    let source_width = bounds.right - bounds.left;
    let source_height = bounds.bottom - bounds.top;
    if source_width <= 0 || source_height <= 0 {
        return None;
    }
    // Only a small, softened image is passed to the Start window.
    let longest = source_width.max(source_height);
    let width = (source_width * 56 / longest).max(1);
    let height = (source_height * 56 / longest).max(1);
    let pixels = sample(bounds, width, height)?;
    let blurred = soften(&pixels, width as usize, height as usize);
    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .ok()?
        .write_image_data(&blurred)
        .ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(encoded)
    ))
}

fn sample(bounds: RECT, width: i32, height: i32) -> Option<Vec<u8>> {
    unsafe {
        let screen = GetDC(null_mut());
        if screen.is_null() {
            return None;
        }
        let canvas = CreateCompatibleDC(screen);
        let result = if canvas.is_null() {
            None
        } else {
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut address = null_mut();
            let bitmap =
                CreateDIBSection(canvas, &info, DIB_RGB_COLORS, &mut address, null_mut(), 0);
            let pixels = if bitmap.is_null() || address.is_null() {
                None
            } else {
                let previous = SelectObject(canvas, bitmap);
                SetStretchBltMode(canvas, HALFTONE);
                let copied = !previous.is_null()
                    && StretchBlt(
                        canvas,
                        0,
                        0,
                        width,
                        height,
                        screen,
                        bounds.left,
                        bounds.top,
                        bounds.right - bounds.left,
                        bounds.bottom - bounds.top,
                        SRCCOPY,
                    ) != 0;
                let pixels = copied.then(|| {
                    GdiFlush();
                    slice::from_raw_parts(address.cast::<u8>(), (width * height * 4) as usize)
                        .to_vec()
                });
                if !previous.is_null() {
                    SelectObject(canvas, previous);
                }
                pixels
            };
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            DeleteDC(canvas);
            pixels
        };
        ReleaseDC(null_mut(), screen);
        result
    }
}

fn soften(pixels: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut output = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let mut total = [0u32; 3];
            let mut count = 0;
            for row in y.saturating_sub(2)..=(y + 2).min(height - 1) {
                for column in x.saturating_sub(2)..=(x + 2).min(width - 1) {
                    let index = (row * width + column) * 4;
                    total[0] += u32::from(pixels[index + 2]);
                    total[1] += u32::from(pixels[index + 1]);
                    total[2] += u32::from(pixels[index]);
                    count += 1;
                }
            }
            output.extend(total.map(|channel| (channel / count) as u8));
        }
    }
    output
}
