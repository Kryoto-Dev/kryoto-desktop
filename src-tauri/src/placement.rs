//! Putting a child web view (the Store, a menu) over the shell.
//!
//! The shell measures where the child goes in its own CSS pixels. Handing those
//! to Tauri as *logical* pixels converts them with the window's scale factor,
//! and on Windows that factor and the shell page's own `devicePixelRatio` can
//! disagree: 125% display scaling, a window dragged to a monitor with another
//! scale, a scale changed while the app was open. The Store was then drawn at
//! 1/1.25 of its slot in the top-left corner, with black where the rest of the
//! page should be. Converting with the page's own ratio, to physical pixels,
//! puts it exactly where the shell measured, whatever Windows reports.
//!
//! Linux's native bounds calls are ineffective for these views. Its overlay
//! converts CSS pixels to GTK logical pixels using DPR / GTK's widget scale
//! (see linux_overlay.rs), including fractional X11 display scaling. Measure
//! the shell's physical width per CSS viewport width on Linux: WebKit versions
//! disagree on whether devicePixelRatio includes the text-size zoom.

use tauri::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, Position, Size};

/// The shell's `devicePixelRatio`, when it sent a sane one.
pub fn css_scale(scale: Option<f64>) -> Option<f64> {
    scale.filter(|s| s.is_finite() && (0.25..=8.0).contains(s))
}

/// Physical pixels per CSS pixel, including both display scaling and text zoom.
pub fn viewport_scale(physical_width: u32, css_width: Option<f64>) -> Option<f64> {
    let width = css_width.filter(|w| w.is_finite() && *w > 0.0)?;
    css_scale(Some(f64::from(physical_width) / width))
}

/// Keep Windows' page DPR; on Linux measure the zoomed shell's actual scale.
pub fn shell_scale(window: &tauri::Window, scale: Option<f64>, css_width: Option<f64>) -> Result<Option<f64>, String> {
    if cfg!(target_os = "linux") {
        let width = window.inner_size().map_err(|e| e.to_string())?.width;
        Ok(viewport_scale(width, css_width))
    } else {
        Ok(css_scale(scale))
    }
}

/// Where and how big, in the units this platform should use.
pub fn rect(x: f64, y: f64, width: f64, height: f64, scale: Option<f64>) -> (Position, Size) {
    let (width, height) = (width.max(1.0), height.max(1.0));
    match css_scale(scale) {
        Some(s) if cfg!(not(target_os = "linux")) => (
            PhysicalPosition::new((x * s).round() as i32, (y * s).round() as i32).into(),
            PhysicalSize::new((width * s).round().max(1.0) as u32, (height * s).round().max(1.0) as u32).into(),
        ),
        _ => {
            let z = crate::text_size::css_to_logical();
            (LogicalPosition::new(x * z, y * z).into(), LogicalSize::new(width * z, height * z).into())
        }
    }
}

/// Move and resize a child web view that already exists.
pub fn place<R: tauri::Runtime>(
    view: &tauri::Webview<R>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: Option<f64>,
) -> Result<(), String> {
    let (position, size) = rect(x, y, width, height, scale);
    view.set_position(position).map_err(|e| e.to_string())?;
    view.set_size(size).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_nonsense_scales() {
        assert_eq!(css_scale(Some(1.25)), Some(1.25));
        assert_eq!(css_scale(Some(0.0)), None);
        assert_eq!(css_scale(Some(f64::NAN)), None);
        assert_eq!(css_scale(None), None);
    }

    #[test]
    fn viewport_measurement_combines_fractional_display_scaling_and_text_zoom() {
        assert_eq!(viewport_scale(1500, Some(1000.0)), Some(1.5));
        assert_eq!(viewport_scale(1500, Some(800.0)), Some(1.875));
        assert_eq!(viewport_scale(1800, Some(1000.0)), Some(1.8));
        assert_eq!(viewport_scale(2000, Some(800.0)), Some(2.5));
        for width in [None, Some(0.0), Some(-1.0), Some(f64::NAN), Some(f64::INFINITY)] {
            assert_eq!(viewport_scale(1500, width), None);
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn converts_css_pixels_with_the_page_ratio() {
        let (p, s) = rect(0.0, 90.0, 1536.0, 736.0, Some(1.25));
        assert_eq!(p, Position::Physical(PhysicalPosition::new(0, 113)));
        assert_eq!(s, Size::Physical(PhysicalSize::new(1920, 920)));
    }
}
