//! About overlay: a modal card describing the app build and its dependencies.
//!
//! Rendered by `Root` on top of whichever screen is active, so the layer itself is absolutely
//! positioned over the full window. The card reuses the landing screen's look (white surface,
//! 5 px radius, `shadow_sm`, `#333`/`#666`/`#999` neutral text) instead of the reader theme: the
//! About dialog is app chrome, not book content.
//!
//! Interaction: a click on the dimmed backdrop closes the overlay; a click inside the card is
//! stopped (`stop_propagation`) so it never reaches the backdrop, and the × button in the card
//! header closes it explicitly.

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, FontWeight, Hsla, Image, ObjectFit, SharedString, Window, div, hsla,
    img, px, rgb,
};
use std::rc::Rc;
use std::sync::Arc;

/// Click handler shared by the backdrop and the × button. Collected in an `Rc` because both
/// consumers need the same `impl Fn`, which is not `Clone`.
type CloseHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App)>;

/// Width of the modal card.
const CARD_WIDTH: f32 = 600.0;
/// Height cap for the modal card (the package list scrolls inside it).
const CARD_MAX_HEIGHT: f32 = 520.0;

/// Build the About overlay.
///
/// `logo` is the already-decoded app icon (the landing screen owns the same `Arc<Image>`), and
/// `on_close` is invoked for both the backdrop click and the × button.
pub fn overlay(
    info: &crate::about::AboutInfo,
    logo: Arc<Image>,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    // `impl Fn` is not `Clone`, but both the backdrop and the × need the same handler.
    let on_close: CloseHandler = Rc::new(on_close);
    let backdrop_close = on_close.clone();

    div()
        .id("about-backdrop")
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .right(px(0.0))
        .bottom(px(0.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(backdrop_color())
        // Keep clicks and wheel events from reaching the screen underneath the modal: GPUI stops
        // dispatching to lower hitboxes only for `HitboxBehavior::BlockMouse`, set by `.occlude()`.
        .occlude()
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            backdrop_close(event, window, cx)
        })
        .child(card(info, logo, on_close))
        .into_any_element()
}

