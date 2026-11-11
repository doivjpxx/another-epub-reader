//! Zero-size anchor probe element used for in-chapter TOC jumps.
//!
//! One probe is emitted immediately before each block's element. During `prepaint` the probe
//! records the laid-out top of its block (content-space y in window pixels) into a shared map. The
//! reader then converts `positions[target] - positions[first]` into a scroll offset once the frame
//! has been laid out.
//!
//! Layout coordinates inside a scroll container are content-relative (the scroll offset is applied
//! at paint time), so the recorded y values do not change while scrolling and the difference is
//! exactly the pixel offset needed to bring the target block to the top of the viewer.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Style, Window,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Shared per-render map of `block index → laid-out top y` (window/content pixels).
pub type AnchorPositions = Rc<RefCell<HashMap<usize, f32>>>;

/// Create an empty positions map for one reader/chapter pass.
pub fn anchor_positions() -> AnchorPositions {
    Rc::new(RefCell::new(HashMap::new()))
}

/// Records its own laid-out y into [`AnchorPositions`] and paints nothing.
pub struct AnchorProbe {
    block: usize,
    sink: AnchorPositions,
}

impl AnchorProbe {
    pub fn new(block: usize, sink: &AnchorPositions) -> Self {
        Self {
            block,
            sink: sink.clone(),
        }
    }
}

impl IntoElement for AnchorProbe {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for AnchorProbe {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // A childless block with auto size lays out as a 0×0 zero-height row: it never affects
        // spacing and its `bounds.origin.y` is the top of the following block.
        let layout_id = window.request_layout(Style::default(), [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        self.sink
            .borrow_mut()
            .insert(self.block, f32::from(bounds.origin.y));
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
        // Intentionally paints nothing.
    }
}
