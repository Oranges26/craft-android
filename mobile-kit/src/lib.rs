//! Shared pieces of the Craft Android ports.
//!
//! * [`platform`]: the Android bridge (file picker, gallery, insets, back button).
//! * [`theme`]: a dark, touch-sized egui style.
//! * [`widgets`]: touch sliders, chip rows, icon buttons, the bottom sheet.
//! * [`viewer`]: a pinch-zoom / pan image view.

pub mod icons;
pub mod platform;
pub mod theme;
pub mod viewer;
pub mod widgets;

pub use platform::{Event, PickKind};

/// Layout class of the current screen: phones in portrait put the tools under the picture, wide
/// screens (landscape phones, tablets) put them beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Portrait,
    Landscape,
}

impl Layout {
    pub fn of(rect: egui::Rect) -> Layout {
        if rect.width() > rect.height() * 1.15 && rect.width() > 560.0 { Layout::Landscape } else { Layout::Portrait }
    }
}