/// The white modal card centered by the backdrop.
fn card(
    info: &crate::about::AboutInfo,
    logo: Arc<Image>,
    on_close: CloseHandler,
) -> AnyElement {
    div()
        .id("about-card")
        .w(px(CARD_WIDTH))
        .max_h(px(CARD_MAX_HEIGHT))
        .p(px(24.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .bg(color(0xffffff))
        .rounded(px(5.0))
        .shadow_sm()
        // Clicks inside the card must not bubble to the backdrop and dismiss it.
        .on_click(|_: &ClickEvent, _: &mut Window, cx: &mut gpui::App| cx.stop_propagation())
        .child(header(info, logo, on_close))
        .child(description(info))
        .child(build_section(info))
        .child(divider())
        .child(packages_section(info))
        .child(footer(info.license))
        .into_any_element()
}

/// Logo + product name/version + the × close button.
fn header(
    info: &crate::about::AboutInfo,
    logo: Arc<Image>,
    on_close: CloseHandler,
) -> AnyElement {
    div()
        .w_full()
        .flex_shrink_0()
        .flex()
        .items_start()
        .gap(px(14.0))
        .child(
            div()
                .flex_shrink_0()
                .w(px(48.0))
                .h(px(48.0))
                .overflow_hidden()
                .rounded(px(3.0))
                .child(
                    img(logo)
                        .w(px(48.0))
                        .h(px(48.0))
                        .object_fit(ObjectFit::Contain),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(px(20.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(color(0x333333))
                        .text_ellipsis()
                        .child(SharedString::from(info.product_name.to_string())),
                )
                .child(
                    div()
                        .text_size(px(14.0))
                        .text_color(color(0x666666))
                        .child(SharedString::from(format!("Version v{}", info.version))),
                ),
        )
        .child(close_button(on_close))
        .into_any_element()
}

/// The × in the card's top-right corner.
fn close_button(
    on_close: CloseHandler,
) -> AnyElement {
    div()
        .id("about-close")
        .flex_shrink_0()
        .size(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .text_size(px(18.0))
        .text_color(color(0x888888))
        .cursor_pointer()
        .hover(|style| style.bg(color(0xeeeeee)))
        .on_click(move |event, window, cx| on_close(event, window, cx))
        .child("×")
        .into_any_element()
}

/// One-line product tagline under the header.
fn description(info: &crate::about::AboutInfo) -> AnyElement {
    div()
        .w_full()
        // Fixed rows must never be shrunk by the card's height cap; only the package list flexes.
        .flex_shrink_0()
        .text_size(px(14.0))
        .text_color(color(0x666666))
        .line_clamp(2)
        .child(SharedString::from(info.description.to_string()))
        .into_any_element()
}

/// "Build" section: Rust/GPUI/Target/Profile label-value rows.
fn build_section(info: &crate::about::AboutInfo) -> AnyElement {
    let gpui_version = info
        .gpui_version
        .clone()
        .map(|version| version.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let rows: Vec<AnyElement> = [
        ("Rust", info.rust_version.to_string()),
        ("GPUI", gpui_version),
        ("Target", info.target.to_string()),
        ("Profile", info.profile.to_string()),
    ]
    .into_iter()
    .map(|(label, value)| build_row(label, value))
    .collect();

    section("Build", rows)
}

/// A section heading followed by its content rows.
fn section(title: impl Into<SharedString>, children: Vec<AnyElement>) -> AnyElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .child(section_title(title))
        .children(children)
        .into_any_element()
}

fn section_title(title: impl Into<SharedString>) -> AnyElement {
    div()
        .flex_shrink_0()
        .pb(px(2.0))
        .text_size(px(13.0))
        .font_weight(FontWeight::BOLD)
        .text_color(color(0x999999))
        .child(title.into())
        .into_any_element()
}

/// A label (#999, left) / mono value (#333, right) row with a 1 px `#f1f1f1` bottom border.
fn build_row(label: &'static str, value: String) -> AnyElement {
    div()
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .py(px(5.0))
        .border_b_1()
        .border_color(color(0xf1f1f1))
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(13.0))
                .text_color(color(0x999999))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_right()
                .text_ellipsis()
                .text_size(px(13.0))
                .font_family(crate::theme::mono_font())
                .text_color(color(0x333333))
                .child(value),
        )
        .into_any_element()
}

/// "Packages (n)" section with a scrollable dependency list.
fn packages_section(info: &crate::about::AboutInfo) -> AnyElement {
    let mut list = div()
        .id("about-packages")
        .w_full()
        // This list is the only flexible row: when a long dependency list exceeds the card's
        // `CARD_MAX_HEIGHT`, the list shrinks (down to the fixed rows' floor) and scrolls instead
        // of pushing the footer out of the card.
        .flex_1()
        .min_h(px(0.0))
        .overflow_y_scroll()
        .flex()
        .flex_col();

    if info.packages.is_empty() {
        list = list.child(
            div()
                .py(px(6.0))
                .text_size(px(13.0))
                .text_color(color(0x999999))
                .child("No packages recorded"),
        );
    } else {
        list = list.children(
            info.packages
                .iter()
                .map(package_row),
        );
    }

    div()
        .w_full()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .child(section_title(format!("Packages ({})", info.packages.len())))
        .child(list)
        .into_any_element()
}

/// One dependency: mono package name (left) + version (#999, right).
fn package_row(package: &crate::about::Package) -> AnyElement {
    div()
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .py(px(4.0))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_ellipsis()
                .text_size(px(13.0))
                .font_family(crate::theme::mono_font())
                .text_color(color(0x333333))
                .child(SharedString::from(package.name.to_string())),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(13.0))
                .text_color(color(0x999999))
                .child(SharedString::from(package.version.to_string())),
        )
        .into_any_element()
}

/// Card footer: license on the left, build credit on the right.
///
/// The license label is derived from the package metadata (rather than hard-coded) so it cannot
/// drift from `Cargo.toml` if the license ever changes.
fn footer(license: &str) -> AnyElement {
    div()
        .w_full()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_between()
        .pt(px(2.0))
        .child(
            div()
                .text_size(px(12.0))
                .text_color(color(0x999999))
                .child(SharedString::from(format!("{license} License"))),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(color(0x999999))
                .child("Built with Rust + GPUI"),
        )
        .into_any_element()
}

/// Hairline separator between the Build and Packages sections.
fn divider() -> AnyElement {
    div()
        .w_full()
        .flex_shrink_0()
        .h(px(1.0))
        .bg(color(0xf1f1f1))
        .into_any_element()
}

/// `rgba(0, 0, 0, 0.35)` backdrop.
fn backdrop_color() -> Hsla {
    hsla(0.0, 0.0, 0.0, 0.35)
}

fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}
