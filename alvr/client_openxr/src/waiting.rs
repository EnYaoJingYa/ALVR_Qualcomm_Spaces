//! Native fallback surface: works even when the OpenXR loader or instance is unavailable.
use crate::StageFailure;
use ab_glyph::{Font, FontRef, ScaleFont, point};
use std::sync::Mutex;
static FAILURE: Mutex<Option<StageFailure>> = Mutex::new(None);

pub fn set_failure(error: StageFailure) {
    if let Ok(mut failure) = FAILURE.lock() {
        *failure = Some(error);
    }
}
pub fn clear_failure() {
    if let Ok(mut failure) = FAILURE.lock() {
        *failure = None;
    }
}
pub fn draw(app: &android_activity::AndroidApp) {
    if let Err(error) = draw_inner(app) {
        alvr_common::error!("event=waiting_surface_failed detail={error}");
    }
}
fn draw_inner(app: &android_activity::AndroidApp) -> Result<(), String> {
    let window = app.native_window().ok_or("Waiting for Android window")?;
    window
        .set_buffers_geometry(
            960,
            540,
            Some(ndk::hardware_buffer_format::HardwareBufferFormat::R8G8B8A8_UNORM),
        )
        .map_err(|e| e.to_string())?;
    let mut buffer = window.lock(None).map_err(|e| e.to_string())?;
    let (width, height, stride) = (buffer.width(), buffer.height(), buffer.stride());
    // RGBA8888 format was explicitly requested above. The guard owns the writable buffer.
    let pixels =
        unsafe { std::slice::from_raw_parts_mut(buffer.bits().cast::<u32>(), stride * height) };
    pixels.fill(0xff201810);
    let failure = FAILURE.lock().map_err(|e| e.to_string())?;
    let detail = failure
        .as_ref()
        .map(|e| format!("Stage: {}\nCode: {}", e.stage, e.code))
        .unwrap_or_else(|| "Streaming stopped".into());
    let message = format!(
        "ALVR - Waiting\n\n{detail}\n\nCheck the PC log for details.\nPress a button or tap to retry."
    );
    let font =
        FontRef::try_from_slice(include_bytes!("../../graphics/resources/Ubuntu-Medium.ttf"))
            .map_err(|e| e.to_string())?;
    let scaled = font.as_scaled(26.0);
    let mut cursor = point(32.0, 56.0);
    for ch in message.chars().take(600) {
        if ch == '\n' || cursor.x > width as f32 - 50.0 {
            cursor.x = 32.0;
            cursor.y += 36.0;
            if ch == '\n' {
                continue;
            }
        }
        let id = scaled.glyph_id(ch);
        let mut glyph = id.with_scale(26.0);
        glyph.position = cursor;
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|x, y, coverage| {
                let x = x as i32 + bounds.min.x as i32;
                let y = y as i32 + bounds.min.y as i32;
                if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                    let c = (coverage * 220.0) as u32;
                    pixels[y as usize * stride + x as usize] = 0xff000000 | c | c << 8 | c << 16;
                }
            });
        }
        cursor.x += scaled.h_advance(id);
    }
    Ok(())
}
