use crate::persistence::AppState;
use gpui::{px, size, App, Bounds, Pixels, Size, Window, WindowBounds};

pub fn default_bounds(cx: &App, state: &AppState) -> Bounds<Pixels> {
    let w = state.window_size.0.clamp(900.0, 3840.0);
    let h = state.window_size.1.clamp(600.0, 2160.0);
    Bounds::centered(None, size(px(w), px(h)), cx)
}

pub fn min_window_size() -> Size<Pixels> {
    size(px(900.), px(600.))
}

pub fn window_bounds(bounds: Bounds<Pixels>) -> WindowBounds {
    WindowBounds::Windowed(bounds)
}

#[allow(dead_code)]
pub fn remember_size(state: &mut AppState, window: &Window) {
    let bounds = window.bounds();
    state.window_size = (
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    );
}
