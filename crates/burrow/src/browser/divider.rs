//! The divider between the two tabs of a split view (D-038). Dragging it moves the split;
//! double-clicking shares the width equally. CEF's native views have no mouse-move events, so
//! the divider is a tiny page (`ui/divider.html`), like the sidebar's own edge (D-033); it talks
//! to Rust the way the sidebar does.

use burrow_core::{model::clamp_split_ratio, Command};
use cef::*;

use super::{argb, cef, mix_with_white, Burrow};
use crate::{clients, consts, paths, views};

/// Flex weights are integers: the split's ratio in thousandths.
const FLEX_SCALE: f32 = 1000.0;

pub(super) struct Divider {
    view: BrowserView,
    /// The ratio when the current drag started.
    drag_from: Option<f32>,
}

fn css_color(c: u32) -> String {
    format!("#{:06x}", c & 0x00FF_FFFF)
}

impl Burrow {
    /// The gap color between the tabs: the content card's own tint.
    fn divider_color(&self) -> u32 {
        mix_with_white(argb(&self.state.active_space().color), 0.75)
    }

    fn ensure_divider(&mut self) -> Option<()> {
        if self.divider.is_some() {
            return Some(());
        }
        let mut client = clients::UiClient::new();
        let mut delegate = views::DividerViewDelegate::new();
        let view = browser_view_create(
            Some(&mut client),
            Some(&cef(&paths::ui_url("divider.html"))),
            Some(&BrowserSettings { background_color: self.divider_color(), ..Default::default() }),
            None,
            None,
            Some(&mut delegate),
        )?;
        view.set_visible(0);
        self.content.add_child_view(Some(&mut View::from(&view)));
        self.divider = Some(Divider { view, drag_from: None });
        Some(())
    }

    /// Orders and sizes the content area's children: one tab fills it; a split puts the divider
    /// between its two tabs and shares the width by the split's ratio.
    pub(super) fn layout_split(&mut self) {
        if self.visible.len() == 2 {
            self.ensure_divider();
        }
        let Some(layout) = self.content.get_layout().and_then(|l| l.as_box_layout()) else { return };
        let ratio = self.state.active_space().split.as_ref().map_or(0.5, |s| s.ratio);
        let left = (clamp_split_ratio(ratio) * FLEX_SCALE).round() as i32;
        let views: Vec<BrowserView> = self.visible.iter().filter_map(|id| self.tabs.get(id)).map(|t| t.view.clone()).collect();
        match (views.as_slice(), &self.divider) {
            ([a, b], Some(d)) => {
                self.content.reorder_child_view(Some(&mut View::from(a)), 0);
                self.content.reorder_child_view(Some(&mut View::from(&d.view)), 1);
                self.content.reorder_child_view(Some(&mut View::from(b)), 2);
                layout.set_flex_for_view(Some(&mut View::from(a)), left);
                layout.set_flex_for_view(Some(&mut View::from(b)), FLEX_SCALE as i32 - left);
                layout.set_flex_for_view(Some(&mut View::from(&d.view)), 0);
                d.view.set_visible(1);
            }
            _ => {
                for v in &views {
                    layout.set_flex_for_view(Some(&mut View::from(v)), 1);
                }
                if let Some(d) = &self.divider {
                    d.view.set_visible(0);
                }
            }
        }
        // New flex weights alone don't make the panel lay out again.
        View::from(&self.content).invalidate_layout();
    }

    /// Repaints the divider after the space (and so its color) changed.
    pub(super) fn paint_divider(&self) {
        let Some(frame) = self.divider.as_ref().and_then(|d| d.view.browser()).and_then(|b| b.main_frame()) else { return };
        let js = format!("document.documentElement.style.background = '{}'", css_color(self.divider_color()));
        frame.execute_java_script(Some(&cef(&js)), Some(&CefString::from(&frame.url())), 0);
    }

    /// The divider is being dragged `dx` pixels from where the drag started.
    pub(super) fn split_drag(&mut self, dx: f64, phase: &str) {
        let Some(split) = self.state.active_space().split.clone() else { return };
        let Some(d) = &mut self.divider else { return };
        if phase == "start" || d.drag_from.is_none() {
            d.drag_from = Some(split.ratio);
        }
        let from = d.drag_from.unwrap_or(split.ratio);
        let room = (View::from(&self.content).bounds().width - consts::CARD_MARGIN).max(1) as f32;
        let ratio = from + dx as f32 / room;
        if phase == "end" {
            d.drag_from = None;
        }
        self.set_split_ratio(ratio, phase == "end");
    }

    pub(super) fn set_split_ratio(&mut self, ratio: f32, save: bool) {
        if self.state.apply(Command::SetSplitRatio { space_id: None, ratio }).is_err() {
            return;
        }
        self.layout_split();
        self.content.layout();
        if save {
            self.changed();
            self.focus_content();
        }
    }

    /// The divider's browser, for shutdown (it closes with the window).
    pub(super) fn divider_browser(&self) -> Option<Browser> {
        self.divider.as_ref().and_then(|d| d.view.browser())
    }
}
