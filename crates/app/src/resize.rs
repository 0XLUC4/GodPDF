//! Side panels dragged wider or narrower by their inner edge.

use gpui::prelude::*;
use gpui::{App, Bounds, Div, DragMoveEvent, EmptyView, Pixels, Stateful, Window, div, px};

/// Which edge is being dragged: the left (tools) panel's right edge or the right panel's left.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
}

/// What a resize drag carries.
pub struct Resize(pub Edge);

/// A 6 px grab strip over the panel's separator; put it in a `relative` panel.
pub fn handle(edge: Edge) -> Stateful<Div> {
    let strip = div()
        .id(match edge {
            Edge::Left => "resize-left-panel",
            Edge::Right => "resize-right-panel",
        })
        .absolute()
        .top_0()
        .h_full()
        .w(px(6.))
        .cursor_col_resize()
        .on_drag(Resize(edge), |_, _, _, cx| cx.new(|_| EmptyView));
    match edge {
        Edge::Left => strip.right(px(-3.)),
        Edge::Right => strip.left(px(-3.)),
    }
}

/// The panel width for the pointer at `x`, the panel at `bounds`, within `range`.
pub fn width(edge: Edge, bounds: Bounds<Pixels>, x: Pixels, range: (f32, f32)) -> f32 {
    let wanted = match edge {
        Edge::Left => f32::from(x - bounds.left()),
        Edge::Right => f32::from(bounds.right() - x),
    };
    wanted.clamp(range.0, range.1)
}

/// Run `set(width)` while `edge` is dragged over the panel that has this listener.
pub fn on_move(
    edge: Edge,
    range: (f32, f32),
    set: impl Fn(f32, &mut App) + 'static,
) -> impl Fn(&DragMoveEvent<Resize>, &mut Window, &mut App) + 'static {
    move |event, window, cx| {
        if event.drag(cx).0 == edge {
            set(width(edge, event.bounds, event.event.position.x, range), cx);
            window.refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};

    #[test]
    fn widths_follow_the_pointer_within_limits() {
        let bounds = Bounds::new(point(px(100.), px(0.)), size(px(300.), px(500.)));
        assert_eq!(width(Edge::Left, bounds, px(400.), (220., 420.)), 300.);
        assert_eq!(width(Edge::Left, bounds, px(900.), (220., 420.)), 420.);
        assert_eq!(width(Edge::Right, bounds, px(150.), (240., 520.)), 250.);
        assert_eq!(width(Edge::Right, bounds, px(390.), (240., 520.)), 240.);
    }
}
