//! What the OS screens are drawn with: the palette and the shared numbers (`look`), the canvas the
//! mockup's drawing words are set on (`canvas`), and the painter that paints a screen whole from a
//! view and says which rows changed (`painter`).

pub mod canvas;
pub mod look;
pub mod painter;

pub use canvas::{Canvas, Face};
pub use painter::ScreenPainter;
