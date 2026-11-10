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
//! Linux keeps logical pixels: GTK allocates in them (see linux_overlay.rs),
//! and a CSS pixel is as many of them as the text size's zoom (text_size.rs).

use tauri::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, Position, Size};

/// The shell's `devicePixelRatio`, when it sent a sane one.
pub fn css_scale(scale: Option<f64>) -> Option<f64> {
    scale.filter(|s| s.is_finite() && (0.25..=8.0).contains(s))
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

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn converts_css_pixels_with_the_page_ratio() {
        let (p, s) = rect(0.0, 90.0, 1536.0, 736.0, Some(1.25));
        assert_eq!(p, Position::Physical(PhysicalPosition::new(0, 113)));
        assert_eq!(s, Size::Physical(PhysicalSize::new(1920, 920)));
    }
}
